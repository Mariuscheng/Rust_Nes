//! Mapper 1 — MMC1.

use super::super::mapper::{read_u8_slice, write_u8_vec};
use super::Mapper;

pub struct Mapper1 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    shift_reg: u8,
    shift_count: u8,
    control: u8,
    chr_bank0: u8,
    chr_bank1: u8,
    prg_bank: u8,
    prg_ram: [u8; 8192],
    /// This cartridge has battery-backed PRG RAM.
    battery: bool,
}

impl Mapper1 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>, battery: bool) -> Self {
        Self {
            prg_rom,
            chr_rom,
            shift_reg: 0x10,
            shift_count: 0,
            control: 0x0C,
            // control is OR'd with 0x0C on reset (bit 2 and 3 set)
            chr_bank0: 0,
            chr_bank1: 0,
            prg_bank: 0,
            prg_ram: [0; 8192],
            battery,
        }
    }

    fn prg_bank_count(&self) -> usize {
        if self.prg_rom.is_empty() {
            0
        } else {
            self.prg_rom.len() / 0x4000
        }
    }

    fn internal_register_write(&mut self, reg: u8, value: u8) {
        match reg {
            0 => {
                self.control = value;
            }
            1 => self.chr_bank0 = value,
            2 => self.chr_bank1 = value,
            3 => {
                self.prg_bank = value & 0x0F;
            }
            _ => {}
        }
    }
}

impl Mapper for Mapper1 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => self
                .prg_ram
                .get((addr - 0x6000) as usize)
                .copied()
                .unwrap_or(0),
            0x8000..=0xFFFF => {
                let units = self.prg_bank_count();
                if units == 0 {
                    return 0;
                }
                let rel = addr as usize - 0x8000;
                let prg_mode = (self.control >> 2) & 0x03;
                let bank16: usize = match prg_mode {
                    0 | 1 => {
                        let b = (self.prg_bank & 0x0E) as usize;
                        b + rel / 0x4000
                    }
                    2 => {
                        if rel < 0x4000 {
                            0
                        } else {
                            (self.prg_bank as usize) % units
                        }
                    }
                    3 => {
                        if rel < 0x4000 {
                            (self.prg_bank as usize) % units
                        } else {
                            units.saturating_sub(1)
                        }
                    }
                    _ => 0,
                };
                let offset = (bank16 % units) * 0x4000 + (rel % 0x4000);
                self.prg_rom.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let chr_mode = (self.control >> 4) & 0x01;
        if !self.chr_rom.is_empty() {
            if chr_mode == 0 {
                self.chr_rom
                    .get(((self.chr_bank0 & 0x1E) as usize) * 0x1000 + addr as usize)
                    .copied()
                    .unwrap_or(0)
            } else if addr < 0x1000 {
                self.chr_rom
                    .get((self.chr_bank0 as usize * 0x1000) + addr as usize)
                    .copied()
                    .unwrap_or(0)
            } else {
                self.chr_rom
                    .get((self.chr_bank1 as usize * 0x1000) + (addr as usize - 0x1000))
                    .copied()
                    .unwrap_or(0)
            }
        } else {
            0
        }
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram[(addr - 0x6000) as usize] = data;
            }
            0x8000..=0xFFFF => {
                if data & 0x80 != 0 {
                    self.shift_reg = 0x10;
                    self.shift_count = 0;
                    self.control = 0x0C;
                    return;
                }
                self.shift_reg = (self.shift_reg >> 1) | ((data & 0x01) << 4);
                self.shift_count += 1;
                if self.shift_count == 5 {
                    let reg = ((addr - 0x8000) >> 13) & 0x03;
                    let value = self.shift_reg & 0x1F;
                    self.shift_reg = 0x10;
                    self.shift_count = 0;
                    self.internal_register_write(reg as u8, value);
                }
            }
            _ => {}
        }
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }

    fn battery_ram(&self) -> Option<&[u8]> {
        self.battery.then_some(self.prg_ram.as_slice())
    }

    fn battery_ram_mut(&mut self) -> Option<&mut [u8]> {
        if self.battery {
            Some(self.prg_ram.as_mut_slice())
        } else {
            None
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        write_u8_vec(&mut v, self.shift_reg);
        write_u8_vec(&mut v, self.shift_count);
        write_u8_vec(&mut v, self.control);
        write_u8_vec(&mut v, self.chr_bank0);
        write_u8_vec(&mut v, self.chr_bank1);
        write_u8_vec(&mut v, self.prg_bank);
        v.extend_from_slice(&self.prg_ram);
        v
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.len() < 6 {
            return;
        }
        let mut off = 0;
        self.shift_reg = read_u8_slice(data, &mut off);
        self.shift_count = read_u8_slice(data, &mut off);
        self.control = read_u8_slice(data, &mut off);
        self.chr_bank0 = read_u8_slice(data, &mut off);
        self.chr_bank1 = read_u8_slice(data, &mut off);
        self.prg_bank = read_u8_slice(data, &mut off);
        if data.len() >= off + 8192 {
            self.prg_ram.copy_from_slice(&data[off..off + 8192]);
        }
    }
}
