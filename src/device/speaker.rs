use std::sync::Arc;
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;

const AMPLITUDE: f32 = 0.1;
const CYCLES_PER_SECOND: f64 = 1_023_000.0;
// Decay time in seconds — after this long without a toggle, output goes silent
const DECAY_SECONDS: f64 = 0.02; // ~20ms

pub struct Speaker {
    producer: Caching<Arc<HeapRb<f32>>, true, false>,
    sample_rate: u32,
    last_cycle: u64,
    state: f32, // Speaker cone position: +AMPLITUDE or -AMPLITUDE
    filtered: f32, // Single-pole low-pass filter state
    last_toggle_cycle: u64, // Cycle of most recent toggle (for idle detection)
    toggles: VecDeque<u64>, // Cycle counts when toggles occurred
    _stream: cpal::Stream, // Keep stream alive
}

impl Speaker {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host.default_output_device().expect("no output device available");
        let config = device.default_output_config().expect("no default config");
        let sample_rate = config.sample_rate().0;

        let ring = HeapRb::<f32>::new(sample_rate as usize / 2); // 0.5 seconds buffer
        let (producer, mut consumer) = ring.split();

        let channels = config.channels() as usize;

        let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(channels) {
                        let sample = consumer.try_pop().unwrap_or(0.0);
                        for sample_out in frame.iter_mut() {
                            *sample_out = sample;
                        }
                    }
                },
                err_fn,
                None,
            ),
            _ => panic!("Unsupported sample format"),
        }.unwrap();

        stream.play().unwrap();

        Self {
            producer,
            sample_rate,
            last_cycle: 0,
            state: -AMPLITUDE,
            filtered: 0.0,
            last_toggle_cycle: 0,
            toggles: VecDeque::new(),
            _stream: stream,
        }
    }

    pub fn toggle(&mut self, cycle: u64) {
        self.toggles.push_back(cycle);
        self.last_toggle_cycle = cycle;
    }

    pub fn update(&mut self, current_cycle: u64) {
        let cycles_per_sample = CYCLES_PER_SECOND / self.sample_rate as f64;
        let decay_cycles = (DECAY_SECONDS * CYCLES_PER_SECOND) as u64;

        // Single-pole low-pass filter coefficient (~7kHz cutoff)
        // alpha = 1 - e^(-2*pi*fc/fs)
        let cutoff = 12000.0_f64;
        let alpha = (1.0 - (-std::f64::consts::TAU * cutoff / self.sample_rate as f64).exp()) as f32;

        let mut cycle_cursor = self.last_cycle as f64;
        let end_cycle = current_cycle as f64;

        // Limit catch-up to avoid huge bursts (e.g. fast-disk mode)
        // Only generate audio for the real-time portion
        let max_catchup = CYCLES_PER_SECOND / 60.0 * 2.0; // ~2 frames worth
        if end_cycle - cycle_cursor > max_catchup {
            // Fast-forward: process all toggles but don't generate samples
            while let Some(&toggle_cycle) = self.toggles.front() {
                if (toggle_cycle as f64) < end_cycle - max_catchup {
                    self.state = -self.state;
                    self.last_toggle_cycle = toggle_cycle;
                    self.toggles.pop_front();
                } else {
                    break;
                }
            }
            cycle_cursor = end_cycle - max_catchup;
        }

        while cycle_cursor < end_cycle {
            cycle_cursor += cycles_per_sample;

            // Process toggles that happened before this sample time
            while let Some(&toggle_cycle) = self.toggles.front() {
                if (toggle_cycle as f64) <= cycle_cursor {
                    self.state = -self.state;
                    self.last_toggle_cycle = toggle_cycle;
                    self.toggles.pop_front();
                } else {
                    break;
                }
            }

            // Decay to silence when idle (use per-sample cycle position)
            let sample_cycle = cycle_cursor as u64;
            let raw = if sample_cycle.saturating_sub(self.last_toggle_cycle) > decay_cycles {
                0.0
            } else {
                self.state
            };

            // Low-pass filter to reduce aliasing
            self.filtered += alpha * (raw - self.filtered);

            let _ = self.producer.try_push(self.filtered);
        }

        self.last_cycle = current_cycle;
    }
}
