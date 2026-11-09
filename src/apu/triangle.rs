//! APU Triangle 通道
//!
//! 輸出固定波形的三角波，無音量控制（僅 on/off），
//! 但有獨立的 linear counter 替代 envelope。

use super::constants::{LENGTH_TABLE, TRIANGLE_SEQUENCE};

/// Triangle 通道
///
/// 輸出 32-step 三角波序列。
/// 無 envelope/sweep，但有 linear counter 用於自動衰減。
pub struct TriangleChannel {
    /// 通道是否啟用
    pub enabled: bool,
    /// 計時器週期（11-bit）
    pub timer_period: u16,
    /// 當前計時器計數
    pub timer_value: u16,
    /// 長度計數器
    pub length_counter: u8,
    /// 長度計數 halt flag
    pub length_halt: bool,
    /// linear counter 當前值
    pub linear_counter: u8,
    /// linear counter 重載值
    pub linear_reload_value: u8,
    /// linear counter reload flag（寫入 register 3 時設為 true）
    pub linear_reload_flag: bool,
    /// 當前波形序列位置（0-31）
    pub sequence_pos: u8,
}

impl TriangleChannel {
    /// 建立新的 Triangle 通道
    pub fn new() -> Self {
        Self {
            enabled: false,
            timer_period: 0,
            timer_value: 0,
            length_counter: 0,
            length_halt: false,
            linear_counter: 0,
            linear_reload_value: 0,
            linear_reload_flag: false,
            sequence_pos: 0,
        }
    }

    /// 處理 register write（reg 0/2/3，對應 $4008/$400A/$400B）
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.length_halt = (data & 0x80) != 0;
                self.linear_reload_value = data & 0x7F;
            }
            2 => {
                self.timer_period = (self.timer_period & 0x700) | data as u16;
            }
            3 => {
                self.timer_period =
                    (self.timer_period & 0xFF) | (((data & 7) as u16) << 8);
                if self.enabled {
                    self.length_counter =
                        LENGTH_TABLE[(data >> 3) as usize];
                }
                self.linear_reload_flag = true;
            }
            _ => {}
        }
    }

    /// 每個 CPU cycle 呼叫一次
    pub fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            if self.length_counter > 0 && self.linear_counter > 0 {
                self.sequence_pos = (self.sequence_pos + 1) % 32;
            }
        } else {
            self.timer_value -= 1;
        }
    }

    /// 每個 frame quarter 呼叫一次
    pub fn clock_linear_counter(&mut self) {
        if self.linear_reload_flag {
            self.linear_counter = self.linear_reload_value;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.length_halt {
            self.linear_reload_flag = false;
        }
    }

    /// 每個 frame half 呼叫一次
    pub fn clock_length(&mut self) {
        if !self.length_halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    /// 取得當前輸出值（0-15，4-bit）
    pub fn output(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 || self.linear_counter == 0 {
            0
        } else {
            TRIANGLE_SEQUENCE[self.sequence_pos as usize]
        }
    }
}