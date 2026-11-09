//! APU Pulse 通道（方波 1 與方波 2）
//!
//! NES 有兩個方波通道，各自輸出不同 duty cycle 的矩形波。

use super::constants::{DUTY_TABLE, LENGTH_TABLE};
use super::envelope::Envelope;
use super::sweep::Sweep;

/// Pulse 通道（方波）
///
/// 輸出四種 duty cycle 的矩形波。
/// 包含 envelope、sweep、length counter 等子系統。
pub struct PulseChannel {
    /// 通道是否啟用（由 $4015 write 控制）
    pub enabled: bool,
    /// duty cycle 類型（0-3，對應 12.5%, 25%, 50%, 75%）
    pub duty: u8,
    /// 當前 duty 位置（0-7）
    pub duty_pos: u8,
    /// 計時器週期（11-bit）
    pub timer_period: u16,
    /// 當前計時器計數（向下計數至 0 時觸發 duty_pos 前進）
    pub timer_value: u16,
    /// 長度計數器（5-bit，0 = 靜音）
    pub length_counter: u8,
    /// 長度計數 halt flag（由 register 0 bit 5 控制）
    pub length_halt: bool,
    /// 波封產生器
    pub envelope: Envelope,
    /// 頻率掃描單元
    pub sweep: Sweep,
}

impl PulseChannel {
    /// 建立新的 Pulse 通道
    pub fn new(is_pulse1: bool) -> Self {
        Self {
            enabled: false,
            duty: 0,
            duty_pos: 0,
            timer_period: 0,
            timer_value: 0,
            length_counter: 0,
            length_halt: false,
            envelope: Envelope::new(),
            sweep: Sweep::new(is_pulse1),
        }
    }

    /// 處理 register write（reg 0-3，對應 $4000-$4003 或 $4004-$4007）
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.duty = (data >> 6) & 3;
                self.length_halt = (data & 0x20) != 0;
                self.envelope.loop_flag = self.length_halt;
                self.envelope.constant_volume = (data & 0x10) != 0;
                self.envelope.volume = data & 0x0F;
            }
            1 => {
                self.sweep.enabled = (data & 0x80) != 0;
                self.sweep.period = (data >> 4) & 7;
                self.sweep.negate = (data & 0x08) != 0;
                self.sweep.shift = data & 7;
                self.sweep.reload = true;
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
                self.envelope.start = true;
                self.duty_pos = 0;
            }
            _ => {}
        }
    }

    /// 每個 CPU cycle 呼叫一次（或每兩個 cycle 一次，取決於 APU 內部時序）
    pub fn clock_timer(&mut self) {
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            self.duty_pos = (self.duty_pos + 1) % 8;
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
            || DUTY_TABLE[self.duty as usize][self.duty_pos as usize] == 0
            || self.sweep.muting(self.timer_period)
        {
            0
        } else {
            self.envelope.output()
        }
    }
}