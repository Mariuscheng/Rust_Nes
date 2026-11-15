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
mod timing;

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/frame_tests.rs"]
mod frame_tests;
#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/rom_tests.rs"]
mod rom_tests;

/// Test-ROM `$6000` observer, kept local with the rest of the harness.
#[cfg(feature = "local_harness")]
#[path = "../testing/rom_output.rs"]
mod rom_output;

use audio::AudioOut;
use cartridge::Cartridge;
use joypad::Button;
use nes::NesConsole;
use minifb::{Key, Window, WindowOptions};
use std::time::Instant;

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
    enable_dpi_awareness();

    // ── 1. Load ROM via Cartridge (black box input) ──
    let cart = Cartridge::from_file(&rom_path)
        .unwrap_or_else(|e| panic!("Failed to load ROM: {}", e));

    // ── 2. Build pure-hardware NES console ──
    let mut console = NesConsole::from_cartridge(cart);

    // ── 3. Attach the test-ROM observer (local harness only) ──
    #[cfg(feature = "local_harness")]
    console.set_frame_callback(rom_output::RomOutput::new().make_callback());

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
    println!(
        "Ran {} frames in {:.2?} ({:.1} fps)",
        max_frames,
        total,
        max_frames as f64 / total.as_secs_f64(),
    );
}

fn run_with_window(console: &mut NesConsole) {
    let mut frame_buf = [0u32; 256 * 240];

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
        window.update_with_buffer(&scaled, WIN_W, WIN_H).unwrap();

        // ── Save state hotkeys ──
        if window.is_key_down(Key::F5) {
            console.save_state("savestate.dat");
        }
        if window.is_key_down(Key::F7) {
            console.load_state("savestate.dat");
        }
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
    // Fails harmlessly on builds that predate this API.
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
