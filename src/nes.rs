//! NES 模擬器核心 — 純硬體封裝。
//!
//! `NesConsole` 不包含任何平台相關的 I/O（無視窗、無鍵盤、無音訊輸出）。
//! 外部平台層負責注入 Cartridge（ROM 黑盒）、讀取 frame buffer、設定按鍵、拉取音訊。

use crate::apu;
use crate::bus::{BusInterface, MainBus, PpuSnapshot};
use crate::cartridge::Cartridge;
use crate::cpu::CPU;
use crate::joypad::{Button, Joypad};
use crate::ppu::PPU;
use crate::ram::Ram;
use crate::state::{Reader, Writer};
use std::fs;
use std::path::PathBuf;

/// 純硬體 NES 主機封裝。
/// 無平台 I/O 依賴；ROM 透過 Cartridge 黑盒注入。
pub struct NesConsole {
    cpu: CPU,
    bus: MainBus,
    /// PPU frame counter snapshot used to detect frame completion.
    last_ppu_frame: u64,
    /// `<rom>.sav` for battery-backed cartridges, so the platform layer can
    /// persist PRG RAM when the emulator exits.
    battery_path: Option<PathBuf>,
}

impl NesConsole {
    /// Build an NES console from a `Cartridge` (the black-box ROM input).
    pub fn from_cartridge(cart: Cartridge) -> Self {
        let ram = Ram::new();
        let mut ppu = PPU::new(cart.mirroring);

        // Load CHR-ROM into PPU (CHR-RAM mappers skip this — games write via $2007).
        cart.init_chr_ram(&mut ppu.chr_ram);
        ppu.chr_is_ram = cart.is_chr_ram();

        let joy1 = Joypad::new();
        let joy2 = Joypad::new();
        let apu = apu::APU::new(crate::constants::DEFAULT_SAMPLE_RATE);

        let battery_path = cart.battery_path().map(PathBuf::from);
        let mut bus = MainBus::new(cart.mapper, ram, ppu, joy1, joy2, apu);

        let mut cpu = CPU::new();
        cpu.reset(&mut bus);
        // The reset sequence advanced the machine by seven cycles; set the
        // frame counter up as if `$4017` was written 9-12 cycles before the
        // first instruction (blargg 09.reset_timing).
        bus.power_on_apu();

        let last_ppu_frame = bus.ppu_frame();

        NesConsole {
            cpu,
            bus,
            last_ppu_frame,
            battery_path,
        }
    }

    /// Perform a hardware reset (`/RESET`), as the front-panel button does.
    ///
    /// The PPU and APU registers go back to their power-on values and the CPU
    /// runs its 7-cycle reset sequence, but everything a battery or the old
    /// program could have left behind survives: the 2 KB of work RAM, PRG RAM,
    /// VRAM, palette RAM, OAM, CHR data and the write-only APU/mapper
    /// registers. A game can therefore detect a warm boot and reuse its RAM, a
    /// multicart can return to its menu, and a front-end hotkey can offer
    /// "reset" next to "power cycle".
    ///
    /// Not reset either: the frame/scanline counters (the PPU keeps
    /// free-running) or the audio buffer already queued for the host.
    pub fn reset(&mut self) {
        self.bus.reset_peripherals();
        self.cpu.reset(&mut self.bus);
        // Hardware replays the last `$4017` value and lets the counter run for
        // the same 9 clocks as power-up before the first instruction.
        self.bus.rearm_apu_frame_counter();
        // The PPU was not restarted, so move the frame baseline with it;
        // otherwise the next `step_frame` would return immediately.
        self.last_ppu_frame = self.bus.ppu_frame();
    }

    /// Write battery-backed PRG RAM back to the cartridge's `<rom>.sav`.
    ///
    /// Called by the platform layer when emulation ends; cartridges without
    /// battery RAM are a no-op.
    pub fn save_battery(&self) {
        let Some(path) = &self.battery_path else {
            return;
        };
        if let Some(ram) = self.bus.mapper.battery_ram() {
            let _ = fs::write(path, ram);
        }
    }

    /// Set a button state on joypad 1 (called by the platform input layer).
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        self.bus.joypad1_mut().set_button(button, pressed);
    }

    /// Copy the PPU frame buffer (256×240) into `buf`.
    pub fn read_frame_buffer(&self, buf: &mut [u32]) {
        self.bus.read_frame_buffer(buf);
    }

    /// Drain accumulated audio samples from APU (call once per frame).
    pub fn drain_audio_samples(&mut self) -> Vec<f32> {
        self.bus.drain_audio()
    }

    // ════════════════════════════════════════════════════
    // Core emulation
    // ════════════════════════════════════════════════════

    /// Execute one CPU instruction and advance PPU/APU.
    /// The caller is responsible for handling frame boundaries.
    ///
    /// Every bus access advances the machine by one CPU cycle, so the PPU is in
    /// the correct phase when the CPU touches its registers. Interrupts are
    /// dispatched at instruction boundaries; the PPU drives the NMI line as a
    /// level and the CPU edge-detects it.
    pub fn step(&mut self) {
        // ── 1. Execute CPU instruction ──
        self.cpu.step(&mut self.bus);

        // ── 2. OAM DMA requested by $4014 runs right after the instruction.
        //       The CPU is halted for 513/514 cycles while PPU and APU keep
        //       advancing. The interrupt poll for the instruction that wrote
        //       $4014 happens before the transfer: an IRQ already pending there
        //       is serviced as soon as the DMA finishes (return address is the
        //       instruction after the write), while an IRQ that only shows up
        //       during the DMA waits for one more instruction. cpu_interrupts
        //       4-irq_and_dma pins exactly that split. ──
        let delay = self.cpu.poll_delay;
        let irq_before_dma = self.bus.irq_asserted_before_tick(delay);
        if let Some(page) = self.bus.take_dma_request() {
            let _ = self.bus.run_dma(page);
        }

        // ── 3. Interrupt dispatch (NMI first: it is not maskable) ──
        // The 6502 polls the interrupt lines on the cycle before its last one
        // (a taken branch without a page cross is one cycle earlier again,
        // see `CPU::poll_delay`).
        let irq_sample = irq_before_dma;

        // NMI is edge-triggered: the bus latches the rising edge on the PPU's
        // NMI line, and the poll consumes it. A $2002 read that clears the flag
        // before the edge is sampled therefore cancels the NMI, which is what
        // the suppression window in ppu_vbl_nmi tests. BRK polls NMI itself
        // (op_brk) and must not be polled again here.
        if !self.cpu.nmi_polled && self.bus.take_nmi_edge() {
            let _ = self.cpu.nmi(&mut self.bus);
        } else if irq_sample && self.cpu.irq_pending() {
            self.cpu.irq(&mut self.bus);
        }

        // ── 4. Pay DMC DMA stalls raised during interrupt vectoring ──
        let stalled = self.bus.take_immediate_stolen_cycles();
        for _ in 0..stalled {
            self.bus.tick();
        }
    }

    /// Step until a new frame is completed.
    /// Returns `true` when a frame was rendered; the registered frame callback
    /// (local harness only) runs before returning.
    pub fn step_frame(&mut self) -> bool {
        loop {
            self.step();
            let current_frame = self.bus.ppu_frame();
            if current_frame != self.last_ppu_frame {
                self.last_ppu_frame = current_frame;
                return true;
            }
        }
    }

    // ════════════════════════════════════════════════════
    // Save / Load state
    // ════════════════════════════════════════════════════

    pub fn save_state(&self, path: &str) {
        let mut out = Writer::new();
        out.u8(0x04);

        out.u8(self.cpu.a);
        out.u8(self.cpu.x);
        out.u8(self.cpu.y);
        out.u16(self.cpu.pc);
        out.u8(self.cpu.sp);
        out.u8(self.cpu.as_u8());

        let ppu = self.bus.ppu_save_state();
        out.u16(ppu.v);
        out.u16(ppu.t);
        out.u8(ppu.x);
        out.bool(ppu.w);
        out.u16(ppu.scanline);
        out.u16(ppu.cycle);
        out.u64(ppu.frame);
        out.bool(ppu.odd_frame);
        out.u8(ppu.read_buffer);
        out.u8(ppu.oam_addr);
        out.bytes(&ppu.oam);
        out.bytes(&ppu.vram);
        out.bytes(&ppu.palette);
        out.bytes(&ppu.chr_ram);
        out.u8(ppu.ctrl);
        out.u8(ppu.status);
        out.u8(ppu.mask);

        out.bytes(self.bus.ram_data());
        out.len_prefixed(&self.bus.mapper_save_state());
        out.len_prefixed(&self.bus.apu_save_state());

        let _ = fs::write(path, out.finish());
    }

    pub fn load_state(&mut self, path: &str) {
        let Ok(data) = fs::read(path) else {
            return;
        };
        let _ = self.apply_state(&data);
    }

    fn apply_state(&mut self, data: &[u8]) -> Option<()> {
        let mut reader = Reader::new(data);
        let version = reader.u8()?;
        if version > 0x04 {
            return None;
        }

        self.cpu.a = reader.u8()?;
        self.cpu.x = reader.u8()?;
        self.cpu.y = reader.u8()?;
        self.cpu.pc = reader.u16()?;
        self.cpu.sp = reader.u8()?;
        self.cpu.p = reader.u8()?;

        let mut ppu = PpuSnapshot {
            v: reader.u16()?,
            t: reader.u16()?,
            x: reader.u8()?,
            w: reader.bool()?,
            scanline: reader.u16()?,
            cycle: reader.u16()?,
            frame: reader.u64()?,
            odd_frame: reader.bool()?,
            read_buffer: reader.u8()?,
            oam_addr: reader.u8()?,
            oam: reader.array()?,
            vram: reader.array()?,
            palette: reader.array()?,
            chr_ram: reader.array()?,
            ctrl: 0,
            status: 0,
            mask: 0,
            has_registers: version >= 0x04,
        };
        if ppu.has_registers {
            ppu.ctrl = reader.u8()?;
            ppu.status = reader.u8()?;
            ppu.mask = reader.u8()?;
        }
        self.bus.ppu_load_state(&ppu);

        if version == 0x01 {
            return Some(());
        }

        let ram = reader.array::<2048>()?;
        self.bus.write_ram_raw(&ram);

        if version == 0x02 {
            reader.skip(8192)?;
        }

        let mapper = reader.len_prefixed()?;
        if !mapper.is_empty() {
            self.bus.mapper_load_state(mapper);
        }
        let apu = reader.len_prefixed()?;
        self.bus.apu_load_state(apu);
        Some(())
    }
}

/// Local-only ROM regression harness (`testing/`, not published). It lives
/// under `nes` so it can read the private bus state (`$6000` PRG RAM, PPU)
/// without adding test-only accessors to the production API.
#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/rom_tests.rs"]
mod rom_tests;

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/reset_tests.rs"]
mod reset_tests;
