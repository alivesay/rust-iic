use std::sync::Arc;
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;

pub struct Speaker {
    producer: Caching<Arc<HeapRb<f32>>, true, false>,
    sample_rate: u32,
    last_cycle: u64,
    state: bool, // Speaker cone position (in/out)
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
            state: false,
            toggles: VecDeque::new(),
            _stream: stream,
        }
    }

    pub fn toggle(&mut self, cycle: u64) {
        self.toggles.push_back(cycle);
    }

    pub fn update(&mut self, current_cycle: u64) {
        let cycles_per_second = 1_023_000.0;
        let cycles_per_sample = cycles_per_second / self.sample_rate as f64;
        
        let mut cycle_cursor = self.last_cycle as f64;
        let end_cycle = current_cycle as f64;

        // limit catch-up to avoid huge bursts if we fall behind
        if end_cycle - cycle_cursor > cycles_per_second {
             cycle_cursor = end_cycle - cycles_per_second;
             self.last_cycle = cycle_cursor as u64;
             self.toggles.clear();
        }

        while cycle_cursor < end_cycle {
            cycle_cursor += cycles_per_sample;
            
            // process toggles that happened before this sample time
            while let Some(&toggle_cycle) = self.toggles.front() {
                if (toggle_cycle as f64) < cycle_cursor {
                    self.state = !self.state;
                    self.toggles.pop_front();
                } else {
                    break;
                }
            }

            let sample = if self.state { 0.1 } else { -0.1 };
            let _ = self.producer.try_push(sample);
        }

        self.last_cycle = current_cycle;
        
        // if self.producer.len() < 100 {
        //     println!("Buffer low: {}", self.producer.len());
        // }
    }
}
