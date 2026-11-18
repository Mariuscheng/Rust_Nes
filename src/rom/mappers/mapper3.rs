//! Mapper 3 — CNROM.

use super::Mapper;

pub struct Mapper3 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    chr_bank: u8,
}

impl Mapper3 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>) -> Self {
        Self {
            prg_rom,
            chr_rom,
            chr_bank: 0,
        }
    }
}

impl Mapper for Mapper3 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
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

    fn read_chr(&self, addr: u16) -> u8 {
        self.chr_rom
            .get((self.chr_bank as usize) * 8192 + addr as usize)
            .copied()
            .unwrap_or(0)
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.chr_bank = data & 0x03;
    }

    fn save_state(&self) -> Vec<u8> {
        vec![self.chr_bank]
    }

    fn load_state(&mut self, data: &[u8]) -> Option<()> {
        self.chr_bank = *data.first()?;
        Some(())
    }
}
