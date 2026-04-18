//! Apple IIc Disk Drive Audio Synthesis
//!
//! Generates synthesized drive sounds in real-time using physically accurate models:
//! - Stepper motor clicks: filtered noise + dual-frequency damped oscillators
//! - Motor relay click: sharp impact on motor start
//! - Motor sound: disk air turbulence (filtered noise) + subtle cogging + spinup/spindown
//!
//! All parameters are runtime-adjustable via DriveAudioParams.

use std::sync::Arc;
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;

pub type AudioProducer = Caching<Arc<HeapRb<f32>>, true, false>;

const CYCLES_PER_SECOND: f64 = 1_023_000.0;

/// Runtime-adjustable drive audio synthesis parameters
#[derive(Clone, Debug)]
pub struct DriveAudioParams {
    // Overall
    pub master_volume: f32,
    pub enabled: bool,
    
    // Click (stepper) parameters
    pub click_volume: f32,
    pub click_noise_decay_ms: f32,      // Noise burst decay time in ms
    pub click_filter_freq: f32,          // Noise lowpass cutoff Hz
    // Body clack (multi-stage impact synthesizer)
    pub click_body_freq: f32,            // Base body frequency (~650 Hz)
    pub click_body_decay_ms: f32,        // Body tone decay
    pub click_body_mix: f32,             // Overall body level
    pub click_attack_mix: f32,           // Initial attack transient level (0-1)
    pub click_attack_decay_ms: f32,      // Attack decay time (very fast, ~1ms)
    pub click_pitch_sweep: f32,          // Pitch sweep start (1.0 = no sweep, 1.3 = minor 3rd up)
    pub click_pitch_sweep_ms: f32,       // Time for pitch to settle
    pub click_harmonic_mix: f32,         // 2nd harmonic level (0-1)
    // Metallic tick
    pub click_tick_freq: f32,            // High metallic tick (~1500 Hz)
    pub click_tick_decay_ms: f32,
    pub click_tick_mix: f32,
    // Click crunch layer (high-freq grit)
    pub click_crunch_decay_ms: f32,
    pub click_crunch_freq: f32,
    pub click_crunch_mix: f32,
    
    // Motor relay click (fires once on motor start)
    pub relay_volume: f32,
    pub relay_freq: f32,                 // ~700 Hz
    pub relay_decay_ms: f32,             // 5-8ms
    
    // Motor parameters
    pub motor_volume: f32,
    pub motor_filter_freq: f32,
    pub motor_cog_freq: f32,
    pub motor_cog_mix: f32,
    pub motor_spinup_ms: f32,
    pub motor_spindown_ms: f32,
}

impl Default for DriveAudioParams {
    fn default() -> Self {
        Self {
            master_volume: 0.80,
            enabled: true,
            
            // Click - multi-stage body clack + metallic tick
            click_volume: 0.15,
            click_noise_decay_ms: 12.2,
            click_filter_freq: 3600.0,
            click_body_freq: 280.0,
            click_body_decay_ms: 15.0,
            click_body_mix: 0.75,
            click_attack_mix: 0.28,
            click_attack_decay_ms: 1.2,
            click_pitch_sweep: 1.38,
            click_pitch_sweep_ms: 6.5,
            click_harmonic_mix: 0.95,
            click_tick_freq: 1530.0,
            click_tick_decay_ms: 10.5,
            click_tick_mix: 0.72,
            click_crunch_decay_ms: 2.0,
            click_crunch_freq: 1000.0,
            click_crunch_mix: 0.58,
            
            // Motor relay click
            relay_volume: 0.12,
            relay_freq: 940.0,
            relay_decay_ms: 5.9,
            
            // Motor
            motor_volume: 0.015,
            motor_filter_freq: 120.0,
            motor_cog_freq: 22.0,
            motor_cog_mix: 0.33,
            motor_spinup_ms: 250.0,
            motor_spindown_ms: 400.0,
        }
    }
}

/// Simple linear congruential RNG for noise generation
struct NoiseGen {
    state: u32,
}

impl NoiseGen {
    fn new() -> Self {
        Self { state: 0x12345678 }
    }
    
    fn with_seed(seed: u32) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.state as i32 as f32) / (i32::MAX as f32)
    }
}

/// Damped oscillator - correct model for mechanical transients
/// Simulates impulse response of a resonant physical system
struct DampedOscillator {
    phase: f32,
    amplitude: f32,
    freq: f32,
    decay: f32,  // per-sample multiplier
}

impl DampedOscillator {
    fn new(freq: f32, decay: f32) -> Self {
        Self {
            phase: 0.0,
            amplitude: 0.0,
            freq,
            decay,
        }
    }

    fn trigger(&mut self, amp: f32) {
        // Accumulate amplitude on retrigger (max 1.5× base) to avoid pops
        self.amplitude = (self.amplitude + amp).min(amp * 1.5);
        self.phase = 0.0;
    }

    fn tick(&mut self, sample_rate: f32) -> f32 {
        if self.amplitude < 0.0001 {
            return 0.0;
        }
        self.phase += self.freq / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.amplitude *= self.decay;
        (self.phase * std::f32::consts::TAU).sin() * self.amplitude
    }

    fn is_active(&self) -> bool {
        self.amplitude > 0.0001
    }
}

/// Multi-stage impact synthesizer for realistic hammer/clack sounds
/// Combines: attack transient, body with pitch sweep, and harmonics
struct ImpactSynthesizer {
    sample_rate: f32,
    
    // Attack transient - very short high-freq burst for initial "crack"
    attack_phase: f32,
    attack_amp: f32,
    attack_freq: f32,        // ~2000-3000 Hz
    attack_decay: f32,       // Very fast decay (~1ms)
    attack_mix: f32,         // Configurable level (0-1)
    
    // Body tone with pitch sweep
    body_phase: f32,
    body_amp: f32,
    body_freq: f32,          // Starting freq (~600-800 Hz)
    body_decay: f32,
    pitch_sweep: f32,        // Current pitch multiplier (starts > 1, decays to 1)
    pitch_sweep_start: f32,  // Starting pitch sweep value
    pitch_sweep_decay: f32,  // How fast pitch settles
    
    // 2nd harmonic for complexity
    harmonic_phase: f32,
    harmonic_amp: f32,
    harmonic_decay: f32,     // Decays faster than body
    harmonic_mix: f32,       // Configurable level (0-1)
}

impl ImpactSynthesizer {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            attack_phase: 0.0,
            attack_amp: 0.0,
            attack_freq: 2800.0,
            attack_decay: 0.001_f32.powf(1.0 / (0.001 * sample_rate)), // 1ms decay
            attack_mix: 0.8,
            body_phase: 0.0,
            body_amp: 0.0,
            body_freq: 650.0,
            body_decay: 0.001_f32.powf(1.0 / (0.005 * sample_rate)), // 5ms decay
            pitch_sweep: 1.0,
            pitch_sweep_start: 1.3,
            pitch_sweep_decay: 0.001_f32.powf(1.0 / (0.003 * sample_rate)), // 3ms sweep
            harmonic_phase: 0.0,
            harmonic_amp: 0.0,
            harmonic_decay: 0.001_f32.powf(1.0 / (0.003 * sample_rate)), // 3ms (faster than body)
            harmonic_mix: 0.5,
        }
    }
    
    fn configure(&mut self, body_freq: f32, body_decay_ms: f32, attack_mix: f32, 
                 attack_decay_ms: f32, pitch_sweep_start: f32, pitch_sweep_ms: f32,
                 harmonic_mix: f32) {
        self.body_freq = body_freq;
        self.body_decay = 0.001_f32.powf(1.0 / (body_decay_ms * 0.001 * self.sample_rate));
        
        // Attack freq is ~4x body for crisp transient
        self.attack_freq = (body_freq * 4.0).min(4000.0);
        self.attack_decay = 0.001_f32.powf(1.0 / (attack_decay_ms * 0.001 * self.sample_rate));
        self.attack_mix = attack_mix;
        
        // Pitch sweep
        self.pitch_sweep_start = pitch_sweep_start;
        self.pitch_sweep_decay = 0.001_f32.powf(1.0 / (pitch_sweep_ms * 0.001 * self.sample_rate));
        
        // Harmonic decays 60% faster than body for natural falloff
        self.harmonic_decay = 0.001_f32.powf(1.0 / (body_decay_ms * 0.0004 * self.sample_rate));
        self.harmonic_mix = harmonic_mix;
    }
    
    fn trigger(&mut self, amp: f32) {
        // Attack starts at configured mix level
        self.attack_amp = (self.attack_amp + amp * self.attack_mix).min(amp * self.attack_mix * 1.5);
        self.attack_phase = 0.0;
        
        // Body tone
        self.body_amp = (self.body_amp + amp).min(amp * 1.5);
        self.body_phase = 0.0;
        
        // Start pitch sweep at configured amount
        self.pitch_sweep = self.pitch_sweep_start;
        
        // Harmonic at configured mix level
        self.harmonic_amp = (self.harmonic_amp + amp * self.harmonic_mix).min(amp * self.harmonic_mix * 1.5);
        self.harmonic_phase = 0.0;
    }
    
    fn tick(&mut self) -> f32 {
        let mut out = 0.0;
        
        // Attack transient (sharp crack)
        if self.attack_amp > 0.0001 {
            self.attack_phase += self.attack_freq / self.sample_rate;
            if self.attack_phase >= 1.0 { self.attack_phase -= 1.0; }
            // Use triangle wave for sharper attack transient
            let tri = if self.attack_phase < 0.5 {
                self.attack_phase * 4.0 - 1.0
            } else {
                3.0 - self.attack_phase * 4.0
            };
            out += tri * self.attack_amp;
            self.attack_amp *= self.attack_decay;
        }
        
        // Body tone with pitch sweep
        if self.body_amp > 0.0001 {
            let swept_freq = self.body_freq * self.pitch_sweep;
            self.body_phase += swept_freq / self.sample_rate;
            if self.body_phase >= 1.0 { self.body_phase -= 1.0; }
            out += (self.body_phase * std::f32::consts::TAU).sin() * self.body_amp;
            self.body_amp *= self.body_decay;
            
            // Decay pitch sweep toward 1.0
            self.pitch_sweep = 1.0 + (self.pitch_sweep - 1.0) * self.pitch_sweep_decay;
        }
        
        // 2nd harmonic (2x frequency, faster decay)
        if self.harmonic_amp > 0.0001 {
            let harm_freq = self.body_freq * 2.0 * self.pitch_sweep;
            self.harmonic_phase += harm_freq / self.sample_rate;
            if self.harmonic_phase >= 1.0 { self.harmonic_phase -= 1.0; }
            out += (self.harmonic_phase * std::f32::consts::TAU).sin() * self.harmonic_amp;
            self.harmonic_amp *= self.harmonic_decay;
        }
        
        out
    }
    
    fn is_active(&self) -> bool {
        self.attack_amp > 0.0001 || self.body_amp > 0.0001 || self.harmonic_amp > 0.0001
    }
}

/// Simple one-pole lowpass filter for noise shaping
struct LowpassFilter {
    state: f32,
    coeff: f32,
}

impl LowpassFilter {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        let dt = 1.0 / sample_rate;
        let coeff = dt / (rc + dt);
        Self { state: 0.0, coeff }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.state += self.coeff * (input - self.state);
        self.state
    }

    #[allow(dead_code)]
    fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// Drive audio event types
#[derive(Clone, Copy, Debug)]
pub enum DriveEvent {
    Step { quarter_track: u8 },
    MotorOn,
    MotorOff,
}

/// Drive audio synthesizer using physically-based models
pub struct DriveAudio {
    producer: Option<AudioProducer>,
    sample_rate: u32,
    last_cycle: u64,

    pub params: DriveAudioParams,

    // Separate noise generators per filter to avoid correlation
    noise_click: NoiseGen,
    noise_crunch: NoiseGen,
    noise_motor: NoiseGen,

    // Click synthesis: noise burst + impact synth (body clack) + tick
    click_noise_amp: f32,
    click_noise_decay: f32,
    click_filter: LowpassFilter,
    click_body: ImpactSynthesizer,      // Multi-stage body clack
    click_tick: DampedOscillator,      // High metallic ~1500 Hz
    click_crunch_amp: f32,
    click_crunch_decay: f32,
    click_crunch_filter: LowpassFilter,
    last_quarter_track: u8,
    last_step_cycle: u64,              // For seek speed detection

    // Motor relay click (sharp clack on motor start)
    relay_noise: NoiseGen,
    relay_noise_amp: f32,
    relay_noise_decay: f32,
    relay_filter: LowpassFilter,
    relay_body: DampedOscillator,      // Body resonance ~750 Hz
    relay_tick: DampedOscillator,      // Metallic tick ~2000 Hz

    // Motor synthesis with spinup/spindown
    motor_on: bool,
    motor_envelope: f32,               // 0.0-1.0, for spinup/spindown
    motor_spinup_rate: f32,            // Per-sample increment
    motor_spindown_rate: f32,          // Per-sample decrement
    motor_filter: LowpassFilter,
    motor_cog_phase: f32,
    motor_cog_freq: f32,

    events: std::collections::VecDeque<(u64, DriveEvent)>,
}

impl DriveAudio {
    pub fn new() -> Self {
        let sample_rate = 44100.0;
        let params = DriveAudioParams::default();
        
        Self {
            producer: None,
            sample_rate: 44100,
            last_cycle: 0,
            params: params.clone(),

            // Different seeds for each noise source
            noise_click: NoiseGen::with_seed(0x12345678),
            noise_crunch: NoiseGen::with_seed(0xDEADBEEF),
            noise_motor: NoiseGen::with_seed(0x8BADF00D),

            click_noise_amp: 0.0,
            click_noise_decay: Self::decay_from_ms(params.click_noise_decay_ms, sample_rate),
            click_filter: LowpassFilter::new(params.click_filter_freq, sample_rate),
            click_body: {
                let mut synth = ImpactSynthesizer::new(sample_rate);
                synth.configure(
                    params.click_body_freq, params.click_body_decay_ms,
                    params.click_attack_mix, params.click_attack_decay_ms,
                    params.click_pitch_sweep, params.click_pitch_sweep_ms,
                    params.click_harmonic_mix
                );
                synth
            },
            click_tick: DampedOscillator::new(
                params.click_tick_freq,
                Self::decay_from_ms(params.click_tick_decay_ms, sample_rate),
            ),
            click_crunch_amp: 0.0,
            click_crunch_decay: Self::decay_from_ms(params.click_crunch_decay_ms, sample_rate),
            click_crunch_filter: LowpassFilter::new(params.click_crunch_freq, sample_rate),
            last_quarter_track: 0,
            last_step_cycle: 0,

            relay_noise: NoiseGen::with_seed(0xFEEDFACE),
            relay_noise_amp: 0.0,
            relay_noise_decay: Self::decay_from_ms(3.0, sample_rate), // Very fast 3ms burst
            relay_filter: LowpassFilter::new(2500.0, sample_rate),    // Crisp high-freq
            relay_body: DampedOscillator::new(
                params.relay_freq,
                Self::decay_from_ms(params.relay_decay_ms, sample_rate),
            ),
            relay_tick: DampedOscillator::new(
                2000.0,  // High metallic tick
                Self::decay_from_ms(4.0, sample_rate),
            ),

            motor_on: false,
            motor_envelope: 0.0,
            motor_spinup_rate: 1.0 / (params.motor_spinup_ms * 0.001 * sample_rate),
            motor_spindown_rate: 1.0 / (params.motor_spindown_ms * 0.001 * sample_rate),
            motor_filter: LowpassFilter::new(params.motor_filter_freq, sample_rate),
            motor_cog_phase: 0.0,
            motor_cog_freq: params.motor_cog_freq,

            events: std::collections::VecDeque::new(),
        }
    }

    /// Convert decay time in milliseconds to per-sample decay multiplier
    /// Decays to -60dB (0.001) over the given time
    fn decay_from_ms(ms: f32, sample_rate: f32) -> f32 {
        let samples = ms * 0.001 * sample_rate;
        if samples < 1.0 {
            0.001  // Instant decay
        } else {
            0.001_f32.powf(1.0 / samples)
        }
    }

    pub fn with_audio(producer: AudioProducer, sample_rate: u32) -> Self {
        let mut audio = Self::new();
        audio.producer = Some(producer);
        audio.sample_rate = sample_rate;
        audio.apply_params();
        audio
    }

    /// Apply current params to internal state (call after changing params)
    pub fn apply_params(&mut self) {
        let sr = self.sample_rate as f32;
        let p = &self.params;
        
        self.click_noise_decay = Self::decay_from_ms(p.click_noise_decay_ms, sr);
        self.click_filter = LowpassFilter::new(p.click_filter_freq, sr);
        self.click_body.configure(
            p.click_body_freq, p.click_body_decay_ms,
            p.click_attack_mix, p.click_attack_decay_ms,
            p.click_pitch_sweep, p.click_pitch_sweep_ms,
            p.click_harmonic_mix
        );
        self.click_tick.freq = p.click_tick_freq;
        self.click_tick.decay = Self::decay_from_ms(p.click_tick_decay_ms, sr);
        self.click_crunch_decay = Self::decay_from_ms(p.click_crunch_decay_ms, sr);
        self.click_crunch_filter = LowpassFilter::new(p.click_crunch_freq, sr);
        
        self.relay_noise_decay = Self::decay_from_ms(3.0, sr);  // Fixed 3ms burst
        self.relay_filter = LowpassFilter::new(2500.0, sr);
        self.relay_body.freq = p.relay_freq;
        self.relay_body.decay = Self::decay_from_ms(p.relay_decay_ms, sr);
        self.relay_tick.decay = Self::decay_from_ms(4.0, sr);
        
        self.motor_filter = LowpassFilter::new(p.motor_filter_freq, sr);
        self.motor_cog_freq = p.motor_cog_freq;
        self.motor_spinup_rate = 1.0 / (p.motor_spinup_ms * 0.001 * sr);
        self.motor_spindown_rate = 1.0 / (p.motor_spindown_ms * 0.001 * sr);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.params.enabled = enabled;
        if !enabled {
            self.motor_on = false;
            self.motor_envelope = 0.0;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.params.enabled
    }

    pub fn queue_event(&mut self, cycle: u64, event: DriveEvent) {
        if !self.params.enabled {
            return;
        }
        self.events.push_back((cycle, event));
    }

    pub fn update(&mut self, current_cycle: u64) {
        if self.producer.is_none() || !self.params.enabled {
            self.last_cycle = current_cycle;
            return;
        }

        let cycles_per_sample = CYCLES_PER_SECOND / self.sample_rate as f64;
        let max_catchup = CYCLES_PER_SECOND / 60.0 * 2.0;
        let mut cycle_cursor = self.last_cycle as f64;
        let end_cycle = current_cycle as f64;

        if end_cycle - cycle_cursor > max_catchup {
            while let Some(&(event_cycle, event)) = self.events.front() {
                if (event_cycle as f64) < end_cycle - max_catchup {
                    self.process_event(event, event_cycle);
                    self.events.pop_front();
                } else {
                    break;
                }
            }
            cycle_cursor = end_cycle - max_catchup;
        }

        // Pre-allocate for typical frame size (~735 samples at 44100Hz/60fps)
        let estimated_samples = ((end_cycle - cycle_cursor) / cycles_per_sample) as usize + 16;
        let mut samples = Vec::with_capacity(estimated_samples.min(2048));
        
        while cycle_cursor < end_cycle {
            cycle_cursor += cycles_per_sample;

            while let Some(&(event_cycle, event)) = self.events.front() {
                if (event_cycle as f64) <= cycle_cursor {
                    self.process_event(event, event_cycle);
                    self.events.pop_front();
                } else {
                    break;
                }
            }

            samples.push(self.generate_sample());
        }

        // Batch push all samples at once
        if let Some(producer) = &mut self.producer {
            let _ = producer.push_slice(&samples);
        }

        self.last_cycle = current_cycle;
    }

    fn process_event(&mut self, event: DriveEvent, cycle: u64) {
        let p = &self.params;
        match event {
            DriveEvent::Step { quarter_track } => {
                // Calculate time since last step for seek speed modulation
                let ms_since_last = if self.last_step_cycle > 0 {
                    ((cycle - self.last_step_cycle) as f64 / CYCLES_PER_SECOND * 1000.0) as f32
                } else {
                    100.0  // First step, assume slow
                };
                
                // Fast seek: harder impact, less ring (carriage doesn't settle)
                let speed_factor = (ms_since_last / 30.0).min(1.0);
                
                // Toward track 0 is slightly harder
                let toward_zero = quarter_track < self.last_quarter_track;
                let direction_mult = if toward_zero { 1.15 } else { 1.0 };
                
                let vol = p.click_volume * direction_mult * (0.8 + 0.2 * speed_factor);
                
                // Accumulate noise amplitude (don't reset filter - causes pops)
                self.click_noise_amp = (self.click_noise_amp + vol).min(vol * 1.5);
                
                // Body clack (multi-stage impact) + metallic tick
                self.click_body.trigger(vol);  // ImpactSynthesizer handles internal levels
                self.click_tick.trigger(vol * 0.6); // Tick is subtler
                
                // Crunch layer
                self.click_crunch_amp = (self.click_crunch_amp + vol).min(vol * 1.5);
                
                self.last_quarter_track = quarter_track;
                self.last_step_cycle = cycle;
            }
            DriveEvent::MotorOn => {
                self.motor_on = true;
                // Fire relay click - sharp clack with noise burst + dual oscillators
                self.relay_noise_amp = p.relay_volume;
                self.relay_body.trigger(p.relay_volume * 0.7);   // Body resonance
                self.relay_tick.trigger(p.relay_volume * 0.4);   // High metallic tick
            }
            DriveEvent::MotorOff => {
                self.motor_on = false;
                // Spindown handled by envelope in generate_sample
            }
        }
    }

    fn generate_sample(&mut self) -> f32 {
        let sr = self.sample_rate as f32;
        let p = &self.params;
        let mut output = 0.0;

        // === Click synthesis ===
        // Noise layer (independent volume, not subtracted)
        if self.click_noise_amp > 0.001 {
            let noise = self.noise_click.next();
            let filtered = self.click_filter.process(noise);
            output += filtered * self.click_noise_amp * 0.5;  // Noise at 50% presence
            self.click_noise_amp *= self.click_noise_decay;
        }
        // Body clack (multi-stage impact synthesizer)
        if self.click_body.is_active() {
            output += self.click_body.tick() * p.click_body_mix;
        }
        // Metallic tick oscillator
        if self.click_tick.is_active() {
            output += self.click_tick.tick(sr) * p.click_tick_mix;
        }
        // Crunch layer
        if self.click_crunch_amp > 0.001 {
            let noise = self.noise_crunch.next();
            let filtered = self.click_crunch_filter.process(noise);
            output += filtered * self.click_crunch_amp * p.click_crunch_mix;
            self.click_crunch_amp *= self.click_crunch_decay;
        }

        // === Relay click (sharp clack on motor start) ===
        // Noise burst for impact transient
        if self.relay_noise_amp > 0.001 {
            let noise = self.relay_noise.next();
            let filtered = self.relay_filter.process(noise);
            output += filtered * self.relay_noise_amp * 0.6;
            self.relay_noise_amp *= self.relay_noise_decay;
        }
        // Body resonance
        if self.relay_body.is_active() {
            output += self.relay_body.tick(sr);
        }
        // High metallic tick
        if self.relay_tick.is_active() {
            output += self.relay_tick.tick(sr);
        }

        // === Motor synthesis with spinup/spindown ===
        // Update envelope
        if self.motor_on {
            self.motor_envelope = (self.motor_envelope + self.motor_spinup_rate).min(1.0);
        } else {
            self.motor_envelope = (self.motor_envelope - self.motor_spindown_rate).max(0.0);
        }
        
        // Generate motor sound when envelope > 0
        if self.motor_envelope > 0.001 {
            let motor_vol = p.motor_volume * self.motor_envelope;
            
            // Disk air turbulence
            let noise = self.noise_motor.next();
            let filtered = self.motor_filter.process(noise);
            output += filtered * motor_vol * (1.0 - p.motor_cog_mix);

            // Cogging
            self.motor_cog_phase += self.motor_cog_freq / sr;
            if self.motor_cog_phase >= 1.0 {
                self.motor_cog_phase -= 1.0;
            }
            let cog = (self.motor_cog_phase * std::f32::consts::TAU).sin();
            output += cog * motor_vol * p.motor_cog_mix;
        }

        soft_clip(output * p.master_volume)
    }
}

/// Soft clipper using tanh for smooth saturation
fn soft_clip(x: f32) -> f32 {
    (x * 1.5).tanh() * 0.9
}

impl Default for DriveAudio {
    fn default() -> Self {
        Self::new()
    }
}
