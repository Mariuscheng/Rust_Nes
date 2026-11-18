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
                self.oam_eval_addr = 0;
                self.oam_eval_m = 0;
                self.oam_eval_phase = 0;
                self.sprite_count_next = 0;
                self.sprite_zero_next = false;
            }
        }

        // Cycles 65-256: read one OAM byte every two dots. The write half of
        // the pair is folded into the read: only the overflow flag's timing
        // depends on the dot, and that is driven by the read.
        if (65..=256).contains(&cycle) && !cycle.is_multiple_of(2) {
            self.oam_eval_read_byte();
        }

        // At end of scanline (cycle 340), initialize sprites for NEXT scanline
        if cycle == 340 {
            self.init_sprites_for_next_scanline();
        }
    }

    /// True when a sprite whose OAM Y byte is `y` covers the next scanline.
    ///
    /// OAM stores the sprite's top line minus one, so Y = 24 covers scanlines
    /// 25..32 (or 25..40 for 8x16 sprites).
    fn sprite_covers_next_line(&self, y: u8) -> bool {
        let next_line = if self.scanline == 261 {
            0u16
        } else {
            self.scanline + 1
        };
        let height: u16 = if self.ctrl.sprite_size { 16 } else { 8 };
        let top = y as u16 + 1;
        next_line >= top && next_line < top + height
    }

    /// Reads the byte at the evaluator's current OAM address and advances.
    ///
    /// The hardware walks OAM four bytes at a time. When an in-range sprite is
    /// found its four bytes are copied to secondary OAM; otherwise the rest of
    /// the sprite is skipped. Once eight sprites are queued the copy stops and
    /// the flag is latched on the *next* in-range Y, but the byte phase is no
    /// longer reset, so the tile/attribute/X bytes of later sprites get read as
    /// Y coordinates (blargg `4.Obscure`).
    fn oam_eval_read_byte(&mut self) {
        let address = self.oam_eval_addr + self.oam_eval_phase as u16;
        if address >= 256 {
            return; // past the last sprite: hardware stops scanning, never wraps
        }
        let byte = self.oam[address as usize];

        if self.oam_eval_m >= 8 {
            // All eight slots are full. Hardware keeps scanning, but it stops
            // resetting the byte phase, so the tile/attribute/X bytes of later
            // sprites get compared as Y coordinates (blargg `4.Obscure`). The
            // search ends at the first match.
            if self.sprite_covers_next_line(byte) {
                self.status.sprite_overflow = true;
                self.oam_eval_addr = 256;
            } else {
                self.oam_eval_addr += 4;
                self.oam_eval_phase = (self.oam_eval_phase + 1) & 3;
            }
            return;
        }

        if self.oam_eval_phase == 0 {
            if self.sprite_covers_next_line(byte) {
                self.oam_eval_sprite_write(0, byte);
                self.oam_eval_phase = 1;
            } else {
                // Skip the rest of the sprite.
                self.oam_eval_addr += 4;
            }
            return;
        }

        // Tile, attribute and X bytes of a sprite that is being copied.
        self.oam_eval_sprite_write(self.oam_eval_phase, byte);
        self.oam_eval_phase += 1;
        if self.oam_eval_phase == 4 {
            if self.oam_eval_addr == 0 {
                self.sprite_zero_next = true;
            }
            self.oam_eval_m += 1;
            self.sprite_count_next = self.oam_eval_m;
            self.oam_eval_addr += 4;
            self.oam_eval_phase = 0;
        }
    }

    /// Copies one byte of the current sprite into secondary OAM.
    fn oam_eval_sprite_write(&mut self, phase: u8, byte: u8) {
        if self.oam_eval_m >= 8 {
            return;
        }
        let index = self.oam_eval_m as usize * 4 + phase as usize;
        if index < 32 {
            self.sec_oam[index] = byte;
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
