//! Golden-frame regression tests.
//!
//! Boots a ROM, runs a fixed number of frames headlessly and compares a hash
//! of the 256×240 framebuffer against `golden_frames.txt`. This is the safety
//! net for the cycle-stepped timing refactor: any CPU/PPU/APU change that
//! alters rendered pixels shows up immediately.
//!
//! ```text
//! cargo test --release -- --ignored golden_frames
//! ```
//!
//! Regenerate the file only after an *intended* rendering change:
//!
//! ```text
//! $env:REGEN_GOLDEN = "1"
//! cargo test --release -- --ignored golden_frames
//! ```

use crate::cartridge::Cartridge;
use crate::nes::NesConsole;
use std::collections::BTreeMap;

const GOLDEN_FILE: &str = "golden_frames.txt";

#[derive(Clone, Debug)]
struct GoldenEntry {
    rom: String,
    frame: u64,
    hash: u64,
    non_black: u32,
}

fn fnv1a(pixels: &[u32]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &px in pixels {
        hash ^= px as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn parse_golden(text: &str) -> Vec<GoldenEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(rom), Some(frame), Some(hash), Some(non_black)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Ok(frame) = frame.parse() else { continue };
        let Ok(hash) = u64::from_str_radix(hash.trim_start_matches("0x"), 16) else {
            continue;
        };
        let Ok(non_black) = non_black.parse() else {
            continue;
        };
        entries.push(GoldenEntry {
            rom: rom.to_string(),
            frame,
            hash,
            non_black,
        });
    }
    entries
}

fn format_golden(entries: &[GoldenEntry]) -> String {
    let mut out = String::from(
        "# Golden framebuffer hashes: <rom> <frame> <fnv1a64 hash> <non-black pixels>\n\
         # Regenerate with REGEN_GOLDEN=1 after an intended rendering change.\n",
    );
    for entry in entries {
        out.push_str(&format!(
            "{} {} {:016x} {}\n",
            entry.rom, entry.frame, entry.hash, entry.non_black
        ));
    }
    out
}

/// Run `rom` and hash the framebuffer at each requested frame.
/// Returns `None` when the ROM file is missing.
fn checkpoints(rom: &str, frames: &[u64]) -> Option<Vec<GoldenEntry>> {
    if !std::path::Path::new(rom).is_file() {
        eprintln!("skipping {rom}: ROM not found");
        return None;
    }

    let cart = Cartridge::from_file(rom).expect("failed to load ROM");
    let mut console = NesConsole::from_cartridge(cart);

    let last = *frames.iter().max().expect("at least one checkpoint");
    let mut buffer = [0u32; 256 * 240];
    let mut entries = Vec::new();

    for frame in 1..=last {
        console.step_frame();
        if frames.contains(&frame) {
            console.read_frame_buffer(&mut buffer);
            entries.push(GoldenEntry {
                rom: rom.to_string(),
                frame,
                hash: fnv1a(&buffer),
                non_black: buffer.iter().filter(|&&px| px != 0).count() as u32,
            });
        }
    }

    Some(entries)
}

#[test]
#[ignore = "ROM suite is slow; run: cargo test --release -- --ignored"]
fn golden_frames_match() {
    let regen = std::env::var_os("REGEN_GOLDEN").is_some();

    let text = match std::fs::read_to_string(GOLDEN_FILE) {
        Ok(text) => text,
        Err(_) => {
            if !regen {
                eprintln!("{GOLDEN_FILE} not found; run with REGEN_GOLDEN=1 to create it");
                return;
            }
            String::new()
        }
    };
    let expected = parse_golden(&text);

    // Group checkpoints per ROM so each ROM only boots once.
    let mut wanted: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for entry in &expected {
        wanted.entry(entry.rom.clone()).or_default().push(entry.frame);
    }
    if regen && wanted.is_empty() {
        panic!("REGEN_GOLDEN set but {GOLDEN_FILE} is empty; nothing to regenerate");
    }

    let mut actual = Vec::new();
    for (rom, mut frames) in wanted {
        frames.sort_unstable();
        frames.dedup();
        if let Some(mut entries) = checkpoints(&rom, &frames) {
            actual.append(&mut entries);
        }
    }

    if regen {
        std::fs::write(GOLDEN_FILE, format_golden(&actual)).expect("failed to write golden file");
        eprintln!("wrote {} golden entries to {GOLDEN_FILE}", actual.len());
        return;
    }

    if actual.is_empty() {
        eprintln!("no golden entries checked (missing ROMs?)");
        return;
    }

    let mut failures = Vec::new();
    for entry in &actual {
        let Some(expected) = expected
            .iter()
            .find(|e| e.rom == entry.rom && e.frame == entry.frame)
        else {
            failures.push(format!(
                "{}: no golden entry for frame {} (new checkpoint)",
                entry.rom, entry.frame
            ));
            continue;
        };
        if expected.hash != entry.hash || expected.non_black != entry.non_black {
            failures.push(format!(
                "{} frame {}: hash {:016x} != {:016x} (non-black {} != {})",
                entry.rom, entry.frame, entry.hash, expected.hash, entry.non_black, expected.non_black
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "golden frame mismatches:\n{}\nIf the rendering change is intended, \
         regenerate with REGEN_GOLDEN=1 and commit the updated {GOLDEN_FILE}.",
        failures.join("\n")
    );
}
