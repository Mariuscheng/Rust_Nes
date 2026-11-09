//! APU Noise 通道
//!
//! 輸出 pseudo-random 噪音，使用 LFSR 產生。

use super::constants::{LENGTH_TABLE, NOISE_PERIOD_TABLE};
use super::envelope::Envelope;

/// Noise 通道
///
/// 使用 15-bit LFSR 產生 pseudo-random 噪音。
/// 支援兩種模式（"long" / "short"），由 register 2 bit 7 控制。
pub struct NoiseChannel {
    /// 通道是否啟用
    pub enabled: bool,
    /// LFSR 模式：false = long (bit 1 feedback)，true = short (bit 6 feedback)
    pub mode: bool,
    /// 計時器週期（來自 lookup table）
    pub timer_period: u16,
    /// 當前計時器計數
    pub timer_value: u16,
    /// 長度計數器
    pub length_counter: u8,
    /// 長度計數 halt flag
    pub length_halt: bool,
    /// 波封產生器
    pub envelope: Envelope,
    /// 15-bit LFSR 位移暫存器
    pub shift_register: u16,
}

impl NoiseChannel {
    /// 建立新的 Noise 通道
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: false,
            timer_period: 0,
            timer_value: 0,
            length_counter: 0,
            length_halt: false,
            envelope: Envelope::new(),
            shift_register: 1,
        }
    }

    /// 處理 register write（reg 0/2/3，對應 $400C/$400E/$400F）
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.length_halt = (data & 0x20) != 0;
                self.envelope.loop_flag = self.length_halt;
                self.envelope.constant_volume = (data & 0x10) != 0;
                self.envelope.volume = data & 0x0F;
            }
            2 => {
                self.mode = (data & 0x80) != 0;
                self.timer_period =
                    NOISE_PERIOD_TABLE[(data & 0x0F) as usize];
                self.shift_register = 1;
            }
            3 => {
                if self.enabled {
                    self.length_counter =
                        LENGTH_TABLE[(data >> 3) as usize];
                }
                self.envelope.start = true;
            }
            _ => {}
        }
    }

    /// 每個 APU 偶數 cycle 呼叫一次
    pub fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            let bit = if self.mode { 6 } else { 1 };
            let feedback =
                (self.shift_register & 1) ^ ((self.shift_register >> bit) & 1);
            self.shift_register >>= 1;
            self.shift_register |= feedback << 14;
        } else {
            self.timer_value -= 1;
        }
    }

    /// 每個 frame half 呼叫一次
    pub fn clock_length(&mut self) {
        if !self.length_halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    /// 取得當前輸出音量（0-15）
    pub fn output(&self) -> u8 {
        if !self.enabled
            || self.length_counter == 0
            || (self.shift_register & 1) != 0
        {
            0
        } else {
            self.envelope.output()
        }
    }
}