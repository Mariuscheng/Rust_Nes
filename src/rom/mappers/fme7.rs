// src/rom/mappers/fme7.rs
// Mapper 69 — FME-7 (Sunsoft 5B)
// Sunsoft Batman: Return of the Joker and other Sunsoft games.

use super::super::mapper::{read_u8_slice, write_u8_vec};
use super::Mapper;

pub struct Fme7 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    has_chr_ram: bool,
    prg_ram: [u8; 8192],
    // Bank select register
    command: u8,
    // Bank registers (8)
    banks: [u8; 8],
    // IRQ
    irq_counter: u16,
    irq_latch: u16,
    irq_enable: bool,
    irq_cycle_mode: bool,
    /// Bit i set = 1 KiB CHR block i may have changed (see `take_chr_dirty_blocks`).
    chr_dirty: u8,
}

impl Fme7 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>, has_chr_ram: bool, battery: bool) -> Self {
        let mut mapper = Self {
            prg_rom,
            chr_rom,
            has_chr_ram,
            prg_ram: [0; 8192],
            command: 0,
            banks: [0; 8],
            irq_counter: 0,
            irq_latch: 0,
            irq_enable: false,
            irq_cycle_mode: false,
            chr_dirty: 0xFF,
        };
        if battery {
            super::super::mapper::load_battery_save("fme7", &mut mapper.prg_ram);
        }
        // Default bank values
        mapper.banks[0] = 0;
        mapper.banks[1] = 1;
        mapper.banks[2] = 2;
        mapper.banks[3] = 3;
        mapper.banks[4] = 4;
        mapper.banks[5] = 5;
        mapper.banks[6] = 6;
        mapper.banks[7] = 7;
        mapper
    }
}

impl Mapper for Fme7 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram.get((addr - 0x6000) as usize).copied().unwrap_or(0)
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg_rom.len() / 0x2000).max(1);
                let bank = match addr {
                    0x8000..=0x9FFF => self.banks[6] as usize,
                    0xA000..=0xBFFF => self.banks[7] as usize,
                    0xC000..=0xFFFF => banks.saturating_sub(1),
                    _ => 0,
                };
                let offset = (bank % banks) * 0x2000 + (addr as usize % 0x2000);
                self.prg_rom.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        if self.chr_rom.is_empty() {
            return 0;
        }
        let banks = (self.chr_rom.len() / 0x0400).max(1);
        let bank_idx = self.banks[(addr as usize / 0x0400) & 0x07] as usize;
        let offset = (bank_idx % banks) * 0x0400 + (addr as usize % 0x0400);
        self.chr_rom.get(offset).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram[(addr - 0x6000) as usize] = data;
            }
            0x8000..=0x9FFF => {
                self.command = data & 0x0F;
            }
            0xA000..=0xBFFF => {
                match self.command {
                    0..=7 => {
                        self.banks[self.command as usize] = data;
                        // Command n feeds 1 KiB CHR block n; nothing else moves.
                        self.chr_dirty |= 1 << self.command;
                    }
                    8 => {
                        // Mirroring (not implemented fully here)
                    }
                    9 => {
                        self.irq_latch = (self.irq_latch & 0xFF00) | data as u16;
                    }
                    0x0A => {
                        self.irq_latch = (self.irq_latch & 0x00FF) | ((data as u16) << 8);
                    }
                    0x0B => {
                        self.irq_counter = self.irq_latch;
                        self.irq_enable = (data & 0x01) != 0;
                        self.irq_cycle_mode = (data & 0x02) != 0;
                    }
                    0x0C => {
                        self.irq_enable = (data & 0x01) != 0;
                    }
                    0x0D => {
                        // Additional register
                    }
                    0x0E => {
                        // Additional register
                    }
                    0x0F => {
                        // Additional register
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn has_chr_ram(&self) -> bool {
        self.has_chr_ram
    }

    fn take_chr_dirty_blocks(&mut self) -> u8 {
        std::mem::take(&mut self.chr_dirty)
    }

    fn tick_mapper_irq(&mut self) -> bool {
        if !self.irq_enable {
            return false;
        }
        if self.irq_cycle_mode {
            // Counter on every CPU cycle (tick called by PPU which is slower, simplified)
            if self.irq_counter > 0 {
                self.irq_counter -= 1;
                if self.irq_counter == 0 {
                    return true;
                }
            }
        }
        false
    }

    fn has_extended_audio(&self) -> bool {
        true
    }

    fn write_audio_register(&mut self, addr: u16, _data: u8) {
        match addr & 0x7FFF {
            0xC000 => {
                // FME-7 has basic audio extensions (simplified stub)
            }
            0xE000 => {}
            _ => {}
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        write_u8_vec(&mut v, self.command);
        v.extend_from_slice(&self.banks);
        v.extend_from_slice(&self.irq_counter.to_le_bytes());
        v.extend_from_slice(&self.irq_latch.to_le_bytes());
        v.push(if self.irq_enable { 1 } else { 0 });
        v.push(if self.irq_cycle_mode { 1 } else { 0 });
        v.extend_from_slice(&self.prg_ram);
        v
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.len() < 16 {
            return;
        }
        let mut off = 0;
        self.command = read_u8_slice(data, &mut off);
        self.banks.copy_from_slice(&data[off..off + 8]);
        off += 8;
        self.irq_counter = u16::from_le_bytes([data[off], data[off + 1]]);
        off += 2;
        self.irq_latch = u16::from_le_bytes([data[off], data[off + 1]]);
        off += 2;
        self.irq_enable = data[off] != 0;
        off += 1;
        self.irq_cycle_mode = data[off] != 0;
        off += 1;
        if data.len() >= off + 8192 {
            self.prg_ram.copy_from_slice(&data[off..off + 8192]);
        }
        // Restored banks may differ from what the PPU holds; force a full copy.
        self.chr_dirty = 0xFF;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper() -> Fme7 {
        Fme7::new(vec![0u8; 0x8000], vec![0u8; 16 * 1024], false, false)
    }

    #[test]
    fn chr_dirty_marks_only_the_bank_a_write_touches() {
        let mut m = mapper();
        assert_eq!(
            m.take_chr_dirty_blocks(),
            0xFF,
            "boot refresh must copy all of CHR"
        );

        // Command 3 selects CHR 1 KiB block 3 — and nothing else.
        m.write_prg(0x8000, 3);
        assert_eq!(m.take_chr_dirty_blocks(), 0, "latch alone moves no banks");
        m.write_prg(0xA000, 9);
        assert_eq!(m.take_chr_dirty_blocks(), 1 << 3);

        // IRQ latch / control commands leave the pattern table alone.
        m.write_prg(0x8000, 9);
        m.write_prg(0xA000, 0x20);
        m.write_prg(0x8000, 0x0B);
        m.write_prg(0xA000, 0x01);
        assert_eq!(m.take_chr_dirty_blocks(), 0);
    }
}