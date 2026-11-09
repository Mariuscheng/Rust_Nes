// src/rom/mappers/mapper7.rs
// Mapper 7 — AxROM

use super::Mapper;

pub struct Mapper7 {
    prg_rom: Vec<u8>,
    prg_bank: u8,
}

impl Mapper7 {
    pub fn new(prg_rom: Vec<u8>) -> Self {
        Self {
            prg_rom,
            prg_bank: 0,
        }
    }
}

impl Mapper for Mapper7 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0xFFFF => {
                let bank_count = self.prg_rom.len() / 0x8000;
                if bank_count == 0 {
                    return 0;
                }
                self.prg_rom
                    .get(((self.prg_bank as usize) % bank_count) * 0x8000 + (addr - 0x8000) as usize)
                    .copied()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, _addr: u16) -> u8 {
        // Mapper7 永遠使用 CHR-RAM，PPU 直接持有資料。
        // read_chr 對 CHR-RAM 遊戲不會被呼叫。
        0
    }

    fn write_prg(&mut self, _addr: u16, data: u8) {
        self.prg_bank = data & 0x07;
    }

    fn has_chr_ram(&self) -> bool {
        true
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