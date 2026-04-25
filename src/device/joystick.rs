use std::cell::Cell;

pub struct Joystick {
    // Cycle count when PTRIG ($C070) was last accessed.
    // u64::MAX indicates PTRIG has never been triggered.
    trigger_cycle: Cell<u64>,
    
    // Paddle 0 position (0-255). None = no joystick connected (open circuit).
    paddle0: Cell<Option<u8>>,
    
    // Paddle 1 position (0-255). None = no joystick connected (open circuit).
    paddle1: Cell<Option<u8>>,
}

impl Joystick {
    pub fn new() -> Self {
        Self {
            trigger_cycle: Cell::new(u64::MAX),
            paddle0: Cell::new(Some(128)),
            paddle1: Cell::new(Some(128)),
        }
    }
    
    // Trigger the paddle timer (called when PTRIG $C070 is accessed).
    // This discharges the capacitors and starts the RC timing cycle.
    pub fn trigger(&self, cycles: u64) {
        self.trigger_cycle.set(cycles);
    }
    
    // Read a paddle input ($C064 for paddle 0, $C065 for paddle 1).
    // Returns the byte to be read, with bit 7 indicating timer status.
    pub fn read(&self, paddle: u8, cycles: u64) -> u8 {
        let trigger = self.trigger_cycle.get();
        
        // Get the paddle position (None = no joystick = open circuit)
        let position = match paddle {
            0 => self.paddle0.get(),
            1 => self.paddle1.get(),
            _ => None,
        };
        
        // No joystick connected (open circuit): bit 7 always HIGH.
        // The capacitor has no resistor path to charge through, so the
        // comparator never fires. PDL() loops until timeout → returns 255.
        // Games see 255 on both axes and treat it as "no joystick."
        if position.is_none() {
            return 0x80;
        }

        if trigger == u64::MAX {
            // PTRIG never accessed - capacitor in settled state, bit 7 = 0
            return 0x00;
        }
        
        let elapsed = cycles.saturating_sub(trigger);
        let pos = position.unwrap();
        
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
    
    // Set paddle 0 position. Use None for no joystick/open circuit.
    #[allow(dead_code)]
    pub fn set_paddle0(&self, position: Option<u8>) {
        self.paddle0.set(position);
    }
    
    // Set paddle 1 position. Use None for no joystick/open circuit.
    #[allow(dead_code)]
    pub fn set_paddle1(&self, position: Option<u8>) {
        self.paddle1.set(position);
    }
    
    // Check if a joystick is connected (either paddle has a position).
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
