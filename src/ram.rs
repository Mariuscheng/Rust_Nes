//! 2 KiB internal work RAM with mirroring.

/// [API] NES 內部工作 RAM (2KB)
pub struct Ram {
    data: [u8; 2048],
}

impl Ram {
    /// [API] 初始化 RAM，通常 NES 啟動時記憶體內容是隨機或全 0
    pub fn new() -> Self {
        Ram { data: [0; 2048] }
    }

    /// [API] 讀取數據：自動處理 0x0000 - 0x1FFF 之間的鏡像
    pub fn read(&self, addr: u16) -> u8 {
        // 使用遮罩 (0x07FF = 2047) 達到快速取模的效果，處理 4 次鏡像
        let mirror_addr = (addr & 0x07FF) as usize;
        self.data[mirror_addr]
    }

    /// [API] 寫入數據：同樣處理鏡像邏輯
    pub fn write(&mut self, addr: u16, value: u8) {
        let mirror_addr = (addr & 0x07FF) as usize;
        self.data[mirror_addr] = value;
    }

    /// Return a reference to the raw RAM array (for save states).
    pub fn raw_data(&self) -> &[u8; 2048] {
        &self.data
    }

    /// Bulk-write raw RAM data (for load states).
    pub fn write_raw(&mut self, data: &[u8]) {
        let len = data.len().min(2048);
        self.data[..len].copy_from_slice(&data[..len]);
    }
}
