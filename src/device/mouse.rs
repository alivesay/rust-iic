use std::cell::Cell;

/// Cycles between quadrature edge generation.
/// Must be fast enough that the firmware's IRQ handler becomes the
/// bottleneck (~100 cycles to service an interrupt). We use a small
/// value so edges drain near the hardware-natural rate.
const MOUSE_EDGE_INTERVAL: u64 = 4;

/// Maximum pending edges per axis. Caps lag to ~32 * IRQ_time cycles
/// (~3ms at 1MHz). Fast swipes are slightly "lossy" but responsive.
const MOUSE_PENDING_CAP: i16 = 32;

/// Sensitivity multiplier for host mouse → IIc quadrature edges.
const MOUSE_SENSITIVITY: f32 = 1.0;

pub struct Mouse {
    pub x: Cell<u16>,
    pub y: Cell<u16>,
    pub button0: Cell<bool>,
    pub button1: Cell<bool>,

    // Direction indicators (X1/Y1 quadrature signals)
    // true = positive direction (right/down), false = negative (left/up)
    pub x_dir: Cell<bool>,
    pub y_dir: Cell<bool>,

    // Accumulated movement (fractional precision for scaling)
    accum_x: Cell<f32>,
    accum_y: Cell<f32>,
    // Integer pending edges after scaling
    pending_x: Cell<i16>,
    pending_y: Cell<i16>,
    edge_timer: Cell<u64>,

    // Interrupt Masks (1 = Masked/Disabled, 0 = Enabled)
    // Note: softswitches.txt says "1 = mask on" for RdXYMsk
    pub xy_mask: Cell<bool>,
    pub vbl_mask: Cell<bool>,

    // Edge Selectors (0 = Rising, 1 = Falling)
    pub x0_edge: Cell<bool>,
    pub y0_edge: Cell<bool>,

    // Interrupt Status
    pub x_int: Cell<bool>,
    pub y_int: Cell<bool>,
    pub vbl_int: Cell<bool>,
}

impl Mouse {
    pub fn new() -> Self {
        Self {
            x: Cell::new(0),
            y: Cell::new(0),
            button0: Cell::new(false),
            button1: Cell::new(false),
            x_dir: Cell::new(false),
            y_dir: Cell::new(false),
            accum_x: Cell::new(0.0),
            accum_y: Cell::new(0.0),
            pending_x: Cell::new(0),
            pending_y: Cell::new(0),
            edge_timer: Cell::new(MOUSE_EDGE_INTERVAL),
            xy_mask: Cell::new(true), // Default to masked (disabled)
            vbl_mask: Cell::new(true),
            x0_edge: Cell::new(false),
            y0_edge: Cell::new(false),
            x_int: Cell::new(false),
            y_int: Cell::new(false),
            vbl_int: Cell::new(false),
        }
    }

    /// Accumulate relative mouse movement from host.
    /// dx/dy are raw host pixel deltas (positive = right/down).
    /// Y is inverted here because Apple IIc Y increases upward
    /// in the firmware's coordinate system.
    pub fn add_delta(&self, dx: f64, dy: f64) {
        // Scale and accumulate with fractional precision
        let ax = self.accum_x.get() + (dx as f32) * MOUSE_SENSITIVITY;
        let ay = self.accum_y.get() + (-dy as f32) * MOUSE_SENSITIVITY; // invert Y

        // Extract integer part as pending edges
        let ix = ax as i16;
        let iy = ay as i16;

        if ix != 0 {
            self.pending_x.set((self.pending_x.get() + ix).clamp(-MOUSE_PENDING_CAP, MOUSE_PENDING_CAP));
            self.accum_x.set(ax - ix as f32);
        } else {
            self.accum_x.set(ax);
        }

        if iy != 0 {
            self.pending_y.set((self.pending_y.get() + iy).clamp(-MOUSE_PENDING_CAP, MOUSE_PENDING_CAP));
            self.accum_y.set(ay - iy as f32);
        } else {
            self.accum_y.set(ay);
        }
    }

    /// Feed accumulated movement as individual quadrature edges.
    /// Called from bus.tick() each CPU cycle group.
    /// Only generates a new edge if the previous one was acknowledged
    /// (x_int/y_int cleared by firmware reading $C015/$C017).
    pub fn tick(&self, cycles: u64) {
        let timer = self.edge_timer.get();
        if timer > cycles {
            self.edge_timer.set(timer - cycles);
            return;
        }
        self.edge_timer.set(MOUSE_EDGE_INTERVAL);

        // Consume one pending X edge if previous was acknowledged
        let px = self.pending_x.get();
        if px != 0 && !self.x_int.get() {
            self.x_dir.set(px > 0);
            self.pending_x.set(if px > 0 { px - 1 } else { px + 1 });
            self.x_int.set(true);
        }

        // Consume one pending Y edge if previous was acknowledged
        let py = self.pending_y.get();
        if py != 0 && !self.y_int.get() {
            self.y_dir.set(py > 0);
            self.pending_y.set(if py > 0 { py - 1 } else { py + 1 });
            self.y_int.set(true);
        }
    }

    pub fn set_button(&self, btn: usize, pressed: bool) {
        match btn {
            0 => self.button0.set(pressed),
            1 => self.button1.set(pressed),
            _ => {}
        }
    }
}
