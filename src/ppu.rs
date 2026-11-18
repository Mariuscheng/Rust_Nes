//! PPU: core state, register I/O, palette, sprite evaluation and the per-dot
//! rendering pipeline.
//!
//! `PPU` itself lives here; `registers`, `sprite` and `frame` hold the register
//! bitfields, OAM evaluation and the dot loop respectively.

pub mod frame;
pub mod registers;
pub mod sprite;

use self::registers::{PpuControl, PpuStatus};
use crate::rom::Mirroring;

// ── NES 64-color palette ──
pub const NES_PALETTE: [u32; 64] = [
    0x757575, 0x271B8F, 0x0000AB, 0x47009F, 0x8F0077, 0xAB0013, 0xA70000, 0x7F0B00, 0x432F00,
    0x004700, 0x005100, 0x00393B, 0x004058, 0x000000, 0x000000, 0x000000, 0xBCBCBC, 0x0073EF,
    0x233BEF, 0x8300F3, 0xBF00BF, 0xE7005B, 0xDB2B00, 0xCB4F00, 0x8B7300, 0x009700, 0x00AB00,
    0x00933B, 0x00838B, 0x000000, 0x000000, 0x000000, 0xFFFFFF, 0x3FBFFF, 0x5F97FF, 0xA78BFD,
    0xF77BFF, 0xFF77B7, 0xFF7763, 0xFF9B3B, 0xF3BF3F, 0x72D64F, 0x6BEFB2, 0x54F3E0, 0x5AEFFF,
    0x5B5B5B, 0x000000, 0x000000, 0xFFFFFF, 0xABE7FF, 0xC7D7FF, 0xD7CBFF, 0xF7C7FF, 0xFFC7DB,
    0xFFC7B7, 0xFFCFAB, 0xFFF7C7, 0xF3D7A3, 0xA7E3BB, 0xB7F3DB, 0xB3F3FF, 0xB3B3B3, 0x000000,
    0x000000,
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
    /// True when the cartridge exposes CHR RAM. With CHR ROM the pattern
    /// memory is read-only: `$2007` writes to `$0000-$1FFF` are ignored by
    /// hardware, so they must not corrupt the pattern tables here either.
    pub chr_is_ram: bool,
    pub mirroring: Mirroring,
    pub scanline: u16,
    pub cycle: u16,
    pub frame: u64,
    pub nmi_interrupt: bool,
    /// Sticky rising edge of the NMI line, sampled at every PPU dot. The 6502
    /// edge-detects its NMI input, so a line pulse shorter than one CPU cycle
    /// must still latch an edge (e.g. NMI enabled a dot before the pre-render
    /// clear).
    pub nmi_rise: bool,
    /// NMI line level at the previous PPU dot boundary (edge sampling).
    pub(super) nmi_line_sampled: bool,
    pub nmi_delay: u8,
    pub odd_frame: bool,
    pub mapper_irq: bool,
    /// A12 rising edge count accumulated during the current batch of PPU ticks.
    /// Incremented each time the address bus presents a 0→1 transition on A12
    /// that survives the MMC3 low-pass filter. Cleared after the caller drains
    /// it via `tick_mapper_irq()` calls.
    pub a12_rising_count: u8,
    /// Current level of address bus bit 12. MMC3-family cartridges clock their
    /// IRQ counter on its rising edge, so the PPU tracks the real value the bus
    /// presents each dot instead of guessing from the pattern-table bits.
    pub a12: bool,
    /// Consecutive dots A12 has been low. The MMC3 ignores a rising edge until
    /// A12 has stayed low for three M2 cycles, which is what turns the eight
    /// per-scanline sprite pattern fetches into a single clock.
    pub(super) a12_low_dots: u8,
    /// Accepted A12 edges as (frame, scanline, dot); debug aid for the local
    /// harness only.
    #[cfg(all(test, feature = "local_harness"))]
    pub a12_log: Vec<(u64, u16, u16)>,
    /// True when the VBL flag was set on the most recently processed PPU dot.
    /// A $2002 read on that exact dot returns 0 and suppresses the flag.
    pub(super) vblank_just_set: bool,
    /// Scanline/cycle of the most recent sprite-0 hit, and the fine-X latched at
    /// cycle 1 of the first 40 scanlines. Diagnostic data only the local
    /// harness reads, so it is not part of a published build.
    #[cfg(all(test, feature = "local_harness"))]
    pub split_hit: (u16, u16),
    #[cfg(all(test, feature = "local_harness"))]
    pub row_x: [u8; 40],
    pub(super) vram_increment: u8,

    // ── Frame Buffer (boxed to avoid stack overflow) ──
    pub frame_buffer: Box<[u32]>,

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
    /// OAM byte address the evaluator is about to read. It is a `u16` so the
    /// scan can run off the end of OAM: hardware stops at the last sprite and
    /// never wraps (blargg `2.Details` test 9).
    pub(super) oam_eval_addr: u16,
    /// Sprites found for the next scanline (0..=8).
    pub(super) oam_eval_m: u8,
    /// Byte within the current sprite: 0 = Y, 1 = tile, 2 = attribute, 3 = X.
    /// Once all eight slots are full a mismatch does *not* reset it, which is
    /// the hardware bug `4.Obscure` pins down.
    pub(super) oam_eval_phase: u8,

    /// Open bus latch
    open_bus_latch: u8,

    /// Per-bit decay tracking: the PPU frame number when each bit was last refreshed.
    /// Bits that haven't been refreshed for ≥ DECAY_FRAMES are treated as 0 on reads.
    decay_refresh: [u64; 8],
}

/// ~600ms–1s decay at ~60Hz: 60 frames ≈ 1 second.
const DECAY_FRAMES: u64 = 55;

/// MMC3 A12 low-pass filter: a rising edge only clocks the IRQ counter once
/// A12 has been low for three M2 cycles, and one M2 cycle is three PPU dots, so
/// the threshold is nine dots. That is what collapses the eight sprite pattern
/// fetches into one clock per scanline: the four-dot gaps inside a fetch phase
/// never arm the filter, and neither does the eight-dot gap across a scanline
/// boundary (337-340 plus 1-4), which is why a BG table at $1000 clocks once
/// per line instead of twice (blargg `4-scanline_timing` test 12).
const A12_FILTER_DOTS: u8 = 10;

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
            palette: [
                // Power-up palette RAM as measured by blargg on a real NES (the
                // values power_up_palette checks). 32 bytes, $3F00-$3F1F.
                0x09, 0x01, 0x00, 0x01, 0x00, 0x02, 0x02, 0x0D, 0x08, 0x10, 0x08, 0x24, 0x00, 0x00,
                0x04, 0x2C, 0x09, 0x01, 0x34, 0x03, 0x00, 0x04, 0x00, 0x14, 0x08, 0x3A, 0x00, 0x02,
                0x00, 0x20, 0x2C, 0x08,
            ],
            chr_ram: [0; 8192],
            chr_is_ram: true,
            mirroring,
            scanline: 0,
            cycle: 0,
            frame: 0,
            nmi_interrupt: false,
            nmi_rise: false,
            nmi_line_sampled: false,
            nmi_delay: 0,
            odd_frame: false,
            mapper_irq: false,
            a12_rising_count: 0,
            a12: false,
            a12_low_dots: A12_FILTER_DOTS,
            #[cfg(all(test, feature = "local_harness"))]
            a12_log: Vec::new(),
            vblank_just_set: false,
            #[cfg(all(test, feature = "local_harness"))]
            split_hit: (0, 0),
            #[cfg(all(test, feature = "local_harness"))]
            row_x: [0; 40],
            vram_increment: 1,
            frame_buffer: vec![0u32; 256 * 240].into_boxed_slice(),
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
            oam_eval_addr: 0,
            oam_eval_m: 0,
            oam_eval_phase: 0,
            open_bus_latch: 0,
            decay_refresh: [0; 8],
        }
    }

    /// Apply a hardware reset to the PPU.
    ///
    /// Reset clears the internal address/scroll latches (`v`, `t`, `x`, `w`),
    /// the control, mask and status registers and the NMI line, and stops
    /// rendering in-progress. Video memory — nametables, palette RAM, OAM, CHR
    /// — and the frame/scanline counters survive: the PPU keeps free-running
    /// through a reset and a game cannot tell how warm the video memory is.
    ///
    /// `$2007` read buffer and open-bus latch are also left alone (hardware
    /// does not clear them), as is the cartridge's mirroring until the mapper
    /// re-programs it.
    pub fn reset(&mut self) {
        self.v = 0;
        self.t = 0;
        self.x = 0;
        self.w = false;
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.ctrl = PpuControl::default();
        self.mask = 0;
        self.status = PpuStatus::default();
        self.vram_increment = 1;
        self.oam_addr = 0;
        self.nmi_interrupt = false;
        self.nmi_rise = false;
        self.nmi_line_sampled = false;
        self.nmi_delay = 0;
        self.mapper_irq = false;
        self.a12_rising_count = 0;
        self.a12_low_dots = A12_FILTER_DOTS;
        self.vblank_just_set = false;

        // Drop the in-progress rendering pipeline; the next enabled frame
        // rebuilds it. OAM and secondary OAM keep their bytes.
        self.rendering_enabled = false;
        self.bg_shift_lo = 0;
        self.bg_shift_hi = 0;
        self.bg_attr_lo = 0;
        self.bg_attr_hi = 0;
        self.nt_byte = 0;
        self.at_byte = 0;
        self.bg_low_byte = 0;
        self.bg_high_byte = 0;
        self.sprite_count_next = 0;
        self.sprite_zero_next = false;
        self.sprite_zero_rendering = false;
        self.spr_shift_lo = [0; 8];
        self.spr_shift_hi = [0; 8];
        self.spr_attr = [0; 8];
        self.spr_x_pos = [0; 8];
        self.spr_x_counter = [0; 8];
        self.spr_active = [false; 8];
        self.sprite_count_render = 0;
        self.oam_eval_addr = 0;
        self.oam_eval_m = 0;
        self.oam_eval_phase = 0;
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

    /// Assert the NMI line. The rising edge is latched by the per-dot sampler
    /// in the PPU tick, so asserting an already-high line is not a new edge.
    pub fn raise_nmi(&mut self) {
        self.nmi_interrupt = true;
    }

    /// Consume the latched NMI rising edge, if any.
    pub fn take_nmi_rise(&mut self) -> bool {
        std::mem::take(&mut self.nmi_rise)
    }

    /// Drive address-bus bit 12 for this dot, applying the MMC3 filter.
    ///
    /// The counter input is edge triggered, but the cartridge only accepts a
    /// rising edge after A12 has stayed low for three M2 cycles; without that
    /// filter every pattern fetch would clock the counter.
    pub(super) fn drive_a12(&mut self, high: bool) {
        if high && !self.a12 && self.a12_low_dots >= A12_FILTER_DOTS {
            self.a12_rising_count = self.a12_rising_count.saturating_add(1);
            #[cfg(all(test, feature = "local_harness"))]
            if self.a12_log.len() < 20_000 {
                self.a12_log.push((self.frame, self.scanline, self.cycle));
            }
        }
        self.a12 = high;
        if high {
            self.a12_low_dots = 0;
        } else {
            self.a12_low_dots = self.a12_low_dots.saturating_add(1);
        }
    }

    /// A dot where the PPU leaves the address bus alone: the level holds, but
    /// time keeps running so the filter can arm.
    pub(super) fn hold_a12(&mut self) {
        let level = self.a12;
        self.drive_a12(level);
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
                    self.raise_nmi();
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
                    // The address bus follows v, so the second write is what can
                    // toggle A12 for the MMC3 IRQ counter (blargg `3.A12_clocking`
                    // test 4). The first write only builds t and leaves the bus
                    // alone.
                    self.drive_a12((self.v & 0x1000) != 0);
                }
                self.w = !self.w;
            }
            7 => {
                let addr = self.v & 0x3FFF;
                // A $2007 access puts the current VRAM address on the bus, so
                // its A12 bit is visible to the cartridge (test 5 and 6).
                self.drive_a12((addr & 0x1000) != 0);
                self.internal_write(addr, data);
                self.v = self.v.wrapping_add(self.vram_increment as u16) & 0x3FFF;
                // The increment is visible on the bus for the rest of the
                // access, so a write that carries v across $0FFF still clocks
                // the MMC3 counter.
                self.drive_a12((self.v & 0x1000) != 0);
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
                // The read puts `v` on the bus before the automatic increment,
                // so the cartridge sees that A12 bit (blargg `3.A12_clocking`
                // test 5).
                self.drive_a12((addr & 0x1000) != 0);
                self.v = self.v.wrapping_add(self.vram_increment as u16) & 0x3FFF;
                // ...and the incremented value for the rest of the access, so a
                // read that carries v across $0FFF produces the rising edge the
                // test looks for.
                self.drive_a12((self.v & 0x1000) != 0);
                if addr <= 0x3EFF {
                    // $2007 read buffer: always return buffered value, then read new value into buffer.
                    // This applies regardless of rendering state.
                    let result = self.read_buffer;
                    self.read_buffer = self.internal_read(addr);
                    // The bus carries the value handed to the CPU (the old
                    // buffer), so that is what the decay register latches.
                    // ppu_open_bus test 4 checks exactly this.
                    self.refresh_decay_bits(0xFF, result);
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
                if pa >= 0x10 && pa.is_multiple_of(4) {
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
            0x0000..=0x1FFF if self.chr_is_ram => self.chr_ram[addr as usize] = data,
            0x2000..=0x3EFF => {
                let idx = self.mirror_nametable_addr(addr);
                self.vram[idx] = data;
            }
            0x3F00..=0x3FFF => {
                let mut pa = (addr - 0x3F00) % 32;
                if pa >= 0x10 && pa.is_multiple_of(4) {
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
            Mirroring::OneScreenLower => off,
            Mirroring::OneScreenUpper => 0x0400 + off,
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
        let r = if (emphasis & 0x01) != 0 {
            (r as f32 * 0.75) as u32
        } else {
            r
        };
        let g = if (emphasis & 0x02) != 0 {
            (g as f32 * 0.75) as u32
        } else {
            g
        };
        let b = if (emphasis & 0x04) != 0 {
            (b as f32 * 0.75) as u32
        } else {
            b
        };
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

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/ppu_tests.rs"]
mod tests;
