//! Mapper 69 — FME-7 (Sunsoft 5B).
// Sunsoft Batman: Return of the Joker and other Sunsoft games.

use super::Mapper;
use crate::state::{Reader, Writer};

pub struct Fme7 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    has_chr_ram: bool,
    prg_ram: [u8; 8192],
    /// This cartridge has battery-backed PRG RAM.
    battery: bool,
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
            battery,
            command: 0,
            banks: [0; 8],
            irq_counter: 0,
            irq_latch: 0,
            irq_enable: false,
            irq_cycle_mode: false,
            chr_dirty: 0xFF,
        };
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
            0x6000..=0x7FFF => self
                .prg_ram
                .get((addr - 0x6000) as usize)
                .copied()
                .unwrap_or(0),
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

    fn battery_ram(&self) -> Option<&[u8]> {
        self.battery.then_some(&self.prg_ram[..])
    }

    fn battery_ram_mut(&mut self) -> Option<&mut [u8]> {
        if self.battery {
            Some(&mut self.prg_ram[..])
        } else {
            None
        }
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
        let mut out = Writer::new();
        out.u8(self.command);
        out.bytes(&self.banks);
        out.u16(self.irq_counter);
        out.u16(self.irq_latch);
        out.bool(self.irq_enable);
        out.bool(self.irq_cycle_mode);
        out.bytes(&self.prg_ram);
        out.finish()
    }

    fn load_state(&mut self, data: &[u8]) -> Option<()> {
        let mut reader = Reader::new(data);
        self.command = reader.u8()?;
        self.banks = reader.array()?;
        self.irq_counter = reader.u16()?;
        self.irq_latch = reader.u16()?;
        self.irq_enable = reader.bool()?;
        self.irq_cycle_mode = reader.bool()?;
        if reader.remaining() >= 8192 {
            self.prg_ram.copy_from_slice(reader.bytes(8192)?);
        }
        self.chr_dirty = 0xFF;
        Some(())
    }
}

#[cfg(all(test, feature = "local_harness"))]
#[path = "../../../testing/fme7_tests.rs"]
mod tests;
