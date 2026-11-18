//! Mapper 0 — NROM (no bank switching; up to 32 KiB PRG + 8 KiB CHR).

use super::Mapper;

pub struct Mapper0 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    /// PRG RAM at $6000-$7FFF. NROM test ROMs use it for the `$6000` text
    /// protocol; battery-backed NROM cartridges keep their save here too.
    wram: [u8; 8192],
    /// This cartridge has battery-backed PRG RAM.
    battery: bool,
}

impl Mapper0 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>, battery: bool) -> Self {
        Self {
            prg_rom,
            chr_rom,
            wram: [0; 8192],
            battery,
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
        if let 0x6000..=0x7FFF = addr {
            self.wram[(addr - 0x6000) as usize] = data;
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        self.chr_rom.get(addr as usize).copied().unwrap_or(0)
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }

    fn battery_ram(&self) -> Option<&[u8]> {
        self.battery.then_some(&self.wram[..])
    }

    fn battery_ram_mut(&mut self) -> Option<&mut [u8]> {
        if self.battery {
            Some(&mut self.wram[..])
        } else {
            None
        }
    }

    fn save_state(&self) -> Vec<u8> {
        self.wram.to_vec()
    }

    fn load_state(&mut self, data: &[u8]) -> Option<()> {
        let len = data.len().min(self.wram.len());
        self.wram[..len].copy_from_slice(&data[..len]);
        Some(())
    }
}
