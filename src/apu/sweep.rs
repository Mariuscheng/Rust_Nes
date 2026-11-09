//! APU Sweep 單元（Pulse 1 專用）
//!
//! 控制方波頻率的自動上升/下降（portamento 效果）。

/// 頻率掃描單元（Sweep Unit）
///
/// 僅 Pulse 1 通道使用完整 sweep（Pulse 2 的 sweep 行為相同，
/// 但 negate 模式下 Pulse 2 少減 1 — 由 `is_pulse1` 標記區分）。
pub struct Sweep {
    /// sweep 是否啟用
    pub enabled: bool,
    /// sweep divider 重載值（週期）
    pub period: u8,
    /// 是否為負向掃描（頻率下降）
    pub negate: bool,
    /// 位移量（0-7，決定掃描速度）
    pub shift: u8,
    /// reload flag（寫入 register 1 時設為 true）
    pub reload: bool,
    /// 當前 divider 值
    pub divider: u8,
    /// 是否為 Pulse 1（Pulse 2 的 negate 行為略有不同）
    is_pulse1: bool,
}

impl Sweep {
    /// 建立新的 Sweep 單元
    pub fn new(is_pulse1: bool) -> Self {
        Self {
            enabled: false,
            period: 0,
            negate: false,
            shift: 0,
            reload: false,
            divider: 0,
            is_pulse1,
        }
    }

    /// 計算 sweep 後的目標週期值
    pub fn target_period(&self, current: u16) -> u16 {
        let change = current >> self.shift;
        if self.negate {
            current
                .wrapping_sub(change)
                .wrapping_sub(if self.is_pulse1 { 1 } else { 0 })
        } else {
            current.wrapping_add(change)
        }
    }

    /// 每個 frame half 呼叫一次，更新 timer period
    pub fn clock(&mut self, timer_period: &mut u16) {
        let target = self.target_period(*timer_period);
        if self.divider == 0
            && self.enabled
            && self.shift > 0
            && *timer_period >= 8
            && target <= 0x7FF
        {
            *timer_period = target;
        }
        if self.divider == 0 || self.reload {
            self.divider = self.period;
            self.reload = false;
        } else {
            self.divider -= 1;
        }
    }

    /// sweep 是否導致通道靜音
    pub fn muting(&self, timer_period: u16) -> bool {
        timer_period < 8 || self.target_period(timer_period) > 0x7FF
    }
}