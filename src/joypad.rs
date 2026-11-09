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
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            state: 0,
            shift: 0,
            strobe: false,
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
        }
    }

    /// CPU 讀取 $4016/$4017（回傳 bit0，每次讀取後右移）
    pub fn cpu_read(&mut self) -> u8 {
        let res = self.shift & 1;
        if !self.strobe {
            self.shift >>= 1;
        }
        res
    }

    #[cfg(test)]
    pub fn reset(&mut self) {
        self.state = 0;
        self.shift = 0;
        self.strobe = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joypad_serial_reads() {
        let mut joy = Joypad::new();
        joy.set_button(Button::A, true);
        joy.set_button(Button::B, true);
        joy.cpu_write(1); // strobe high
        joy.cpu_write(0); // strobe low → latch

        assert_eq!(joy.cpu_read(), 1); // A
        assert_eq!(joy.cpu_read(), 1); // B
        assert_eq!(joy.cpu_read(), 0); // Select (not pressed)
    }

    #[test]
    fn joypad_reset_clears_state() {
        let mut joy = Joypad::new();
        joy.set_button(Button::Start, true);
        joy.reset();
        joy.cpu_write(1);
        joy.cpu_write(0);
        assert_eq!(joy.cpu_read(), 0);
    }

    #[test]
    fn joypad_strobe_high_continuously_returns_a() {
        let mut joy = Joypad::new();
        joy.set_button(Button::A, true);
        joy.cpu_write(1); // strobe high → 持續 latch
        // strobe 高電位期間，每次讀取都回傳 button A 的狀態
        assert_eq!(joy.cpu_read(), 1);
        assert_eq!(joy.cpu_read(), 1);
        assert_eq!(joy.cpu_read(), 1);
    }

    #[test]
    fn joypad_all_buttons_read_order() {
        let mut joy = Joypad::new();
        // 按下所有按鍵
        for &btn in &[
            Button::A,
            Button::B,
            Button::Select,
            Button::Start,
            Button::Up,
            Button::Down,
            Button::Left,
            Button::Right,
        ] {
            joy.set_button(btn, true);
        }
        joy.cpu_write(1);
        joy.cpu_write(0);
        // 所有 8 個 bit 都應該是 1
        for _ in 0..8 {
            assert_eq!(joy.cpu_read(), 1);
        }
        // 第 9 次讀取應該回傳 0（shift register 已空）
        assert_eq!(joy.cpu_read(), 0);
    }
}
