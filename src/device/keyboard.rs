use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

pub struct Keyboard {
    // Last key code (Apple II ASCII, 0-127)
    last_key: Cell<u8>,
    // Strobe flag, set when new key pressed, cleared by reading $C010
    strobe: Cell<bool>,
    // True if $C000 was read while strobe was set
    strobe_read: Cell<bool>,
    // Physical keys currently held: physical_key_id -> apple_ii_code
    held_keys: RefCell<HashMap<u16, u8>>,
    // Queue of pending keypresses (for rapid taps between game frames)
    key_queue: RefCell<VecDeque<u8>>,
    // Cycle count when strobe was last cleared (for auto-repeat timing)
    repeat_cycle: Cell<u64>,
    // True after first auto-repeat fires (then use faster rate)
    first_repeat_done: Cell<bool>,
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            last_key: Cell::new(0),
            strobe: Cell::new(false),
            strobe_read: Cell::new(false),
            held_keys: RefCell::new(HashMap::new()),
            key_queue: RefCell::new(VecDeque::new()),
            repeat_cycle: Cell::new(0),
            first_repeat_done: Cell::new(false),
        }
    }

    // Reset keyboard state
    pub fn reset(&self) {
        self.last_key.set(0);
        self.strobe.set(false);
        self.strobe_read.set(false);
        self.held_keys.borrow_mut().clear();
        self.key_queue.borrow_mut().clear();
        self.repeat_cycle.set(0);
        self.first_repeat_done.set(false);
    }

    // Handle key press event
    // 
    // # Arguments
    // * `physical_key` - Physical key identifier (scancode), unchanged by modifiers
    // * `apple_code` - Apple II ASCII code for this key
    // * `cycles` - Current CPU cycle count
    pub fn key_down(&self, physical_key: u16, apple_code: u8, cycles: u64) {
        let mut held = self.held_keys.borrow_mut();
        
        // Check if this physical key is already held
        if held.contains_key(&physical_key) {
            return; // Ignore OS auto-repeat
        }
        
        let had_other_keys = !held.is_empty();
        held.insert(physical_key, apple_code);
        drop(held);
        
        // Check if there's a pending strobe that hasn't been read yet
        let strobe_pending = self.strobe.get() && !self.strobe_read.get();
        
        if strobe_pending {
            // Queue this keypress, will be dequeued when game clears strobe
            self.key_queue.borrow_mut().push_back(apple_code);
        } else {
            // No pending strobe, set this key immediately
            self.last_key.set(apple_code);
            self.strobe.set(true);
            self.strobe_read.set(false);
        }
        
        // Record timing for auto-repeat
        self.repeat_cycle.set(cycles);
        
        // Only reset auto-repeat state if no other keys were held
        if !had_other_keys {
            self.first_repeat_done.set(false);
        }
    }

    // Handle key release event
    //
    // # Arguments
    // * `physical_key` - Physical key identifier that was released
    // * `cycles` - Current CPU cycle count
    pub fn key_up(&self, physical_key: u16, cycles: u64) {
        let mut held = self.held_keys.borrow_mut();
        
        // Get the Apple code for this physical key before removing
        let released_code = held.remove(&physical_key);
        
        if released_code.is_none() {
            return; // Key wasn't tracked
        }
        let released_code = released_code.unwrap();
        
        let strobe_pending = self.strobe.get() && !self.strobe_read.get();
        
        // If the released key was the current last_key and another key is still held,
        // we may need to switch, but NOT if strobe is pending!
        if self.last_key.get() == released_code && !held.is_empty() && !strobe_pending {
            // Safe to switch - strobe has been consumed
            if let Some((&_phys, &other_code)) = held.iter().next() {
                self.last_key.set(other_code);
                // Start fast auto-repeat for the held key
                self.first_repeat_done.set(true);
                self.repeat_cycle.set(cycles);
            }
        }
    }

    // Read $C000: keyboard data with strobe
    //
    // Returns key code in bits 0-6, strobe in bit 7.
    // Also handles auto-repeat timing.
    pub fn read_data(&self, cycles: u64) -> u8 {
        // Check for auto-repeat if no strobe pending
        if !self.strobe.get() {
            let held = self.held_keys.borrow();
            if !held.is_empty() {
                let cycles_since_clear = cycles.saturating_sub(self.repeat_cycle.get());
                
                // 1.023 MHz: 500ms = 511,500 cycles, 66.7ms = 68,241 cycles
                const PRE_REPEAT_CYCLES: u64 = 511_500;
                const REPEAT_CYCLES: u64 = 68_241;
                
                let threshold = if self.first_repeat_done.get() {
                    REPEAT_CYCLES
                } else {
                    PRE_REPEAT_CYCLES
                };
                
                if cycles_since_clear >= threshold {
                    // Auto-repeat! Set strobe for currently held key
                    if let Some((&_phys, &key)) = held.iter().next() {
                        drop(held);
                        self.last_key.set(key);
                        self.strobe.set(true);
                        self.strobe_read.set(false);
                        self.repeat_cycle.set(cycles);
                        self.first_repeat_done.set(true);
                    }
                }
            }
        }
        
        let mut key = self.last_key.get();
        if self.strobe.get() {
            key |= 0x80;
            self.strobe_read.set(true);
        } else {
            key &= 0x7F;
        }
        key
    }

    // Read $C010: clear strobe, return AKD status
    //
    // Returns key code in bits 0-6, AKD (Any Key Down) in bit 7.
    // Clears the strobe flag and may dequeue next key.
    pub fn read_strobe(&self, cycles: u64) -> u8 {
        self.strobe.set(false);
        self.strobe_read.set(false);
        
        // Check if there's a queued key to process
        let mut queue = self.key_queue.borrow_mut();
        if let Some(queued_key) = queue.pop_front() {
            drop(queue);
            self.last_key.set(queued_key);
            self.strobe.set(true);
            self.strobe_read.set(false);
            
            let held = self.held_keys.borrow();
            let akd = !held.is_empty();
            return if akd { queued_key | 0x80 } else { queued_key & 0x7F };
        }
        drop(queue);
        
        let held = self.held_keys.borrow();
        let last = self.last_key.get();
        
        // If the last key is no longer held but another is, switch to it
        let last_still_held = held.values().any(|&v| v == last);
        if !last_still_held && !held.is_empty() {
            if let Some((&_phys, &new_key)) = held.iter().next() {
                drop(held);
                self.last_key.set(new_key);
                self.first_repeat_done.set(true);
                self.repeat_cycle.set(cycles);
                
                return new_key | 0x80; // AKD = 1
            }
        }
        
        // Reset auto-repeat timer if key still held
        if !held.is_empty() {
            self.repeat_cycle.set(cycles);
        }
        
        // Return with AKD status
        let akd = !held.is_empty();
        if akd { last | 0x80 } else { last & 0x7F }
    }

    // Write to $C010, also clears strobe
    pub fn write_strobe(&self) {
        self.strobe.set(false);
    }

    // Check if any key is currently held (for AKD)
    #[allow(dead_code)]
    pub fn any_key_down(&self) -> bool {
        !self.held_keys.borrow().is_empty()
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}
