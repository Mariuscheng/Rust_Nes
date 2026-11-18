//! Joypad (Controller) — NES 手柄完整實作
//!
//! 支援按鍵狀態設定、strobe latch 以及 NES 標準的序列讀出

/// NES 按鍵定義（bit 0..7 的讀出順序）
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Button {
    A,
    B,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

/// NES 手柄：以 8-bit 狀態暫存器保存按鍵，序列輸出模擬硬體行為
pub struct Joypad {
    state: u8, // 當前按鍵狀態（即時更新）
    shift: u8, // latch 後的序列移位暫存器
    strobe: bool,
    /// Bits already shifted out since the last latch. Once the eight buttons
    /// have been read the data line floats high, so further reads return 1;
    /// `dmc_dma_during_read4/dma_4016_read` counts on exactly that.
    bits_read: u8,
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            state: 0,
            shift: 0,
            strobe: false,
            bits_read: 0,
        }
    }

    fn button_bit(button: Button) -> u8 {
        match button {
            Button::A => 0,
            Button::B => 1,
            Button::Select => 2,
            Button::Start => 3,
            Button::Up => 4,
            Button::Down => 5,
            Button::Left => 6,
            Button::Right => 7,
        }
    }

    /// 設定按鍵狀態（由外部輸入層呼叫）
    pub fn set_button(&mut self, button: Button, pressed: bool) {
        let bit = 1u8 << Self::button_bit(button);
        if pressed {
            self.state |= bit;
        } else {
            self.state &= !bit;
        }
        // strobe 高電位時持續 latch
        if self.strobe {
            self.shift = self.state;
        }
    }

    /// CPU 寫入 $4016（strobe 控制，同時影響兩個手柄）
    pub fn cpu_write(&mut self, value: u8) {
        self.strobe = (value & 1) != 0;
        if self.strobe {
            self.shift = self.state; // 立即 latch
            self.bits_read = 0;
        }
    }

    /// CPU 讀取 $4016/$4017（回傳 bit0，每次讀取後右移）
    pub fn cpu_read(&mut self) -> u8 {
        if self.strobe {
            return self.shift & 1;
        }
        if self.bits_read >= 8 {
            // The shift register has run out; the data line idles high.
            return 1;
        }
        let res = self.shift & 1;
        self.shift >>= 1;
        self.bits_read += 1;
        res
    }
}

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/joypad_tests.rs"]
mod tests;
