//! Cartridge — ROM + Mapper 的黑盒封裝。
//!
//! 模擬器核心（NesConsole）只接受 `Cartridge`，完全不知道檔案路徑、
//! iNES header 格式或 ROM 載入細節。這是 ROM 成為「黑盒輸入」的關鍵。

use std::path::{Path, PathBuf};

use crate::rom::{self, MapperVariant, Mirroring};

/// 完整的卡匣封裝：包含 mapper（bank switching / IRQ）與鏡像模式。
/// 模擬器核心僅透過 `mapper` 欄位與 `init_chr_ram()` 使用卡匣資料。
pub struct Cartridge {
    pub mapper: MapperVariant,
    pub mirroring: Mirroring,
    has_chr_ram: bool,
    /// `<rom>.sav` when this cartridge has battery-backed PRG RAM.
    battery_path: Option<PathBuf>,
}

impl Cartridge {
    /// 從 `.nes` 檔案路徑載入卡匣。
    /// 這是唯一一個知道檔案與 iNES 格式的地方 — 平台層負責呼叫。
    ///
    /// Battery-backed cartridges load `<rom>.sav` next to the ROM; the platform
    /// layer calls [`NesConsole::save_battery`](crate::nes::NesConsole::save_battery)
    /// to write it back.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let rom = rom::rom_entry(path)?;
        let mirroring = rom.screen_mirroring;
        let has_chr_ram = rom.has_chr_ram();
        let battery = rom.prg_ram_battery;
        let mut mapper = rom.get_mapper();

        let battery_path = battery.then(|| Path::new(path).with_extension("sav"));
        if let Some(sav) = &battery_path
            && let Ok(data) = std::fs::read(sav)
            && let Some(ram) = mapper.battery_ram_mut()
        {
            let len = data.len().min(ram.len());
            ram[..len].copy_from_slice(&data[..len]);
        }

        Ok(Self {
            mapper,
            mirroring,
            has_chr_ram,
            battery_path,
        })
    }

    /// True when the cartridge exposes CHR RAM (writable pattern memory).
    pub fn is_chr_ram(&self) -> bool {
        self.has_chr_ram
    }

    /// Where this cartridge's battery save lives, if it has one.
    pub fn battery_path(&self) -> Option<&Path> {
        self.battery_path.as_deref()
    }

    /// 將 CHR-ROM 資料寫入 PPU 的 chr_ram。
    /// CHR-RAM mappers 不需要呼叫此方法（遊戲透過 $2007 寫入圖形資料）。
    pub fn init_chr_ram(&self, chr_ram: &mut [u8; 8192]) {
        if !self.has_chr_ram {
            for (addr, byte) in chr_ram.iter_mut().enumerate() {
                *byte = self.mapper.read_chr(addr as u16);
            }
        }
    }
}
