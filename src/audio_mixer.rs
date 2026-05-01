use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// Audio sample producer type
pub type AudioProducer = Caching<Arc<HeapRb<f32>>, true, false>;

#[derive(Clone)]
pub struct AudioControls {
    inner: Arc<AudioControlsInner>,
}

struct AudioControlsInner {
    master: AtomicU32,
    speaker: AtomicU32,
    mockingboard1: AtomicU32,
    mockingboard2: AtomicU32,
    drive: AtomicU32,
    muted: std::sync::atomic::AtomicBool,
}

impl AudioControls {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AudioControlsInner {
                master: AtomicU32::new(1.0_f32.to_bits()),
                speaker: AtomicU32::new(1.0_f32.to_bits()),
                mockingboard1: AtomicU32::new(1.0_f32.to_bits()),
                mockingboard2: AtomicU32::new(1.0_f32.to_bits()),
                drive: AtomicU32::new(1.0_f32.to_bits()),
                muted: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    #[inline]
    fn load(a: &AtomicU32) -> f32 { f32::from_bits(a.load(Ordering::Relaxed)) }
    #[inline]
    fn store(a: &AtomicU32, v: f32) { a.store(v.max(0.0).to_bits(), Ordering::Relaxed); }

    pub fn master(&self) -> f32 { Self::load(&self.inner.master) }
    pub fn speaker(&self) -> f32 { Self::load(&self.inner.speaker) }
    pub fn mockingboard1(&self) -> f32 { Self::load(&self.inner.mockingboard1) }
    pub fn mockingboard2(&self) -> f32 { Self::load(&self.inner.mockingboard2) }
    pub fn drive(&self) -> f32 { Self::load(&self.inner.drive) }
    pub fn muted(&self) -> bool { self.inner.muted.load(Ordering::Relaxed) }

    pub fn set_master(&self, v: f32) { Self::store(&self.inner.master, v); }
    pub fn set_speaker(&self, v: f32) { Self::store(&self.inner.speaker, v); }
    pub fn set_mockingboard1(&self, v: f32) { Self::store(&self.inner.mockingboard1, v); }
    pub fn set_mockingboard2(&self, v: f32) { Self::store(&self.inner.mockingboard2, v); }
    pub fn set_drive(&self, v: f32) { Self::store(&self.inner.drive, v); }
    pub fn set_muted(&self, v: bool) { self.inner.muted.store(v, Ordering::Relaxed); }
}

impl Default for AudioControls {
    fn default() -> Self { Self::new() }
}

// Central audio mixer - mixes multiple sources into a single output
pub struct AudioMixer {
    _stream: cpal::Stream,
    sample_rate: u32,
    controls: AudioControls,
}

// Producers for each audio channel
pub struct AudioProducers {
    pub speaker: AudioProducer,
    pub mockingboard1: AudioProducer,
    pub mockingboard2: AudioProducer,
    pub drive_audio: AudioProducer,
}

// Maximum callback buffer size we expect (4096 samples is typical max)
const MAX_CALLBACK_SIZE: usize = 4096;

impl AudioMixer {
    // Create a new audio mixer with producers for all channels
    pub fn new() -> (Self, AudioProducers) {
        let host = cpal::default_host();
        let device = host.default_output_device().expect("no output device available");
        let config = device.default_output_config().expect("no default config");
        let sample_rate = config.sample_rate().0;

        // Create ring buffers - 100ms buffer is plenty for smooth playback
        // Mockingboard buffers are 2x size for stereo (interleaved L/R)
        let buffer_size = (sample_rate as usize) / 10;
        
        let speaker_ring = HeapRb::<f32>::new(buffer_size);
        let (speaker_prod, mut speaker_cons) = speaker_ring.split();
        
        let mb1_ring = HeapRb::<f32>::new(buffer_size * 2);  // Stereo
        let (mb1_prod, mut mb1_cons) = mb1_ring.split();
        
        let mb2_ring = HeapRb::<f32>::new(buffer_size * 2);  // Stereo
        let (mb2_prod, mut mb2_cons) = mb2_ring.split();
        
        let drive_ring = HeapRb::<f32>::new(buffer_size);
        let (drive_prod, mut drive_cons) = drive_ring.split();

        let output_channels = config.channels() as usize;
        let err_fn = |err| eprintln!("audio stream error: {}", err);
        
        // Pre-allocated scratch buffers for batch processing
        // Mockingboard buffers are 2x size for stereo (interleaved L/R)
        let mut speaker_buf = vec![0.0f32; MAX_CALLBACK_SIZE];
        let mut mb1_buf = vec![0.0f32; MAX_CALLBACK_SIZE * 2];
        let mut mb2_buf = vec![0.0f32; MAX_CALLBACK_SIZE * 2];
        let mut drive_buf = vec![0.0f32; MAX_CALLBACK_SIZE];
        
        // Track last sample values to prevent clicks on underrun (stereo: L/R pairs)
        let mut last_mb1_l = 0.0f32;
        let mut last_mb1_r = 0.0f32;
        let mut last_mb2_l = 0.0f32;
        let mut last_mb2_r = 0.0f32;

        let controls = AudioControls::new();
        let cb_controls = controls.clone();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let num_frames = data.len() / output_channels;

                    // Snapshot per-source volumes for this callback. Reading
                    // once per buffer rather than per-sample avoids any
                    // perceived stepping if the UI is dragging a slider.
                    let muted = cb_controls.muted();
                    let master = if muted { 0.0 } else { cb_controls.master() };
                    let v_speaker = master * cb_controls.speaker();
                    let v_mb1 = master * cb_controls.mockingboard1();
                    let v_mb2 = master * cb_controls.mockingboard2();
                    let v_drive = master * cb_controls.drive();

                    // Batch pop from each ring buffer into scratch buffers
                    // Mockingboard is stereo (2 samples per frame)
                    let speaker_count = speaker_cons.pop_slice(&mut speaker_buf[..num_frames]);
                    let mb1_stereo_count = mb1_cons.pop_slice(&mut mb1_buf[..num_frames * 2]);
                    let mb2_stereo_count = mb2_cons.pop_slice(&mut mb2_buf[..num_frames * 2]);
                    let drive_count = drive_cons.pop_slice(&mut drive_buf[..num_frames]);
                    
                    // Number of complete stereo frames we got
                    let mb1_frames = mb1_stereo_count / 2;
                    let mb2_frames = mb2_stereo_count / 2;
                    
                    // Mix and output
                    for (i, frame) in data.chunks_mut(output_channels).enumerate() {
                        // Speaker and drive audio: 0 on underrun (transient sounds), mono
                        let speaker = if i < speaker_count { speaker_buf[i] } else { 0.0 };
                        let drive = if i < drive_count { drive_buf[i] } else { 0.0 };
                        
                        // Mockingboard: stereo samples (interleaved L, R)
                        // Hold last value on underrun to prevent clicks
                        let (mb1_l, mb1_r) = if i < mb1_frames {
                            last_mb1_l = mb1_buf[i * 2];
                            last_mb1_r = mb1_buf[i * 2 + 1];
                            (mb1_buf[i * 2], mb1_buf[i * 2 + 1])
                        } else {
                            (last_mb1_l, last_mb1_r)
                        };
                        let (mb2_l, mb2_r) = if i < mb2_frames {
                            last_mb2_l = mb2_buf[i * 2];
                            last_mb2_r = mb2_buf[i * 2 + 1];
                            (mb2_buf[i * 2], mb2_buf[i * 2 + 1])
                        } else {
                            (last_mb2_l, last_mb2_r)
                        };

                        // Apply per-source volumes (master folded in).
                        let sp = speaker * v_speaker;
                        let dr = drive * v_drive;
                        let m1l = mb1_l * v_mb1;
                        let m1r = mb1_r * v_mb1;
                        let m2l = mb2_l * v_mb2;
                        let m2r = mb2_r * v_mb2;

                        // Mix stereo: left channel, right channel
                        let left = sp + m1l + m2l + dr;
                        let right = sp + m1r + m2r + dr;
                        
                        // Output to stereo
                        if output_channels >= 2 {
                            frame[0] = soft_clip(left);
                            frame[1] = soft_clip(right);
                            // Fill any additional channels with average
                            for ch in frame.iter_mut().skip(2) {
                                *ch = soft_clip((left + right) * 0.5);
                            }
                        } else {
                            // Mono output: mix down
                            frame[0] = soft_clip((left + right) * 0.5);
                        }
                    }
                },
                err_fn,
                None,
            ),
            _ => panic!("Unsupported audio sample format"),
        }.expect("failed to build audio stream");

        stream.play().expect("failed to start audio stream");

        let mixer = AudioMixer {
            _stream: stream,
            sample_rate,
            controls,
        };

        let producers = AudioProducers {
            speaker: speaker_prod,
            mockingboard1: mb1_prod,
            mockingboard2: mb2_prod,
            drive_audio: drive_prod,
        };

        (mixer, producers)
    }

    // Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn controls(&self) -> AudioControls {
        self.controls.clone()
    }
}

pub struct DummyAudioMixer {
    sample_rate: u32,
    controls: AudioControls,
}

impl DummyAudioMixer {
    pub fn new() -> (Self, AudioProducers) {
        let sample_rate = 44100;
        let buffer_size = sample_rate as usize / 10;

        let speaker_ring = HeapRb::<f32>::new(buffer_size);
        let (speaker_prod, _speaker_cons) = speaker_ring.split();

        let mb1_ring = HeapRb::<f32>::new(buffer_size * 2);
        let (mb1_prod, _mb1_cons) = mb1_ring.split();

        let mb2_ring = HeapRb::<f32>::new(buffer_size * 2);
        let (mb2_prod, _mb2_cons) = mb2_ring.split();

        let drive_ring = HeapRb::<f32>::new(buffer_size);
        let (drive_prod, _drive_cons) = drive_ring.split();

        let mixer = DummyAudioMixer { sample_rate, controls: AudioControls::new() };
        let producers = AudioProducers {
            speaker: speaker_prod,
            mockingboard1: mb1_prod,
            mockingboard2: mb2_prod,
            drive_audio: drive_prod,
        };

        (mixer, producers)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn controls(&self) -> AudioControls {
        self.controls.clone()
    }
}

#[inline(always)]
fn soft_clip(x: f32) -> f32 {
    // Fast soft clipper using polynomial approximation for small values
    // tanh for larger values
    if x >= -0.5 && x <= 0.5 {
        x
    } else if x >= -1.5 && x <= 1.5 {
        // Cubic approximation of tanh in the -1.5..1.5 range
        x - x * x * x / 3.0
    } else {
        x.tanh()
    }
}
