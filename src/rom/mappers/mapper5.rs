//! Mapper 5 — MMC5 (partial: linear PRG/CHR banking).

use super::super::mapper::{read_u8_slice, write_u8_vec};
use super::Mapper;

pub struct Mapper5 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    /// MMC5 can have up to 64 KiB of PRG RAM. Fixed capacity with a known size,
    /// so it lives on the heap: otherwise every Cartridge/MainBus would carry
    /// 64 KiB inline just because this enum variant exists.
    prg_ram: Box<[u8; 65536]>,
    prg_mode: u8,
    chr_mode: u8,
    prg_banks: [u8; 5],
    chr_banks: [u8; 12],
    // Extended audio state
    pulse_a_volume: u8,
    pulse_b_volume: u8,
    pulse_a_duty: u8,
    pulse_b_duty: u8,
    pulse_a_timer: u16,
    pulse_b_timer: u16,
    pulse_a_divider: u16,
    pulse_b_divider: u16,
    pulse_a_enabled: bool,
    pulse_b_enabled: bool,
}

impl Mapper5 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>, battery: bool) -> Self {
        let mut mapper = Self {
            prg_rom,
            chr_rom,
            prg_ram: vec![0u8; 65536]
                .into_boxed_slice()
                .try_into()
                .expect("fixed 64 KiB PRG RAM"),
            prg_mode: 0,
            chr_mode: 0,
            prg_banks: [0; 5],
            chr_banks: [0; 12],
            pulse_a_volume: 0,
            pulse_b_volume: 0,
            pulse_a_duty: 0,
            pulse_b_duty: 0,
            pulse_a_timer: 0,
            pulse_b_timer: 0,
            pulse_a_divider: 0,
            pulse_b_divider: 0,
            pulse_a_enabled: false,
            pulse_b_enabled: false,
        };
        if battery {
            super::super::mapper::load_battery_save("mmc5", &mut *mapper.prg_ram);
        }
        mapper
    }

    fn prg_bank_count(&self) -> usize {
        if self.prg_rom.is_empty() {
            0
        } else {
            self.prg_rom.len() / 0x2000
        }
    }

    fn chr_bank_count(&self) -> usize {
        if self.chr_rom.is_empty() {
            0
        } else {
            self.chr_rom.len() / 0x0400
        }
    }
}

impl Mapper for Mapper5 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => {
                let offset = addr as usize - 0x6000;
                self.prg_ram.get(offset).copied().unwrap_or(0)
            }
            0x8000..=0xFFFF => {
                let banks = self.prg_bank_count();
                if banks == 0 {
                    return 0;
                }
                let rel = addr as usize - 0x8000;
                let bank_idx: usize = match addr {
                    0x8000..=0x9FFF => (self.prg_banks[1] as usize) % banks.max(1),
                    0xA000..=0xBFFF => (self.prg_banks[2] as usize) % banks.max(1),
                    0xC000..=0xDFFF => (self.prg_banks[3] as usize) % banks.max(1),
                    0xE000..=0xFFFF => banks.saturating_sub(1),
                    _ => 0,
                };
                let offset = bank_idx * 0x2000 + (rel % 0x2000);
                self.prg_rom.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        if self.chr_rom.is_empty() {
            return 0;
        }
        let banks = self.chr_bank_count();
        if banks == 0 {
            return 0;
        }
        let bank_idx = (addr as usize / 0x0400) % banks;
        let offset = bank_idx * 0x0400 + (addr as usize % 0x0400);
        self.chr_rom.get(offset).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                let offset = addr as usize - 0x6000;
                if offset < self.prg_ram.len() {
                    self.prg_ram[offset] = data;
                }
            }
            0x8000..=0xFFFF => match addr {
                0x8000..=0x9FFF => {
                    self.prg_mode = data & 0x03;
                }
                0xA000..=0xBFFF => {
                    self.chr_mode = data & 0x03;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }

    fn has_extended_audio(&self) -> bool {
        true
    }

    fn write_audio_register(&mut self, addr: u16, data: u8) {
        match addr & 0x7FFF {
            0x5000 => {
                self.pulse_a_volume = data & 0x0F;
                self.pulse_a_duty = (data >> 4) & 0x03;
                self.pulse_a_enabled = (data & 0x30) != 0;
            }
            0x5002 => {
                self.pulse_a_timer = (self.pulse_a_timer & 0xFF00) | data as u16;
            }
            0x5003 => {
                self.pulse_a_timer = (self.pulse_a_timer & 0x00FF) | ((data as u16 & 0x07) << 8);
                self.pulse_a_divider = self.pulse_a_timer;
            }
            0x5004 => {
                self.pulse_b_volume = data & 0x0F;
                self.pulse_b_duty = (data >> 4) & 0x03;
                self.pulse_b_enabled = (data & 0x30) != 0;
            }
            0x5006 => {
                self.pulse_b_timer = (self.pulse_b_timer & 0xFF00) | data as u16;
            }
            0x5007 => {
                self.pulse_b_timer = (self.pulse_b_timer & 0x00FF) | ((data as u16 & 0x07) << 8);
                self.pulse_b_divider = self.pulse_b_timer;
            }
            _ => {}
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        write_u8_vec(&mut v, self.prg_mode);
        write_u8_vec(&mut v, self.chr_mode);
        v.extend_from_slice(&self.prg_banks);
        v.extend_from_slice(&self.chr_banks);
        v.extend_from_slice(&self.prg_ram[..]);
        v
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.len() < 19 {
            return;
        }
        let mut off = 0;
        self.prg_mode = read_u8_slice(data, &mut off);
        self.chr_mode = read_u8_slice(data, &mut off);
        self.prg_banks.copy_from_slice(&data[off..off + 5]);
        off += 5;
        self.chr_banks.copy_from_slice(&data[off..off + 12]);
        off += 12;
        if data.len() >= off + 65536 {
            self.prg_ram.copy_from_slice(&data[off..off + 65536]);
        }
    }
}
