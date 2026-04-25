use std::sync::Arc;
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;
use std::collections::VecDeque;

const AMPLITUDE: f32 = 0.1;
use crate::timing;

const CYCLES_PER_SECOND: f64 = timing::CYCLES_PER_SECOND;
const DECAY_SECONDS: f64 = 0.02; // ~20ms

pub type AudioProducer = Caching<Arc<HeapRb<f32>>, true, false>;

pub struct Speaker {
    producer: AudioProducer,
    sample_rate: u32,
    last_cycle: u64,
    state: f32, // Speaker cone position: +AMPLITUDE or -AMPLITUDE
    filtered: f32, // Single-pole low-pass filter state
    last_toggle_cycle: u64, // Cycle of most recent toggle (for idle detection)
    toggles: VecDeque<u64>, // Cycle counts when toggles occurred
}

impl Speaker {
    pub fn new(producer: AudioProducer, sample_rate: u32) -> Self {
        Self {
            producer,
            sample_rate,
            last_cycle: 0,
            state: -AMPLITUDE,
            filtered: 0.0,
            last_toggle_cycle: 0,
            toggles: VecDeque::new(),
        }
    }

    // Called when $C030 is accessed.
    pub fn toggle(&mut self, cycle: u64) {
        self.toggles.push_back(cycle);
        self.last_toggle_cycle = cycle;
    }

    pub fn update(&mut self, current_cycle: u64) {
        let cycles_per_sample = CYCLES_PER_SECOND / self.sample_rate as f64;
        let decay_cycles = (DECAY_SECONDS * CYCLES_PER_SECOND) as u64;

        // Single-pole low-pass filter coefficient (~12kHz cutoff)
        let cutoff = 12000.0_f64;
        let alpha = (1.0 - (-std::f64::consts::TAU * cutoff / self.sample_rate as f64).exp()) as f32;

        let mut cycle_cursor = self.last_cycle as f64;
        let end_cycle = current_cycle as f64;

        // Limit catch-up to avoid huge bursts (e.g. fast-disk mode)
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

        // Pre-allocate sample buffer for batch pushing
        // ~735 samples per frame at 44100Hz/60fps, give some headroom
        let estimated_samples = ((end_cycle - cycle_cursor) / cycles_per_sample) as usize + 16;
        let mut samples = Vec::with_capacity(estimated_samples.min(2048));

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

            // Decay to silence when idle
            let sample_cycle = cycle_cursor as u64;
            let raw = if sample_cycle.saturating_sub(self.last_toggle_cycle) > decay_cycles {
                0.0
            } else {
                self.state
            };

            // Low-pass filter to reduce aliasing
            self.filtered += alpha * (raw - self.filtered);

            samples.push(self.filtered);
        }

        // Batch push all samples at once
        let _ = self.producer.push_slice(&samples);

        self.last_cycle = current_cycle;
    }
}
