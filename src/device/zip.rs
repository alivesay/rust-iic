use crate::timing;

// ZIP CHIP II-8 (Model 8000) Accelerator emulation.
//
// The ZIP CHIP II-8 was a 65C02 replacement that ran at 8MHz (8x normal speed).
// Key features:
// - Automatic slowdown to 1MHz for slot I/O operations ($C000-$CFFF)
// - Press ESC during 2-second boot window to disable acceleration
// - 8KB internal cache (not emulated - not needed for correctness)
//
// The real ZIP CHIP also had software control via $C05A, but this conflicts
// with Apple IIc mouse registers. For simplicity, we use keyboard toggle (Ctrl+Z)
// and the boot-time ESC key behavior.

pub struct ZipChip {
    pub present: bool,      // ZIP Chip installed (--zip flag)
    pub enabled: bool,      // Acceleration currently enabled
    
    // Boot detection window (2 seconds = ~2M cycles at 1MHz)
    boot_window_cycles: u32,
    boot_window_active: bool,
    
    // Slowdown tracking for I/O
    slowdown_cycles: u32,   // Cycles remaining at 1MHz for I/O
}

impl ZipChip {
    // Create a new ZIP CHIP II-8 (Model 8000).
    pub fn new(present: bool) -> Self {
        Self {
            present,
            enabled: present,  // Start enabled if present
            boot_window_cycles: (timing::CYCLES_PER_SECOND * 2.0) as u32,  // ~2 seconds
            boot_window_active: present,
            slowdown_cycles: 0,
        }
    }

    // Reset ZIP Chip state (preserves present flag).
    pub fn reset(&mut self) {
        self.enabled = self.present;
        self.boot_window_cycles = (timing::CYCLES_PER_SECOND * 2.0) as u32;
        self.boot_window_active = self.present;
        self.slowdown_cycles = 0;
    }

    // Get the current effective speed multiplier.
    // Returns 1 or 8 depending on state.
    pub fn speed_multiplier(&self) -> u32 {
        if !self.present || !self.enabled || self.slowdown_cycles > 0 || self.boot_window_active {
            1
        } else {
            8  // ZIP CHIP II-8 runs at 8MHz
        }
    }

    // Called when CPU accesses I/O region ($C000-$CFFF).
    // Triggers automatic slowdown to 1MHz for a period.
    pub fn io_access(&mut self) {
        if self.present && self.enabled && !self.boot_window_active {
            // Slow down for ~16 cycles (approximate - real ZIP has complex logic)
            self.slowdown_cycles = 16;
        }
    }

    // Called each CPU cycle to decrement counters.
    pub fn tick(&mut self) {
        if self.slowdown_cycles > 0 {
            self.slowdown_cycles -= 1;
        }
        if self.boot_window_active && self.boot_window_cycles > 0 {
            self.boot_window_cycles -= 1;
            if self.boot_window_cycles == 0 {
                self.boot_window_active = false;
            }
        }
    }

    // Called when ESC key is pressed during boot window.
    // Disables acceleration for this session (per ZIP CHIP manual).
    pub fn check_boot_escape(&mut self) {
        if self.boot_window_active {
            self.enabled = false;
            self.boot_window_active = false;
            println!("ZIP CHIP: Disabled by ESC key during boot window");
        }
    }

    // Toggle acceleration on/off (for keyboard shortcut Ctrl+Z).
    pub fn toggle(&mut self) {
        if self.present {
            self.enabled = !self.enabled;
            println!("ZIP CHIP II-8: {}", if self.enabled { "ENABLED (8MHz)" } else { "DISABLED (1MHz)" });
        }
    }
}

impl Default for ZipChip {
    fn default() -> Self {
        Self::new(false)
    }
}
