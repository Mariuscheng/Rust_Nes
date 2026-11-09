/// Debug output observer for test ROMs that use the WRAM-based protocol
/// ($6000-$6003 magic signature = $DE $B0 $61, $6004+ = null-terminated text).
///
/// This module has NO knowledge of NES hardware — it only knows about the
/// WRAM-based serial protocol used by certain test ROMs (e.g. blargg tests).
/// It is attached to the emulator via `Nes::set_frame_callback()`, so the
/// core emulator stays completely agnostic of any test-ROM-specific behavior.

/// State machine for the `DebugPrinter` frame callback.
pub struct DebugPrinter {
    last_status: Option<u8>,
    last_text: String,
    frame_count: u64,
}

impl DebugPrinter {
    /// Create a new debug output printer.
    pub fn new() -> Self {
        Self {
            last_status: None,
            last_text: String::new(),
            frame_count: 0,
        }
    }

    /// Returns a `FrameCallback` closure that can be registered with `Nes::set_frame_callback()`.
    ///
    /// The printer state is moved into the closure, so the callback owns it for
    /// its whole lifetime — no raw pointers or `unsafe` required.
    pub fn make_callback(mut self) -> Box<dyn FnMut(u64, &[u8; 8192])> {
        Box::new(move |_frame, wram: &[u8; 8192]| {
            self.process(wram);
        })
    }

    fn process(&mut self, wram: &[u8; 8192]) {
        self.frame_count += 1;

        // Check magic bytes at $6001-$6003
        if wram[0x0001] != 0xDE || wram[0x0002] != 0xB0 || wram[0x0003] != 0x61 {
            return;
        }

        let status = wram[0x0000];
        let mut text_bytes = Vec::new();
        for i in 4..8192 {
            let b = wram[i];
            if b == 0 {
                break;
            }
            text_bytes.push(b);
        }
        let text = String::from_utf8_lossy(&text_bytes).into_owned();

        let status_changed = self.last_status != Some(status);
        let text_changed = self.last_text != text;
        const THROTTLE_FRAMES: u64 = 60;

        let should_print = status_changed
            || (text_changed && self.frame_count % THROTTLE_FRAMES == 0);

        if should_print {
            self.last_status = Some(status);
            self.last_text = text.clone();

            let status_desc = match status {
                0x80 => "Running...",
                0x81 => "Reset pending (delay 100ms)",
                0x00..=0x7F => "Test completed",
                _ => "Unknown status",
            };
            println!(
                "[DEBUG $6000] Status: 0x{:02X} ({})",
                status, status_desc
            );
            if !text.is_empty() {
                println!("[DEBUG $6000] Output:\n{}", text);
            }
        }
    }
}