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
        let mut console = NesConsole {
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
        console.bus.sync_chr_to_ppu();
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
    /// NMI timing: On real 6502, NMI is sampled on the penultimate cycle
    /// of each instruction. We simulate this by ticking PPU one CPU cycle
    /// at a time and checking NMI on the penultimate cycle.
    pub fn step(&mut self) {
        let mut total_cpu_cycles: u16 = 0;
        let mut instr_cycles: u16 = 0;
        let mut nmi_cycles_acc: u16 = 0;
        let mut irq_cycles_acc: u16 = 0;

        // ── 1. Execute CPU instruction ──
        let ic = self.cpu.step(&mut self.bus);
        total_cpu_cycles += ic;
        instr_cycles += ic;

        // ── 2. Tick remaining PPU cycles one at a time ──
        // Some opcodes (BRK, IRQ) consume PPU cycles mid-instruction.
        // We tick only the remaining cycles here.
        let consumed_dots = self.cpu.ppu_ticks_consumed;
        let total_dots = ic.saturating_mul(3);
        let remaining_dots = total_dots.saturating_sub(consumed_dots);
        let remaining_cpu_cycles = remaining_dots / 3;

        let mut nmi_latched = false;
        for i in 0..remaining_cpu_cycles {
            let is_penultimate = i == remaining_cpu_cycles.saturating_sub(2);

            // Tick one CPU cycle (3 PPU dots) through advance_ppu_cycle.
            // This returns true if NMI was asserted during this cycle.
            if self.bus.advance_ppu_cycle() {
                self.cpu.set_nmi_line(true);
            }

            // On the penultimate cycle, sample NMI (as real 6502 does).
            if is_penultimate {
                if self.cpu.poll_nmi() {
                    nmi_latched = true;
                }
            }

            // Tick mapper IRQ for A12 edges that occurred during this cycle.
            // advance_ppu_cycle already handles most A12 edges internally,
            // but we also need to process any accumulated ones here.
            // (They are already processed inside advance_ppu_cycle via
            // the a12_rising_count drain.)
        }

        // ── 3. Handle NMI if detected on penultimate cycle ──
        if nmi_latched {
            let nc = self.cpu.nmi(&mut self.bus);
            total_cpu_cycles += nc;
            nmi_cycles_acc += nc;
            let a12 = self.bus.tick_ppu_cycles(nc);
            for _ in 0..a12 {
                self.bus.tick_mapper_irq();
            }
        } else {
            // ── 4. Check NMI after all cycles (for NMI that fires on last cycle) ──
            self.sync_nmi();
            if self.cpu.poll_nmi() {
                let nc = self.cpu.nmi(&mut self.bus);
                total_cpu_cycles += nc;
                nmi_cycles_acc += nc;
                let a12 = self.bus.tick_ppu_cycles(nc);
                for _ in 0..a12 {
                    self.bus.tick_mapper_irq();
                }
            } else if !self.cpu.skip_irq {
                // ── 5. Handle IRQ ──
                if self.bus.irq_asserted() && self.cpu.irq_pending() {
                    self.cpu.irq(&mut self.bus);
                    total_cpu_cycles += 7;
                    irq_cycles_acc += 7;
                    let irq_consumed = self.cpu.ppu_ticks_consumed;
                    let irq_remaining = 21u16.saturating_sub(irq_consumed);
                    if irq_remaining > 0 {
                        let a12 = self.bus.tick_ppu_cycles(irq_remaining / 3);
                        for _ in 0..a12 {
                            self.bus.tick_mapper_irq();
                        }
                    }
                    self.sync_nmi();
                    if self.cpu.poll_nmi() {
                        let nc = self.cpu.nmi(&mut self.bus);
                        total_cpu_cycles += nc;
                        nmi_cycles_acc += nc;
                        let a12 = self.bus.tick_ppu_cycles(nc);
                        for _ in 0..a12 {
                            self.bus.tick_mapper_irq();
                        }
                    }
                }
            }
        }

        // ── 6. APU tick (may steal cycles via DMC DMA) ──
        let stolen = self.bus.step_apu(total_cpu_cycles);
        if stolen > 0 {
            let stolen_cpu = stolen as u16;
            let a12 = self.bus.tick_ppu_cycles(stolen_cpu);
            for _ in 0..a12 {
                self.bus.tick_mapper_irq();
            }
            self.sync_nmi();
            if self.cpu.poll_nmi() {
                let nc = self.cpu.nmi(&mut self.bus);
                total_cpu_cycles += nc;
                nmi_cycles_acc += nc;
                let a12 = self.bus.tick_ppu_cycles(nc);
                for _ in 0..a12 {
                    self.bus.tick_mapper_irq();
                }
            }
        }

        // ── Accumulate timing counters ──
        let total = total_cpu_cycles as u64 + stolen;
        self.frame_cycles += total;
        self.timing_instr_cycles += instr_cycles as u64;
        self.timing_nmi_cycles += nmi_cycles_acc as u64;
        self.timing_irq_cycles += irq_cycles_acc as u64;
        self.timing_apu_stolen += stolen;
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
                self.bus.sync_chr_to_ppu();
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

    fn sync_nmi(&mut self) {
        let nmi_state = self.bus.consume_nmi();
        self.cpu.set_nmi_line(nmi_state);
    }


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