// src/ppu/frame.rs
// Per-cycle PPU rendering pipeline: tick, fetch, render, scroll

use crate::constants::{DOTS_PER_SCANLINE, SCANLINES_PER_FRAME};
use super::ppu::PPU;
use super::ppu::NES_PALETTE;
use super::ppu::NT_BASE;

impl PPU {
    // ── Per-cycle tick ──

    pub fn tick(&mut self, ppu_cycles: u16) {
        for _ in 0..ppu_cycles {
            // The $2002 read-suppression window is a single PPU dot.
            self.vblank_just_set = false;
            // Odd frame skip: on odd frames with rendering enabled, the
            // pre-render line (261) is one dot shorter, so the next frame
            // starts one real dot earlier. Drop dot 339 and let this same
            // tick iteration process dot 340: consuming the iteration instead
            // would leave the PPU a dot behind the CPU and the frame would
            // not get shorter at all. 339 (not 340) is where the truncated
            // line ends for the CPU-visible timing (10-even_odd_timing).
            if self.odd_frame
                && (self.mask & 0x18) != 0
                && self.scanline == 261
                && self.cycle == 339
            {
                self.cycle = 340;
            }
            // NMI line trails the VBL flag by a couple of dots (see the set
            // below). A $2002 read cancels the pending delay.
            if self.nmi_delay > 0 {
                self.nmi_delay -= 1;
                if self.nmi_delay == 0 && self.ctrl.nmi_output && self.status.vblank {
                    self.raise_nmi();
                }
            }

            // VBlank set at scanline 241, cycle 1, NMI line asserted at the
            // same time. A $2002 read in the same CPU cycle cancels the edge
            // before the CPU commits it (ppu_vbl_nmi 06-suppression).
            if self.scanline == 241 && self.cycle == 1 {
                self.status.vblank = true;
                self.vblank_just_set = true;
                if self.ctrl.nmi_output {
                    self.raise_nmi();
                }
            }

            // VBlank (and the sprite flags) clear at the start of the pre-render
            // line. Cycle 2 matches hardware when reads sample at the end of a
            // CPU cycle's three dots (calibrated with ppu_vbl_nmi).
            if self.scanline == 261 && self.cycle == 2 {
                self.status.vblank = false;
                self.status.sprite_zero_hit = false;
                self.status.sprite_overflow = false;
                self.nmi_interrupt = false;
                self.nmi_delay = 0;
            }

            self.rendering_enabled = (self.mask & 0x18) != 0;
            if self.cycle == 1 && self.scanline < 40 {
                self.row_x[self.scanline as usize] = self.x;
            }
            let is_visible_line = self.scanline < 240;
            let is_pre_render = self.scanline == 261;
            let is_render_line = is_visible_line || is_pre_render;

            // ── Per-cycle rendering ──
            if self.rendering_enabled && is_render_line {
                self.tick_render_line();
            } else if is_visible_line && self.cycle >= 1 && self.cycle <= 256 {
                // Rendering disabled: output backdrop color
                let px = (self.cycle - 1) as usize;
                let py = self.scanline as usize;
                if px < 256 && py < 240 {
                    let backdrop = self.palette[0] as usize;
                    let mut color = NES_PALETTE[backdrop & 0x3F];
                    color = self.apply_color_emphasis(color);
                    color = self.apply_grayscale(color);
                    self.frame_buffer[py * 256 + px] = color;
                }
            }

            // Copy horizontal bits from t to v at cycle 257
            if self.rendering_enabled && is_render_line && self.cycle == 257 {
                self.v = (self.v & 0x7BE0) | (self.t & 0x041F);
            }

            // Copy vertical bits from t to v on pre-render line cycles 280-304
            if self.rendering_enabled && is_pre_render
                && self.cycle >= 280 && self.cycle <= 304
            {
                self.v = (self.v & 0x041F) | (self.t & 0x7BE0);
            }

            // Sample the NMI line at every dot boundary. The CPU's NMI input
            // edge-detects the line, so a pulse shorter than one CPU cycle
            // still counts (07-nmi_on_timing enables NMI just before the
            // pre-render clear). Sampling after the dot's events means a write
            // that raised the line is only seen once a dot has actually run
            // with the line high.
            if self.nmi_interrupt && !self.nmi_line_sampled {
                self.nmi_rise = true;
            }
            self.nmi_line_sampled = self.nmi_interrupt;

            // Advance cycle/scanline
            self.cycle += 1;
            if self.cycle >= DOTS_PER_SCANLINE {
                self.cycle = 0;
                self.scanline += 1;
                if self.scanline >= SCANLINES_PER_FRAME {
                    self.scanline = 0;
                    self.frame = self.frame.wrapping_add(1);
                    self.odd_frame = !self.odd_frame;
                }
            }
        }
    }

    /// Called on every PPU cycle during active rendering lines
    fn tick_render_line(&mut self) {
        let cycle = self.cycle;
        let is_visible = self.scanline < 240;
        let is_pre_render = self.scanline == 261;

        // ── A12 rising edge detection (for MMC3 IRQ) ──
        self.compute_a12_rising();

        // ── OAM evaluation (cycles 1-256) ──
        if is_visible || is_pre_render {
            self.oam_evaluate();
        }

        // ── Tile fetches ──
        let sub_cycle = cycle % 8;

        // Render the pixel from the current shift-register state: bit 15 is
        // the pixel leaving the register on this dot.
        if cycle >= 1 && cycle <= 256 {
            if is_visible {
                self.render_pixel();
            }
        }

        // ── BG shift register advancement ──
        // Shift AFTER rendering, so the pixel at px uses the bit that was at
        // position 15 at the start of the dot. The prefetch also shifts, which
        // moves the first prefetched tile into the high byte before the second
        // one is loaded.
        if (cycle >= 1 && cycle <= 256) || (cycle >= 321 && cycle <= 336) {
            self.bg_shift_lo <<= 1;
            self.bg_shift_hi <<= 1;
            self.bg_attr_lo <<= 1;
            self.bg_attr_hi <<= 1;
        }

        // Fetch new tile data. The fetched tile is loaded into the low byte at
        // sub_cycle 0 (the end of the eight-dot fetch group).
        if cycle >= 1 && cycle <= 256 {
            self.fetch_bg_tile(sub_cycle);
        }

        if cycle == 256 {
            self.increment_y();
        }

        // Pre-fetch first two tiles for next scanline
        if cycle >= 321 && cycle <= 336 {
            self.fetch_bg_tile(sub_cycle);
        }
    }

    // ── A12 rising edge detection (MMC3 IRQ) ──
    fn compute_a12_rising(&mut self) {
        let cycle = self.cycle;
        if !self.rendering_enabled {
            return;
        }

        let current_a12 = if cycle >= 1 && cycle <= 256 {
            let sub = cycle % 8;
            (sub == 5 || sub == 7) && self.ctrl.background_pattern_addr != 0
        } else if cycle >= 257 && cycle <= 320 {
            true // conservative: assume A12 can be high during sprite fetches
        } else if cycle >= 321 && cycle <= 340 {
            let sub = cycle % 8;
            (sub == 5 || sub == 7) && self.ctrl.background_pattern_addr != 0
        } else {
            false
        };

        if current_a12 && !self.prev_a12 {
            self.a12_rising_count = self.a12_rising_count.saturating_add(1);
        }
        self.prev_a12 = current_a12;
    }

    // ── Background tile fetching ──

    fn fetch_bg_tile(&mut self, sub_cycle: u16) {
        let coarse_x = (self.v as u8) & 0x1F;
        let coarse_y = ((self.v >> 5) as u8) & 0x1F;
        let nt = ((self.v >> 10) as u8) & 0x03;
        let fine_y = ((self.v >> 12) as u8) & 0x07;
        let nt_base = NT_BASE[nt as usize];

        match sub_cycle {
            1 => {
                let nt_addr = nt_base + (coarse_y as u16) * 32 + coarse_x as u16;
                self.nt_byte = self.vram[self.mirror_nametable_addr(nt_addr)];
            }
            3 => {
                let attr_addr = nt_base + 0x03C0
                    + ((coarse_y as u16) / 4) * 8
                    + (coarse_x as u16) / 4;
                self.at_byte = self.vram[self.mirror_nametable_addr(attr_addr)];
            }
            5 => {
                // Use the nt_byte latched during sub-cycle 1 (mirrors real hardware)
                let tile_id = self.nt_byte as u16;
                let bank = self.ctrl.background_pattern_addr;
                let pattern_addr = bank + tile_id * 16 + fine_y as u16;
                self.bg_low_byte = self.chr_ram[pattern_addr as usize];
            }
            7 => {
                // Use the nt_byte latched during sub-cycle 1 (mirrors real hardware)
                let tile_id = self.nt_byte as u16;
                let bank = self.ctrl.background_pattern_addr;
                let pattern_addr = bank + tile_id * 16 + fine_y as u16 + 8;
                self.bg_high_byte = self.chr_ram[pattern_addr as usize];
            }
            0 => {
                // End of the eight-dot fetch group: move the fetched tile into
                // the low byte. The next group's NT read starts on the
                // following dot, so the load belongs to sub-cycle 0.
                let quad_x = (coarse_x % 4) >= 2;
                let quad_y = (coarse_y % 4) >= 2;
                let shift = (if quad_y { 4 } else { 0 }) + (if quad_x { 2 } else { 0 });
                let palette_bits = (self.at_byte >> shift) & 0x03;

                let attr_lo = if (palette_bits & 0x01) != 0 { 0xFF } else { 0x00 };
                let attr_hi = if (palette_bits & 0x02) != 0 { 0xFF } else { 0x00 };

                self.bg_shift_lo = (self.bg_shift_lo & 0xFF00) | (self.bg_low_byte as u16);
                self.bg_shift_hi = (self.bg_shift_hi & 0xFF00) | (self.bg_high_byte as u16);
                self.bg_attr_lo = (self.bg_attr_lo & 0xFF00) | (attr_lo as u16);
                self.bg_attr_hi = (self.bg_attr_hi & 0xFF00) | (attr_hi as u16);

                self.increment_x();
            }
            _ => {}
        }
    }

    fn increment_x(&mut self) {
        let coarse_x = (self.v as u8) & 0x1F;
        if coarse_x == 31 {
            self.v = (self.v & 0x7FE0) ^ 0x0400;
        } else {
            self.v = (self.v & 0x7FE0) | ((self.v + 1) & 0x001F);
        }
    }

    fn increment_y(&mut self) {
        let fine_y = (self.v >> 12) & 0x07;
        if fine_y < 7 {
            self.v += 0x1000;
        } else {
            self.v &= 0x0FFF;
            let coarse_y = (self.v >> 5) & 0x1F;
            if coarse_y == 29 {
                self.v = (self.v & 0x7C1F) ^ 0x0800;
            } else if coarse_y == 31 {
                self.v &= 0x7C1F;
            } else {
                self.v += 0x0020;
            }
        }
    }

    // ── Per-pixel output ──

    fn render_pixel(&mut self) {
        let px = (self.cycle - 1) as usize;
        let py = self.scanline as usize;
        if px >= 256 || py >= 240 {
            return;
        }

        let bg_enabled = (self.mask & 0x08) != 0;
        let spr_enabled = (self.mask & 0x10) != 0;
        let bg_clip = (self.mask & 0x02) == 0;
        let spr_clip = (self.mask & 0x04) == 0;

        // Background pixel (from shift register MSB)
        let bit_mux: u16 = 0x8000 >> self.x;
        let bg_pixel = if bg_enabled && !(bg_clip && px < 8) {
            ((self.bg_shift_hi & bit_mux) != 0) as u8 * 2
                | ((self.bg_shift_lo & bit_mux) != 0) as u8
        } else {
            0
        };
        let bg_palette = if bg_enabled {
            ((self.bg_attr_hi & bit_mux) != 0) as u8 * 2
                | ((self.bg_attr_lo & bit_mux) != 0) as u8
        } else {
            0
        };
        let bg_opaque = bg_pixel != 0;

        // ── Sprite pixel ──
        let mut spr_pixel: u8 = 0;
        let mut spr_palette: u8 = 0;
        let mut spr_behind = false;
        let mut spr_visible = false;
        let mut spr_is_zero = false;

        if spr_enabled {
            for i in 0..self.sprite_count_render as usize {
                if !self.spr_active[i] {
                    continue;
                }
                // The x counter runs even for clipped pixels; only the output
                // is suppressed in the left 8 pixels, otherwise sprites would
                // shift right by 8 whenever clipping is enabled.
                if self.spr_x_counter[i] > 0 {
                    self.spr_x_counter[i] -= 1;
                } else {
                    // First pixel of this sprite: bit 7 is the leftmost pixel.
                    // Shift the register here, at the end of the rendered
                    // pixel, so the counter reaching zero on the previous dot
                    // does not advance the pattern before its first pixel.
                    if !(spr_clip && px < 8) {
                        let pix = ((self.spr_shift_hi[i] & 0x80) != 0) as u8 * 2
                            | ((self.spr_shift_lo[i] & 0x80) != 0) as u8;
                        if pix != 0 && !spr_visible {
                            spr_pixel = pix;
                            spr_palette = (self.spr_attr[i] & 0x03) + 4;
                            spr_behind = (self.spr_attr[i] & 0x20) != 0;
                            spr_visible = true;
                            if i == 0 && self.sprite_zero_rendering {
                                spr_is_zero = true;
                            }
                        }
                    }
                    self.spr_shift_lo[i] <<= 1;
                    self.spr_shift_hi[i] <<= 1;
                }
            }
        }

        // Sprite 0 hit
        if spr_is_zero && bg_opaque && bg_enabled && spr_enabled && px < 255 {
            if !self.status.sprite_zero_hit {
                self.split_hit = (self.scanline, self.cycle);
            }
            self.status.sprite_zero_hit = true;
        }

        // ── Final color ──
        let final_idx: u8;
        if spr_visible {
            if bg_opaque {
                if spr_behind {
                    final_idx = self.read_palette_entry((bg_palette as usize) * 4 + bg_pixel as usize);
                } else {
                    final_idx = self.read_palette_entry((spr_palette as usize) * 4 + spr_pixel as usize);
                }
            } else {
                final_idx = self.read_palette_entry((spr_palette as usize) * 4 + spr_pixel as usize);
            }
        } else if bg_opaque {
            final_idx = self.read_palette_entry((bg_palette as usize) * 4 + bg_pixel as usize);
        } else {
            final_idx = self.palette[0];
        }

        let mut color = NES_PALETTE[(final_idx & 0x3F) as usize];
        color = self.apply_color_emphasis(color);
        color = self.apply_grayscale(color);
        self.frame_buffer[py * 256 + px] = color;
    }
}
