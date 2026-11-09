//! Regression harness for the blargg test-ROM suite.
//!
//! Each test boots a ROM headlessly, reads the standard `$6000` text protocol
//! (status byte at `$6000`, `DE B0 61` signature at `$6001`, NUL-terminated
//! text from `$6004`) and asserts the reported result.
//!
//! The suite is `#[ignore]`d by default because it takes tens of seconds even
//! in release mode:
//!
//! ```text
//! cargo test --release -- --ignored
//! ```
//!
//! ROMs are not committed to the repository, so tests whose ROM is missing are
//! skipped instead of failing.

use crate::cartridge::Cartridge;
use crate::joypad::Button;
use crate::nes::NesConsole;
use std::cell::RefCell;
use std::rc::Rc;

/// Result reported through the `$6000` protocol by a blargg test ROM.
#[derive(Clone, Debug)]
struct RomResult {
    status: u8,
    text: String,
}

impl RomResult {
    fn passed(&self) -> bool {
        self.status == 0x00
    }
}

/// Boot `path` and run until the ROM reports a result (status != 0x80).
///
/// Returns `None` when the ROM file is not present.
fn run_rom(path: &str, max_frames: u64) -> Option<RomResult> {
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping {path}: ROM not found");
        return None;
    }

    let cart = Cartridge::from_file(path).expect("failed to load test ROM");
    let mut console = NesConsole::from_cartridge(cart);

    // Latest (status, text) reported by the ROM. `None` until the magic
    // signature appears, so the loop below doesn't stop on the initial zeros.
    let observed: Rc<RefCell<Option<(u8, String)>>> = Rc::new(RefCell::new(None));
    let sink = Rc::clone(&observed);
    console.set_frame_callback(Box::new(move |_frame, wram: &[u8; 8192]| {
        if wram[1] == 0xDE && wram[2] == 0xB0 && wram[3] == 0x61 {
            let mut text = Vec::new();
            for &b in &wram[4..] {
                if b == 0 {
                    break;
                }
                text.push(b);
            }
            *sink.borrow_mut() = Some((wram[0], String::from_utf8_lossy(&text).into_owned()));
        }
    }));

    for _ in 0..max_frames {
        console.step_frame();
        let snapshot = observed.borrow().clone();
        if let Some((status, text)) = snapshot {
            if status != 0x80 {
                return Some(RomResult { status, text });
            }
        }
    }

    let snapshot = observed.borrow().clone();
    Some(match snapshot {
        Some((status, text)) => RomResult { status, text },
        None => RomResult {
            status: 0x80,
            text: String::new(),
        },
    })
}

fn assert_pass(path: &str, max_frames: u64) {
    let Some(result) = run_rom(path, max_frames) else {
        return;
    };
    assert!(
        result.passed(),
        "{path}: reported status 0x{:02X} (expected 0x00 = pass)\n{}",
        result.status,
        result.text
    );
}

/// Documents a *known* timing failure. The assertion fails as soon as the ROM
/// starts passing, which is the signal to promote it to `assert_pass()`.
fn assert_known_failure(path: &str, max_frames: u64, expected_text_fragment: &str) {
    let Some(result) = run_rom(path, max_frames) else {
        return;
    };
    assert!(
        !result.passed(),
        "{path}: expected a known failure but the ROM passed — promote this test to assert_pass()"
    );
    assert!(
        result.text.contains(expected_text_fragment),
        "{path}: failure output changed (expected text containing {expected_text_fragment:?})\n{}",
        result.text
    );
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn all_instrs_passes() {
    assert_pass("all_instrs.nes", 25_000);
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn official_only_passes() {
    assert_pass("official_only.nes", 25_000);
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn instr_timing_passes() {
    assert_pass("instr_timing.nes", 6_000);
}

// ── Known timing debt ────────────────────────────────────────────────────
// These need sub-instruction CPU/PPU/APU interleaving (see the cycle-stepped
// core work). They are expected to fail today.

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn cpu_interrupts_known_nmi_brk_failure() {
    assert_known_failure("cpu_interrupts.nes", 6_000, "2 of 5");
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn ppu_vbl_nmi_known_set_time_failure() {
    assert_known_failure("ppu_vbl_nmi.nes", 6_000, "2 of 10");
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn apu_test_known_irq_flag_timing_failure() {
    assert_known_failure("apu_test.nes", 6_000, "6 of 8");
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn apu_produces_audio() {
    let rom = "Mario.nes";
    if !std::path::Path::new(rom).is_file() {
        eprintln!("skipping {rom}: ROM not found");
        return;
    }

    let cart = Cartridge::from_file(rom).expect("failed to load ROM");
    let mut console = NesConsole::from_cartridge(cart);

    // Press Start on the title screen so the gameplay music starts.
    for frame in 0..400u64 {
        if frame == 60 {
            console.set_button(Button::Start, true);
        }
        if frame == 66 {
            console.set_button(Button::Start, false);
        }
        console.step_frame();
    }

    let samples = console.drain_audio_samples();
    assert!(!samples.is_empty(), "APU produced no samples");
    assert!(
        samples.iter().all(|s| s.is_finite()),
        "APU produced non-finite samples"
    );
    let peak = samples.iter().fold(0.0f32, |peak, s| s.max(s.abs()));
    assert!(peak > 1e-4, "APU output is silent (peak {peak})");

    // One APU cycle per CPU cycle: ~735 samples per frame at 44.1 kHz.
    let expected = 400.0 * 44_100.0 / 60.0988;
    let ratio = samples.len() as f32 / expected;
    assert!(
        (0.98..1.02).contains(&ratio),
        "unexpected APU sample count: {} ({ratio:.3}x expected) - APU may be stepped twice or not at all",
        samples.len()
    );
}
