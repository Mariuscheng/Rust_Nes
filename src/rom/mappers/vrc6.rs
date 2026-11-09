// src/rom/mappers/vrc6.rs
// Mapper 24/26 — VRC6 (Konami)
// Supports both VRC6a (mapper 24) and VRC6b (mapper 26).

use super::super::mapper::{read_u8_slice, write_u8_vec};
use super::Mirroring;
use super::Mapper;

pub struct Vrc6 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    mirroring: Mirroring,
    has_chr_ram: bool,
    prg_bank0: u8,
    prg_mode: u8,
    chr_banks: [u8; 8],
    irq_latch: u8,
    irq_counter: u8,
    irq_enable: bool,
    irq_mode: bool,
    // Extended audio — Pulse 1
    pulse1_volume: u8,
    pulse1_duty: u8,
    pulse1_timer: u16,
    pulse1_divider: u16,
    pulse1_enabled: bool,
    // Extended audio — Pulse 2
    pulse2_volume: u8,
    pulse2_duty: u8,
    pulse2_timer: u16,
    pulse2_divider: u16,
    pulse2_enabled: bool,
    // Extended audio — Sawtooth
    saw_volume: u8,
    saw_timer: u16,
    saw_divider: u16,
    saw_enabled: bool,
}

impl Vrc6 {
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        _mapper_id: u8,
        mirroring: Mirroring,
        has_chr_ram: bool,
    ) -> Self {
        Self {
            prg_rom,
            chr_rom,
            mirroring,
            has_chr_ram,
            prg_bank0: 0,
            prg_mode: 0,
            chr_banks: [0; 8],
            irq_latch: 0,
            irq_counter: 0,
            irq_enable: false,
            irq_mode: false,
            pulse1_volume: 0,
            pulse1_duty: 0,
            pulse1_timer: 0,
            pulse1_divider: 0,
            pulse1_enabled: false,
            pulse2_volume: 0,
            pulse2_duty: 0,
            pulse2_timer: 0,
            pulse2_divider: 0,
            pulse2_enabled: false,
            saw_volume: 0,
            saw_timer: 0,
            saw_divider: 0,
            saw_enabled: false,
        }
    }

    fn prg_bank_count(&self) -> usize {
        if self.prg_rom.is_empty() {
            0
        } else {
            self.prg_rom.len() / 0x4000
        }
    }
}

impl Mapper for Vrc6 {
    fn read_prg(&self, addr: u16) -> u8 {
        let banks = self.prg_bank_count();
        if banks == 0 {
            return 0;
        }
        match addr {
            0x8000..=0xFFFF => {
                let rel = addr as usize - 0x8000;
                let bank = if self.prg_mode == 0 {
                    match addr {
                        0x8000..=0xBFFF => (self.prg_bank0 & 0x0F) as usize,
                        _ => banks.saturating_sub(1),
                    }
                } else {
                    match addr {
                        0x8000..=0xBFFF => banks.saturating_sub(2),
                        _ => (self.prg_bank0 & 0x0F) as usize,
                    }
                };
                let offset = (bank % banks) * 0x4000 + (rel % 0x4000);
                self.prg_rom.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        if self.chr_rom.is_empty() {
            return 0;
        }
        let bank = (addr as usize / 0x0400) % 8;
        let chr_banks_count = (self.chr_rom.len() / 0x0400).max(1);
        let offset = (self.chr_banks[bank] as usize % chr_banks_count) * 0x0400
            + (addr as usize % 0x0400);
        self.chr_rom.get(offset).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x8000..=0x8003 => {
                self.prg_bank0 = data & 0x0F;
            }
            0xB003 => {
                self.mirroring = if data & 0x01 != 0 {
                    Mirroring::HORIZONTAL
                } else {
                    Mirroring::VERTICAL
                };
            }
            0xC000..=0xC003 => {}
            0xD000..=0xDFFF => {
                let idx = ((addr - 0xD000) >> 2) as usize;
                if idx < 4 {
                    self.chr_banks[idx] = data;
                }
            }
            0xE000..=0xEFFF => {
                let idx = (((addr - 0xE000) >> 2) as usize).min(3);
                self.chr_banks[4 + idx] = data;
            }
            0xF000..=0xF003 => {
                self.irq_latch = data;
            }
            _ => {}
        }
    }

    fn has_chr_ram(&self) -> bool {
        self.has_chr_ram
    }

    fn tick_mapper_irq(&mut self) -> bool {
        if !self.irq_enable {
            return false;
        }
        if self.irq_mode {
            self.irq_counter = self.irq_counter.wrapping_add(1);
            if self.irq_counter == 0 {
                self.irq_counter = self.irq_latch;
                return true;
            }
        }
        false
    }

    fn has_extended_audio(&self) -> bool {
        true
    }

    fn write_audio_register(&mut self, addr: u16, data: u8) {
        match addr & 0xF003 {
            0x9000 => {
                self.pulse1_volume = data & 0x0F;
                self.pulse1_duty = (data >> 4) & 0x07;
                self.pulse1_enabled = self.pulse1_volume > 0;
            }
            0x9001 => {
                self.pulse1_timer = (self.pulse1_timer & 0x0F00) | data as u16;
            }
            0x9002 => {
                self.pulse1_timer = (self.pulse1_timer & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.pulse1_enabled = (data & 0x80) != 0;
                self.pulse1_divider = self.pulse1_timer;
            }
            0xA000 => {
                self.pulse2_volume = data & 0x0F;
                self.pulse2_duty = (data >> 4) & 0x07;
                self.pulse2_enabled = self.pulse2_volume > 0;
            }
            0xA001 => {
                self.pulse2_timer = (self.pulse2_timer & 0x0F00) | data as u16;
            }
            0xA002 => {
                self.pulse2_timer = (self.pulse2_timer & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.pulse2_enabled = (data & 0x80) != 0;
                self.pulse2_divider = self.pulse2_timer;
            }
            0xB000 => {
                self.saw_volume = data & 0x3F;
                self.saw_enabled = self.saw_volume > 0;
            }
            0xB001 => {
                self.saw_timer = (self.saw_timer & 0x0F00) | data as u16;
            }
            0xB002 => {
                self.saw_timer = (self.saw_timer & 0x00FF) | (((data & 0x0F) as u16) << 8);
                self.saw_enabled = (data & 0x80) != 0;
                self.saw_divider = self.saw_timer;
            }
            _ => {}
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        write_u8_vec(&mut v, self.prg_bank0);
        write_u8_vec(&mut v, self.prg_mode);
        v.extend_from_slice(&self.chr_banks);
        write_u8_vec(&mut v, self.irq_latch);
        write_u8_vec(&mut v, self.irq_counter);
        v.push(if self.irq_enable { 1 } else { 0 });
        v.push(if self.irq_mode { 1 } else { 0 });
        v
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.len() < 13 {
            return;
        }
        let mut off = 0;
        self.prg_bank0 = read_u8_slice(data, &mut off);
        self.prg_mode = read_u8_slice(data, &mut off);
        self.chr_banks.copy_from_slice(&data[off..off + 8]);
        off += 8;
        self.irq_latch = read_u8_slice(data, &mut off);
        self.irq_counter = read_u8_slice(data, &mut off);
        self.irq_enable = data[off] != 0;
        off += 1;
        self.irq_mode = data[off] != 0;
    }
}