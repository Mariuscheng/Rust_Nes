use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};

/// Frames of audio allowed to wait in the sink before new samples are dropped.
///
/// One buffer is one emulated frame (~16.6 ms). The player paces itself, so
/// this cap is only an emergency valve for the case where the window was
/// blocked for a while; in steady state the queue sits at
/// [`AudioOut::TARGET_QUEUED_FRAMES`].
const MAX_QUEUED_FRAMES: usize = 6;

pub struct AudioOut {
    _stream: OutputStream,
    sink: Sink,
    sample_rate: u32,
}

impl AudioOut {
    /// The queue depth the frame pacer steers towards, in per-frame buffers.
    ///
    /// Three buffers are ~33 ms of buffered audio plus the one being played:
    /// enough to ride out scheduling jitter without adding noticeable latency.
    pub const TARGET_QUEUED_FRAMES: usize = 3;

    #[expect(
        clippy::expect_used,
        reason = "no output device is fatal here: the sink is the frame pacer's clock, so a silent fallback would silently change frame timing instead of failing at startup"
    )]
    pub fn new(sample_rate: u32) -> Self {
        let (_stream, stream_handle) =
            OutputStream::try_default().expect("Failed to open audio output");
        let sink = Sink::try_new(&stream_handle).expect("Failed to create audio sink");
        sink.set_volume(0.8);
        Self {
            _stream,
            sink,
            sample_rate,
        }
    }

    /// Number of per-frame buffers queued, including the one being played.
    ///
    /// This is the signal the frame pacer uses to correct for the difference
    /// between the emulator's nominal 44100 Hz and the audio device's real
    /// clock: a growing queue means the emulator is ahead, a shrinking one
    /// means it will underrun.
    pub fn queued_frames(&self) -> usize {
        self.sink.len()
    }

    /// Append samples (mono, f32) to the playback sink without blocking.
    ///
    /// Pacing belongs to the caller ([`crate::run_with_window`]): the old
    /// implementation slept while the queue was long, which handed the frame
    /// rate to the audio callback and made video stutter. If the queue has
    /// drifted past [`MAX_QUEUED_FRAMES`] (e.g. the window was frozen), this
    /// frame's samples are skipped instead of growing latency; playback of the
    /// already-queued audio continues, so the gap is a missing sliver of sound
    /// rather than a click at the end of the queue.
    pub fn append(&self, samples: Vec<f32>) {
        if samples.is_empty() || self.queued_frames() >= MAX_QUEUED_FRAMES {
            return;
        }
        let source = SamplesBuffer::new(1, self.sample_rate, samples);
        self.sink.append(source);
    }
}
