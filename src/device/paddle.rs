// Apple IIc 556 dual-timer paddle model with optional host gamepad input via gilrs.
//
// The 556 timer IC works as two independent monostable multivibrators.
// When PTRIG ($C070) is accessed, both outputs go HIGH. Each output
// stays HIGH for a duration proportional to the connected paddle
// resistance (position 0-255 × ~11 CPU cycles). Software polls
// $C064/$C065 bit 7 to time when the output goes LOW.
//
// With no paddle connected (open circuit, R=∞), the output stays HIGH
// forever after PTRIG — this is how games detect "no joystick."

use std::cell::Cell;
use gilrs::{Gilrs, GamepadId, Event, EventType, Axis, Button};

// Each position unit ≈ 11 CPU cycles at 1.023 MHz.
const CYCLES_PER_POSITION: u64 = 11;

pub struct Paddle {
    // Cycle count when PTRIG ($C070) was last accessed.
    // u64::MAX indicates PTRIG has never been triggered.
    trigger_cycle: Cell<u64>,

    // Paddle 0 position (0-255). None = no joystick connected (open circuit).
    paddle0: Cell<Option<u8>>,

    // Paddle 1 position (0-255). None = no joystick connected (open circuit).
    paddle1: Cell<Option<u8>>,

    // Button state (directly read by IOU at $C061/$C062)
    pub button0: Cell<bool>,
    pub button1: Cell<bool>,

    // gilrs gamepad context (None if --paddle not specified)
    gilrs: Option<Gilrs>,
    active_gamepad: Option<GamepadId>,
}

impl Paddle {
    pub fn new() -> Self {
        Self {
            trigger_cycle: Cell::new(u64::MAX),
            paddle0: Cell::new(None),
            paddle1: Cell::new(None),
            button0: Cell::new(false),
            button1: Cell::new(false),
            gilrs: None,
            active_gamepad: None,
        }
    }

    // Enable host gamepad input. Call once at startup if --paddle is specified.
    pub fn enable_gamepad(&mut self) {
        match Gilrs::new() {
            Ok(g) => {
                // Pick the first connected gamepad, if any
                let first = g.gamepads().next().map(|(id, gp)| {
                    println!("Gamepad connected: {} ({:?})", gp.name(), id);
                    id
                });
                self.active_gamepad = first;
                self.gilrs = Some(g);
                // Mark paddles as connected at center position
                self.paddle0.set(Some(128));
                self.paddle1.set(Some(128));
                if first.is_none() {
                    println!("Joystick enabled but no gamepad detected — plug one in");
                }
            }
            Err(e) => {
                eprintln!("Failed to initialize gamepad library: {}", e);
            }
        }
    }

    // Poll host gamepad for new events. Call once per frame from the main loop.
    pub fn poll(&mut self) {
        let gilrs = match self.gilrs.as_mut() {
            Some(g) => g,
            None => return,
        };

        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            match event {
                EventType::Connected => {
                    println!("Gamepad connected: {} ({:?})", gilrs.gamepad(id).name(), id);
                    if self.active_gamepad.is_none() {
                        self.active_gamepad = Some(id);
                    }
                }
                EventType::Disconnected => {
                    println!("Gamepad disconnected: {:?}", id);
                    if self.active_gamepad == Some(id) {
                        self.active_gamepad = None;
                        // Open circuit — no joystick
                        self.paddle0.set(None);
                        self.paddle1.set(None);
                        self.button0.set(false);
                        self.button1.set(false);
                    }
                }
                _ => {
                    if self.active_gamepad.is_none() {
                        self.active_gamepad = Some(id);
                    }
                }
            }
        }

        // Read cached axis/button state from the active gamepad
        if let Some(gp_id) = self.active_gamepad {
            let gp = gilrs.gamepad(gp_id);

            // Map left stick X/Y (-1.0..1.0) to paddle 0-255
            if let Some(axis_data) = gp.axis_data(Axis::LeftStickX) {
                let v = axis_data.value();
                let pos = ((v + 1.0) * 0.5 * 255.0).clamp(0.0, 255.0) as u8;
                self.paddle0.set(Some(pos));
            }
            if let Some(axis_data) = gp.axis_data(Axis::LeftStickY) {
                let v = axis_data.value();
                // Invert Y: stick up = paddle 0 (top), stick down = paddle 255 (bottom)
                let pos = ((-v + 1.0) * 0.5 * 255.0).clamp(0.0, 255.0) as u8;
                self.paddle1.set(Some(pos));
            }

            // Map face buttons to Apple II buttons
            // South (A/Cross) = Open Apple (button 0)
            // West (X/Square) = Solid Apple (button 1)
            self.button0.set(gp.is_pressed(Button::South));
            self.button1.set(gp.is_pressed(Button::West));
        }
    }

    // Trigger the paddle timer (called when PTRIG $C070 is accessed).
    pub fn trigger(&self, cycles: u64) {
        self.trigger_cycle.set(cycles);
    }

    // Read a paddle input ($C064 for paddle 0, $C065 for paddle 1).
    // Bit 7: 0x80 = timer still counting, 0x00 = timer expired.
    pub fn read(&self, paddle: u8, cycles: u64) -> u8 {
        let trigger = self.trigger_cycle.get();

        // Before PTRIG is ever accessed, 556 is in settled state, output LOW.
        if trigger == u64::MAX {
            return 0x00;
        }

        let elapsed = cycles.saturating_sub(trigger);

        let position = match paddle {
            0 => self.paddle0.get(),
            1 => self.paddle1.get(),
            _ => None,
        };

        match position {
            Some(pos) => {
                let timeout_cycles = (pos as u64) * CYCLES_PER_POSITION;
                if elapsed < timeout_cycles { 0x80 } else { 0x00 }
            }
            None => {
                // Open circuit: always HIGH after PTRIG (never expires)
                0x80
            }
        }
    }

    /// Returns true if a gamepad is enabled (gilrs initialized).
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.gilrs.is_some()
    }
}

impl Default for Paddle {
    fn default() -> Self {
        Self::new()
    }
}
