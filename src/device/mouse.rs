use std::cell::Cell;

pub struct Mouse {
    pub x: Cell<u16>,
    pub y: Cell<u16>,
    pub button0: Cell<bool>,
    pub button1: Cell<bool>,

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
            xy_mask: Cell::new(true), // Default to masked (disabled)
            vbl_mask: Cell::new(true),
            x0_edge: Cell::new(false),
            y0_edge: Cell::new(false),
            x_int: Cell::new(false),
            y_int: Cell::new(false),
            vbl_int: Cell::new(false),
        }
    }

    pub fn set_xy(&self, x: u16, y: u16) {
        let old_x = self.x.get();
        let old_y = self.y.get();

        self.x.set(x);
        self.y.set(y);

        // Edge detection on bit 0 (X0/Y0)
        let old_x0 = (old_x & 1) != 0;
        let new_x0 = (x & 1) != 0;

        if old_x0 != new_x0 {
            // Edge detected
            let rising = new_x0;
            let falling = !new_x0;

            // x0_edge: false = Rising, true = Falling
            let trigger_edge = self.x0_edge.get();

            if (rising && !trigger_edge) || (falling && trigger_edge) {
                self.x_int.set(true);
            }
        }

        let old_y0 = (old_y & 1) != 0;
        let new_y0 = (y & 1) != 0;

        if old_y0 != new_y0 {
            let rising = new_y0;
            let falling = !new_y0;
            let trigger_edge = self.y0_edge.get();

            if (rising && !trigger_edge) || (falling && trigger_edge) {
                self.y_int.set(true);
            }
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
