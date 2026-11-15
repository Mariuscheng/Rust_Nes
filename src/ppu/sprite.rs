//! OAM evaluation and sprite initialization for the next scanline.

use super::PPU;

/// Pattern-table base address used by a sprite tile.
///
/// 8x8 sprites read from the table selected by PPUCTRL bit 3. In 8x16 mode that
/// bit is ignored: the tile index's low bit picks the 4 KiB bank instead, and
/// the index itself is masked to even so the pair `$00/$01` maps to `$0000`
/// tiles 0-1, `$02/$03` to `$0000` tiles 2-3, `$FE/$FF` to `$1000` tiles
/// `$FE/$FF`, and so on.
pub(super) fn sprite_pattern_bank(sprite_h: u16, tile_id: u8, ctrl_bank: u16) -> usize {
    let bank = if sprite_h == 16 {
        if (tile_id & 0x01) != 0 {
            0x1000
        } else {
            0x0000
        }
    } else {
        ctrl_bank
    };
    bank as usize
}

impl PPU {
    // ── OAM evaluation (for NEXT scanline) ──
    // Evaluates sprites on the CURRENT scanline to determine which ones
    // will be visible on the NEXT scanline.

    pub(super) fn oam_evaluate(&mut self) {
        let cycle = self.cycle;

        // Cycles 1-64: Clear secondary OAM
        if (1..=64).contains(&cycle) {
            let idx = ((cycle - 1) / 2) as usize;
            if idx < 32 {
                self.sec_oam[idx] = 0xFF;
            }
            if cycle == 64 {
                // Reset evaluation state
                self.oam_eval_n = 0;
                self.oam_eval_m = 0;
                self.oam_eval_byte = 0;
                self.oam_eval_reading = true;
                self.oam_eval_overflow = false;
                self.sprite_count_next = 0;
                self.sprite_zero_next = false;
            }
        }

        // Cycles 65-256: Read and evaluate sprites
        if (65..=256).contains(&cycle) {
            let is_read_cycle = !cycle.is_multiple_of(2); // odd = read OAM, even = write sec OAM

            if is_read_cycle {
                // ── Odd cycle: read from primary OAM ──
                if self.oam_eval_byte == 0 {
                    // Reading sprite Y
                    let oam_idx = (self.oam_eval_n as usize) * 4;
                    if oam_idx < 256 {
                        self.oam_eval_sprite_buf[0] = self.oam[oam_idx];
                    }

                    // Check if sprite is in range for next scanline
                    let sprite_y = self.oam_eval_sprite_buf[0] as u16;
                    let next_scanline = if self.scanline == 261 {
                        0u16
                    } else {
                        self.scanline + 1
                    };
                    let sprite_h = if self.ctrl.sprite_size { 16 } else { 8 } as u16;
                    // OAM stores the sprite's top line minus one, so a sprite
                    // with Y = 24 covers scanlines 25..32.
                    let top = sprite_y + 1;
                    let in_range = next_scanline >= top && next_scanline < top + sprite_h;

                    if in_range {
                        self.oam_eval_byte = 1; // next: write Y on even, then read tile_id on next odd
                        self.oam_eval_reading = true;
                    } else {
                        self.oam_eval_n = self.oam_eval_n.wrapping_add(1);
                        if self.oam_eval_n >= 64 {
                            self.oam_eval_n = 0;
                        }
                        // byte stays 0
                    }
                } else if self.oam_eval_byte >= 1 && self.oam_eval_byte <= 3 {
                    // Read byte at offset oam_eval_byte (1=tile_id, 2=attr, 3=X)
                    let oam_idx = (self.oam_eval_n as usize) * 4 + self.oam_eval_byte as usize;
                    if oam_idx < 256 {
                        self.oam_eval_sprite_buf[self.oam_eval_byte as usize] = self.oam[oam_idx];
                    }
                    // Advance to next byte AFTER reading
                    self.oam_eval_byte += 1;
                }
                // byte == 4 on odd: nothing to read (all 4 bytes already fetched)
            } else {
                // ── Even cycle: write previously-read byte to secondary OAM ──
                // Write at oam_eval_byte - 1 (the byte we read on the previous odd cycle)
                if self.oam_eval_byte >= 1 && self.oam_eval_byte <= 4 {
                    if self.oam_eval_m < 8 {
                        let sec_idx =
                            (self.oam_eval_m as usize) * 4 + (self.oam_eval_byte - 1) as usize;
                        if sec_idx < 32 {
                            self.sec_oam[sec_idx] =
                                self.oam_eval_sprite_buf[(self.oam_eval_byte - 1) as usize];
                        }
                    }
                    if self.oam_eval_byte == 4 {
                        // Finished writing all 4 bytes (Y,tile,attr,X)
                        if self.oam_eval_n == 0 {
                            self.sprite_zero_next = true;
                        }
                        if self.oam_eval_m < 8 {
                            self.oam_eval_m += 1;
                            self.sprite_count_next = self.oam_eval_m;
                        } else {
                            self.oam_eval_overflow = true;
                        }
                        self.oam_eval_n = self.oam_eval_n.wrapping_add(1);
                        if self.oam_eval_n >= 64 {
                            self.oam_eval_n = 0;
                        }
                        self.oam_eval_byte = 0;
                    }
                }
            }
        }

        // Set overflow at end of evaluation
        if cycle == 256 && self.oam_eval_overflow {
            self.status.sprite_overflow = true;
        }

        // At end of scanline (cycle 340), initialize sprites for NEXT scanline
        if cycle == 340 {
            self.init_sprites_for_next_scanline();
        }
    }

    /// Load sprite shift registers from secondary OAM for the NEXT scanline
    fn init_sprites_for_next_scanline(&mut self) {
        // Sprites were evaluated for the next scanline during OAM evaluation.
        // For pre-render line (261), next visible line is 0.
        let next_line = if self.scanline == 261 {
            0u16
        } else {
            self.scanline + 1
        };
        let sprite_h = if self.ctrl.sprite_size { 16 } else { 8 };

        // Reset sprite render state for the next line
        self.sprite_count_render = 0;
        self.sprite_zero_rendering = false;
        for i in 0..8 {
            self.spr_active[i] = false;
            self.spr_shift_lo[i] = 0;
            self.spr_shift_hi[i] = 0;
            self.spr_x_counter[i] = 0;
        }

        for i in 0..self.sprite_count_next as usize {
            if i >= 8 {
                break;
            }
            let base = i * 4;
            let sprite_y = self.sec_oam[base] as u16;
            let tile_id = self.sec_oam[base + 1];
            let attr = self.sec_oam[base + 2];
            let sprite_x = self.sec_oam[base + 3];

            // OAM Y is the sprite's top line minus one.
            let top = sprite_y + 1;
            if next_line < top || next_line >= top + sprite_h {
                continue;
            }

            let row = next_line - top;
            let flip_v = (attr & 0x80) != 0;
            let flip_h = (attr & 0x40) != 0;
            let row_in_sprite = if flip_v { (sprite_h - 1) - row } else { row };

            // Determine tile number and row within tile
            let (tile_num, row_in_tile) = if sprite_h == 16 {
                if row_in_sprite < 8 {
                    ((tile_id & 0xFE) as u16, row_in_sprite as u8)
                } else {
                    ((tile_id & 0xFE) as u16 + 1, (row_in_sprite - 8) as u8)
                }
            } else {
                (tile_id as u16, row_in_sprite as u8)
            };

            let bank = sprite_pattern_bank(sprite_h, tile_id, self.ctrl.sprite_pattern_addr);

            let lo = self.chr_ram[bank + tile_num as usize * 16 + row_in_tile as usize];
            let hi = self.chr_ram[bank + tile_num as usize * 16 + row_in_tile as usize + 8];

            let idx = self.sprite_count_render as usize;
            if flip_h {
                self.spr_shift_lo[idx] = lo.reverse_bits();
                self.spr_shift_hi[idx] = hi.reverse_bits();
            } else {
                self.spr_shift_lo[idx] = lo;
                self.spr_shift_hi[idx] = hi;
            }
            self.spr_attr[idx] = attr;
            self.spr_x_pos[idx] = sprite_x;
            self.spr_x_counter[idx] = sprite_x;
            self.spr_active[idx] = true;

            if i == 0 && self.sprite_zero_next {
                self.sprite_zero_rendering = true;
            }

            self.sprite_count_render += 1;
        }
    }
}

#[cfg(all(test, feature = "local_harness"))]
#[path = "../../testing/sprite_tests.rs"]
mod tests;
