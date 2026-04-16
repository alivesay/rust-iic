//! Host audio backend using cpal.
//!
//! This module is platform-specific and should NOT be part of the core
//! emulation crate. It provides the actual audio playback via cpal.
//! The Speaker device generates samples; this module plays them.

use ringbuf::{HeapRb, traits::*};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::device::speaker::AudioProducer;

/// Audio output stream powered by cpal.
/// 
/// Holds the cpal stream and the consumer side of the sample ringbuffer.
/// The Speaker device pushes samples to the producer side.
pub struct AudioOutput {
    _stream: cpal::Stream,
}

/// Creates the audio ringbuffer and output stream.
/// 
/// Returns (producer, sample_rate, AudioOutput) - pass producer and sample_rate to Speaker::new().
pub fn create_audio() -> (AudioProducer, u32, AudioOutput) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let config = device.default_output_config().expect("no default config");
    let sample_rate = config.sample_rate().0;

    let ring = HeapRb::<f32>::new(sample_rate as usize / 2); // 0.5 seconds buffer
    let (producer, mut consumer) = ring.split();

    let channels = config.channels() as usize;

    let err_fn = |err| eprintln!("audio stream error: {}", err);

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
        _ => panic!("Unsupported audio sample format"),
    }.expect("failed to build audio stream");

    stream.play().expect("failed to start audio stream");

    (producer, sample_rate, AudioOutput { _stream: stream })
}
