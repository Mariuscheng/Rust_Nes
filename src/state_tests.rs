//! Save-state round-trip tests.
//!
//! They use only the public API and a synthetic NROM, so they also run in a
//! checkout without the local harness (`testing/` is gitignored); that is what
//! gives CI a behavioural signal.
//!
//! Two properties, from weakest to strongest:
//!
//! 1. `save → load → save` is byte-identical: the reader consumes exactly what
//!    the writer produced.
//! 2. A state loaded into a **freshly booted** console resumes identically.
//!    Loading into the console that saved it hides every field the format
//!    forgot, because that console still holds the value in its registers; a
//!    fresh console starts at power-on, so a missing field shows up as a
//!    diverging frame.

use crate::cartridge::Cartridge;
use crate::nes::NesConsole;
use crate::rom::Mirroring;
use crate::rom::mapper::Mapper;
use crate::rom::mappers::{
    Fme7, Mapper0, Mapper1, Mapper2, Mapper3, Mapper4, Mapper5, Mapper7, Vrc6,
};
use std::fs;
use std::path::PathBuf;

/// NROM program placed at `$8000`: backdrop `$3F00 = $0F`, colour 1 =
/// `$3F01 = $01`, CHR tile 0 filled solid with colour 1, then
/// `PPUCTRL = $00` / `PPUMASK = $0A` (background + left column on) and an
/// infinite loop.
///
/// Rendering has to be *enabled* at save time: it is the one piece of PPU
/// register state a save can silently drop, and a blank or backdrop-only screen
/// is exactly how that shows up after loading.
const RENDER_PROGRAM: [u8; 70] = [
    0xA9, 0x00, // LDA #$00
    0x8D, 0x00, 0x20, // STA $2000
    0x8D, 0x01, 0x20, // STA $2001
    0xA9, 0x3F, // LDA #$3F
    0x8D, 0x06, 0x20, // STA $2006
    0xA9, 0x00, // LDA #$00
    0x8D, 0x06, 0x20, // STA $2006
    0xA9, 0x0F, // LDA #$0F
    0x8D, 0x07, 0x20, // STA $2007
    0xA9, 0x01, // LDA #$01
    0x8D, 0x07, 0x20, // STA $2007
    0xA9, 0x00, // LDA #$00
    0x8D, 0x06, 0x20, // STA $2006
    0xA9, 0x00, // LDA #$00
    0x8D, 0x06, 0x20, // STA $2006
    0xA2, 0x08, // LDX #$08
    0xA9, 0xFF, // LDA #$FF
    0x8D, 0x07, 0x20, // STA $2007
    0xCA, // DEX
    0xD0, 0xF8, // BNE -8
    0xA2, 0x08, // LDX #$08
    0xA9, 0x00, // LDA #$00
    0x8D, 0x07, 0x20, // STA $2007
    0xCA, // DEX
    0xD0, 0xF8, // BNE -8
    0xA9, 0x00, // LDA #$00
    0x8D, 0x00, 0x20, // STA $2000
    0xA9, 0x0A, // LDA #$0A
    0x8D, 0x01, 0x20, // STA $2001
    0xD0, 0xFE, // BNE -2
];

/// Minimal NROM ROM: 16 KiB PRG (mirrored), CHR RAM, mapper 0.
fn nrom_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 16];
    rom[0..4].copy_from_slice(b"NES\x1A");
    rom[4] = 1; // 16 KiB PRG
    rom[5] = 0; // CHR RAM
    rom[6] = 0x00; // mapper 0
    rom[7] = 0x00;
    let mut prg = vec![0xEAu8; 16 * 1024];
    prg[..RENDER_PROGRAM.len()].copy_from_slice(&RENDER_PROGRAM);
    prg[0x3FFC..0x3FFE].copy_from_slice(&0x8000u16.to_le_bytes());
    rom.extend(prg);
    rom
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rust_nes_{name}_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn boot(dir: &std::path::Path) -> NesConsole {
    let rom_path = dir.join("state.nes");
    fs::write(&rom_path, nrom_rom()).unwrap();
    NesConsole::from_cartridge(Cartridge::from_file(rom_path.to_str().unwrap()).unwrap())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[test]
fn state_format_is_pinned() {
    let dir = temp_dir("state_format");
    let mut console = boot(&dir);
    for _ in 0..60 {
        console.step_frame();
    }

    let path = dir.join("pinned.state");
    console.save_state(path.to_str().unwrap());
    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(
        (bytes.len(), fnv1a(&bytes)),
        (20917usize, 12664245174736396932u64),
        "save-state layout changed: bump the format version and update this pin"
    );
}

#[test]
fn save_load_save_is_byte_identical() {
    let dir = temp_dir("state_bytes");
    let mut console = boot(&dir);
    for _ in 0..90 {
        console.step_frame();
    }

    let first = dir.join("first.state");
    let second = dir.join("second.state");
    console.save_state(first.to_str().unwrap());
    console.load_state(first.to_str().unwrap());
    console.save_state(second.to_str().unwrap());

    let a = fs::read(&first).unwrap();
    let b = fs::read(&second).unwrap();
    assert_eq!(a.len(), b.len(), "state size changed across save/load/save");
    assert!(
        a == b,
        "load_state does not restore everything save_state writes\
         ({} bytes, first difference at {:?})",
        a.len(),
        a.iter().zip(&b).position(|(x, y)| x != y)
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn loaded_state_resumes_identically_on_a_fresh_console() {
    let dir = temp_dir("state_resume");
    let state = dir.join("mid.state");

    let mut reference = boot(&dir);
    for _ in 0..60 {
        reference.step_frame();
    }
    reference.save_state(state.to_str().unwrap());

    let mut expected = vec![0u32; 256 * 240];
    for _ in 0..60 {
        reference.step_frame();
    }
    reference.read_frame_buffer(&mut expected);

    let mut replay = boot(&dir);
    replay.load_state(state.to_str().unwrap());
    for _ in 0..60 {
        replay.step_frame();
    }
    let mut actual = vec![0u32; 256 * 240];
    replay.read_frame_buffer(&mut actual);

    let mismatched = expected.iter().zip(&actual).filter(|(a, b)| a != b).count();
    assert_eq!(
        mismatched,
        0,
        "{mismatched} of {} pixels differ after loading into a fresh console",
        expected.len()
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Save a mapper's state, load it into a second instance of the same mapper,
/// and require the second instance to serialize back to identical bytes.
///
/// `poke` must change something the mapper serializes: a round-trip over a
/// pristine state would also pass if `load_state` did nothing at all.
fn assert_mapper_round_trips<M: Mapper>(mut source: M, mut target: M, poke: impl FnOnce(&mut M)) {
    let pristine = target.save_state();
    poke(&mut source);
    assert_ne!(
        source.save_state(),
        pristine,
        "the pokes changed no serialized field, so this round-trip proves nothing"
    );

    let saved = source.save_state();
    target.load_state(&saved);
    assert_eq!(target.save_state(), saved, "save/load is not symmetric");
}

#[test]
fn every_mapper_round_trips_its_state() {
    let make0 = || Mapper0::new(vec![0u8; 16 * 1024], Vec::new(), true);
    assert_mapper_round_trips(make0(), make0(), |m| {
        m.battery_ram_mut()
            .expect("NROM with the battery bit set exposes its WRAM")
            .fill(0x5A);
    });

    let make1 = || {
        Mapper1::new(
            vec![0u8; 32 * 1024],
            Vec::new(),
            Mirroring::HORIZONTAL,
            true,
        )
    };
    assert_mapper_round_trips(make1(), make1(), |m| m.write_prg(0x8000, 1));

    let make2 = || Mapper2::new(vec![0u8; 32 * 1024], Vec::new());
    assert_mapper_round_trips(make2(), make2(), |m| m.write_prg(0x8000, 0x05));

    let make3 = || Mapper3::new(vec![0u8; 16 * 1024], vec![0u8; 32 * 1024]);
    assert_mapper_round_trips(make3(), make3(), |m| m.write_prg(0x8000, 0x03));

    let make4 = || {
        Mapper4::new(
            vec![0u8; 128 * 1024],
            vec![0u8; 256 * 1024],
            Mirroring::VERTICAL,
            true,
        )
    };
    assert_mapper_round_trips(make4(), make4(), |m| m.write_prg(0x8000, 0x06));

    let make5 = || Mapper5::new(vec![0u8; 256 * 1024], vec![0u8; 256 * 1024], false);
    assert_mapper_round_trips(make5(), make5(), |m| m.write_prg(0x8000, 0x02));

    let make7 = || Mapper7::new(vec![0u8; 32 * 1024]);
    assert_mapper_round_trips(make7(), make7(), |m| m.write_prg(0x8000, 0x07));

    let make_vrc6 = || {
        Vrc6::new(
            vec![0u8; 128 * 1024],
            vec![0u8; 128 * 1024],
            24,
            Mirroring::VERTICAL,
            false,
        )
    };
    assert_mapper_round_trips(make_vrc6(), make_vrc6(), |m| m.write_prg(0x8000, 0x02));

    let make_fme7 = || Fme7::new(vec![0u8; 128 * 1024], vec![0u8; 128 * 1024], false, true);
    assert_mapper_round_trips(make_fme7(), make_fme7(), |m| {
        m.write_prg(0x8000, 0x00);
        m.write_prg(0xA000, 0x05);
    });
}
