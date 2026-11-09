// src/rom/mapper.rs
// Mapper trait 定義與輔助函式

use std::fs;
use std::path::Path;

// ── Helper functions ──

pub(super) fn write_u8_vec(v: &mut Vec<u8>, val: u8) {
    v.push(val);
}
pub(super) fn read_u8_slice(data: &[u8], off: &mut usize) -> u8 {
    let v = data[*off];
    *off += 1;
    v
}

/// Attempt to load a battery-backed save file into the given buffer.
/// Returns true if a save file existed and was loaded.
pub fn load_battery_save<PRAM: AsMut<[u8]>>(path: &str, ram: &mut PRAM) -> bool {
    let sav_path = Path::new(path).with_extension("sav");
    if sav_path.exists() {
        if let Ok(data) = fs::read(&sav_path) {
            let ram = ram.as_mut();
            let len = data.len().min(ram.len());
            ram[..len].copy_from_slice(&data[..len]);
            return true;
        }
    }
    false
}

// ── Mapper trait ──

pub trait Mapper {
    fn read_prg(&self, addr: u16) -> u8;
    fn read_chr(&self, addr: u16) -> u8;
    fn write_prg(&mut self, _addr: u16, _data: u8) {}
    /// 是否使用 CHR-RAM（由 mapper 決定，PPU 持有實際 RAM）
    fn has_chr_ram(&self) -> bool {
        false
    }
    /// MMC3 IRQ tick: called by PPU on A12 rising edge (PPU cycle 256 end).
    /// Returns true if IRQ should be triggered.
    fn tick_mapper_irq(&mut self) -> bool {
        false
    }
    /// Serialize mapper-specific state to bytes for save states.
    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Restore mapper-specific state from bytes.
    fn load_state(&mut self, _data: &[u8]) {}

    // ── Extended audio (VRC6, MMC5, FME-7, etc.) ──
    /// Write to the mapper's extended audio registers ($5xxx or specific address ranges).
    fn write_audio_register(&mut self, _addr: u16, _data: u8) {}
    /// Whether this mapper has extended audio hardware.
    fn has_extended_audio(&self) -> bool {
        false
    }
}