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
use crate::state::{Reader, Writer};

/// Frame counter event thresholds, counted in CPU cycles after a `$4017` write.
///
/// `[q1, q2, q3, q4, h1, h2, irq, reset, s5_last]`
///
/// - `q1..q4`: quarter-frame clocks (envelope, linear counter)
/// - `h1, h2`: half-frame clocks (length counters, sweep)
/// - `irq`: 4-step frame IRQ
/// - `reset`: frame counter wraps back to 0
/// - `s5_last`: 5-step final quarter+half+reset
///
/// These values are calibrated against blargg's `apu_test` and are the single
/// place to tune frame-counter timing.
const FRAME_EVENTS: [i64; 9] = [7457, 14913, 22371, 29829, 14913, 29829, 29830, 29830, 37281];

/// CPU cycles between the power-up `$4017` write and the first instruction.
///
/// After power-up the APU behaves as if `$00` were written to `$4017` 9-12
/// CPU clocks before the first instruction (blargg `09.reset_timing`; the
/// observed delay is usually 9). `APU::power_on` pre-ages the frame counter by
/// this much so the fourth step lands in the window that test checks.
const POWER_ON_FRAME_AGE: i64 = 9;

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
    /// Last value written to `$4017`. Hardware re-applies it when the console
    /// is reset (see [`APU::rearm_frame_counter`]); `$00` after power-up.
    last_4017_write: u8,
    /// Frame counter IRQ 是否已觸發（讀取 $4015 時清除）
    pub frame_interrupt: bool,
    /// Remaining CPU cycles of the frame-IRQ assertion window. The flag is set
    /// on three consecutive cycles; a $4015 read clears it, but the rest of the
    /// window sets it again (apu_test 6-irq_flag_timing 4/5).
    frame_irq_arm: u8,
    /// Cycles until the next frame-counter IRQ. Counted in wall-clock CPU
    /// cycles rather than sequencer steps, because the sequencer's first
    /// sequence is phase-shifted but the IRQ still lands 29831 clocks after a
    /// mode-0 `$4017` write (apu_test 4-jitter).
    frame_irq_countdown: u32,
    /// DMC IRQ 是否已觸發
    pub dmc_interrupt: bool,
    frame_step: i64,
    /// Off-by-one offset for odd-cycle alignment: 0 (even) or -1 (odd).
    /// When -1, the first tick increments to 0 (effectively skipping one count)
    /// and the final combined event resets back to -1, keeping the sequence
    /// length at 29830 ticks.
    frame_step_offset: i64,
    /// True until the first frame-counter reset after a $4017 write.
    frame_first_sequence: bool,
    /// Length counters as of the start of the current half-frame clock, so a
    /// `$4015` read on the clock cycle still sees the pre-clock state
    /// (blargg `05.len_timing_mode0`). Order: pulse1, pulse2, triangle, noise.
    length_before_clock: [u8; 4],
    /// True from the half-frame clock of the current APU cycle until the next
    /// cycle starts. A `$4003`/`$4007`/`$400B`/`$400F` write in this cycle is
    /// ignored when the length counter was nonzero before the clock
    /// (blargg `11.len_reload_timing`).
    length_clocked: bool,
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
            last_4017_write: 0,
            frame_interrupt: false,
            frame_irq_arm: 0,
            frame_irq_countdown: 0,
            dmc_interrupt: false,
            frame_step: 0,
            frame_step_offset: 0,
            frame_first_sequence: false,
            length_before_clock: [0; 4],
            length_clocked: false,
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
            0x4000..=0x4002 => {
                self.pulse1.write_reg((addr - 0x4000) as u8, data);
            }
            0x4003 => {
                let old = self.pulse1.length_counter;
                let blocked = self.length_reload_blocked(0);
                self.pulse1.write_reg(3, data);
                if blocked {
                    self.pulse1.length_counter = old;
                }
            }
            0x4004..=0x4006 => {
                self.pulse2.write_reg((addr - 0x4004) as u8, data);
            }
            0x4007 => {
                let old = self.pulse2.length_counter;
                let blocked = self.length_reload_blocked(1);
                self.pulse2.write_reg(3, data);
                if blocked {
                    self.pulse2.length_counter = old;
                }
            }
            0x4008 => self.triangle.write_reg(0, data),
            0x400A => self.triangle.write_reg(2, data),
            0x400B => {
                let old = self.triangle.length_counter;
                let blocked = self.length_reload_blocked(2);
                self.triangle.write_reg(3, data);
                if blocked {
                    self.triangle.length_counter = old;
                }
            }
            0x400C => self.noise.write_reg(0, data),
            0x400E => self.noise.write_reg(2, data),
            0x400F => {
                let old = self.noise.length_counter;
                let blocked = self.length_reload_blocked(3);
                self.noise.write_reg(3, data);
                if blocked {
                    self.noise.length_counter = old;
                }
            }
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
                    // Starting the DMC reloads its length counter. The sample
                    // buffer is only reset when no byte is still being played:
                    // with a one-byte sample the length counter is already 0
                    // while its byte is still in the output unit, and the DMC
                    // must keep playing it (7-dmc_basics test 19).
                    let still_playing = self.dmc.enabled && !self.dmc.sample_buffer_empty;
                    self.dmc.reload_sample();
                    if !still_playing {
                        self.dmc.bits_remaining = 8;
                        self.dmc.sample_buffer_empty = true;
                        self.dmc.silence_flag = true;
                    }
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
                self.last_4017_write = data;
                self.write_frame_counter(data);
            }
            _ => {}
        }
        None
    }

    /// Apply a write to the frame counter register (`$4017`).
    fn write_frame_counter(&mut self, data: u8) {
        self.frame_mode = (data >> 7) & 1;
        self.frame_irq_inhibit = (data & 0x40) != 0;
        if self.frame_irq_inhibit {
            self.frame_interrupt = false;
            self.frame_irq_arm = 0;
            self.frame_irq_countdown = 0;
        }
        // 依照寫入當下的 CPU cycle 奇偶決定 jitter 模式。
        // Even cycle: offset=0，下一個 tick 就開始計數。
        // Odd cycle:  offset=-2，寫入落在 2-cycle 網格之間，整個第一段
        //             序列延後 2 個 tick（跳過一個 clock）。
        self.frame_step_offset = if (self.total_cpu_cycles & 1) == 0 {
            0
        } else {
            -2
        };
        self.frame_step = self.frame_step_offset;
        self.frame_first_sequence = true;
        // Mode 0 re-arms the frame IRQ: first flag 29831 clocks after
        // the write on an even cycle, one clock later on an odd cycle
        // (the write lands between two APU clocks).
        self.frame_irq_arm = 0;
        self.frame_irq_countdown = if self.frame_mode == 0 && !self.frame_irq_inhibit {
            29831
                + if (self.total_cpu_cycles & 1) == 0 {
                    0
                } else {
                    1
                }
        } else {
            0
        };
        // mode 1 ($80)：寫入後立刻觸發一次 quarter + half，
        // 發生在 sequencer 重置之後，不計入 frame_step。
        if self.frame_mode == 1 {
            self.clock_quarter_frame();
            self.clock_half_frame();
        }
    }

    /// Establish the power-up frame-counter state.
    ///
    /// Hardware behaves as if `$4017` were written with `$00` 9-12 CPU clocks
    /// before the first instruction (blargg `09.reset_timing`), so the counter
    /// is already `POWER_ON_FRAME_AGE` clocks into its first mode-0 sequence
    /// when the first opcode is fetched. Call this right before that fetch —
    /// `NesConsole` does it after the reset vector has been read.
    pub fn power_on(&mut self) {
        // Power-up leaves the frame IRQ flag low (blargg 4015_cleared and
        // irq_flag_cleared): a plain $4017 write would not clear it.
        self.frame_interrupt = false;
        self.frame_irq_arm = 0;
        self.write_frame_counter(0x00);
        self.age_frame_counter();
    }

    /// Apply a hardware reset to the APU.
    ///
    /// Reset drives the APU as if `$4015` had been written with `$00`: every
    /// channel is disabled and its length counter is cleared, and the frame and
    /// DMC IRQ flags go low. The write-only registers — duty, period, halt,
    /// sweep, envelope, DMC sample address/length — keep their contents, which
    /// is why a game can tell a reset from a power cycle (blargg `apu_reset`
    /// checks exactly this: a triangle still halting keeps playing).
    ///
    /// The frame counter is re-armed separately by
    /// [`APU::rearm_frame_counter`] once the CPU has finished its reset
    /// sequence, since the replay has to be aged against the first instruction.
    pub fn reset(&mut self) {
        let _ = self.write_register(0x4015, 0x00);
        self.frame_interrupt = false;
        self.frame_irq_arm = 0;
        self.dmc_interrupt = false;
        // Drop the half-frame snapshot so a $4015 read right after the reset
        // cannot see pre-reset length counters.
        self.length_before_clock = [0; 4];
        self.length_clocked = false;
    }

    /// Replay the last `$4017` value after a reset, as hardware does.
    ///
    /// The frame counter is armed from the surviving `$4017` register and then
    /// pre-aged by the same 9 clocks as power-up, so the first instruction sees
    /// the same phase a cold boot produces (blargg `4017_timing`).
    pub fn rearm_frame_counter(&mut self) {
        let value = self.last_4017_write;
        self.write_frame_counter(value);
        self.age_frame_counter();
    }

    /// Advance the frame counter by the post-write reset delay without
    /// generating samples: the APU clock has not started yet.
    fn age_frame_counter(&mut self) {
        for _ in 0..POWER_ON_FRAME_AGE {
            self.frame_counter_tick();
        }
    }

    /// 讀取 APU status register（$4015）。
    ///
    /// 回傳各通道 length counter 狀態與中斷 flags。
    /// 讀取後清除 frame interrupt。
    pub fn read_status(&mut self) -> u8 {
        let mut s = 0u8;
        let lengths = if self.length_clocked {
            self.length_before_clock
        } else {
            [
                self.pulse1.length_counter,
                self.pulse2.length_counter,
                self.triangle.length_counter,
                self.noise.length_counter,
            ]
        };
        if lengths[0] > 0 {
            s |= 0x01;
        }
        if lengths[1] > 0 {
            s |= 0x02;
        }
        if lengths[2] > 0 {
            s |= 0x04;
        }
        if lengths[3] > 0 {
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

    /// Whether a length-reload write in the current APU cycle must be ignored.
    ///
    /// A reload written on the same cycle as the half-frame clock is only
    /// honoured when the counter was zero before that clock; otherwise the
    /// clock wins and the reload is discarded (blargg `11.len_reload_timing`).
    fn length_reload_blocked(&self, channel: usize) -> bool {
        self.length_clocked && self.length_before_clock[channel] > 0
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
        self.length_before_clock = [
            self.pulse1.length_counter,
            self.pulse2.length_counter,
            self.triangle.length_counter,
            self.noise.length_counter,
        ];
        self.length_clocked = true;
        self.pulse1.clock_length();
        self.pulse2.clock_length();
        self.triangle.clock_length();
        self.noise.clock_length();
        self.pulse1.sweep.clock(&mut self.pulse1.timer_period);
        self.pulse2.sweep.clock(&mut self.pulse2.timer_period);
    }

    /// 每個 CPU cycle 呼叫一次，推進 frame counter。
    ///
    /// 事件門檻集中在 [`FRAME_EVENTS`]，方便用測試 ROM 校準：
    /// quarter 在 q1..q4、half（length/sweep）在 h1/h2、4-step 的 IRQ 與 reset
    /// 分別在 irq/reset、5-step 的收尾在 s5_last。
    ///
    /// Even jitter（offset 0）與 odd jitter（offset -2）共用同一組門檻；
    /// odd 相位的第一段序列會多 2 個 tick，之後恢復穩態週期。
    fn frame_counter_tick(&mut self) {
        // Continue an assertion window: the IRQ flag covers three consecutive
        // CPU cycles, so a $4015 read in the middle does not keep it cleared.
        if self.frame_irq_arm > 0 {
            self.frame_interrupt = true;
            self.frame_irq_arm -= 1;
        }

        self.frame_step += 1;
        let s = self.frame_step;

        // The first half-frame after a $4017 write lands two cycles after the
        // nominal FRAME_EVENTS offsets: 14915/29831/37283 instead of
        // 14913/29829/37281 (blargg 05.len_timing_mode0 and 10.len_halt_timing
        // pin those edges). Later sequences run at the nominal period.
        let shift = if self.frame_first_sequence { 2 } else { 0 };

        if s == FRAME_EVENTS[0]
            || s == FRAME_EVENTS[1]
            || s == FRAME_EVENTS[2]
            || s == FRAME_EVENTS[3]
        {
            self.clock_quarter_frame();
        }
        if s == FRAME_EVENTS[4] + shift {
            self.clock_half_frame();
        }
        // In 5-step mode the second half-frame clock does not happen; the
        // final one comes with the sequence reset at s5_last.
        if self.frame_mode == 0 && s == FRAME_EVENTS[5] + shift {
            self.clock_half_frame();
        }

        // Frame-IRQ countdown runs in wall-clock cycles, independent of the
        // sequencer's first-sequence phase shift. The flag is set on three
        // consecutive cycles; a $4015 read in between does not keep it clear.
        if self.frame_irq_countdown > 0 {
            self.frame_irq_countdown -= 1;
            if self.frame_irq_countdown == 0 {
                if self.frame_mode == 0 && !self.frame_irq_inhibit {
                    self.frame_interrupt = true;
                    self.frame_irq_arm = 2;
                }
                self.frame_irq_countdown = 29830;
            }
        }

        if self.frame_mode == 0 {
            if s == FRAME_EVENTS[7] + shift {
                self.frame_step = 0;
                self.frame_first_sequence = false;
            }
        } else {
            if s == FRAME_EVENTS[8] + shift {
                // 5-step mode: final quarter+half...
                self.clock_quarter_frame();
                self.clock_half_frame();
            }
            // ...and the sequence reset one cycle later.
            if s == FRAME_EVENTS[8] + shift + 1 {
                self.frame_step = 0;
                self.frame_first_sequence = false;
            }
        }
    }

    /// 推進 APU 一個 CPU cycle。
    ///
    /// 若 DMC 需要新的 sample byte，回傳要讀取的位址；bus 讀取後以
    /// `fill_sample()` 回填（DMC 本身不做記憶體存取）。
    pub fn tick_one(&mut self) -> Option<u16> {
        self.total_cpu_cycles += 1;
        self.length_clocked = false;
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
            let filtered = self.hpf_alpha * (self.hpf_last_output + raw - self.hpf_last_input);
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
        let mut out = Writer::new();

        out.bool(self.pulse1.enabled);
        out.u8(self.pulse1.duty);
        out.u8(self.pulse1.duty_pos);
        out.u16(self.pulse1.timer_period);
        out.u16(self.pulse1.timer_value);
        out.u8(self.pulse1.length_counter);
        out.bool(self.pulse1.length_halt);
        out.bool(self.pulse1.envelope.start);
        out.bool(self.pulse1.envelope.loop_flag);
        out.bool(self.pulse1.envelope.constant_volume);
        out.u8(self.pulse1.envelope.volume);
        out.u8(self.pulse1.envelope.decay_level);
        out.u8(self.pulse1.envelope.divider);
        out.bool(self.pulse1.sweep.enabled);
        out.u8(self.pulse1.sweep.period);
        out.bool(self.pulse1.sweep.negate);
        out.u8(self.pulse1.sweep.shift);
        out.bool(self.pulse1.sweep.reload);
        out.u8(self.pulse1.sweep.divider);

        out.bool(self.pulse2.enabled);
        out.u8(self.pulse2.duty);
        out.u8(self.pulse2.duty_pos);
        out.u16(self.pulse2.timer_period);
        out.u16(self.pulse2.timer_value);
        out.u8(self.pulse2.length_counter);
        out.bool(self.pulse2.length_halt);
        out.bool(self.pulse2.envelope.start);
        out.bool(self.pulse2.envelope.loop_flag);
        out.bool(self.pulse2.envelope.constant_volume);
        out.u8(self.pulse2.envelope.volume);
        out.u8(self.pulse2.envelope.decay_level);
        out.u8(self.pulse2.envelope.divider);
        out.bool(self.pulse2.sweep.enabled);
        out.u8(self.pulse2.sweep.period);
        out.bool(self.pulse2.sweep.negate);
        out.u8(self.pulse2.sweep.shift);
        out.bool(self.pulse2.sweep.reload);
        out.u8(self.pulse2.sweep.divider);

        out.bool(self.triangle.enabled);
        out.u16(self.triangle.timer_period);
        out.u16(self.triangle.timer_value);
        out.u8(self.triangle.length_counter);
        out.bool(self.triangle.length_halt);
        out.u8(self.triangle.linear_counter);
        out.u8(self.triangle.linear_reload_value);
        out.bool(self.triangle.linear_reload_flag);
        out.u8(self.triangle.sequence_pos);

        out.bool(self.noise.enabled);
        out.bool(self.noise.mode);
        out.u16(self.noise.timer_period);
        out.u16(self.noise.timer_value);
        out.u8(self.noise.length_counter);
        out.bool(self.noise.length_halt);
        out.bool(self.noise.envelope.start);
        out.bool(self.noise.envelope.loop_flag);
        out.bool(self.noise.envelope.constant_volume);
        out.u8(self.noise.envelope.volume);
        out.u8(self.noise.envelope.decay_level);
        out.u8(self.noise.envelope.divider);
        out.u16(self.noise.shift_register);

        out.bool(self.dmc.enabled);
        out.u8(self.dmc.output_level);
        out.bool(self.dmc.irq_enabled);
        out.bool(self.dmc.loop_flag);
        out.u16(self.dmc.timer_period);
        out.u16(self.dmc.timer_value);
        out.u16(self.dmc.sample_address);
        out.u16(self.dmc.sample_length);
        out.u16(self.dmc.current_address);
        out.u16(self.dmc.bytes_remaining);
        out.u8(self.dmc.sample_buffer);
        out.u8(self.dmc.bits_remaining);
        out.bool(self.dmc.silence_flag);
        out.bool(self.dmc.sample_buffer_empty);

        out.u8(self.frame_mode);
        out.bool(self.frame_irq_inhibit);
        out.bool(self.frame_interrupt);
        out.bool(self.dmc_interrupt);
        out.i64(self.frame_step);
        out.i64(self.frame_step_offset);

        out.finish()
    }

    /// 從位元組 slice 反序列化 APU 狀態
    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let mut reader = Reader::new(data);
        self.restore(&mut reader).is_some()
    }

    fn restore(&mut self, reader: &mut Reader<'_>) -> Option<()> {
        self.pulse1.enabled = reader.bool()?;
        self.pulse1.duty = reader.u8()?;
        self.pulse1.duty_pos = reader.u8()?;
        self.pulse1.timer_period = reader.u16()?;
        self.pulse1.timer_value = reader.u16()?;
        self.pulse1.length_counter = reader.u8()?;
        self.pulse1.length_halt = reader.bool()?;
        self.pulse1.envelope.start = reader.bool()?;
        self.pulse1.envelope.loop_flag = reader.bool()?;
        self.pulse1.envelope.constant_volume = reader.bool()?;
        self.pulse1.envelope.volume = reader.u8()?;
        self.pulse1.envelope.decay_level = reader.u8()?;
        self.pulse1.envelope.divider = reader.u8()?;
        self.pulse1.sweep.enabled = reader.bool()?;
        self.pulse1.sweep.period = reader.u8()?;
        self.pulse1.sweep.negate = reader.bool()?;
        self.pulse1.sweep.shift = reader.u8()?;
        self.pulse1.sweep.reload = reader.bool()?;
        self.pulse1.sweep.divider = reader.u8()?;

        self.pulse2.enabled = reader.bool()?;
        self.pulse2.duty = reader.u8()?;
        self.pulse2.duty_pos = reader.u8()?;
        self.pulse2.timer_period = reader.u16()?;
        self.pulse2.timer_value = reader.u16()?;
        self.pulse2.length_counter = reader.u8()?;
        self.pulse2.length_halt = reader.bool()?;
        self.pulse2.envelope.start = reader.bool()?;
        self.pulse2.envelope.loop_flag = reader.bool()?;
        self.pulse2.envelope.constant_volume = reader.bool()?;
        self.pulse2.envelope.volume = reader.u8()?;
        self.pulse2.envelope.decay_level = reader.u8()?;
        self.pulse2.envelope.divider = reader.u8()?;
        self.pulse2.sweep.enabled = reader.bool()?;
        self.pulse2.sweep.period = reader.u8()?;
        self.pulse2.sweep.negate = reader.bool()?;
        self.pulse2.sweep.shift = reader.u8()?;
        self.pulse2.sweep.reload = reader.bool()?;
        self.pulse2.sweep.divider = reader.u8()?;

        self.triangle.enabled = reader.bool()?;
        self.triangle.timer_period = reader.u16()?;
        self.triangle.timer_value = reader.u16()?;
        self.triangle.length_counter = reader.u8()?;
        self.triangle.length_halt = reader.bool()?;
        self.triangle.linear_counter = reader.u8()?;
        self.triangle.linear_reload_value = reader.u8()?;
        self.triangle.linear_reload_flag = reader.bool()?;
        self.triangle.sequence_pos = reader.u8()?;

        self.noise.enabled = reader.bool()?;
        self.noise.mode = reader.bool()?;
        self.noise.timer_period = reader.u16()?;
        self.noise.timer_value = reader.u16()?;
        self.noise.length_counter = reader.u8()?;
        self.noise.length_halt = reader.bool()?;
        self.noise.envelope.start = reader.bool()?;
        self.noise.envelope.loop_flag = reader.bool()?;
        self.noise.envelope.constant_volume = reader.bool()?;
        self.noise.envelope.volume = reader.u8()?;
        self.noise.envelope.decay_level = reader.u8()?;
        self.noise.envelope.divider = reader.u8()?;
        self.noise.shift_register = reader.u16()?;

        self.dmc.enabled = reader.bool()?;
        self.dmc.output_level = reader.u8()?;
        self.dmc.irq_enabled = reader.bool()?;
        self.dmc.loop_flag = reader.bool()?;
        self.dmc.timer_period = reader.u16()?;
        self.dmc.timer_value = reader.u16()?;
        self.dmc.sample_address = reader.u16()?;
        self.dmc.sample_length = reader.u16()?;
        self.dmc.current_address = reader.u16()?;
        self.dmc.bytes_remaining = reader.u16()?;
        self.dmc.sample_buffer = reader.u8()?;
        self.dmc.bits_remaining = reader.u8()?;
        self.dmc.silence_flag = reader.bool()?;
        self.dmc.sample_buffer_empty = reader.bool()?;

        self.frame_mode = reader.u8()?;
        self.frame_irq_inhibit = reader.bool()?;
        self.frame_interrupt = reader.bool()?;
        self.dmc_interrupt = reader.bool()?;
        if let Some(step) = reader.i64() {
            self.frame_step = step;
        }
        self.frame_step_offset = reader.i64().unwrap_or(0);
        Some(())
    }
}

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/apu_tests.rs"]
mod tests;
