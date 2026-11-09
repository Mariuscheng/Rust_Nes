//! APU DMC (Delta Modulation Channel) 通道
//!
//! 輸出 7-bit PCM 樣本，可從記憶體直接讀取 sample data。
//! 支援循環播放與 IRQ 觸發。

use super::constants::DMC_PERIOD_TABLE;

/// DMC (Delta Modulation Channel) 通道
///
/// 輸出 7-bit DPCM（differential pulse-code modulation）音訊。
/// 從 CPU 記憶體中讀取 sample data，可觸發 IRQ 中斷。
pub struct DmcChannel {
    /// 通道是否啟用
    pub enabled: bool,
    /// 當前輸出電平（0-127）
    pub output_level: u8,
    /// IRQ enable flag（sample 播放完成時觸發中斷）
    pub irq_enabled: bool,
    /// 循環播放 flag
    pub loop_flag: bool,
    /// 計時器週期（來自 lookup table）
    pub timer_period: u16,
    /// 當前計時器計數
    pub timer_value: u16,
    /// sample 起始位址（$C000-$FFC0，64-byte aligned）
    pub sample_address: u16,
    /// sample 長度（bytes）
    pub sample_length: u16,
    /// 當前讀取位址
    pub current_address: u16,
    /// 剩餘 bytes 數
    pub bytes_remaining: u16,
    /// sample buffer（1 byte，存放從記憶體讀取的 8-bit sample）
    pub sample_buffer: u8,
    /// 剩餘 bits 數（sample buffer 中的剩餘位元數）
    pub bits_remaining: u8,
    /// 靜音 flag（sample buffer 為空且無資料可播放時為 true）
    pub silence_flag: bool,
    /// sample buffer 是否為空
    pub sample_buffer_empty: bool,
}

impl DmcChannel {
    /// 建立新的 DMC 通道
    pub fn new() -> Self {
        Self {
            enabled: false,
            output_level: 0,
            irq_enabled: false,
            loop_flag: false,
            timer_period: DMC_PERIOD_TABLE[0],
            timer_value: 0,
            sample_address: 0xC000,
            sample_length: 0,
            current_address: 0xC000,
            bytes_remaining: 0,
            sample_buffer: 0,
            bits_remaining: 8,
            silence_flag: true,
            sample_buffer_empty: true,
        }
    }

    /// 處理 register write（reg 0-3，對應 $4010-$4013）
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.irq_enabled = (data & 0x80) != 0;
                self.loop_flag = (data & 0x40) != 0;
                self.timer_period =
                    DMC_PERIOD_TABLE[(data & 0x0F) as usize];
            }
            1 => self.output_level = data & 0x7F,
            2 => {
                self.sample_address =
                    0xC000u16.wrapping_add((data as u16) << 6);
            }
            3 => self.sample_length = (data as u16) * 16 + 1,
            _ => {}
        }
    }

    /// 重載 sample 位址與長度
    pub fn reload_sample(&mut self) {
        self.current_address = self.sample_address;
        self.bytes_remaining = self.sample_length;
    }

    /// 嘗試從 bus 讀取一個 byte 填滿 sample buffer。
    ///
    /// 回傳 `(irq_occurred, did_read)`。
    pub fn fill_sample_buffer<'b>(
        &mut self,
        bus: &'b dyn crate::bus::BusInterface,
    ) -> (bool, bool) {
        if !self.sample_buffer_empty || self.bytes_remaining == 0 {
            return (false, false);
        }
        let val = bus.read(self.current_address);
        self.sample_buffer = val;
        self.sample_buffer_empty = false;
        self.silence_flag = false;
        if self.current_address == 0xFFFF {
            self.current_address = 0x8000;
        } else {
            self.current_address = self.current_address.wrapping_add(1);
        }
        self.bytes_remaining = self.bytes_remaining.saturating_sub(1);
        let mut irq_occurred = false;
        if self.bytes_remaining == 0 {
            if self.loop_flag {
                self.reload_sample();
            } else if self.irq_enabled {
                irq_occurred = true;
            }
        }
        (irq_occurred, true)
    }

    /// 每個 CPU cycle 呼叫一次，處理 DMC timer 與 output level。
    ///
    /// 回傳 `(irq_occurred, read_count)`，其中 `read_count` 表示
    /// DMC 從 bus 讀取了多少 bytes（每個 byte 對應 4 CPU cycle 被挪用）。
    pub fn clock_timer<'b>(
        &mut self,
        bus: &'b dyn crate::bus::BusInterface,
    ) -> (bool, u8) {
        let mut reads: u8 = 0;
        let mut irq = false;
        let mut reloaded = false;
        if self.timer_value == 0 {
            self.timer_value = self.timer_period;
            reloaded = true;
            if !self.silence_flag {
                if (self.sample_buffer & 1) != 0 {
                    if self.output_level <= 125 {
                        self.output_level =
                            self.output_level.saturating_add(2);
                    }
                } else if self.output_level >= 2 {
                    self.output_level =
                        self.output_level.saturating_sub(2);
                }
                self.sample_buffer >>= 1;
            }
            if self.bits_remaining > 0 {
                self.bits_remaining -= 1;
            }
            if self.bits_remaining == 0 {
                self.bits_remaining = 8;
                if self.sample_buffer_empty {
                    self.silence_flag = true;
                } else {
                    self.silence_flag = false;
                    self.sample_buffer_empty = true;
                }
            }
        }
        let (dmc_irq, did_read) = self.fill_sample_buffer(bus);
        if did_read {
            reads = 1;
        }
        if dmc_irq {
            irq = true;
        }
        if !reloaded && self.timer_value > 0 {
            self.timer_value -= 1;
        }
        (irq, reads)
    }

    /// 取得當前輸出值（0-127）
    pub fn output(&self) -> u8 {
        self.output_level
    }
}