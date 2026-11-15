//! Mapper 2 — UxROM.

use super::Mapper;

pub struct Mapper2 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    prg_bank: u8,
}

impl Mapper2 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>) -> Self {
        Self {
            prg_rom,
            chr_rom,
            prg_bank: 0,
        }
    }
}

impl Mapper for Mapper2 {
    fn read_prg(&self, addr: u16) -> u8 {
        let bank_count = self.prg_rom.len() / 0x4000;
        if bank_count == 0 {
            return 0;
        }
        match addr {
            0x8000..=0xBFFF => self
                .prg_rom
                .get(((self.prg_bank as usize) % bank_count) * 0x4000 + (addr - 0x8000) as usize)
                .copied()
                .unwrap_or(0),
            0xC000..=0xFFFF => {
                let bank = bank_count - 1;
                self.prg_rom
                    .get(bank * 0x4000 + (addr - 0xC000) as usize)
                    .copied()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        self.chr_rom.get(addr as usize).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.prg_bank = data & 0x0F;
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }

    fn save_state(&self) -> Vec<u8> {
        vec![self.prg_bank]
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.prg_bank = data[0];
    }
}
