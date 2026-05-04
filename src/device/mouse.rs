use std::cell::Cell;

const MOUSE_EDGE_INTERVAL: u64 = 64;
const MOUSE_PENDING_CAP: i16 = 2048;
const MOUSE_SENSITIVITY: f32 = 1.0;

pub struct Mouse {
    pub x: Cell<u16>,
    pub y: Cell<u16>,
    pub button0: Cell<bool>,
    pub button1: Cell<bool>,

    pub open_apple: Cell<bool>,
    pub solid_apple: Cell<bool>,

    pub x_dir: Cell<bool>,
    pub y_dir: Cell<bool>,

    pub x0: Cell<bool>,
    pub y0: Cell<bool>,

    accum_x: Cell<f32>,
    accum_y: Cell<f32>,
    pending_x: Cell<i16>,
    pending_y: Cell<i16>,

    edge_timer: Cell<u64>,

    pub xy_mask: Cell<bool>,
    pub vbl_mask: Cell<bool>,

    pub x0_edge: Cell<bool>,
    pub y0_edge: Cell<bool>,

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
            open_apple: Cell::new(false),
            solid_apple: Cell::new(false),
            x_dir: Cell::new(false),
            y_dir: Cell::new(false),
            x0: Cell::new(false),
            y0: Cell::new(false),
            accum_x: Cell::new(0.0),
            accum_y: Cell::new(0.0),
            pending_x: Cell::new(0),
            pending_y: Cell::new(0),
            edge_timer: Cell::new(MOUSE_EDGE_INTERVAL),
            xy_mask: Cell::new(false),
            vbl_mask: Cell::new(false),
            x0_edge: Cell::new(false),
            y0_edge: Cell::new(false),
            x_int: Cell::new(false),
            y_int: Cell::new(false),
            vbl_int: Cell::new(false),
        }
    }

    pub fn reset(&self) {
        self.x.set(0);
        self.y.set(0);
        self.button0.set(false);
        self.button1.set(false);
        self.x_dir.set(false);
        self.y_dir.set(false);
        self.x0.set(false);
        self.y0.set(false);
        self.accum_x.set(0.0);
        self.accum_y.set(0.0);
        self.pending_x.set(0);
        self.pending_y.set(0);
        self.edge_timer.set(MOUSE_EDGE_INTERVAL);
        self.xy_mask.set(false);
        self.vbl_mask.set(false);
        self.x0_edge.set(false);
        self.y0_edge.set(false);
        self.x_int.set(false);
        self.y_int.set(false);
        self.vbl_int.set(false);
    }

    pub fn add_delta(&self, dx: f64, dy: f64) {
        let ax = self.accum_x.get() + (dx as f32) * MOUSE_SENSITIVITY;
        let ay = self.accum_y.get() + (dy as f32) * MOUSE_SENSITIVITY;

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

    pub fn tick(&self, cycles: u64) {
        let timer = self.edge_timer.get();
        if timer > cycles {
            self.edge_timer.set(timer - cycles);
            return;
        }
        self.edge_timer.set(MOUSE_EDGE_INTERVAL);

        let px = self.pending_x.get();
        if px != 0 {
            if px < 0 {
                self.pending_x.set(px + 1);
                self.x_dir.set(false);
            } else {
                self.pending_x.set(px - 1);
                self.x_dir.set(true);
            }
            let x0 = self.x0.get();
            if ((x0 && self.x0_edge.get()) || (!x0 && !self.x0_edge.get()))
                && self.xy_mask.get() {
                    self.x_int.set(true);
                }
            self.x0.set(!x0);
        }

        let py = self.pending_y.get();
        if py != 0 {
            if py < 0 {
                self.pending_y.set(py + 1);
                self.y_dir.set(true);
            } else {
                self.pending_y.set(py - 1);
                self.y_dir.set(false);
            }
            let y0 = self.y0.get();
            if ((y0 && self.y0_edge.get()) || (!y0 && !self.y0_edge.get()))
                && self.xy_mask.get() {
                    self.y_int.set(true);
                }
            self.y0.set(!y0);
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
