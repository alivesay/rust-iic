use std::cell::Cell;

/// Apple IIc Joystick/Paddle emulation
/// 
/// The Apple IIc has two analog paddle inputs ($C064/$C065) that can be used
/// for joysticks or standalone paddle controllers. The analog reading works via
/// an RC timing circuit:
/// 
/// 1. Program reads PTRIG ($C070) to discharge capacitors and start timer
/// 2. Bit 7 of $C064/$C065 immediately goes HIGH (1)
/// 3. Capacitor charges through joystick's variable resistor (0-150K ohms)
/// 4. When charged, bit 7 goes LOW (0) - this is the "timeout"
/// 5. PDL() in ROM counts loop iterations until timeout (0-255 ≈ 0-2.8ms)
/// 
/// With NO joystick connected (open circuit):
/// - After PTRIG: bit 7 = 1 (timer started)
/// - Capacitor never charges (no resistor path)  
/// - Bit 7 stays HIGH indefinitely → PDL() returns 255
/// 
/// The Apple IIc does not have physical joystick button inputs - instead,
/// the Open Apple and Closed Apple keys serve as button 0 and button 1.
/// These buttons are handled separately via keyboard input.

/// Maximum cycles to keep paddle timer "active" after PTRIG trigger.
/// Real joystick max timeout is ~2850 cycles (2.8ms at 1.023MHz).
/// With no joystick, we use 10ms (10230 cycles) then let it settle.
const PADDLE_TIMEOUT_CYCLES: u64 = 10_230;

pub struct Joystick {
    /// Cycle count when PTRIG ($C070) was last accessed.
    /// u64::MAX indicates PTRIG has never been triggered.
    trigger_cycle: Cell<u64>,
    
    /// Paddle 0 position (0-255). None = no joystick connected (open circuit).
    paddle0: Cell<Option<u8>>,
    
    /// Paddle 1 position (0-255). None = no joystick connected (open circuit).
    paddle1: Cell<Option<u8>>,
}

impl Joystick {
    pub fn new() -> Self {
        Self {
            trigger_cycle: Cell::new(u64::MAX),
            paddle0: Cell::new(None), // No joystick connected by default
            paddle1: Cell::new(None),
        }
    }
    
    /// Trigger the paddle timer (called when PTRIG $C070 is accessed).
    /// This discharges the capacitors and starts the RC timing cycle.
    pub fn trigger(&self, cycles: u64) {
        self.trigger_cycle.set(cycles);
    }
    
    /// Read a paddle input ($C064 for paddle 0, $C065 for paddle 1).
    /// Returns the byte to be read, with bit 7 indicating timer status.
    pub fn read(&self, paddle: u8, cycles: u64) -> u8 {
        let trigger = self.trigger_cycle.get();
        
        if trigger == u64::MAX {
            // PTRIG never accessed - capacitor in settled state, bit 7 = 0
            return 0x00;
        }
        
        let elapsed = cycles.saturating_sub(trigger);
        
        // Get the paddle position (None = no joystick = open circuit)
        let position = match paddle {
            0 => self.paddle0.get(),
            1 => self.paddle1.get(),
            _ => None,
        };
        
        match position {
            Some(pos) => {
                // Joystick connected: timer expires based on position
                // Position 0 = immediate timeout, 255 = max timeout (~2850 cycles)
                // Each position unit ≈ 11 cycles (2850 / 255 ≈ 11.2)
                let timeout_cycles = (pos as u64) * 11;
                if elapsed < timeout_cycles {
                    0x80 // Timer still counting
                } else {
                    0x00 // Timer expired
                }
            }
            None => {
                // No joystick (open circuit): timer never expires naturally
                // Keep bit 7 HIGH for a reasonable window, then let it settle
                if elapsed < PADDLE_TIMEOUT_CYCLES {
                    0x80 // Timer still counting (open circuit = stays high)
                } else {
                    0x00 // Long after timeout window - capacitor settled
                }
            }
        }
    }
    
    /// Set paddle 0 position. Use None for no joystick/open circuit.
    #[allow(dead_code)]
    pub fn set_paddle0(&self, position: Option<u8>) {
        self.paddle0.set(position);
    }
    
    /// Set paddle 1 position. Use None for no joystick/open circuit.
    #[allow(dead_code)]
    pub fn set_paddle1(&self, position: Option<u8>) {
        self.paddle1.set(position);
    }
    
    /// Check if a joystick is connected (either paddle has a position).
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.paddle0.get().is_some() || self.paddle1.get().is_some()
    }
}

impl Default for Joystick {
    fn default() -> Self {
        Self::new()
    }
}
