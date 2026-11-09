// src/ppu/ppu.rs
// Cycle-accurate NES PPU — core struct, register I/O, color processing
//
// Implements:
// - PPU struct with all internal state
// - CPU-visible register read/write ($2000–$2007)
// - VRAM/CHR/Palette memory access
// - Nametable mirroring
// - Color emphasis and grayscale

use super::registers::{PpuControl, PpuStatus};
use crate::rom::Mirroring;

// ── NES 64-color palette ──
pub const NES_PALETTE: [u32; 64] = [
    0x757575, 0x271B8F, 0x0000AB, 0x47009F, 0x8F0077, 0xAB0013, 0xA70000, 0x7F0B00,
    0x432F00, 0x004700, 0x005100, 0x00393B, 0x1B1B1B, 0x000000, 0x000000, 0x000000,
    0xBCBCBC, 0x0073EF, 0x233BEF, 0x8300F3, 0xBF00BF, 0xE7005B, 0xDB2B00, 0xCB4F00,
    0x8B7300, 0x009700, 0x00AB00, 0x00933B, 0x00838B, 0x000000, 0x000000, 0x000000,
    0xFFFFFF, 0x3FBFFF, 0x5F97FF, 0xA78BFD, 0xF77BFF, 0xFF77B7, 0xFF7763, 0xFF9B3B,
    0xF3BF3F, 0x72D64F, 0x6BEFB2, 0x54F3E0, 0x5AEFFF, 0x5B5B5B, 0x000000, 0x000000,
    0xFFFFFF, 0xABE7FF, 0xC7D7FF, 0xD7CBFF, 0xF7C7FF, 0xFFC7DB, 0xFFC7B7, 0xFFCFAB,
    0xFFF7C7, 0xF3D7A3, 0xA7E3BB, 0xB7F3DB, 0xB3F3FF, 0xB3B3B3, 0x000000, 0x000000,
];

pub(crate) const NT_BASE: [u16; 4] = [0x2000, 0x2400, 0x2800, 0x2C00];

pub struct PPU {
    // ── Core Registers ──
    pub v: u16,
    pub t: u16,
    pub x: u8,
    pub w: bool,
    scroll_x: u8,
    scroll_y: u8,
    pub ctrl: PpuControl,
    pub status: PpuStatus,
    pub mask: u8,
    pub read_buffer: u8,
    pub oam: [u8; 256],
    pub oam_addr: u8,
    pub vram: [u8; 4096],
    pub palette: [u8; 32],
    pub chr_ram: [u8; 8192],
    pub mirroring: Mirroring,
    pub scanline: u16,
    pub cycle: u16,
    pub frame: u64,
    pub nmi_interrupt: bool,
    pub nmi_delay: u8,
    pub odd_frame: bool,
    pub mapper_irq: bool,
    /// A12 rising edge count accumulated during the current batch of PPU ticks.
    /// Incremented each time a genuine A12 0→1 transition is detected.
    /// Cleared after the caller drains it via tick_mapper_irq() calls.
    pub a12_rising_count: u8,
    pub(super) prev_a12: bool,
    /// True when the VBL flag was set on the most recently processed PPU dot.
    /// A $2002 read on that exact dot returns 0 and suppresses the flag.
    pub(super) vblank_just_set: bool,
    pub(super) vram_increment: u8,

    // ── Frame Buffer (boxed to avoid stack overflow) ──
    pub frame_buffer: Box<[u32; 256 * 240]>,

    // ── Background Rendering State ──
    pub(super) bg_shift_lo: u16,
    pub(super) bg_shift_hi: u16,
    pub(super) bg_attr_lo: u16,
    pub(super) bg_attr_hi: u16,
    pub(super) nt_byte: u8,
    pub(super) at_byte: u8,
    pub(super) bg_low_byte: u8,
    pub(super) bg_high_byte: u8,
    pub(super) rendering_enabled: bool,

    // ── Sprite Rendering State ──
    pub(super) sec_oam: [u8; 32],
    pub(super) sprite_count_next: u8,
    pub(super) sprite_zero_next: bool,
    pub(super) sprite_zero_rendering: bool,
    pub(super) spr_shift_lo: [u8; 8],
    pub(super) spr_shift_hi: [u8; 8],
    pub(super) spr_attr: [u8; 8],
    pub(super) spr_x_pos: [u8; 8],
    pub(super) spr_x_counter: [u8; 8],
    pub(super) spr_active: [bool; 8],
    pub(super) sprite_count_render: u8,

    // ── OAM Evaluation State Machine ──
    pub(super) oam_eval_n: u8,
    pub(super) oam_eval_m: u8,
    pub(super) oam_eval_byte: u8,
    pub(super) oam_eval_sprite_buf: [u8; 4],
    pub(super) oam_eval_reading: bool,
    pub(super) oam_eval_overflow: bool,

    /// Open bus latch
    open_bus_latch: u8,

    /// Per-bit decay tracking: the PPU frame number when each bit was last refreshed.
    /// Bits that haven't been refreshed for ≥ DECAY_FRAMES are treated as 0 on reads.
    decay_refresh: [u64; 8],
}

/// ~600ms–1s decay at ~60Hz: 60 frames ≈ 1 second.
const DECAY_FRAMES: u64 = 55;

impl PPU {
    pub fn new(mirroring: Mirroring) -> Self {
        Self {
            v: 0,
            t: 0,
            x: 0,
            w: false,
            scroll_x: 0,
            scroll_y: 0,
            ctrl: PpuControl::default(),
            status: PpuStatus::default(),
            mask: 0,
            read_buffer: 0,
            oam: [0; 256],
            oam_addr: 0,
            vram: [0; 4096],
            palette: [0; 32],
            chr_ram: [0; 8192],
            mirroring,
            scanline: 0,
            cycle: 0,
            frame: 0,
            nmi_interrupt: false,
            nmi_delay: 0,
            odd_frame: false,
            mapper_irq: false,
            a12_rising_count: 0,
            prev_a12: false,
            vblank_just_set: false,
            vram_increment: 1,
            frame_buffer: Box::new([0; 256 * 240]),
            bg_shift_lo: 0,
            bg_shift_hi: 0,
            bg_attr_lo: 0,
            bg_attr_hi: 0,
            nt_byte: 0,
            at_byte: 0,
            bg_low_byte: 0,
            bg_high_byte: 0,
            rendering_enabled: false,
            sec_oam: [0xFF; 32],
            sprite_count_next: 0,
            sprite_zero_next: false,
            sprite_zero_rendering: false,
            spr_shift_lo: [0; 8],
            spr_shift_hi: [0; 8],
            spr_attr: [0; 8],
            spr_x_pos: [0; 8],
            spr_x_counter: [0; 8],
            spr_active: [false; 8],
            sprite_count_render: 0,
            oam_eval_n: 0,
            oam_eval_m: 0,
            oam_eval_byte: 0,
            oam_eval_sprite_buf: [0; 4],
            oam_eval_reading: false,
            oam_eval_overflow: false,
            open_bus_latch: 0,
            decay_refresh: [0; 8],
        }
    }

    fn refresh_decay_bits(&mut self, mask: u8, value: u8) {
        self.open_bus_latch = (self.open_bus_latch & !mask) | (value & mask);
        for bit in 0..8 {
            if (mask >> bit) & 1 == 1 {
                self.decay_refresh[bit] = self.frame;
            }
        }
    }

    fn apply_decay(&mut self) {
        for bit in 0..8 {
            if self.frame.wrapping_sub(self.decay_refresh[bit]) >= DECAY_FRAMES {
                self.open_bus_latch &= !(1 << bit);
            }
        }
    }

    // ── PPU Register I/O ──

    pub fn cpu_write(&mut self, reg_idx: u16, data: u8) {
        // Writing to any PPU register sets the entire decay register to the value written
        self.refresh_decay_bits(0xFF, data);
        match reg_idx {
            0 => {
                let prev_nmi_output = self.ctrl.nmi_output;
                self.ctrl.update(data);
                self.t = (self.t & 0xF3FF) | (((data as u16) & 0x03) << 10);
                self.vram_increment = self.ctrl.vram_increment;
                // NES quirk: enabling NMI (bit 7 0→1) during VBlank triggers NMI immediately
                if !prev_nmi_output && self.ctrl.nmi_output && self.status.vblank {
                    self.nmi_interrupt = true;
                }
                // Disabling NMI drops the NMI line so a later re-enable can
                // raise a fresh edge (nmi_control test 9/10).
                if prev_nmi_output && !self.ctrl.nmi_output {
                    self.nmi_interrupt = false;
                }
            }
            1 => {
                self.mask = data;
            }
            3 => {
                self.oam_addr = data;
            }
            4 => {
                let idx = self.oam_addr as usize;
                self.oam[idx] = data;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                if !self.w {
                    self.scroll_x = data;
                    self.x = data & 0x07;
                    self.t = (self.t & 0xFFE0) | ((data as u16) >> 3);
                } else {
                    self.scroll_y = data;
                    self.t = (self.t & 0xFC1F) | (((data as u16) & 0x07) << 12);
                    self.t = (self.t & 0x8FFF) | (((data as u16) & 0xF8) << 2);
                }
                self.w = !self.w;
            }
            6 => {
                if !self.w {
                    self.t = (self.t & 0x00FF) | (((data as u16) & 0x3F) << 8);
                } else {
                    self.t = (self.t & 0xFF00) | (data as u16);
                    self.v = self.t;
                }
                self.w = !self.w;
            }
            7 => {
                let addr = self.v & 0x3FFF;
                self.internal_write(addr, data);
                self.v = self.v.wrapping_add(self.vram_increment as u16) & 0x3FFF;
            }
            _ => {}
        }
    }

    pub fn cpu_read(&mut self, reg_idx: u16) -> u8 {
        // Apply decay before any open-bus bits are used
        self.apply_decay();
        match reg_idx {
            2 => {
                // Reading $2002 on the exact dot the VBL flag is set returns 0
                // and suppresses the flag (and its NMI) for the rest of the frame.
                if self.vblank_just_set {
                    self.status.vblank = false;
                    self.nmi_interrupt = false;
                    self.nmi_delay = 0;
                    self.vblank_just_set = false;
                }
                let status_bits = self.status.as_u8();
                let res = (status_bits & 0xE0) | (self.open_bus_latch & 0x1F);
                // Reading $2002 clears VBlank flag and also suppresses pending NMI
                // (on real NES, reading within the NMI trigger window cancels NMI)
                self.status.vblank = false;
                self.nmi_delay = 0;
                self.w = false;
                // Bits 7-5 refresh from status; bits 4-0 are from decay, do NOT refresh
                self.refresh_decay_bits(0xE0, res);
                res
            }
             7 => {
                let addr = self.v & 0x3FFF;
                self.v = self.v.wrapping_add(self.vram_increment as u16) & 0x3FFF;
                if addr <= 0x3EFF {
                    // $2007 read buffer: always return buffered value, then read new value into buffer.
                    // This applies regardless of rendering state.
                    let result = self.read_buffer;
                    self.read_buffer = self.internal_read(addr);
                    self.refresh_decay_bits(0xFF, self.read_buffer);
                    result
            } else {
                let palette_data = self.internal_read(addr);
                self.read_buffer = self.internal_read(addr & 0x2FFF);
                // Palette read: bits 7-6 from decay register (DD), bits 5-0 from PPU (--)
                // Refresh only bits 5-0; bits 7-6 keep their old decay values
                self.refresh_decay_bits(0x3F, palette_data);
                (self.open_bus_latch & 0xC0) | (palette_data & 0x3F)
            }
        }
        4 => {
            let idx = self.oam_addr as usize;
            let val = self.oam[idx];
            let masked = if (idx & 3) == 2 { val & 0xE3 } else { val };
            self.refresh_decay_bits(0xFF, masked);
            masked
        }
        _ => {
            // Registers $2000, $2001, $2003, $2005, $2006: all 8 bits from decay (DDDD DDDD)
            // Do NOT refresh the latch
            self.open_bus_latch
        }
        }
    }

    pub fn internal_read(&self, addr: u16) -> u8 {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => self.chr_ram[addr as usize],
            0x2000..=0x3EFF => {
                let idx = self.mirror_nametable_addr(addr);
                self.vram[idx]
            }
            0x3F00..=0x3FFF => {
                let mut pa = (addr - 0x3F00) % 32;
                if pa >= 0x10 && pa % 4 == 0 {
                    pa -= 0x10;
                }
                self.palette[pa as usize] & 0x3F
            }
            _ => 0,
        }
    }

    pub fn internal_write(&mut self, addr: u16, data: u8) {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => self.chr_ram[addr as usize] = data,
            0x2000..=0x3EFF => {
                let idx = self.mirror_nametable_addr(addr);
                self.vram[idx] = data;
            }
            0x3F00..=0x3FFF => {
                let mut pa = (addr - 0x3F00) % 32;
                if pa >= 0x10 && pa % 4 == 0 {
                    pa -= 0x10;
                }
                self.palette[pa as usize] = data;
            }
            _ => {}
        }
    }

    pub fn mirror_nametable_addr(&self, addr: u16) -> usize {
        let a = (addr - 0x2000) as usize;
        let nt = a / 0x0400;
        let off = a % 0x0400;
        let mapped = match self.mirroring {
            Mirroring::VERTICAL => match nt {
                0 | 2 => off,
                1 | 3 => 0x0400 + off,
                _ => off,
            },
            Mirroring::HORIZONTAL => match nt {
                0 | 1 => off,
                2 | 3 => 0x0400 + off,
                _ => off,
            },
            Mirroring::FourScreen => a % 4096,
        };
        mapped % 2048
    }

    pub fn read_palette_entry(&self, offset: usize) -> u8 {
        self.palette[offset % 32]
    }

    // ── Color processing ──

    pub(super) fn apply_color_emphasis(&self, color: u32) -> u32 {
        let emphasis = (self.mask >> 5) & 0x07;
        if emphasis == 0 {
            return color;
        }
        let r = (color >> 16) & 0xFF;
        let g = (color >> 8) & 0xFF;
        let b = color & 0xFF;
        // Emphasize: the set bit DIMS its corresponding channel on real hardware
        let r = if (emphasis & 0x01) != 0 { (r as f32 * 0.75) as u32 } else { r };
        let g = if (emphasis & 0x02) != 0 { (g as f32 * 0.75) as u32 } else { g };
        let b = if (emphasis & 0x04) != 0 { (b as f32 * 0.75) as u32 } else { b };
        (r << 16) | (g << 8) | b
    }

    pub(super) fn apply_grayscale(&self, color: u32) -> u32 {
        if (self.mask & 0x01) == 0 {
            return color;
        }
        let r = (color >> 16) & 0xFF;
        let g = (color >> 8) & 0xFF;
        let b = color & 0xFF;
        let gray = (r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114) as u32;
        (gray << 16) | (gray << 8) | gray
    }
}