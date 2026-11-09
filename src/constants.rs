//! NES 硬體常數集中定義
//! 全域使用的位址、計時與尺寸常數。

// ── CPU 相關 ──
/// NMI 中斷向量低位址
pub const NMI_VECTOR_LO: u16 = 0xFFFA;
/// NMI 中斷向量高位址
pub const NMI_VECTOR_HI: u16 = 0xFFFB;
/// IRQ/BRK 中斷向量低位址
pub const IRQ_VECTOR_LO: u16 = 0xFFFE;
/// IRQ/BRK 中斷向量高位址
pub const IRQ_VECTOR_HI: u16 = 0xFFFF;
/// RESET 向量低位址
pub const RESET_VECTOR_LO: u16 = 0xFFFC;
/// RESET 向量高位址
pub const RESET_VECTOR_HI: u16 = 0xFFFD;

// ── PPU 相關 ──
/// 每條 scanline 的 PPU dots
pub const DOTS_PER_SCANLINE: u16 = 341;
/// 每個完整幀的 scanline 數
pub const SCANLINES_PER_FRAME: u16 = 262;

// ── 音頻 ──
/// 預設音頻取樣率
pub const DEFAULT_SAMPLE_RATE: u32 = 44_100;

// ── 匯流排記憶體位址範圍 ──
/// PRG ROM 起點 ($8000)
pub const PRG_ROM_START: u16 = 0x8000;