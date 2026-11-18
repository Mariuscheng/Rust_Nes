//! `Mapper` trait and the helpers shared by mapper implementations.

use super::Mirroring;

// ── Mapper trait ──

pub trait Mapper {
    fn read_prg(&self, addr: u16) -> u8;
    fn read_chr(&self, addr: u16) -> u8;
    fn write_prg(&mut self, _addr: u16, _data: u8) {}
    /// 是否使用 CHR-RAM（由 mapper 決定，PPU 持有實際 RAM）
    fn has_chr_ram(&self) -> bool {
        false
    }
    /// Current nametable mirroring for mappers that can change it at runtime
    /// (MMC1, MMC3, VRC6). `None` means "keep the header's fixed mirroring".
    fn mirroring(&self) -> Option<Mirroring> {
        None
    }

    /// Restore the mapper's power-on register state.
    ///
    /// Hardware has no generic answer here: some mappers reset their registers,
    /// others keep them (or reset only part of them). Only the mappers whose
    /// state is actually observable across a console reset — MMC1's shift
    /// register and control value, MMC3's IRQ — override this; for everything
    /// else a game rewrites its bank registers at the start of its reset
    /// handler and leaves this alone.
    fn reset(&mut self) {}

    /// Battery-backed PRG RAM, if this cartridge has one. `Cartridge` uses it
    /// to load `<rom>.sav` at boot and write it back on exit.
    fn battery_ram(&self) -> Option<&[u8]> {
        None
    }
    /// Mutable view used when loading the save file at boot.
    fn battery_ram_mut(&mut self) -> Option<&mut [u8]> {
        None
    }
    /// Clock the mapper's IRQ counter once. The PPU calls this for every
    /// accepted A12 rising edge; VRC6/FME-7 reuse the same hook.
    ///
    /// Returns the IRQ line level after the clock, so a mapper that latches a
    /// pending IRQ (MMC3) keeps the line asserted until the game acknowledges.
    fn tick_mapper_irq(&mut self) -> bool {
        false
    }
    /// Current IRQ line level without clocking anything. The bus polls this
    /// after register writes so an acknowledge ($E000/$E001) drops the line in
    /// the same instruction.
    fn irq_line(&self) -> bool {
        false
    }
    /// 自上次呼叫以來，哪些 1 KiB CHR bank（bit i = $0000 + i*$400）的內容可能改變。
    ///
    /// 回傳 0xFF 代表「不確定，全部重抄」，這是預設值，所以 mapper 不實作也不會錯，
    /// 只是每次 register 寫入都要重抄 8 KiB。實作的 mapper 可以把不影響 CHR 的寫入
    /// （IRQ latch、mirroring、PRG bank 等）標成 0，讓 mid-frame 的熱路徑免去重抄。
    fn take_chr_dirty_blocks(&mut self) -> u8 {
        0xFF
    }
    /// Serialize mapper-specific state to bytes for save states.
    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Restore mapper-specific state from bytes.
    fn load_state(&mut self, _data: &[u8]) -> Option<()> {
        Some(())
    }

    // ── Extended audio (VRC6, MMC5, FME-7, etc.) ──
    /// Write to the mapper's extended audio registers ($5xxx or specific address ranges).
    fn write_audio_register(&mut self, _addr: u16, _data: u8) {}
    /// Whether this mapper has extended audio hardware.
    fn has_extended_audio(&self) -> bool {
        false
    }
}
