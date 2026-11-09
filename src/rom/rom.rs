use std::fs;
use std::path::Path;

use super::mappers;
use super::mappers::MapperVariant;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Mirroring {
    VERTICAL,
    HORIZONTAL,
    FourScreen,
}

/// Parsed ROM header information with both iNES 1.0 and NES 2.0 fields.
pub struct Rom {
    pub prg_rom: Vec<u8>,
    pub chr_rom: Vec<u8>,
    pub mapper_id: u16,
    pub submapper_id: u8,
    pub screen_mirroring: Mirroring,
    pub has_chr_ram: bool,
    pub prg_ram_size: usize,
    pub chr_ram_size: usize,
    pub prg_ram_battery: bool,
    pub chr_ram_battery: bool,
    pub is_pal: bool,
}

impl Rom {
    pub fn new(raw: &[u8]) -> Result<Self, String> {
        if raw.len() < 16 || raw[0..4] != [0x4E, 0x45, 0x53, 0x1A] {
            return Err("Invalid iNES Header".to_string());
        }

        let flags6 = raw[6];
        let flags7 = raw[7];

        let ines_ver = (flags7 >> 2) & 0b11;
        let is_nes20 = ines_ver == 2;

        let mapper_id: u16 = if is_nes20 {
            let mapper_lsb = (flags7 & 0xF0) | (flags6 >> 4);
            let mapper_msb = ((raw[8] as u16) & 0x0F) << 8;
            mapper_msb | mapper_lsb as u16
        } else {
            ((flags7 & 0xF0) | (flags6 >> 4)) as u16
        };
        let submapper_id: u8 = if is_nes20 { (raw[8] >> 4) & 0x0F } else { 0 };

        let four_screen = (flags6 & 0b1000) != 0;
        let vertical_mirroring = (flags6 & 0b0001) != 0;
        let screen_mirroring = match (four_screen, vertical_mirroring) {
            (true, _) => Mirroring::FourScreen,
            (false, true) => Mirroring::VERTICAL,
            (false, false) => Mirroring::HORIZONTAL,
        };

        let prg_rom_multiplier: usize = if is_nes20 && raw[9] & 0x0F == 0x0F {
            let exponent = (raw[4] >> 2) as usize;
            let multiplier = ((raw[4] & 0x03) as usize) * 2 + 1;
            (1usize << exponent) * multiplier * 16384
        } else {
            let msb = if is_nes20 { ((raw[9] & 0x0F) as usize) << 8 } else { 0 };
            ((msb | raw[4] as usize).max(1)) * 16384
        };
        let chr_rom_multiplier: usize = if is_nes20 && raw[9] & 0xF0 == 0xF0 {
            let exponent = (raw[5] >> 2) as usize;
            let multiplier = ((raw[5] & 0x03) as usize) * 2 + 1;
            (1usize << exponent) * multiplier * 8192
        } else {
            let msb = if is_nes20 { (((raw[9] as usize) >> 4) & 0x0F) << 8 } else { 0 };
            ((msb | raw[5] as usize)) * 8192
        };

        let chr_size = chr_rom_multiplier;
        let has_chr_ram = chr_size == 0;
        let skip_trainer = (flags6 & 0b0100) != 0;

        let prg_ram_size: usize = if is_nes20 {
            let shift = raw[10] & 0x0F;
            if shift == 0 {
                0
            } else {
                64 << shift as usize
            }
        } else {
            8192
        };

        let chr_ram_size: usize = if is_nes20 {
            let shift = raw[11] & 0x0F;
            if shift == 0 && !has_chr_ram {
                0
            } else if shift == 0 && has_chr_ram {
                8192
            } else {
                64 << shift as usize
            }
        } else if has_chr_ram {
            8192
        } else {
            0
        };

        let prg_ram_battery: bool = if is_nes20 {
            (raw[10] & 0x80) != 0
        } else {
            (flags6 & 0x02) != 0
        };
        let chr_ram_battery: bool = if is_nes20 {
            (raw[10] & 0x40) != 0
        } else {
            (flags6 & 0x02) != 0 && has_chr_ram
        };

        let is_pal: bool = if is_nes20 { (raw[12] & 0x01) != 0 } else { false };

        let prg_start = 16 + if skip_trainer { 512 } else { 0 };
        let chr_start = prg_start + prg_rom_multiplier;

        if raw.len() < prg_start + prg_rom_multiplier {
            return Err(format!(
                "ROM too short for PRG data: need {} bytes, have {}",
                prg_start + prg_rom_multiplier,
                raw.len()
            ));
        }

        let prg_rom = raw[prg_start..prg_start + prg_rom_multiplier].to_vec();

        let chr_rom = if chr_size > 0 {
            if raw.len() < chr_start + chr_size {
                return Err(format!(
                    "ROM too short for CHR data: need {} bytes, have {}",
                    chr_start + chr_size,
                    raw.len()
                ));
            }
            raw[chr_start..chr_start + chr_size].to_vec()
        } else {
            Vec::new()
        };

        let rom_obj = Rom {
            prg_rom,
            chr_rom,
            mapper_id,
            submapper_id,
            screen_mirroring,
            has_chr_ram,
            prg_ram_size,
            chr_ram_size,
            prg_ram_battery,
            chr_ram_battery,
            is_pal,
        };

        println!(
            "ROM loaded: mapper={}{}{}, prg_size={}K, chr_size={}K, mirroring={:?}, battery={}, prg_ram={}K, chr_ram={}K, pal={}",
            rom_obj.mapper_id,
            if rom_obj.submapper_id > 0 {
                format!(".{}", rom_obj.submapper_id)
            } else {
                String::new()
            },
            if is_nes20 { " (NES 2.0)" } else { " (iNES 1.0)" },
            rom_obj.prg_rom.len() / 1024,
            rom_obj.chr_rom.len() / 1024,
            rom_obj.screen_mirroring,
            rom_obj.prg_ram_battery || rom_obj.chr_ram_battery,
            rom_obj.prg_ram_size / 1024,
            rom_obj.chr_ram_size / 1024,
            rom_obj.is_pal,
        );
        Ok(rom_obj)
    }

    pub fn get_mapper(self) -> MapperVariant {
        match self.mapper_id {
            0 => MapperVariant::Mapper0(mappers::Mapper0::new(self.prg_rom, self.chr_rom)),
            1 => MapperVariant::Mapper1(mappers::Mapper1::new(
                self.prg_rom,
                self.chr_rom,
                self.prg_ram_battery,
            )),
            2 => MapperVariant::Mapper2(mappers::Mapper2::new(self.prg_rom, self.chr_rom)),
            3 => MapperVariant::Mapper3(mappers::Mapper3::new(self.prg_rom, self.chr_rom)),
            4 => MapperVariant::Mapper4(mappers::Mapper4::new(
                self.prg_rom,
                self.chr_rom,
                self.screen_mirroring,
                self.prg_ram_battery,
            )),
            7 => MapperVariant::Mapper7(mappers::Mapper7::new(self.prg_rom)),
            5 => MapperVariant::Mapper5(mappers::Mapper5::new(
                self.prg_rom,
                self.chr_rom,
                self.prg_ram_battery,
            )),
            24 | 26 => {
                let has_chr = !self.chr_rom.is_empty();
                MapperVariant::Vrc6(mappers::Vrc6::new(
                    self.prg_rom,
                    if has_chr { self.chr_rom } else { vec![0; 8192] },
                    self.mapper_id as u8,
                    self.screen_mirroring,
                    !has_chr,
                ))
            }
            69 => MapperVariant::Fme7(mappers::Fme7::new(
                self.prg_rom,
                if !self.chr_rom.is_empty() {
                    self.chr_rom
                } else {
                    vec![0; 8192]
                },
                self.has_chr_ram,
                self.prg_ram_battery,
            )),
            _ => panic!("不支援 Mapper {}", self.mapper_id),
        }
    }

    pub fn has_chr_ram(&self) -> bool {
        self.has_chr_ram
    }
}

pub fn rom_entry<P: AsRef<Path>>(path: P) -> Result<Rom, String> {
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let rom = Rom::new(&bytes)?;
    if rom.prg_ram_battery {
        let sav_path = path.as_ref().with_extension("sav");
        if sav_path.exists() {
            println!("Battery save file found: {:?}", sav_path);
        }
    }
    Ok(rom)
}