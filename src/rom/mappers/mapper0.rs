// src/rom/mappers/mapper0.rs
// Mapper 0 — NROM

use super::Mapper;

pub struct Mapper0 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    wram: [u8; 8192],
}

impl Mapper0 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>) -> Self {
        Self {
            prg_rom,
            chr_rom,
            wram: [0; 8192],
        }
    }
}

impl Mapper for Mapper0 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            // PRG RAM / test ROM debug protocol ($6000-$7FFF)
            0x6000..=0x7FFF => self.wram[(addr - 0x6000) as usize],
            0x8000..=0xFFFF => {
                let mut rel_addr = addr as usize - 0x8000;
                if self.prg_rom.len() == 16384 && rel_addr >= 16384 {
                    rel_addr %= 16384;
                }
                self.prg_rom.get(rel_addr).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.wram[(addr - 0x6000) as usize] = data;
            }
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        self.chr_rom.get(addr as usize).copied().unwrap_or(0)
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }
}
