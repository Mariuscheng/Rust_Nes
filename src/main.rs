mod apu;
mod audio;
mod bus;
mod cartridge;
mod constants;
mod cpu;
mod debug;
mod joypad;
mod nes;
mod ppu;
mod ram;
mod renderer;
mod rom;
mod timing;

#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/frame_tests.rs"]
mod frame_tests;
#[cfg(all(test, feature = "local_harness"))]
#[path = "../testing/rom_tests.rs"]
mod rom_tests;

use audio::AudioOut;
use cartridge::Cartridge;
use debug::DebugPrinter;
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

    // ── 3. Attach external observers (test ROM debug protocol) ──
    let debug_printer = DebugPrinter::new();
    console.set_frame_callback(debug_printer.make_callback());

    if headless {
        run_headless(&mut console, headless_frames.unwrap_or(3000));
    } else {
        run_with_window(&mut console);
    }
}

fn run_headless(console: &mut NesConsole, max_frames: u64) {
    let start = Instant::now();
    for _ in 0..max_frames {
        console.reset_frame_counters();

        console.step_frame();

        //println!(
        //    "[TIMING] Frame {:>6} | cycles: {:>7} | steps: {:>6} | instr: {:>7} | nmi: {:>5} | irq: {:>5} | apu_stolen: {:>5} | frame_time: {:>8.2?}",
        //    frame_idx,
        //    console.frame_cycles,
        //    console.timing_step_count,
        //    console.timing_instr_cycles,
        //    console.timing_nmi_cycles,
        //    console.timing_irq_cycles,
        //    console.timing_apu_stolen,
        //    frame_elapsed,
        //);
    }
    let total = start.elapsed();
    println!(
        "[TIMING] === Total: {} frames in {:.2?} | avg {:.2?}/frame | ~{:.1} fps ===",
        max_frames,
        total,
        total / (max_frames as u32),
        max_frames as f64 / total.as_secs_f64(),
    );

    // Dump a quick render check: non-zero pixels in last frame
    let mut frame_buf = [0u32; 256 * 240];
    console.read_frame_buffer(&mut frame_buf);
    let non_zero = frame_buf.iter().filter(|&&p| p != 0).count();
    let non_black = frame_buf.iter().filter(|&&p| p != 0xFF000000 && p != 0).count();
    println!(
        "[RENDER] non-zero pixels: {} / {} ({:.1}%), non-black: {}",
        non_zero,
        256 * 240,
        non_zero as f64 / (256.0 * 240.0) * 100.0,
        non_black,
    );

    // Find the bounding box of non-black pixels
    let mut min_x = 256usize;
    let mut max_x = 0usize;
    let mut min_y = 240usize;
    let mut max_y = 0usize;
    for y in 0..240 {
        for x in 0..256 {
            let px = frame_buf[y * 256 + x];
            if px != 0 && px != 0xFF000000 {
                if x < min_x { min_x = x; }
                if x > max_x { max_x = x; }
                if y < min_y { min_y = y; }
                if y > max_y { max_y = y; }
            }
        }
    }
    if non_black > 0 {
        println!("[RENDER] pixel bbox: ({},{}) - ({},{})", min_x, min_y, max_x, max_y);
        // Show center of bbox
        let cy = (min_y + max_y) / 2;
        print!("[RENDER] row {} cols {}-{}: ", cy, min_x, max_x);
        for x in min_x..=max_x {
            let px = frame_buf[cy * 256 + x];
            if px == 0 || px == 0xFF000000 { print!("."); }
            else { print!("#"); }
        }
        println!();
    }
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
    let mut last_fps_print = Instant::now();
    let mut frame_count: u64 = 0;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        poll_input(console, &window);

        console.reset_frame_counters();

        // Run until the next frame completes
        console.step_frame();
        frame_count += 1;

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

        // ── FPS timing ──
        let now = Instant::now();
        if frame_count % 60 == 0 {
            let since_last = now.duration_since(last_fps_print);
            let _fps = 60.0 / since_last.as_secs_f64();
            //println!(
            //    "[TIMING] Frame {:>8} | FPS: {:>6.1?} | cycles/frame: {:>7} | steps/frame: {:>6} | instr: {:>7} | nmi: {:>5} | irq: {:>5} | apu_stolen: {:>5}",
            //    frame_count,
            //    fps,
            //    console.frame_cycles,
            //    console.timing_step_count,
            //    console.timing_instr_cycles,
            //    console.timing_nmi_cycles,
            //    console.timing_irq_cycles,
            //    console.timing_apu_stolen,
            //);
            last_fps_print = now;
        }

        // ── Save state hotkeys ──
        if window.is_key_down(Key::F5) {
            console.save_state("savestate.dat");
            //println!("State saved to savestate.dat");
        }
        if window.is_key_down(Key::F7) {
            console.load_state("savestate.dat");
            //println!("State loaded from savestate.dat");
        }
        // F8: write the current frame plus sprite data for offline analysis.
        if window.is_key_down(Key::F8) {
            let ppu = console.ppu();
            let mut text = String::new();
            text.push_str(&format!(
                "frame={} mask={:02X} ctrl.spr_pt={:04X}\n",
                ppu.frame, ppu.mask, ppu.ctrl.sprite_pattern_addr
            ));
            for i in 0..64usize {
                let b = i * 4;
                let y = ppu.oam[b];
                if y >= 0xEF {
                    continue;
                }
                text.push_str(&format!(
                    "spr{i:02} y={y:3} tile={:02X} attr={:02X} x={:3}\n",
                    ppu.oam[b + 1],
                    ppu.oam[b + 2],
                    ppu.oam[b + 3]
                ));
            }
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            std::fs::write(format!("dq_shot_{n}.txt"), &text).ok();
            let mut ppm = Vec::with_capacity(256 * 240 * 3 + 16);
            ppm.extend_from_slice(b"P6\n256 240\n255\n");
            for &px in frame_buf.iter() {
                ppm.push(((px >> 16) & 0xFF) as u8);
                ppm.push(((px >> 8) & 0xFF) as u8);
                ppm.push((px & 0xFF) as u8);
            }
            std::fs::write(format!("dq_shot_{n}.ppm"), &ppm).ok();
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

fn poll_input(console: &mut NesConsole, window: &Window) {    console.set_button(Button::A, window.is_key_down(Key::Z));
    console.set_button(Button::B, window.is_key_down(Key::X));
    console.set_button(Button::Start, window.is_key_down(Key::Enter));
    console.set_button(Button::Select, window.is_key_down(Key::RightShift));
    console.set_button(Button::Up, window.is_key_down(Key::Up));
    console.set_button(Button::Down, window.is_key_down(Key::Down));
    console.set_button(Button::Left, window.is_key_down(Key::Left));
    console.set_button(Button::Right, window.is_key_down(Key::Right));
}