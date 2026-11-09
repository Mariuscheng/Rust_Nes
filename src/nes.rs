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
use std::fs;

/// Type of a callback invoked at the end of every rendered frame.
/// Receives (frame_number, WRAM_snapshot).
pub type FrameCallback = Box<dyn FnMut(u64, &[u8; 8192])>;

/// 純硬體 NES 主機封裝。
/// 無平台 I/O 依賴；ROM 透過 Cartridge 黑盒注入。
pub struct NesConsole {
    cpu: CPU,
    bus: MainBus,
    /// PPU frame counter snapshot used to detect frame completion.
    last_ppu_frame: u64,
    /// Optional callback invoked after each completed frame.
    frame_callback: Option<FrameCallback>,

    // ── Per-frame cycle counters (for timing diagnostics) ──
    pub frame_cycles: u64,
    pub timing_instr_cycles: u64,
    pub timing_nmi_cycles: u64,
    pub timing_irq_cycles: u64,
    pub timing_apu_stolen: u64,
    pub timing_step_count: u64,
}

impl NesConsole {
    /// Build an NES console from a `Cartridge` (the black-box ROM input).
    pub fn from_cartridge(cart: Cartridge) -> Self {
        let ram = Ram::new();
        let mut ppu = PPU::new(cart.mirroring);

        // Load CHR-ROM into PPU (CHR-RAM mappers skip this — games write via $2007).
        cart.init_chr_ram(&mut ppu.chr_ram);

        let joy1 = Joypad::new();
        let joy2 = Joypad::new();
        let apu = apu::APU::new(crate::constants::DEFAULT_SAMPLE_RATE);

        let mut bus = MainBus::new(cart.mapper, ram, ppu, joy1, joy2, apu);

        let mut cpu = CPU::new();
        cpu.reset(&mut bus);

        let last_ppu_frame = bus.ppu_frame();
        let console = NesConsole {
            cpu,
            bus,
            last_ppu_frame,
            frame_callback: None,
            frame_cycles: 0,
            timing_instr_cycles: 0,
            timing_nmi_cycles: 0,
            timing_irq_cycles: 0,
            timing_apu_stolen: 0,
            timing_step_count: 0,
        };
        console
    }

    /// Register a callback invoked after every completed frame.
    pub fn set_frame_callback(&mut self, cb: FrameCallback) {
        self.frame_callback = Some(cb);
    }

    /// Set a button state on joypad 1 (called by the platform input layer).
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        self.bus.joypad1_mut().set_button(button, pressed);
    }

    /// Copy the PPU frame buffer (256×240) into `buf`.
    pub fn read_frame_buffer(&self, buf: &mut [u32; 256 * 240]) {
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
        let mut total_cpu_cycles: u16 = 0;
        let mut instr_cycles: u16 = 0;
        let mut nmi_cycles_acc: u16 = 0;
        let mut irq_cycles_acc: u16 = 0;

        // ── 1. Execute CPU instruction ──
        let ic = self.cpu.step(&mut self.bus);
        total_cpu_cycles += ic;
        instr_cycles += ic;

        // ── 2. OAM DMA requested by $4014 runs right after the instruction.
        //       The CPU is halted for 513/514 cycles while PPU and APU keep
        //       advancing, so NMI/IRQ raised during the transfer are seen at
        //       the next instruction boundary. ──
        let dma_ran = if let Some(page) = self.bus.take_dma_request() {
            total_cpu_cycles += self.bus.run_dma(page);
            true
        } else {
            false
        };

        // ── 3. Interrupt dispatch (NMI first: it is not maskable) ──
        // The 6502 polls the interrupt lines on the cycle before its last one
        // (a taken branch without a page cross is one cycle earlier again,
        // see `CPU::poll_delay`). After a DMA the CPU was halted, so the live
        // line state is the relevant one.
        let delay = self.cpu.poll_delay;
        let irq_sample = if dma_ran {
            self.bus.irq_asserted()
        } else {
            self.bus.irq_asserted_before_tick(delay)
        };

        // NMI is edge-triggered: the bus latches the rising edge on the PPU's
        // NMI line, and the poll consumes it. A $2002 read that clears the flag
        // before the edge is sampled therefore cancels the NMI, which is what
        // the suppression window in ppu_vbl_nmi tests.
        if self.bus.take_nmi_edge() {
            let nc = self.cpu.nmi(&mut self.bus);
            total_cpu_cycles += nc;
            nmi_cycles_acc += nc;
        } else if irq_sample && self.cpu.irq_pending() {
            self.cpu.irq(&mut self.bus);
            total_cpu_cycles += 7;
            irq_cycles_acc += 7;
        }

        // ── 3. Pay DMC DMA stalls raised during interrupt vectoring ──
        let mut apu_stolen: u64 = 0;
        let stalled = self.bus.take_immediate_stolen_cycles();
        if stalled > 0 {
            for _ in 0..stalled {
                self.bus.tick();
            }
            apu_stolen += stalled as u64;
            total_cpu_cycles = total_cpu_cycles.saturating_add(stalled);
        }

        // ── 4. Accumulate timing counters ──
        self.frame_cycles += total_cpu_cycles as u64;
        self.timing_instr_cycles += instr_cycles as u64;
        self.timing_nmi_cycles += nmi_cycles_acc as u64;
        self.timing_irq_cycles += irq_cycles_acc as u64;
        self.timing_apu_stolen += apu_stolen;
        self.timing_step_count += 1;
    }

    /// Step until a new frame is completed.
    /// Returns `true` if a new frame was rendered, with callback invoked.
    /// Returns `false` if no frame completed yet (should not happen in normal operation).
    pub fn step_frame(&mut self) -> bool {
        loop {
            self.step();
            let current_frame = self.bus.ppu_frame();
            if current_frame != self.last_ppu_frame {
                self.last_ppu_frame = current_frame;
                self.invoke_frame_callback(current_frame);
                return true;
            }
        }
    }

    /// Reset per-frame timing counters (call at end of each frame).
    pub fn reset_frame_counters(&mut self) {
        self.frame_cycles = 0;
        self.timing_instr_cycles = 0;
        self.timing_nmi_cycles = 0;
        self.timing_irq_cycles = 0;
        self.timing_apu_stolen = 0;
        self.timing_step_count = 0;
    }

    // ════════════════════════════════════════════════════
    // Private helpers
    // ════════════════════════════════════════════════════

    fn invoke_frame_callback(&mut self, frame_number: u64) {
        if let Some(ref mut cb) = self.frame_callback {
            let wram = self.bus.wram_snapshot();
            cb(frame_number, &wram);
        }
    }

    // ════════════════════════════════════════════════════
    // Save / Load state
    // ════════════════════════════════════════════════════

    pub fn save_state(&self, path: &str) {
        let mut data = Vec::new();
        data.push(0x03);

        // CPU
        data.extend_from_slice(&self.cpu.a.to_le_bytes());
        data.extend_from_slice(&self.cpu.x.to_le_bytes());
        data.extend_from_slice(&self.cpu.y.to_le_bytes());
        data.extend_from_slice(&self.cpu.pc.to_le_bytes());
        data.push(self.cpu.sp);
        data.push(self.cpu.as_u8());

        // PPU
        let ppu_snap = self.bus.ppu_save_state();
        data.extend_from_slice(&ppu_snap.v.to_le_bytes());
        data.extend_from_slice(&ppu_snap.t.to_le_bytes());
        data.push(ppu_snap.x);
        data.push(if ppu_snap.w { 1 } else { 0 });
        data.extend_from_slice(&ppu_snap.scanline.to_le_bytes());
        data.extend_from_slice(&ppu_snap.cycle.to_le_bytes());
        data.extend_from_slice(&ppu_snap.frame.to_le_bytes());
        data.push(if ppu_snap.odd_frame { 1 } else { 0 });
        data.push(ppu_snap.read_buffer);
        data.push(ppu_snap.oam_addr);
        data.extend_from_slice(&ppu_snap.oam);
        data.extend_from_slice(&ppu_snap.vram);
        data.extend_from_slice(&ppu_snap.palette);
        data.extend_from_slice(&ppu_snap.chr_ram);

        // RAM, Mapper, APU
        data.extend_from_slice(self.bus.ram_data());

        let mapper_bytes = self.bus.mapper_save_state();
        data.extend_from_slice(&(mapper_bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(&mapper_bytes);

        let apu_bytes = self.bus.apu_save_state();
        data.extend_from_slice(&(apu_bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(&apu_bytes);

        let _ = fs::write(path, &data);
    }

    pub fn load_state(&mut self, path: &str) {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(_) => return,
        };
        if data.len() < 2 {
            return;
        }
        let version = data[0];
        if version > 0x03 {
            return;
        }
        let mut offset = 1usize;
        let rd = |d: &[u8], o: &mut usize| -> u8 { let v = d[*o]; *o += 1; v };
        let rd16 = |d: &[u8], o: &mut usize| -> u16 {
            let v = u16::from_le_bytes([d[*o], d[*o + 1]]);
            *o += 2;
            v
        };
        let rd64 = |d: &[u8], o: &mut usize| -> u64 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&d[*o..*o + 8]);
            *o += 8;
            u64::from_le_bytes(bytes)
        };

        self.cpu.a = rd(&data, &mut offset);
        self.cpu.x = rd(&data, &mut offset);
        self.cpu.y = rd(&data, &mut offset);
        self.cpu.pc = rd16(&data, &mut offset);
        self.cpu.sp = rd(&data, &mut offset);
        self.cpu.p = rd(&data, &mut offset);

        let ppu_snap = PpuSnapshot {
            v: rd16(&data, &mut offset),
            t: rd16(&data, &mut offset),
            x: rd(&data, &mut offset),
            w: rd(&data, &mut offset) != 0,
            scanline: rd16(&data, &mut offset),
            cycle: rd16(&data, &mut offset),
            frame: rd64(&data, &mut offset),
            odd_frame: rd(&data, &mut offset) != 0,
            read_buffer: rd(&data, &mut offset),
            oam_addr: rd(&data, &mut offset),
            oam: {
                let mut arr = [0u8; 256];
                arr.copy_from_slice(&data[offset..offset + 256]);
                offset += 256;
                arr
            },
            vram: {
                let mut arr = [0u8; 2048];
                arr.copy_from_slice(&data[offset..offset + 2048]);
                offset += 2048;
                arr
            },
            palette: {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&data[offset..offset + 32]);
                offset += 32;
                arr
            },
            chr_ram: {
                let mut arr = [0u8; 8192];
                arr.copy_from_slice(&data[offset..offset + 8192]);
                offset += 8192;
                arr
            },
        };
        self.bus.ppu_load_state(&ppu_snap);

        if version == 0x01 {
            return;
        }

        self.bus.write_ram_raw(&data[offset..offset + 2048]);
        offset += 2048;

        // v0x02 wrote 8192 bytes of dead WRAM here — skip for backwards compat
        if version == 0x02 {
            offset += 8192;
        }

        let mapper_len = {
            let len = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
            offset += 4;
            len as usize
        };
        if mapper_len > 0 && offset + mapper_len <= data.len() {
            self.bus.mapper_load_state(&data[offset..offset + mapper_len]);
            offset += mapper_len;
        }

        let apu_len = {
            let len = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
            offset += 4;
            len as usize
        };
        if apu_len > 0 && offset + apu_len <= data.len() {
            self.bus.apu_load_state(&data[offset..offset + apu_len]);
        }
    }
}