//! Cartridge — ROM + Mapper 的黑盒封裝。
//!
//! 模擬器核心（NesConsole）只接受 `Cartridge`，完全不知道檔案路徑、
//! iNES header 格式或 ROM 載入細節。這是 ROM 成為「黑盒輸入」的關鍵。

use crate::rom::{self, MapperVariant, Mirroring};

/// 完整的卡匣封裝：包含 mapper（bank switching / IRQ）與鏡像模式。
/// 模擬器核心僅透過 `mapper` 欄位與 `init_chr_ram()` 使用卡匣資料。
pub struct Cartridge {
    pub mapper: MapperVariant,
    pub mirroring: Mirroring,
    has_chr_ram: bool,
}

impl Cartridge {
    /// 從 `.nes` 檔案路徑載入卡匣。
    /// 這是唯一一個知道檔案與 iNES 格式的地方 — 平台層負責呼叫。
    pub fn from_file(path: &str) -> Result<Self, String> {
        let rom = rom::rom_entry(path)?;
        let mirroring = rom.screen_mirroring;
        let has_chr_ram = rom.has_chr_ram();
        let mapper = rom.get_mapper();
        Ok(Self {
            mapper,
            mirroring,
            has_chr_ram,
        })
    }

    /// 將 CHR-ROM 資料寫入 PPU 的 chr_ram。
    /// CHR-RAM mappers 不需要呼叫此方法（遊戲透過 $2007 寫入圖形資料）。
    pub fn init_chr_ram(&self, chr_ram: &mut [u8; 8192]) {
        if !self.has_chr_ram {
            for i in 0..8192 {
                chr_ram[i] = self.mapper.read_chr(i as u16);
            }
        }
    }
}