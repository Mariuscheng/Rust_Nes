//! PPU register bitfields (`PPUCTRL` / `PPUMASK` / `PPUSTATUS`).

#[derive(Debug, Default)]
pub struct PpuStatus {
    pub vblank: bool,
    pub sprite_zero_hit: bool,
    pub sprite_overflow: bool,
}

impl PpuStatus {
    pub fn as_u8(&self) -> u8 {
        let mut res = 0u8;
        if self.vblank {
            res |= 0x80;
        }
        if self.sprite_zero_hit {
            res |= 0x40;
        }
        if self.sprite_overflow {
            res |= 0x20;
        }
        res
    }
}

#[derive(Debug)]
pub struct PpuControl {
    pub nmi_output: bool,
    pub vram_increment: u8,           // 1 or 32
    pub nametable_addr: u8,           // 0..3
    pub sprite_pattern_addr: u16,     // 0x0000 or 0x1000
    pub background_pattern_addr: u16, // 0x0000 or 0x1000
    pub sprite_size: bool,            // false=8x8, true=8x16
}

impl Default for PpuControl {
    fn default() -> Self {
        Self {
            nmi_output: false,
            vram_increment: 1, // NES 預設為 1（PPUCTRL bit 2 = 0）
            nametable_addr: 0,
            sprite_pattern_addr: 0x0000,
            background_pattern_addr: 0x0000,
            sprite_size: false,
        }
    }
}

impl PpuControl {
    pub fn update(&mut self, val: u8) {
        self.nmi_output = (val & 0x80) != 0;
        self.sprite_size = (val & 0x20) != 0;
        self.background_pattern_addr = if (val & 0x10) != 0 { 0x1000 } else { 0x0000 };
        self.sprite_pattern_addr = if (val & 0x08) != 0 { 0x1000 } else { 0x0000 };
        self.vram_increment = if (val & 0x04) != 0 { 32 } else { 1 };
        self.nametable_addr = val & 0x03;
    }
}
