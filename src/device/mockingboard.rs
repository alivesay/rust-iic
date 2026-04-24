// Mockingboard Sound Card Emulation
//
// Emulates the Sweet Micro Systems Mockingboard sound card:
// - Two 6522 VIA chips for timing and control
// - Two AY-3-8910 PSG (Programmable Sound Generator) chips
//
// Memory map (typically slot 4, $C4xx):
// - $C400-$C40F: VIA 1 registers (controls PSG 1)
// - $C480-$C48F: VIA 2 registers (controls PSG 2)
//
// References:
// - AY-3-8910 datasheet
// - MOS 6522 VIA datasheet
// - MAME ay8910.cpp (envelope timing, DAC model)
// - Matthew Westcott's AY-3-8910 voltage measurements (December 2001)
// - gyurco/apple2efpga FPGA implementation (LFSR algorithm)

use std::sync::Arc;
use ringbuf::{HeapRb, traits::*};
use ringbuf::wrap::caching::Caching;

// Audio sample producer type
pub type AudioProducer = Caching<Arc<HeapRb<f32>>, true, false>;

const AMPLITUDE: f32 = 0.5;
// Apple II clock is 14.318181 MHz / 14 = 1.022727 MHz
const CYCLES_PER_SECOND: f64 = 1_022_727.0;

// AY-3-8910 PSG register indices
mod ay_reg {
    pub const TONE_A_PERIOD_FINE: u8 = 0;
    pub const TONE_A_PERIOD_COARSE: u8 = 1;
    pub const TONE_B_PERIOD_FINE: u8 = 2;
    pub const TONE_B_PERIOD_COARSE: u8 = 3;
    pub const TONE_C_PERIOD_FINE: u8 = 4;
    pub const TONE_C_PERIOD_COARSE: u8 = 5;
    pub const NOISE_PERIOD: u8 = 6;
    pub const MIXER: u8 = 7;
    pub const AMPLITUDE_A: u8 = 8;
    pub const AMPLITUDE_B: u8 = 9;
    pub const AMPLITUDE_C: u8 = 10;
    pub const ENVELOPE_PERIOD_FINE: u8 = 11;
    pub const ENVELOPE_PERIOD_COARSE: u8 = 12;
    pub const ENVELOPE_SHAPE: u8 = 13;
}

// AY-3-8910 channel state
#[derive(Clone)]
struct AyChannel {
    period: u16,        // 12-bit tone period (1-4095, 0 treated as 1)
    amplitude: u8,      // 4-bit amplitude (or envelope mode if bit 4 set)
    counter: u16,       // Countdown counter
    output: bool,       // Current output state (high/low)
}

impl Default for AyChannel {
    fn default() -> Self {
        Self {
            period: 1,
            amplitude: 0,
            counter: 0,
            output: false,
        }
    }
}

// AY-3-8910 PSG chip: simple tick-based emulation
#[derive(Clone)]
struct Ay8910 {
    // Registers (as written by software)
    registers: [u8; 16],
    selected_register: u8,
    
    // Channel state
    channels: [AyChannel; 3],
    
    // Noise generator
    noise_period: u8,
    noise_counter: u8,
    noise_output: bool,
    rng: u32, // 17-bit LFSR for noise
    
    // Envelope generator
    envelope_period: u16,
    envelope_counter: u16,
    envelope_step: u8,
    envelope_shape: u8,
    envelope_volume: u8,
    envelope_holding: bool,
    envelope_attack: bool,
    envelope_prescaler: u8, // AY-3-8910 envelope runs at half the rate of tones
    
    // Prescaler (AY runs at CLK/8)
    prescaler: u8,
    
    // Simple low-pass filter to simulate analog output circuitry
    // This reduces harsh aliasing from square waves
    filter_state: f32,
}

impl Default for Ay8910 {
    fn default() -> Self {
        Self {
            registers: [0; 16],
            selected_register: 0,
            channels: Default::default(),
            noise_period: 0,
            noise_counter: 0,
            noise_output: false,
            rng: 1, // Must be non-zero
            envelope_period: 0,
            envelope_counter: 0,
            envelope_step: 0,
            envelope_shape: 0,
            envelope_volume: 15,
            envelope_holding: false,
            envelope_attack: false,
            envelope_prescaler: 0,
            prescaler: 0,
            filter_state: 0.0,
        }
    }
}

impl Ay8910 {
    fn reset(&mut self) {
        *self = Self::default();
    }
    
    fn write_register(&mut self, reg: u8, value: u8) {
        let reg = reg & 0x0F;
        self.registers[reg as usize] = value;
        
        match reg {
            ay_reg::TONE_A_PERIOD_FINE | ay_reg::TONE_A_PERIOD_COARSE => {
                self.channels[0].period = self.get_tone_period(0).max(1);
            }
            ay_reg::TONE_B_PERIOD_FINE | ay_reg::TONE_B_PERIOD_COARSE => {
                self.channels[1].period = self.get_tone_period(1).max(1);
            }
            ay_reg::TONE_C_PERIOD_FINE | ay_reg::TONE_C_PERIOD_COARSE => {
                self.channels[2].period = self.get_tone_period(2).max(1);
            }
            ay_reg::NOISE_PERIOD => {
                self.noise_period = (value & 0x1F).max(1);
            }
            ay_reg::AMPLITUDE_A => {
                self.channels[0].amplitude = value & 0x1F;
            }
            ay_reg::AMPLITUDE_B => {
                self.channels[1].amplitude = value & 0x1F;
            }
            ay_reg::AMPLITUDE_C => {
                self.channels[2].amplitude = value & 0x1F;
            }
            ay_reg::ENVELOPE_PERIOD_FINE | ay_reg::ENVELOPE_PERIOD_COARSE => {
                self.envelope_period = u16::from(self.registers[ay_reg::ENVELOPE_PERIOD_FINE as usize])
                    | (u16::from(self.registers[ay_reg::ENVELOPE_PERIOD_COARSE as usize]) << 8);
            }
            ay_reg::ENVELOPE_SHAPE => {
                self.envelope_shape = value & 0x0F;
                self.envelope_step = 0;
                // Initialize counter to period so we count down a full cycle before first step
                // (if counter starts at 0, we immediately step which causes envelope glitches)
                self.envelope_counter = self.envelope_period;
                self.envelope_prescaler = 0; // Reset prescaler for clean timing
                self.envelope_holding = false;
                self.envelope_attack = (value & 0x04) != 0;
                self.envelope_volume = if self.envelope_attack { 0 } else { 15 };
            }
            _ => {}
        }
    }
    
    fn read_register(&self, reg: u8) -> u8 {
        self.registers[(reg & 0x0F) as usize]
    }
    
    fn get_tone_period(&self, channel: usize) -> u16 {
        let fine = self.registers[channel * 2] as u16;
        let coarse = (self.registers[channel * 2 + 1] & 0x0F) as u16;
        (coarse << 8) | fine
    }
    
    fn get_mixer(&self) -> u8 {
        self.registers[ay_reg::MIXER as usize]
    }
    
    // Tick the PSG by N CPU cycles (batch processing for efficiency)
    // Returns the number of prescaler ticks that occurred
    fn tick_n(&mut self, cycles: u32) -> u32 {
        // AY-3-8910 internal prescaler divides clock by 8
        // Instead of calling tick() N times, calculate how many prescaler
        // overflows occur and only process those
        let total_prescaler = self.prescaler as u32 + cycles;
        let prescaler_ticks = total_prescaler / 8;
        self.prescaler = (total_prescaler % 8) as u8;
        
        if prescaler_ticks == 0 {
            return 0;
        }
        
        // Process prescaler_ticks worth of PSG updates
        for _ in 0..prescaler_ticks {
            // Update tone channels
            for ch in &mut self.channels {
                if ch.counter == 0 {
                    ch.counter = ch.period;
                    ch.output = !ch.output;
                } else {
                    ch.counter -= 1;
                }
            }
            
            // Update noise generator
            if self.noise_counter == 0 {
                self.noise_counter = self.noise_period;
                // 17-bit LFSR: taps at bits 0 and 2
                let bit = ((self.rng >> 0) ^ (self.rng >> 2)) & 1;
                self.rng = (self.rng >> 1) | (bit << 16);
                self.noise_output = (self.rng & 1) != 0;
            } else {
                self.noise_counter -= 1;
            }
            
            // Update envelope generator
            // AY-3-8910 envelope runs at half the rate of tones (prescaler /2)
            // Note: Unlike tone period, envelope period 0 runs at DOUBLE speed (half the period)
            self.envelope_prescaler = self.envelope_prescaler.wrapping_add(1);
            if self.envelope_prescaler >= 2 {
                self.envelope_prescaler = 0;
                
                if !self.envelope_holding {
                    if self.envelope_counter == 0 {
                        // Period 0 = half period of 1, so we don't clamp to max(1) here
                        self.envelope_counter = if self.envelope_period == 0 { 0 } else { self.envelope_period };
                        self.envelope_step = self.envelope_step.wrapping_add(1);
                        
                        if self.envelope_step >= 16 {
                            self.envelope_step = 0;
                            
                            let cont = (self.envelope_shape & 0x08) != 0;
                            let alt = (self.envelope_shape & 0x02) != 0;
                            let hold = (self.envelope_shape & 0x01) != 0;
                            
                            if !cont {
                                self.envelope_volume = 0;
                                self.envelope_holding = true;
                            } else if hold {
                                self.envelope_volume = if self.envelope_attack != alt { 15 } else { 0 };
                                self.envelope_holding = true;
                            } else if alt {
                                self.envelope_attack = !self.envelope_attack;
                            }
                        }
                        
                        if !self.envelope_holding {
                            self.envelope_volume = if self.envelope_attack {
                                self.envelope_step
                            } else {
                                15 - self.envelope_step
                            };
                        }
                    } else {
                        self.envelope_counter -= 1;
                    }
                }
            }
        }
        
        prescaler_ticks
    }
    
    // Get the filtered output sample (-1.0 to 1.0)
    // Applies a simple low-pass filter to reduce aliasing from square waves
    fn output(&mut self) -> f32 {
        let mixer = self.get_mixer();
        let mut sum = 0.0_f32;
        
        // DAC levels based on Matthew Westcott's measurements (December 2001)
        // Posted to comp.sys.sinclair.
        // Original measurements on real AY-3-8910 with RL=2000 ohm:
        // Level 0: 1.147V, Level 15: 2.58V
        // Values normalized to 0.0-1.0 range: (V - 1.147) / (2.58 - 1.147)
        const DAC_TABLE: [f32; 16] = [
            0.0000, 0.0105, 0.0154, 0.0216, 0.0314, 0.0461, 0.0635, 0.1061,
            0.1319, 0.2164, 0.2973, 0.3908, 0.5129, 0.6371, 0.8186, 1.0000
        ];
        
        for (i, ch) in self.channels.iter().enumerate() {
            let tone_disable = (mixer >> i) & 1;
            let noise_disable = (mixer >> (i + 3)) & 1;
            
            // When disabled, that source outputs 1 (high)
            let tone_signal = (tone_disable != 0) || ch.output;
            let noise_signal = (noise_disable != 0) || self.noise_output;
            let gate = tone_signal && noise_signal;
            
            // Get amplitude (use envelope if bit 4 set)
            let amp = if (ch.amplitude & 0x10) != 0 {
                self.envelope_volume
            } else {
                ch.amplitude & 0x0F
            };
            
            let volume = DAC_TABLE[amp as usize];
            
            if gate {
                sum += volume;
            }
        }
        
        // Scale output to 0.0-1.0 range (silence = 0.0, max = 1.0)
        // Don't center around 0, silent MB should contribute nothing to the mix
        let raw_output = sum / 3.0;
        
        // Apply simple one-pole low-pass filter to simulate analog output circuitry
        // This reduces harsh aliasing from square waves
        // Alpha ~0.4 gives roughly 10kHz cutoff at 44.1kHz sample rate
        const FILTER_ALPHA: f32 = 0.4;
        self.filter_state = FILTER_ALPHA * raw_output + (1.0 - FILTER_ALPHA) * self.filter_state;
        self.filter_state
    }
}

// 6522 VIA chip
#[derive(Clone)]
struct Via6522 {
    // Port registers
    ora: u8,    // Output Register A
    orb: u8,    // Output Register B
    ira: u8,    // Input Register A
    irb: u8,    // Input Register B
    ddra: u8,   // Data Direction Register A
    ddrb: u8,   // Data Direction Register B
    
    // Timer 1
    t1c: u16,   // Timer 1 Counter
    t1l: u16,   // Timer 1 Latch
    
    // Timer 2  
    t2c: u16,   // Timer 2 Counter
    t2l: u8,    // Timer 2 Latch (low byte only)
    
    // Shift register
    sr: u8,
    
    // Control registers
    acr: u8,    // Auxiliary Control Register
    pcr: u8,    // Peripheral Control Register
    ifr: u8,    // Interrupt Flag Register
    ier: u8,    // Interrupt Enable Register
}

impl Default for Via6522 {
    fn default() -> Self {
        Self {
            ora: 0,
            orb: 0,
            ira: 0xFF, // Input pins float high when undriven
            irb: 0xFF,
            ddra: 0,
            ddrb: 0,
            t1c: 0xFFFF,
            t1l: 0xFFFF,
            t2c: 0xFFFF,
            t2l: 0xFF,
            sr: 0,
            acr: 0,
            pcr: 0,
            ifr: 0,
            ier: 0,
        }
    }
}

impl Via6522 {
    fn read(&self, reg: u8) -> u8 {
        match reg & 0x0F {
            0x00 => (self.orb & self.ddrb) | (self.irb & !self.ddrb), // ORB/IRB
            0x01 => (self.ora & self.ddra) | (self.ira & !self.ddra), // ORA/IRA
            0x02 => self.ddrb,
            0x03 => self.ddra,
            0x04 => self.t1c as u8,         // T1C-L
            0x05 => (self.t1c >> 8) as u8,  // T1C-H
            0x06 => self.t1l as u8,         // T1L-L
            0x07 => (self.t1l >> 8) as u8,  // T1L-H
            0x08 => self.t2c as u8,         // T2C-L
            0x09 => (self.t2c >> 8) as u8,  // T2C-H
            0x0A => self.sr,
            0x0B => self.acr,
            0x0C => self.pcr,
            0x0D => self.ifr,
            0x0E => self.ier | 0x80,        // Bit 7 always reads as 1
            0x0F => self.ora,               // ORA (no handshake)
            _ => 0,
        }
    }
    
    fn write(&mut self, reg: u8, value: u8) {
        match reg & 0x0F {
            0x00 => self.orb = value,
            0x01 => self.ora = value,
            0x02 => self.ddrb = value,
            0x03 => self.ddra = value,
            0x04 => self.t1l = (self.t1l & 0xFF00) | value as u16,
            0x05 => {
                self.t1l = (self.t1l & 0x00FF) | ((value as u16) << 8);
                self.t1c = self.t1l;
                self.ifr &= !0x40; // Clear T1 interrupt
            }
            0x06 => self.t1l = (self.t1l & 0xFF00) | value as u16,
            0x07 => {
                self.t1l = (self.t1l & 0x00FF) | ((value as u16) << 8);
                // In free-running mode, writing T1L-H clears the interrupt flag
                if self.acr & 0x40 != 0 {
                    self.ifr &= !0x40;
                }
            }
            0x08 => self.t2l = value,
            0x09 => {
                self.t2c = (self.t2l as u16) | ((value as u16) << 8);
                self.ifr &= !0x20; // Clear T2 interrupt
            }
            0x0A => self.sr = value,
            0x0B => self.acr = value,
            0x0C => self.pcr = value,
            0x0D => self.ifr &= !value, // Writing 1 clears bits
            0x0E => {
                if value & 0x80 != 0 {
                    self.ier |= value & 0x7F;  // Set bits
                } else {
                    self.ier &= !(value & 0x7F); // Clear bits
                }
            }
            0x0F => self.ora = value,
            _ => {}
        }
    }
    
    fn tick(&mut self) {
        // Decrement Timer 1
        if self.t1c == 0 {
            self.ifr |= 0x40; // Set T1 interrupt flag
            if self.acr & 0x40 != 0 {
                // Free-running mode, reload from latch
                self.t1c = self.t1l;
            }
        } else {
            self.t1c = self.t1c.wrapping_sub(1);
        }
        
        // Decrement Timer 2 (one-shot, counts down)
        if (self.acr & 0x20) == 0 && self.t2c > 0 {
            self.t2c = self.t2c.wrapping_sub(1);
            if self.t2c == 0 {
                self.ifr |= 0x20; // Set T2 interrupt flag
            }
        }
    }
    
    fn irq_active(&self) -> bool {
        (self.ifr & self.ier & 0x7F) != 0
    }
}

// Mockingboard hardware variant
#[derive(Clone, Copy, PartialEq, Default)]
pub enum MockingboardType {
    // Standard Mockingboard: Two VIAs, each controlling one PSG
    #[default]
    TypeA,
    // Mockingboard C/4c: Single VIA, ORB bits 3/4 select PSG(s)
    TypeC,
}

// Activation delay in CPU cycles (~0.5 sec at 1.023 MHz).
// Allows boot ROM to initialize mouse firmware before Mockingboard takes over $C4xx.
const ACTIVATION_DELAY_CYCLES: u64 = 500_000;

// Mockingboard sound card emulation
pub struct Mockingboard {
    // Two VIA chips (TypeC only uses VIA 0)
    via: [Via6522; 2],
    
    // Two AY-3-8910 PSG chips
    psg: [Ay8910; 2],
    
    // PSG bus control state
    psg_bc1: [bool; 2],
    psg_bdir: [bool; 2],
    
    // Audio output
    producer: Option<AudioProducer>,
    last_cycle: u64,
    sample_accum: u32,  // Integer accumulator for sample timing (in 1/256 cycle units)
    cycles_per_sample_frac: u32,  // Pre-computed cycles/sample in 8.8 fixed point
    
    // For mixing with main speaker
    enabled: bool,
    
    // Hardware variant
    mb_type: MockingboardType,
    
    // Set true after activation (by hook, timer, hotkey, or write to $C4xx).
    // Until activated, reads from $C4xx return ROM instead of VIA registers.
    // This allows IIc mouse firmware to work during boot.
    activated: bool,
    
    // Cycles remaining until auto-activation (0 = ready to activate on next tick)
    // Set to u64::MAX when using hook-based activation
    activation_countdown: u64,
    
    // When true, use hook-based activation instead of countdown timer
    use_hook_activation: bool,
}

// Pre-computed cycles per sample in 8.24 fixed point: (1022727 / 44100) * 256 ≈ 5942
const DEFAULT_CYCLES_PER_SAMPLE_FRAC: u32 = ((CYCLES_PER_SECOND / 44100.0) * 256.0) as u32;

impl Default for Mockingboard {
    fn default() -> Self {
        Self {
            via: Default::default(),
            psg: [Ay8910::default(), Ay8910::default()],
            psg_bc1: [false; 2],
            psg_bdir: [false; 2],
            producer: None,
            last_cycle: 0,
            sample_accum: 0,
            cycles_per_sample_frac: DEFAULT_CYCLES_PER_SAMPLE_FRAC,
            enabled: false,
            mb_type: MockingboardType::TypeA,  // Standard Mockingboard: 2 VIAs, each with 1 PSG
            activated: false,
            activation_countdown: ACTIVATION_DELAY_CYCLES,
            use_hook_activation: false,
        }
    }
}

impl Mockingboard {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_audio(producer: AudioProducer, sample_rate: u32) -> Self {
        // Pre-compute cycles per sample in 8.24 fixed point for integer arithmetic
        let cycles_per_sample_frac = ((CYCLES_PER_SECOND / sample_rate as f64) * 256.0) as u32;
        Self {
            producer: Some(producer),
            cycles_per_sample_frac,
            enabled: true,
            ..Default::default()
        }
    }
    
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    // Enable hook-based activation instead of timer-based.
    // When enabled, the Mockingboard waits for explicit activate() call
    // instead of counting down cycles. This should be used with the
    // ROM hook at $FA6F (after mouse firmware init).
    pub fn set_hook_activation(&mut self, use_hook: bool) {
        self.use_hook_activation = use_hook;
        if use_hook {
            // Disable the countdown timer
            self.activation_countdown = u64::MAX;
        } else {
            self.activation_countdown = ACTIVATION_DELAY_CYCLES;
        }
    }
    
    // Activate the Mockingboard (called on first write to $C4xx or by hook).
    // Mockingboard stays dormant at boot to avoid conflicts with
    // Apple IIc mouse firmware, then activates when software accesses it.
    pub fn activate(&mut self) {
        if self.enabled && !self.activated {
            self.activated = true;
            log::debug!("Mockingboard activated");
        }
    }
    
    // Check if Mockingboard is activated and enabled
    pub fn is_activated(&self) -> bool {
        self.enabled && self.activated
    }
    
    // Read from Mockingboard address space
    // TypeA: $00-$7F = VIA 1, $80-$FF = VIA 2
    // TypeC: Only VIA 1 at $00-$0F
    pub fn read(&mut self, offset: u8) -> u8 {
        if !self.enabled {
            return 0xFF;
        }
        
        let via_idx = if offset >= 0x80 { 1 } else { 0 };
        let reg = offset & 0x0F;
        
        // For TypeC, only VIA 0 is used
        let effective_via = if self.mb_type == MockingboardType::TypeC { 0 } else { via_idx };
        
        let value = self.via[effective_via].read(reg);
        
        // If reading ORA, may need to provide PSG data
        if reg == 0x01 || reg == 0x0F {
            match self.mb_type {
                MockingboardType::TypeA => {
                    // Check if PSG is in read mode (BC1=1, BDIR=0)
                    if self.psg_bc1[via_idx] && !self.psg_bdir[via_idx] {
                        return self.psg[via_idx].read_register(self.psg[via_idx].selected_register);
                    }
                }
                MockingboardType::TypeC => {
                    // TypeC: check ORB bits 3/4 for PSG selection (ACTIVE HIGH)
                    let orb = self.via[0].orb;
                    let bc1 = (orb & 0x01) != 0;
                    let bdir = (orb & 0x02) != 0;
                    // Bit 3 (0x08) = PSG 0, Bit 4 (0x10) = PSG 1
                    let psg0_sel = (orb & 0x08) != 0;
                    let psg1_sel = (orb & 0x10) != 0;
                    
                    // Read mode: BC1=1, BDIR=0
                    if bc1 && !bdir {
                        // Return data from first selected PSG
                        if psg0_sel {
                            return self.psg[0].read_register(self.psg[0].selected_register);
                        }
                        if psg1_sel {
                            return self.psg[1].read_register(self.psg[1].selected_register);
                        }
                    }
                }
            }
        }
        
        value
    }
    
    // Write to Mockingboard address space
    pub fn write(&mut self, offset: u8, value: u8) {
        if !self.enabled {
            return;
        }
        
        log::trace!("Mockingboard write: offset=${:02X} value=${:02X}", offset, value);
        
        let via_idx = if offset >= 0x80 { 1 } else { 0 };
        let reg = offset & 0x0F;
        
        // For TypeC, only VIA 0 is used
        let effective_via = if self.mb_type == MockingboardType::TypeC { 0 } else { via_idx };
        
        self.via[effective_via].write(reg, value);
        
        // Check for PSG control via ORB
        if reg == 0x00 {
            let orb = self.via[effective_via].orb;
            let bc1 = (orb & 0x01) != 0;
            let bdir = (orb & 0x02) != 0;
            let bc2 = (orb & 0x04) != 0;
            
            match self.mb_type {
                MockingboardType::TypeA => {
                    // TypeA: Each VIA controls its own PSG
                    // ORB bit 0 = BC1, bit 1 = BDIR, bit 2 = BC2
                    self.psg_bc1[via_idx] = bc1;
                    self.psg_bdir[via_idx] = bdir;
                    
                    if !bc2 {
                        // BC2=0: Reset the PSG
                        self.psg[via_idx].reset();
                    } else {
                        if bdir && bc1 {
                            // Latch address (BDIR=1, BC1=1)
                            self.psg[via_idx].selected_register = self.via[via_idx].ora & 0x0F;
                        } else if bdir && !bc1 {
                            // Write data (BDIR=1, BC1=0)
                            self.psg[via_idx].write_register(
                                self.psg[via_idx].selected_register,
                                self.via[via_idx].ora
                            );
                        }
                    }
                }
                MockingboardType::TypeC => {
                    // TypeC/Mockingboard C: Single VIA, bits 3/4 select which PSG(s)
                    // ACTIVE HIGH chip selects:
                    // - bit 3 (0x08) = 1 selects PSG 0
                    // - bit 4 (0x10) = 1 selects PSG 1
                    let psg0_sel = (orb & 0x08) != 0;
                    let psg1_sel = (orb & 0x10) != 0;
                    
                    if !bc2 {
                        // BC2=0: Reset the selected PSGs
                        if psg0_sel {
                            self.psg[0].reset();
                        }
                        if psg1_sel {
                            self.psg[1].reset();
                        }
                    } else {
                        if bdir && bc1 {
                            // Latch address (BDIR=1, BC1=1)
                            if psg0_sel {
                                self.psg[0].selected_register = self.via[0].ora & 0x0F;
                            }
                            if psg1_sel {
                                self.psg[1].selected_register = self.via[0].ora & 0x0F;
                            }
                        } else if bdir && !bc1 {
                            // Write data (BDIR=1, BC1=0)
                            if psg0_sel {
                                self.psg[0].write_register(
                                    self.psg[0].selected_register,
                                    self.via[0].ora
                                );
                            }
                            if psg1_sel {
                                self.psg[1].write_register(
                                    self.psg[1].selected_register,
                                    self.via[0].ora
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Tick the Mockingboard by N CPU cycles (batch processing for efficiency)
    // This replaces the old per-cycle tick() with a more efficient batch approach
    pub fn tick_n(&mut self, cycles: u32) {
        if !self.enabled || cycles == 0 {
            return;
        }
        
        // Handle activation countdown
        if !self.activated {
            if self.use_hook_activation {
                return;
            }
            if self.activation_countdown > 0 {
                let decrement = (cycles as u64).min(self.activation_countdown);
                self.activation_countdown -= decrement;
                if self.activation_countdown > 0 {
                    return;
                }
                self.activated = true;
            }
        }
        
        // Tick VIAs (for timer interrupts): VIA needs per-cycle accuracy for timers
        for _ in 0..cycles {
            for via in &mut self.via {
                via.tick();
            }
        }
        
        // Tick PSGs by all incoming CPU cycles first
        // This ensures proper waveform generation regardless of sample timing
        self.psg[0].tick_n(cycles);
        self.psg[1].tick_n(cycles);
        
        // Audio sample generation at the audio sample rate
        // sample_accum is in 8.8 fixed-point (×256) for fractional cycle tracking
        // cycles_per_sample_frac is also ×256 (e.g., 23.22 cycles → 5945)
        self.sample_accum += cycles * 256;
        
        // Generate samples at audio rate by sampling current PSG state
        let num_samples = self.sample_accum / self.cycles_per_sample_frac;
        if num_samples > 0 && self.producer.is_some() {
            self.sample_accum -= num_samples * self.cycles_per_sample_frac;
            
            // Stereo: 2 samples per frame (L, R)
            let mut samples = Vec::with_capacity((num_samples * 2) as usize);
            
            // Sample each time to allow filter to process properly
            for _ in 0..num_samples {
                // PSG0 → Left channel, PSG1 → Right channel
                let left = self.psg[0].output() * AMPLITUDE;
                let right = self.psg[1].output() * AMPLITUDE;
                samples.push(left);
                samples.push(right);
            }
            
            if let Some(producer) = &mut self.producer {
                let _ = producer.push_slice(&samples);
            }
        }
    }
    
    // Called once per frame for bookkeeping, audio is in tick_n()
    pub fn update(&mut self, current_cycle: u64) {
        self.last_cycle = current_cycle;
    }
    
    // Check if any VIA is requesting an interrupt
    pub fn irq_active(&self) -> bool {
        self.enabled && (self.via[0].irq_active() || self.via[1].irq_active())
    }
    
    // Reset the Mockingboard
    pub fn reset(&mut self) {
        self.via = Default::default();
        self.psg = [Ay8910::default(), Ay8910::default()];
        self.psg_bc1 = [false; 2];
        self.psg_bdir = [false; 2];
        self.last_cycle = 0;
        self.sample_accum = 0;
        self.activated = false;  // Require re-activation after reset
        // Preserve hook vs timer activation mode
        if self.use_hook_activation {
            self.activation_countdown = u64::MAX;
        } else {
            self.activation_countdown = ACTIVATION_DELAY_CYCLES;
        }
    }
}
