//! APU Envelope 單元（共用於 Pulse 與 Noise 通道）
//!
//! 控制音量衰減（decay）或固定音量輸出。

/// 波封產生器（Envelope Generator）
///
/// 控制 Pulse 與 Noise 通道的音量變化。
/// 支援三種模式：衰減、循環、固定音量。
pub struct Envelope {
    /// 觸發 start flag（寫入 register 3 時設為 true）
    pub start: bool,
    /// 循環模式（等於 length halt flag）
    pub loop_flag: bool,
    /// 固定音量模式（跳過 decay，直接輸出 volume 值）
    pub constant_volume: bool,
    /// 設定音量（固定模式）或衰減週期（divider reload 值）
    pub volume: u8,
    /// 當前衰減等級（0-15，越小 = 越安靜）
    pub decay_level: u8,
    /// 衰減速度 divider（向下計數，0 時觸發一次 decay clock）
    pub divider: u8,
}

impl Envelope {
    /// 建立新的 Envelope，預設靜音
    pub fn new() -> Self {
        Self {
            start: false,
            loop_flag: false,
            constant_volume: false,
            volume: 0,
            decay_level: 0,
            divider: 0,
        }
    }

    /// 每個 frame quarter 呼叫一次
    pub fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay_level = 15;
            self.divider = self.volume;
        } else if self.divider > 0 {
            self.divider -= 1;
        } else {
            self.divider = self.volume;
            if self.decay_level > 0 {
                self.decay_level -= 1;
            } else if self.loop_flag {
                self.decay_level = 15;
            }
        }
    }

    /// 取得當前輸出音量（0-15）
    pub fn output(&self) -> u8 {
        if self.constant_volume {
            self.volume
        } else {
            self.decay_level
        }
    }
}