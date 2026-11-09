use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};

pub struct AudioOut {
    _stream: OutputStream,
    sink: Sink,
    sample_rate: u32,
}

impl AudioOut {
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

    /// Append samples (mono, f32) to the playback sink.
    /// Limits internal queue to prevent audio latency buildup.
    pub fn append(&self, samples: Vec<f32>) {
        if samples.is_empty() {
            return;
        }
        // 防止音訊累積過多造成延遲：最多保留 2 幀在佇列中
        while self.sink.len() > 2 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let source = SamplesBuffer::new(1, self.sample_rate, samples);
        self.sink.append(source);
    }
}
