//! APU (Audio Processing Unit) — NES 完整音頻處理單元
//!
//! 包含五個聲道：Pulse 1/2、Triangle、Noise、DMC，
//! 以及 Frame Counter、混音、sample rate 轉換等子系統。

pub mod constants;
pub mod dmc;
pub mod envelope;
pub mod noise;
pub mod pulse;
pub mod sweep;
pub mod triangle;

use self::constants::CPU_FREQ_NUMER;
use self::dmc::DmcChannel;
use self::noise::NoiseChannel;
use self::pulse::PulseChannel;
use self::triangle::TriangleChannel;

/// NES APU（Audio Processing Unit）
///
/// 整合五個音頻通道與 frame counter，
/// 提供 per-cycle stepping、register I/O、混音輸出與 save state。
pub struct APU {
    pulse1: PulseChannel,
    pulse2: PulseChannel,
    triangle: TriangleChannel,
    noise: NoiseChannel,
    dmc: DmcChannel,
    frame_mode: u8,
    frame_irq_inhibit: bool,
    /// Frame counter IRQ 是否已觸發（讀取 $4015 時清除）
    pub frame_interrupt: bool,
    /// DMC IRQ 是否已觸發
    pub dmc_interrupt: bool,
    frame_step: i64,
    /// Off-by-one offset for odd-cycle alignment: 0 (even) or -1 (odd).
    /// When -1, the first tick increments to 0 (effectively skipping one count)
    /// and the final combined event resets back to -1, keeping the sequence
    /// length at 29830 ticks.
    frame_step_offset: i64,
    sample_accumulator: u64,
    cycles_per_sample_numer: u64,
    audio_buffer: Vec<f32>,
    hpf_last_input: f32,
    hpf_last_output: f32,
    hpf_alpha: f32,
    total_cpu_cycles: u64,
}

impl APU {
    /// 建立新的 APU 實例
    pub fn new(sample_rate: u32) -> Self {
        Self {
            pulse1: PulseChannel::new(true),
            pulse2: PulseChannel::new(false),
            triangle: TriangleChannel::new(),
            noise: NoiseChannel::new(),
            dmc: DmcChannel::new(),
            frame_mode: 0,
            frame_irq_inhibit: true,
            frame_interrupt: false,
            dmc_interrupt: false,
            frame_step: 0,
            frame_step_offset: 0,
            sample_accumulator: 0,
            cycles_per_sample_numer: sample_rate as u64,
            audio_buffer: Vec::with_capacity(sample_rate as usize / 60 + 32),
            hpf_last_input: 0.0,
            hpf_last_output: 0.0,
            hpf_alpha: {
                let cutoff = 20.0_f32;
                let dt = 1.0_f32 / sample_rate as f32;
                let rc = 1.0_f32 / (2.0 * std::f32::consts::PI * cutoff);
                rc / (rc + dt)
            },
            total_cpu_cycles: 0,
        }
    }

    /// 處理 APU register write（$4000-$4017）。
    ///
    /// 若 `$4015` 的寫入讓 DMC 需要立即取得 sample，回傳要讀取的位址；
    /// bus 讀取後以 `fill_sample()` 回填。
    pub fn write_register(&mut self, addr: u16, data: u8) -> Option<u16> {
        match addr {
            0x4000..=0x4003 => {
                self.pulse1.write_reg((addr - 0x4000) as u8, data);
            }
            0x4004..=0x4007 => {
                self.pulse2.write_reg((addr - 0x4004) as u8, data);
            }
            0x4008 => self.triangle.write_reg(0, data),
            0x400A => self.triangle.write_reg(2, data),
            0x400B => self.triangle.write_reg(3, data),
            0x400C => self.noise.write_reg(0, data),
            0x400E => self.noise.write_reg(2, data),
            0x400F => self.noise.write_reg(3, data),
            0x4010..=0x4013 => {
                self.dmc.write_reg((addr - 0x4010) as u8, data);
                if addr == 0x4010 && !self.dmc.irq_enabled {
                    self.dmc_interrupt = false;
                }
            }
            0x4015 => {
                let enable_p1 = (data & 0x01) != 0;
                let enable_p2 = (data & 0x02) != 0;
                let enable_tri = (data & 0x04) != 0;
                let enable_noise = (data & 0x08) != 0;
                let enable_dmc = (data & 0x10) != 0;
                self.dmc_interrupt = false;
                if enable_dmc && self.dmc.bytes_remaining == 0 {
                    self.dmc.reload_sample();
                    self.dmc.bits_remaining = 8;
                    self.dmc.sample_buffer_empty = true;
                    self.dmc.silence_flag = true;
                }
                self.dmc.enabled = enable_dmc;
                if !self.dmc.enabled {
                    self.dmc.bytes_remaining = 0;
                }
                self.pulse1.enabled = enable_p1;
                if !enable_p1 {
                    self.pulse1.length_counter = 0;
                }
                self.pulse2.enabled = enable_p2;
                if !enable_p2 {
                    self.pulse2.length_counter = 0;
                }
                self.triangle.enabled = enable_tri;
                if !enable_tri {
                    self.triangle.length_counter = 0;
                }
                self.noise.enabled = enable_noise;
                if !enable_noise {
                    self.noise.length_counter = 0;
                }
                if enable_dmc && self.dmc.sample_buffer_empty && self.dmc.bytes_remaining > 0 {
                    return self.dmc.take_sample_request();
                }
            }
            0x4017 => {
                self.frame_mode = (data >> 7) & 1;
                self.frame_irq_inhibit = (data & 0x40) != 0;
                if self.frame_irq_inhibit {
                    self.frame_interrupt = false;
                }
                // 依照寫入當下的 CPU cycle 奇偶決定 jitter 模式。
                // Even cycle: offset=0，從 1 開始計數。
                // Odd cycle:  offset=-1，第一個 tick 變 0（skip），後續事件全部延遲 1 tick。
                self.frame_step_offset = if (self.total_cpu_cycles & 1) == 0 { 0 } else { -1 };
                self.frame_step = self.frame_step_offset;
                // mode 1 ($80)：寫入後立刻觸發一次 quarter + half，
                // 發生在 sequencer 重置之後，不計入 frame_step。
                if self.frame_mode == 1 {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
            _ => {}
        }
        None
    }

    /// 讀取 APU status register（$4015）。
    ///
    /// 回傳各通道 length counter 狀態與中斷 flags。
    /// 讀取後清除 frame interrupt。
    pub fn read_status(&mut self) -> u8 {
        let mut s = 0u8;
        if self.pulse1.length_counter > 0 {
            s |= 0x01;
        }
        if self.pulse2.length_counter > 0 {
            s |= 0x02;
        }
        if self.triangle.length_counter > 0 {
            s |= 0x04;
        }
        if self.noise.length_counter > 0 {
            s |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            s |= 0x10;
        }
        if self.frame_interrupt {
            s |= 0x40;
        }
        if self.dmc_interrupt {
            s |= 0x80;
        }
        self.frame_interrupt = false;
        s
    }

    /// Frame counter quarter-frame clock
    fn clock_quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear_counter();
    }

    /// Frame counter half-frame clock
    fn clock_half_frame(&mut self) {
        self.pulse1.clock_length();
        self.pulse2.clock_length();
        self.triangle.clock_length();
        self.noise.clock_length();
        self.pulse1.sweep.clock(&mut self.pulse1.timer_period);
        self.pulse2.sweep.clock(&mut self.pulse2.timer_period);
    }

    /// 每個 CPU cycle 呼叫一次，推進 frame counter。
    ///
    /// Even jitter (offset=0) 與 odd jitter (offset=-1) 共用相同的事件閾值。
    /// Odd jitter 因為從 -1 開始，第一個 tick 變 0（跳過，無事件），
    /// 後續所有事件因此延遲 1 個 tick。最後在 29829 合併 half/IRQ/reset
    /// 並回到 offset，維持總長 29830 ticks。
    ///
    ///   4-step mode: 29830 ticks per sequence
    ///   5-step mode: 37281 ticks per sequence
    fn frame_counter_tick(&mut self) {
        self.frame_step += 1;

        // ── 共用事件閾值 ──
        match self.frame_step {
            7457 => self.clock_quarter_frame(),
            14913 => {
                self.clock_quarter_frame();
                self.clock_half_frame();
            }
            22371 => self.clock_quarter_frame(),
            29829 => {
                // quarter + half always happens at 29829
                self.clock_quarter_frame();
                self.clock_half_frame();
                if self.frame_mode == 0 {
                    if self.frame_step_offset == 0 {
                        // Even jitter: IRQ 與 reset 推遲到下一 tick (29830)
                        // → on tick 29830, frame_step == 29830 處理
                    } else {
                        // Odd jitter: 合併 half/IRQ/reset 在同一 tick
                        if !self.frame_irq_inhibit {
                            self.frame_interrupt = true;
                        }
                        self.frame_step = self.frame_step_offset;
                    }
                }
                // Mode 1 (5-step): 不在此 reset，繼續等待 37281
            }
            29830 => {
                if self.frame_mode == 0 && self.frame_step_offset == 0 {
                    // Even jitter only: standalone IRQ + reset
                    if !self.frame_irq_inhibit {
                        self.frame_interrupt = true;
                    }
                    self.frame_step = self.frame_step_offset;
                }
                // Odd jitter 不會到達此處（已在 29829 reset 回 -1）
            }
            37281 => {
                // 5-step mode: last quarter+half+reset
                self.clock_quarter_frame();
                self.clock_half_frame();
                self.frame_step = self.frame_step_offset;
            }
            _ => {}
        }
    }

    /// 推進 APU 一個 CPU cycle。
    ///
    /// 若 DMC 需要新的 sample byte，回傳要讀取的位址；bus 讀取後以
    /// `fill_sample()` 回填（DMC 本身不做記憶體存取）。
    pub fn tick_one(&mut self) -> Option<u16> {
        self.total_cpu_cycles += 1;
        self.triangle.clock_timer();
        if self.total_cpu_cycles.is_multiple_of(2) {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
        }
        self.dmc.clock_timer();
        self.frame_counter_tick();
        self.generate_sample();
        self.dmc.take_sample_request()
    }

    /// 回填 DMC 的 sample byte（由 bus 讀取後呼叫）。
    pub fn fill_sample(&mut self, val: u8) {
        if self.dmc.fill_sample(val) {
            self.dmc_interrupt = true;
        }
    }

    /// 混音並產生一個 output sample（若有需要）
    fn generate_sample(&mut self) {
        self.sample_accumulator += self.cycles_per_sample_numer;
        if self.sample_accumulator >= CPU_FREQ_NUMER {
            self.sample_accumulator -= CPU_FREQ_NUMER;
            let raw = self.mix();
            let filtered = self.hpf_alpha
                * (self.hpf_last_output + raw - self.hpf_last_input);
            self.hpf_last_input = raw;
            self.hpf_last_output = filtered;
            self.audio_buffer.push(filtered);
        }
    }

    /// 非線性混音（模擬 NES 硬體的非線性輸出級）
    fn mix(&self) -> f32 {
        let p1 = self.pulse1.output() as f32;
        let p2 = self.pulse2.output() as f32;
        let pulse_out = if (p1 + p2) > 0.0 {
            95.88 / (8128.0 / (p1 + p2) + 100.0)
        } else {
            0.0
        };
        let tnd_sum = self.triangle.output() as f32 / 8227.0
            + self.noise.output() as f32 / 12241.0
            + self.dmc.output() as f32 / 22638.0;
        let tnd_out = if tnd_sum > 0.0 {
            159.79 / (1.0 / tnd_sum + 100.0)
        } else {
            0.0
        };
        pulse_out + tnd_out
    }

    /// 取出累積的音頻緩衝區並清空
    pub fn drain_audio_buffer(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio_buffer)
    }

    /// 序列化 APU 狀態為位元組向量（用於 save state）
    pub fn save_state(&self) -> Vec<u8> {
        let mut v = Vec::new();
        fn pu8(v: &mut Vec<u8>, val: u8) {
            v.push(val);
        }
        fn pu16(v: &mut Vec<u8>, val: u16) {
            v.extend_from_slice(&val.to_le_bytes());
        }
        fn pbool(v: &mut Vec<u8>, val: bool) {
            v.push(if val { 1 } else { 0 });
        }

        // Pulse 1
        pbool(&mut v, self.pulse1.enabled);
        pu8(&mut v, self.pulse1.duty);
        pu8(&mut v, self.pulse1.duty_pos);
        pu16(&mut v, self.pulse1.timer_period);
        pu16(&mut v, self.pulse1.timer_value);
        pu8(&mut v, self.pulse1.length_counter);
        pbool(&mut v, self.pulse1.length_halt);
        pbool(&mut v, self.pulse1.envelope.start);
        pbool(&mut v, self.pulse1.envelope.loop_flag);
        pbool(&mut v, self.pulse1.envelope.constant_volume);
        pu8(&mut v, self.pulse1.envelope.volume);
        pu8(&mut v, self.pulse1.envelope.decay_level);
        pu8(&mut v, self.pulse1.envelope.divider);
        pbool(&mut v, self.pulse1.sweep.enabled);
        pu8(&mut v, self.pulse1.sweep.period);
        pbool(&mut v, self.pulse1.sweep.negate);
        pu8(&mut v, self.pulse1.sweep.shift);
        pbool(&mut v, self.pulse1.sweep.reload);
        pu8(&mut v, self.pulse1.sweep.divider);

        // Pulse 2
        pbool(&mut v, self.pulse2.enabled);
        pu8(&mut v, self.pulse2.duty);
        pu8(&mut v, self.pulse2.duty_pos);
        pu16(&mut v, self.pulse2.timer_period);
        pu16(&mut v, self.pulse2.timer_value);
        pu8(&mut v, self.pulse2.length_counter);
        pbool(&mut v, self.pulse2.length_halt);
        pbool(&mut v, self.pulse2.envelope.start);
        pbool(&mut v, self.pulse2.envelope.loop_flag);
        pbool(&mut v, self.pulse2.envelope.constant_volume);
        pu8(&mut v, self.pulse2.envelope.volume);
        pu8(&mut v, self.pulse2.envelope.decay_level);
        pu8(&mut v, self.pulse2.envelope.divider);
        pbool(&mut v, self.pulse2.sweep.enabled);
        pu8(&mut v, self.pulse2.sweep.period);
        pbool(&mut v, self.pulse2.sweep.negate);
        pu8(&mut v, self.pulse2.sweep.shift);
        pbool(&mut v, self.pulse2.sweep.reload);
        pu8(&mut v, self.pulse2.sweep.divider);

        // Triangle
        pbool(&mut v, self.triangle.enabled);
        pu16(&mut v, self.triangle.timer_period);
        pu16(&mut v, self.triangle.timer_value);
        pu8(&mut v, self.triangle.length_counter);
        pbool(&mut v, self.triangle.length_halt);
        pu8(&mut v, self.triangle.linear_counter);
        pu8(&mut v, self.triangle.linear_reload_value);
        pbool(&mut v, self.triangle.linear_reload_flag);
        pu8(&mut v, self.triangle.sequence_pos);

        // Noise
        pbool(&mut v, self.noise.enabled);
        pbool(&mut v, self.noise.mode);
        pu16(&mut v, self.noise.timer_period);
        pu16(&mut v, self.noise.timer_value);
        pu8(&mut v, self.noise.length_counter);
        pbool(&mut v, self.noise.length_halt);
        pbool(&mut v, self.noise.envelope.start);
        pbool(&mut v, self.noise.envelope.loop_flag);
        pbool(&mut v, self.noise.envelope.constant_volume);
        pu8(&mut v, self.noise.envelope.volume);
        pu8(&mut v, self.noise.envelope.decay_level);
        pu8(&mut v, self.noise.envelope.divider);
        pu16(&mut v, self.noise.shift_register);

        // DMC
        pbool(&mut v, self.dmc.enabled);
        pu8(&mut v, self.dmc.output_level);
        pbool(&mut v, self.dmc.irq_enabled);
        pbool(&mut v, self.dmc.loop_flag);
        pu16(&mut v, self.dmc.timer_period);
        pu16(&mut v, self.dmc.timer_value);
        pu16(&mut v, self.dmc.sample_address);
        pu16(&mut v, self.dmc.sample_length);
        pu16(&mut v, self.dmc.current_address);
        pu16(&mut v, self.dmc.bytes_remaining);
        pu8(&mut v, self.dmc.sample_buffer);
        pu8(&mut v, self.dmc.bits_remaining);
        pbool(&mut v, self.dmc.silence_flag);
        pbool(&mut v, self.dmc.sample_buffer_empty);

        // Frame counter
        pu8(&mut v, self.frame_mode);
        pbool(&mut v, self.frame_irq_inhibit);
        pbool(&mut v, self.frame_interrupt);
        pbool(&mut v, self.dmc_interrupt);
        v.extend_from_slice(&self.frame_step.to_le_bytes());
        // frame_step_offset: 0 or -1 (stored as i64, 8 bytes)
        v.extend_from_slice(&self.frame_step_offset.to_le_bytes());

        v
    }

    /// 從位元組 slice 反序列化 APU 狀態
    pub fn load_state(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }
        let mut off = 0usize;

        fn rd(data: &[u8], off: &mut usize) -> u8 {
            let v = data[*off];
            *off += 1;
            v
        }
        fn rd16(data: &[u8], off: &mut usize) -> u16 {
            let v = u16::from_le_bytes([data[*off], data[*off + 1]]);
            *off += 2;
            v
        }
        fn rdb(data: &[u8], off: &mut usize) -> bool {
            rd(data, off) != 0
        }
        fn rdi64(data: &[u8], off: &mut usize) -> i64 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[*off..*off + 8]);
            *off += 8;
            i64::from_le_bytes(bytes)
        }
        if data.len() < off + 1 {
            return false;
        }

        self.pulse1.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.duty = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.duty_pos = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.timer_period = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.pulse1.timer_value = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.pulse1.length_counter = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.length_halt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.start = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.loop_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.constant_volume = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.volume = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.decay_level = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.envelope.divider = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.period = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.negate = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.shift = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.reload = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse1.sweep.divider = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }

        self.pulse2.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.duty = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.duty_pos = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.timer_period = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.pulse2.timer_value = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.pulse2.length_counter = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.length_halt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.start = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.loop_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.constant_volume = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.volume = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.decay_level = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.envelope.divider = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.period = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.negate = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.shift = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.reload = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.pulse2.sweep.divider = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }

        self.triangle.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.timer_period = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.triangle.timer_value = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.triangle.length_counter = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.length_halt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.linear_counter = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.linear_reload_value = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.linear_reload_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.triangle.sequence_pos = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }

        self.noise.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.mode = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.timer_period = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.noise.timer_value = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.noise.length_counter = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.length_halt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.start = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.loop_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.constant_volume = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.volume = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.decay_level = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.envelope.divider = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.noise.shift_register = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }

        self.dmc.enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.output_level = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.irq_enabled = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.loop_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.timer_period = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.timer_value = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.sample_address = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.sample_length = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.current_address = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.bytes_remaining = rd16(data, &mut off);
        if data.len() < off {
            return false;
        }
        self.dmc.sample_buffer = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.bits_remaining = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.silence_flag = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc.sample_buffer_empty = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }

        self.frame_mode = rd(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.frame_irq_inhibit = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.frame_interrupt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        self.dmc_interrupt = rdb(data, &mut off);
        if data.len() <= off {
            return false;
        }
        if data.len() >= off + 8 {
            self.frame_step = rdi64(data, &mut off);
        }
        // frame_step_offset: 0 or -1 (optional — older saves may not have this)
        self.frame_step_offset = if data.len() >= off + 8 {
            rdi64(data, &mut off)
        } else {
            0
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apu_pulse_produces_samples() {
        let mut apu = APU::new(44100);
        let _ = apu.write_register(0x4015, 0x01);
        let _ = apu.write_register(0x4000, 0b1011_1111);
        let _ = apu.write_register(0x4002, 0x70);
        let _ = apu.write_register(0x4003, 0x08);
        for _ in 0..=44100 / 60 {
            let _ = apu.tick_one();
        }
        let samples = apu.drain_audio_buffer();
        assert!(!samples.is_empty());
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.0, "APU mixer produced only silence");
    }

    #[test]
    fn apu_status_reflects_length_counters() {
        let mut apu = APU::new(44100);
        let _ = apu.write_register(0x4015, 0x0F);
        let _ = apu.write_register(0x4003, 0x08);
        let _ = apu.write_register(0x4007, 0x08);
        let _ = apu.write_register(0x400B, 0x08);
        let _ = apu.write_register(0x400F, 0x08);
        assert_eq!(apu.read_status() & 0x0F, 0x0F);
    }

    #[test]
    fn apu_reset_clears_state() {
        let mut apu1 = APU::new(44100);
        let _ = apu1.write_register(0x4015, 0x0F);
        let _ = apu1.write_register(0x4003, 0x08);
        let mut apu2 = APU::new(44100);
        assert_eq!(apu2.read_status() & 0x0F, 0);
    }

    #[test]
    fn apu_frame_counter_clears_irq_on_read() {
        let mut apu = APU::new(44100);
        apu.frame_interrupt = true;
        assert!(apu.read_status() & 0x40 != 0);
        assert!(apu.read_status() & 0x40 == 0);
    }

    #[test]
    fn apu_frame_counter_odd_jitter_sequence_length() {
        // Verify that odd jitter (offset=-1) 4-step sequence wraps at
        // 29830 ticks total (first tick skips count: -1→0, IRQ+half merged at 29829).
        let mut apu = APU::new(44100);
        apu.frame_mode = 0;
        apu.frame_step_offset = -1;
        apu.frame_step = -1;
        apu.frame_irq_inhibit = false;

        // 29829 ticks: step goes -1 → 29828 (stops at 29828, no trigger)
        for _ in 0..29829 {
            let _ = apu.tick_one();
        }
        assert_eq!(apu.frame_step, 29828);
        assert!(!apu.frame_interrupt);

        // 29830th tick: step → 29829, triggers quarter+half+IRQ, resets to -1
        let _ = apu.tick_one();
        assert_eq!(apu.frame_step, -1);
        assert!(apu.frame_interrupt);
    }

    #[test]
    fn apu_frame_counter_even_jitter_sequence_length() {
        // Verify that even jitter (offset=0) 4-step sequence wraps at
        // 29830 ticks (IRQ at 29830).
        let mut apu = APU::new(44100);
        apu.frame_mode = 0;
        apu.frame_step_offset = 0;
        apu.frame_step = 0;
        apu.frame_irq_inhibit = false;

        // Step to just before the half+Q (29828)
        for _ in 0..29828 {
            let _ = apu.tick_one();
        }
        assert_eq!(apu.frame_step, 29828);
        assert!(!apu.frame_interrupt);

        // 29829: quarter + half (no IRQ yet)
        let _ = apu.tick_one();
        assert_eq!(apu.frame_step, 29829);
        assert!(!apu.frame_interrupt);

        // 29830: IRQ + reset
        let _ = apu.tick_one();
        assert_eq!(apu.frame_step, 0);
        assert!(apu.frame_interrupt);
    }
}