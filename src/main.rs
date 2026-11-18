//! Rust_Nes — a single-threaded NES emulator.
//!
//! This file is the platform layer: it owns the window, audio output and
//! keyboard input, and drives `NesConsole::step_frame`. Everything below
//! `nes.rs` is platform-independent and only talks through the bus.

#![expect(
    clippy::upper_case_acronyms,
    reason = "CPU/PPU/APU/OAM/CHR/PRG and the 6502 mnemonics are the NESdev vocabulary; CamelCase would break the match with the references"
)]

mod apu;
mod audio;
mod bus;
mod cartridge;
mod constants;
mod cpu;
mod joypad;
mod nes;
mod ppu;
mod ram;
mod rom;
mod state;

// ── Local-only test modules (live in `testing/`, which is not published) ──
#[cfg(test)]
#[cfg(feature = "local_harness")]
#[path = "../testing/mod.rs"]
mod tests;

/// Tests that only use the public API, so they also run in a checkout without
/// the local harness (`testing/` is gitignored and absent from CI).
#[cfg(test)]
mod state_tests;

use audio::AudioOut;
use cartridge::Cartridge;
use joypad::Button;
use minifb::{Key, Window, WindowOptions};
use nes::NesConsole;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "rom.nes".to_string());

    // Headless mode can be requested either with `--headless` or with the
    // `HEADLESS_FRAMES=<n>` environment variable documented in README.md.
    let headless_frames = std::env::var("HEADLESS_FRAMES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let headless = headless_frames.is_some() || args.iter().any(|a| a == "--headless");

    // Present the window in physical pixels: without this the OS stretches a
    // non-DPI-aware window, which smears the first scanline into a pale line.
    #[cfg(windows)]
    {
        enable_dpi_awareness();
        raise_timer_resolution();
    }

    // ── 1. Load ROM via Cartridge (black box input) ──
    let cart =
        Cartridge::from_file(&rom_path).unwrap_or_else(|e| panic!("Failed to load ROM: {}", e));

    // ── 2. Build pure-hardware NES console ──
    let mut console = NesConsole::from_cartridge(cart);

    if headless {
        run_headless(&mut console, headless_frames.unwrap_or(3000));
    } else {
        run_with_window(&mut console);
    }
}

fn run_headless(console: &mut NesConsole, max_frames: u64) {
    let start = Instant::now();
    for _ in 0..max_frames {
        console.step_frame();
    }
    let total = start.elapsed();
    console.save_battery();
    println!(
        "Ran {} frames in {:.2?} ({:.1} fps)",
        max_frames,
        total,
        max_frames as f64 / total.as_secs_f64(),
    );
}

/// One NTSC NES frame: 89342 PPU dots ÷ 3 dots per CPU cycle = 29780.67 CPU
/// cycles at 1.7897725 MHz, i.e. 16.639 ms (60.0988 Hz — not exactly 60).
const NES_FRAME_TIME: Duration = Duration::from_nanos(16_639_280);
/// Per-frame pacing correction used to steer the audio queue (±0.24 %).
const FRAME_ADJUST: Duration = Duration::from_micros(40);
/// Clamp so drift correction never becomes visible as a speed change.
const MIN_FRAME_TIME: Duration = Duration::from_micros(16_200);
const MAX_FRAME_TIME: Duration = Duration::from_micros(17_100);
/// How much of the wait is spent spinning instead of sleeping; `thread::sleep`
/// alone is quantized to the system timer (~1–16 ms).
const SPIN_WINDOW: Duration = Duration::from_millis(2);

fn run_with_window(console: &mut NesConsole) {
    let mut frame_buf = vec![0u32; 256 * 240];

    // Integer nearest-neighbour upscale. The window is created at exactly the
    // scaled size, so neither minifb nor the OS resamples the image (resampling
    // is what smeared the top scanline into a white-ish line).
    const SCALE: usize = 2;
    const WIN_W: usize = 256 * SCALE;
    const WIN_H: usize = 240 * SCALE;
    let mut scaled = vec![0u32; WIN_W * WIN_H];

    let mut window = Window::new(
        "NES Emulator - Rust_Nes",
        WIN_W,
        WIN_H,
        WindowOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{}", e));

    let audio_out = AudioOut::new(44100);
    let mut last_battery_save = Instant::now();
    let mut reset_held = false;

    // Frame pacing: one loop iteration = one NES frame, so the loop must be
    // limited to the NES frame rate. Without an explicit limit the loop free-runs
    // and then blocks on the audio queue, which made frame intervals alternate
    // between a few milliseconds and ~20 ms (visible stutter).
    let mut frame_time = NES_FRAME_TIME;
    let mut next_frame = Instant::now();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        poll_input(console, &window);

        // Run until the next frame completes.
        console.step_frame();

        // ── Audio ──
        let samples = console.drain_audio_samples();
        audio_out.append(samples);

        // ── Render ──
        console.read_frame_buffer(&mut frame_buf);
        for y in 0..WIN_H {
            let src_row = &frame_buf[(y / SCALE) * 256..(y / SCALE) * 256 + 256];
            let dst_row = &mut scaled[y * WIN_W..(y + 1) * WIN_W];
            for (x, px) in dst_row.iter_mut().enumerate() {
                *px = src_row[x / SCALE];
            }
        }
        if window.update_with_buffer(&scaled, WIN_W, WIN_H).is_err() {
            break;
        }

        // ── Save state hotkeys ──
        if window.is_key_down(Key::F5) {
            console.save_state("savestate.dat");
        }
        if window.is_key_down(Key::F7) {
            console.load_state("savestate.dat");
        }
        // F2 = hardware reset (like the front-panel button): RAM survives, so
        // games can warm-boot instead of losing their save-in-progress.
        // Edge-detected so holding the key does not reset every frame.
        let reset_down = window.is_key_down(Key::F2);
        if reset_down && !reset_held {
            console.reset();
        }
        reset_held = reset_down;

        // Autosave battery-backed cartridge RAM so a crash or force-quit only
        // loses the last 30 seconds.
        if last_battery_save.elapsed() >= std::time::Duration::from_secs(30) {
            console.save_battery();
            last_battery_save = Instant::now();
        }

        // ── Pace the next frame ──
        // Audio is the clock. The correction is far below one frame (±40 µs),
        // so drift between the emulator's nominal 44100 Hz and the device's
        // real rate is absorbed without changing game speed.
        let queued = audio_out.queued_frames();
        if queued < AudioOut::TARGET_QUEUED_FRAMES {
            frame_time = frame_time.saturating_sub(FRAME_ADJUST).max(MIN_FRAME_TIME);
        } else if queued > AudioOut::TARGET_QUEUED_FRAMES {
            frame_time = (frame_time + FRAME_ADJUST).min(MAX_FRAME_TIME);
        }
        next_frame += frame_time;
        let now = Instant::now();
        if next_frame < now {
            // Fell behind (window drag, breakpoint, ...): resync instead of
            // presenting a burst of frames to catch up.
            next_frame = now + Duration::from_millis(1);
        }
        wait_until(next_frame);
    }

    // Persist battery-backed cartridge RAM (if any) next to the ROM.
    console.save_battery();
}

/// Sleep until `deadline`, spinning the final [`SPIN_WINDOW`].
///
/// `thread::sleep` is quantized to the system timer (1 ms after
/// `timeBeginPeriod(1)`, ~15.6 ms otherwise), which is coarser than the jitter
/// that is visible as stutter; the spin covers the tail so each frame lands
/// within a few hundred microseconds of its deadline.
fn wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remain = deadline.saturating_duration_since(now);
        if remain > SPIN_WINDOW {
            std::thread::sleep(remain.saturating_sub(SPIN_WINDOW));
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Ask the Windows timer for 1 ms resolution.
///
/// Without it `Sleep` is quantized to the ~15.6 ms system tick, which is longer
/// than one NES frame. The audio stack normally requests this too, but the
/// frame pacer should not depend on that.
#[cfg(windows)]
fn raise_timer_resolution() {
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
    }
    // SAFETY: `timeBeginPeriod` is a plain Win32 call with no pointer arguments
    // and no preconditions beyond a valid period; it is safe from any thread and
    // the returned status is discarded.
    unsafe {
        let _ = timeBeginPeriod(1);
    }
}

/// Opt into per-monitor DPI awareness so the window is blitted 1:1 instead of
/// being stretched by the OS (the stretch is what produced a pale first line).
#[cfg(windows)]
fn enable_dpi_awareness() {
    // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
    const PER_MONITOR_AWARE_V2: isize = -4;
    unsafe extern "system" {
        fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    }
    // SAFETY: the function only reads an integer context value and returns a
    // status code; a failure (older Windows) leaves DPI awareness unchanged,
    // which the rest of the code already tolerates.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2);
    }
}

fn poll_input(console: &mut NesConsole, window: &Window) {
    console.set_button(Button::A, window.is_key_down(Key::Z));
    console.set_button(Button::B, window.is_key_down(Key::X));
    console.set_button(Button::Start, window.is_key_down(Key::Enter));
    console.set_button(Button::Select, window.is_key_down(Key::RightShift));
    console.set_button(Button::Up, window.is_key_down(Key::Up));
    console.set_button(Button::Down, window.is_key_down(Key::Down));
    console.set_button(Button::Left, window.is_key_down(Key::Left));
    console.set_button(Button::Right, window.is_key_down(Key::Right));
}
