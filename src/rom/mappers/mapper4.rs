//! Mapper 4 — MMC3, including the A12 scanline IRQ counter.

use super::super::mapper::{read_u8_slice, write_u8_vec};
use super::Mapper;
use super::Mirroring;

pub struct Mapper4 {
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    mirroring: Mirroring,
    bank_select: u8,
    bank_regs: [u8; 8],
    prg_mode: u8,
    chr_mode: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_enable: bool,
    irq_reload: bool,
    prg_ram: [u8; 8192],
    /// Bit i set = 1 KiB CHR block i may have changed (see `take_chr_dirty_blocks`).
    /// Starts at 0xFF so the boot-time refresh copies all of CHR.
    chr_dirty: u8,
}

impl Mapper4 {
    pub fn new(prg_rom: Vec<u8>, chr_rom: Vec<u8>, mirroring: Mirroring, battery: bool) -> Self {
        let mut mapper = Self {
            prg_rom,
            chr_rom,
            mirroring,
            bank_select: 0,
            bank_regs: [0; 8],
            prg_mode: 0,
            chr_mode: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_enable: false,
            irq_reload: false,
            prg_ram: [0; 8192],
            chr_dirty: 0xFF,
        };
        if battery {
            super::super::mapper::load_battery_save("mmc3", &mut mapper.prg_ram);
        }
        mapper
    }
}

impl Mapper for Mapper4 {
    fn read_prg(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => self
                .prg_ram
                .get((addr - 0x6000) as usize)
                .copied()
                .unwrap_or(0),
            0x8000..=0xFFFF => {
                let banks = self.prg_rom.len() / 0x2000;
                if banks == 0 {
                    return 0;
                }
                let bank_idx: usize = match addr {
                    0x8000..=0x9FFF => {
                        if self.prg_mode == 0 {
                            (self.bank_regs[6] as usize) % banks
                        } else {
                            banks.saturating_sub(2)
                        }
                    }
                    0xA000..=0xBFFF => (self.bank_regs[7] as usize) % banks,
                    0xC000..=0xDFFF => {
                        if self.prg_mode == 0 {
                            banks.saturating_sub(2)
                        } else {
                            (self.bank_regs[6] as usize) % banks
                        }
                    }
                    0xE000..=0xFFFF => banks.saturating_sub(1),
                    _ => 0,
                };
                self.prg_rom
                    .get(bank_idx * 0x2000 + (addr as usize % 0x2000))
                    .copied()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let data = &self.chr_rom;
        let banks = data.len() / 0x400;
        if banks == 0 {
            return 0;
        }
        let bank1k: usize = if self.chr_mode == 0 {
            match addr {
                0x0000..=0x03FF => ((self.bank_regs[0] & 0xFE) as usize) % banks,
                0x0400..=0x07FF => ((self.bank_regs[0] & 0xFE) as usize + 1) % banks,
                0x0800..=0x0BFF => ((self.bank_regs[1] & 0xFE) as usize) % banks,
                0x0C00..=0x0FFF => ((self.bank_regs[1] & 0xFE) as usize + 1) % banks,
                0x1000..=0x13FF => (self.bank_regs[2] as usize) % banks,
                0x1400..=0x17FF => (self.bank_regs[3] as usize) % banks,
                0x1800..=0x1BFF => (self.bank_regs[4] as usize) % banks,
                0x1C00..=0x1FFF => (self.bank_regs[5] as usize) % banks,
                _ => 0,
            }
        } else {
            match addr {
                0x0000..=0x03FF => (self.bank_regs[2] as usize) % banks,
                0x0400..=0x07FF => (self.bank_regs[3] as usize) % banks,
                0x0800..=0x0BFF => (self.bank_regs[4] as usize) % banks,
                0x0C00..=0x0FFF => (self.bank_regs[5] as usize) % banks,
                0x1000..=0x13FF => ((self.bank_regs[0] & 0xFE) as usize) % banks,
                0x1400..=0x17FF => ((self.bank_regs[0] & 0xFE) as usize + 1) % banks,
                0x1800..=0x1BFF => ((self.bank_regs[1] & 0xFE) as usize) % banks,
                0x1C00..=0x1FFF => ((self.bank_regs[1] & 0xFE) as usize + 1) % banks,
                _ => 0,
            }
        };
        data.get(bank1k * 0x400 + (addr as usize % 0x400))
            .copied()
            .unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, data: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram[(addr - 0x6000) as usize] = data;
            }
            0x8000..=0x9FFF => {
                if addr & 0x0001 == 0 {
                    let chr_mode = (data >> 7) & 1;
                    if chr_mode != self.chr_mode {
                        // The two 4 KiB windows swap, so every 1 KiB block remaps.
                        self.chr_dirty = 0xFF;
                    }
                    self.bank_select = data & 0x07;
                    self.prg_mode = (data >> 6) & 1;
                    self.chr_mode = chr_mode;
                } else {
                    self.bank_regs[self.bank_select as usize] = data;
                    // R0/R1 drive one 4 KiB half of the pattern table and R2..R5
                    // the other; chr_mode decides which half is where.
                    let half = if self.bank_select <= 1 {
                        if self.chr_mode == 0 { 0x0F } else { 0xF0 }
                    } else if self.chr_mode == 0 {
                        0xF0
                    } else {
                        0x0F
                    };
                    self.chr_dirty |= half;
                }
            }
            0xA000..=0xBFFF => {
                if addr & 0x0001 == 0 {
                    self.mirroring = if data & 0x01 != 0 {
                        Mirroring::HORIZONTAL
                    } else {
                        Mirroring::VERTICAL
                    };
                }
            }
            0xC000..=0xDFFF => {
                if addr & 0x0001 == 0 {
                    self.irq_latch = data;
                } else {
                    self.irq_reload = true;
                }
            }
            0xE000..=0xFFFF => {
                self.irq_enable = addr & 0x0001 != 0;
            }
            _ => {}
        }
    }

    fn has_chr_ram(&self) -> bool {
        self.chr_rom.is_empty()
    }

    fn take_chr_dirty_blocks(&mut self) -> u8 {
        std::mem::take(&mut self.chr_dirty)
    }

    fn tick_mapper_irq(&mut self) -> bool {
        if self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
            return false;
        }
        if self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
        } else {
            self.irq_counter -= 1;
            if self.irq_counter == 0 && self.irq_enable {
                return true;
            }
        }
        false
    }

    fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        write_u8_vec(&mut v, self.bank_select);
        v.extend_from_slice(&self.bank_regs);
        write_u8_vec(&mut v, self.prg_mode);
        write_u8_vec(&mut v, self.chr_mode);
        write_u8_vec(&mut v, self.irq_latch);
        write_u8_vec(&mut v, self.irq_counter);
        write_u8_vec(&mut v, if self.irq_enable { 1 } else { 0 });
        write_u8_vec(&mut v, if self.irq_reload { 1 } else { 0 });
        v.extend_from_slice(&self.prg_ram);
        v
    }

    fn load_state(&mut self, data: &[u8]) {
        if data.len() < 13 {
            return;
        }
        let mut off = 0;
        self.bank_select = read_u8_slice(data, &mut off);
        if data.len() >= 8 + off {
            self.bank_regs.copy_from_slice(&data[off..off + 8]);
            off += 8;
        }
        self.prg_mode = read_u8_slice(data, &mut off);
        self.chr_mode = read_u8_slice(data, &mut off);
        self.irq_latch = read_u8_slice(data, &mut off);
        self.irq_counter = read_u8_slice(data, &mut off);
        self.irq_enable = read_u8_slice(data, &mut off) != 0;
        self.irq_reload = read_u8_slice(data, &mut off) != 0;
        if data.len() >= off + 8192 {
            self.prg_ram.copy_from_slice(&data[off..off + 8192]);
        }
        // Restored banks may differ from what the PPU holds; force a full copy.
        self.chr_dirty = 0xFF;
    }
}
