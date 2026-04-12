use std::path::Path;
use std::time::Instant;
use a2kit::img::{DiskImage, Track};

pub struct Iwm {
    pub motor_on: bool,
    q6: bool,
    q7: bool,
    disk: Option<Box<dyn DiskImage>>,
    disk_path: Option<String>,
    dirty: bool,
    last_save: Instant,
    pub debug: bool,
    
    phases: u8,
    head_pos: u8, // 0-69 (Track = head_pos / 2)
    
    track_data: Vec<u8>,
    track_index: usize,
    loaded_track: Option<u8>,
    
    bit_index: usize,
    latch: u8,
    cycle_remainder: i64,
    pending_cycles: u64,
    
    pub mode: u8,
    pub drive_select: bool, // false = Drive 1, true = Drive 2
    cycles_since_save_check: u64,
    
    // Pre-decoded latch state at each bit position for O(1) reads
    nibble_latch: Vec<u8>,
    nibble_epoch: Vec<u16>,  // Monotonic byte counter at each bit position
    next_epoch_bit: Vec<u32>, // Bit position where the NEXT byte completes
    nibbles_valid: bool,
    consumed_epoch: u16,     // Epoch of the last byte the CPU consumed
    data_ready: bool,        // True when a new byte is available for the CPU
    pub fast_disk: bool,     // Skip BPL wait by fast-forwarding to next byte
    
    // Metrics
    pub bytes_read_counter: u64,
    pub revolutions_counter: u64,
    pub current_track_revolutions: u64,
    pub data_overrun_counter: u64,
}

impl Iwm {
    pub fn new() -> Self {
        Self {
            motor_on: false,
            q6: false,
            q7: false,
            disk: None,
            disk_path: None,
            dirty: false,
            last_save: Instant::now(),
            debug: false,
            phases: 0,
            head_pos: 0,
            track_data: Vec::new(),
            track_index: 0,
            loaded_track: None,
            bit_index: 0,
            latch: 0,
            cycle_remainder: 0,
            pending_cycles: 0,
            
            mode: 0,
            drive_select: false,
            cycles_since_save_check: 0,
            
            nibble_latch: Vec::new(),
            nibble_epoch: Vec::new(),
            next_epoch_bit: Vec::new(),
            nibbles_valid: false,
            consumed_epoch: 0,
            data_ready: false,
            fast_disk: true,
            
            bytes_read_counter: 0,
            revolutions_counter: 0,
            current_track_revolutions: 0,
            data_overrun_counter: 0,
        }
    }

    pub fn get_and_reset_metrics(&mut self) -> (u64, bool, u8, u64, u64) {
        let bytes = self.bytes_read_counter;
        let revs = self.revolutions_counter;
        let overruns = self.data_overrun_counter;
        self.bytes_read_counter = 0;
        self.revolutions_counter = 0;
        self.data_overrun_counter = 0;
        (bytes, self.motor_on, self.head_pos / 2, revs, overruns)
    }


    pub fn load_disk<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let path_str = path.as_ref().to_str().ok_or(anyhow::anyhow!("Invalid path"))?;
        self.disk = Some(a2kit::create_img_from_file(path_str).map_err(|e| anyhow::anyhow!(e.to_string()))?);
        self.disk_path = Some(path_str.to_string());
        self.dirty = false;
        Ok(())
    }

    pub fn set_motor(&mut self, on: bool) {
        self.motor_on = on;
        if on {
            self.cycle_remainder = 0;
        }
    }

    pub fn set_phase(&mut self, phase: u8, on: bool) {
        if on {
            self.phases |= 1 << phase;
        } else {
            self.phases &= !(1 << phase);
        }
        self.step_motor();
    }

    fn step_motor(&mut self) {
        // Calculate the target magnetic angle (0-7) based on active phases
        // 0=P0, 1=P0+P1, 2=P1, 3=P1+P2, 4=P2, 5=P2+P3, 6=P3, 7=P3+P0
        let target_angle = match self.phases & 0xF {
            0x1 => Some(0), // P0
            0x3 => Some(1), // P0 + P1
            0x2 => Some(2), // P1
            0x6 => Some(3), // P1 + P2
            0x4 => Some(4), // P2
            0xC => Some(5), // P2 + P3
            0x8 => Some(6), // P3
            0x9 => Some(7), // P3 + P0
            _ => None,
        };

        if let Some(target) = target_angle {
            let current_angle = (self.head_pos % 8) as i32;
            let mut delta = target - current_angle;

            // Normalize delta to shortest path (-4 to +3)
            if delta > 4 {
                delta -= 8;
            } else if delta <= -4 {
                delta += 8;
            }

            if delta != 0 {
                let new_pos = self.head_pos as i32 + delta;
                // Clamp to valid range (0-160 quarter tracks)
                if new_pos >= 0 && new_pos <= 160 {
                    if self.head_pos != new_pos as u8 {
                        self.head_pos = new_pos as u8;
                        self.current_track_revolutions = 0;
                        if self.debug {
                            println!("IWM: Head moved to {} (Delta: {})", self.head_pos, delta);
                        }
                    }
                }
            }
        }
    }

    pub fn tick(&mut self, cycles: u64) {
        // Only tick if we are selecting Drive 1 (false) and we have a disk
        if self.drive_select {
            return;
        }

        if self.motor_on {
             self.pending_cycles += cycles;
             self.cycles_since_save_check += cycles;

             // Check if we need to load track
             // Load when head is on a full track (0, 4, 8...)
             if self.head_pos % 4 == 0 {
                 let track_num = self.head_pos / 4;
                 
                 if self.loaded_track != Some(track_num) {
                     // Flush previous track if dirty
                     if self.dirty {
                         self.flush_track();
                         self.save_disk();
                     }

                     if let Some(disk) = &mut self.disk {
                         if let Ok(data) = disk.get_track_buf(Track::Num(track_num as usize)) {
                             self.track_data = data;
                             self.track_index = 0;
                             self.bit_index = 0;
                             self.loaded_track = Some(track_num);
                             self.dirty = false;
                             self.nibbles_valid = false;
                             if self.debug {
                                 println!("IWM: Loaded track {} (len {})", track_num, self.track_data.len());
                             }
                         }
                     }
                 }
             }

             // Auto-save every 5 seconds if dirty
             if self.cycles_since_save_check > 1_000_000 {
                 self.cycles_since_save_check = 0;
                 if self.dirty && self.last_save.elapsed().as_secs() >= 5 {
                     self.flush_track();
                     self.save_disk();
                 }
             }
        }
    }

    fn flush_track(&mut self) {
        if let Some(track_num) = self.loaded_track {
            if let Some(disk) = &mut self.disk {
                if let Err(e) = disk.set_track_buf(Track::Num(track_num as usize), &self.track_data) {
                    println!("IWM Error: Failed to flush track {}: {}", track_num, e);
                } else {
                    if self.debug { println!("IWM: Flushed track {}", track_num); }
                }
            }
        }
    }

    fn save_disk(&mut self) {
        if let Some(path) = &self.disk_path {
            if let Some(disk) = &mut self.disk {
                let bytes = disk.to_bytes();
                if let Err(e) = std::fs::write(path, bytes) {
                    println!("IWM Error: Failed to save disk: {}", e);
                } else {
                    if self.debug { println!("IWM: Saved disk to {}", path); }
                }
            }
        }
        self.last_save = Instant::now();
        self.dirty = false;
    }

    /// Pre-decode the bitstream into lookup tables for O(1) reads.
    /// 
    /// Builds three tables:
    /// - nibble_latch[i]: the last complete byte (bit 7 set) at or before position i
    /// - nibble_epoch[i]: monotonic counter that increments each time a byte completes
    /// - next_epoch_bit[i]: bit position where the NEXT byte after position i completes
    ///
    /// The epoch table enables the IWM handshake: the CPU only sees a new byte
    /// (bit 7 set) when the epoch at the current position exceeds the last consumed
    /// epoch. The next_epoch_bit table enables fast-disk mode by allowing O(1) skip
    /// to the next complete byte.
    fn ensure_nibbles(&mut self) {
        if self.nibbles_valid { return; }
        
        let total_bits = self.track_data.len() * 8;
        if total_bits == 0 {
            self.nibble_latch.clear();
            self.nibble_epoch.clear();
            self.next_epoch_bit.clear();
            self.nibbles_valid = true;
            return;
        }
        
        self.nibble_latch.resize(total_bits, 0);
        self.nibble_epoch.resize(total_bits, 0);
        self.next_epoch_bit.resize(total_bits, 0);
        let mut shift_reg: u8 = 0;
        let mut latch: u8 = 0;
        let mut epoch: u16 = 0;
        
        // Run 2 revolutions: first establishes steady-state, second records values
        for rev in 0..2u8 {
            if rev == 1 { epoch = 0; }
            for i in 0..total_bits {
                let bit = (self.track_data[i >> 3] >> (7 - (i & 7))) & 1;
                
                shift_reg = (shift_reg << 1) | bit;
                if shift_reg & 0x80 != 0 {
                    latch = shift_reg;
                    shift_reg = 0;
                    if rev == 1 {
                        epoch = epoch.wrapping_add(1);
                    }
                }
                
                if rev == 1 {
                    self.nibble_latch[i] = latch;
                    self.nibble_epoch[i] = epoch;
                }
            }
        }
        
        // Build next_epoch_bit by scanning backwards
        // For each position, store where the next byte boundary is
        // Find the first boundary in the track for wrap-around
        let wrap_boundary = {
            let mut b = 0u32;
            for i in 0..total_bits {
                if i > 0 && self.nibble_epoch[i] != self.nibble_epoch[i - 1] {
                    b = i as u32;
                    break;
                }
            }
            b
        };
        let mut next_boundary = wrap_boundary;
        for i in (0..total_bits).rev() {
            self.next_epoch_bit[i] = next_boundary;
            if i > 0 && self.nibble_epoch[i] != self.nibble_epoch[i - 1] {
                next_boundary = i as u32;
            }
        }
        
        self.nibbles_valid = true;
        self.consumed_epoch = 0;
        self.data_ready = false;
    }

    fn catch_up(&mut self) {
        if !self.motor_on || self.track_data.is_empty() {
            self.pending_cycles = 0;
            return;
        }

        self.ensure_nibbles();

        let cycles_per_bit: u64 = if (self.mode & 0x10) != 0 { 2 } else { 4 };
        let track_bits = self.track_data.len() as u64 * 8;
        if track_bits == 0 {
            self.pending_cycles = 0;
            return;
        }

        let bits_elapsed = self.pending_cycles / cycles_per_bit;
        self.pending_cycles %= cycles_per_bit;

        if bits_elapsed == 0 { return; }

        // Advance position
        let new_pos = self.bit_index as u64 + bits_elapsed;
        let revolutions = new_pos / track_bits;
        let target_bit = (new_pos % track_bits) as usize;

        self.revolutions_counter += revolutions;
        self.current_track_revolutions += revolutions;
        self.bit_index = target_bit;

        // Check if a new byte has completed since the last CPU read.
        // The epoch wraps around the track, so also account for revolutions.
        let current_epoch = self.nibble_epoch[target_bit];
        if current_epoch != self.consumed_epoch || revolutions > 0 {
            self.latch = self.nibble_latch[target_bit];
            self.data_ready = true;
        }
    }

    #[allow(dead_code)]
    fn fast_forward_zeros(&mut self) {
        if self.track_data.is_empty() { return; }

        let mut bits_checked = 0;
        while bits_checked < 10000 {
            let byte_index = self.bit_index / 8;
            let bit_offset = 7 - (self.bit_index % 8);
            
            if byte_index >= self.track_data.len() {
                self.bit_index = 0;
                self.revolutions_counter += 1;
                self.current_track_revolutions += 1;
                continue;
            }

            let bit = (self.track_data[byte_index] >> bit_offset) & 1;
            
            if bit == 1 {
                // found a 1, shift in and stop.
                self.latch = (self.latch << 1) | 1;
                self.bit_index += 1;
                return;
            }

            // it's a zero, skip
            self.bit_index += 1;
            bits_checked += 1;
        }
    }

    pub fn read_data(&mut self) -> u8 {
        if self.drive_select {
            return 0;
        }

        if self.motor_on {
            self.catch_up();

            let result = if self.data_ready {
                // New byte available — return it with bit 7 set and consume it
                self.consumed_epoch = if !self.nibble_epoch.is_empty() {
                    self.nibble_epoch[self.bit_index]
                } else {
                    0
                };
                self.data_ready = false;
                self.bytes_read_counter += 1;
                self.latch
            } else if self.fast_disk && !self.next_epoch_bit.is_empty() {
                // Fast disk: skip ahead to the next complete byte instantly
                let next_bit = self.next_epoch_bit[self.bit_index] as usize;
                let total_bits = self.track_data.len() * 8;
                if next_bit < total_bits {
                    self.bit_index = next_bit;
                    self.latch = self.nibble_latch[next_bit];
                    self.consumed_epoch = self.nibble_epoch[next_bit];
                    self.pending_cycles = 0;
                    self.bytes_read_counter += 1;
                    self.latch
                } else {
                    // Wrap around
                    self.latch & 0x7F
                }
            } else {
                // No new byte since last read — return latch with bit 7 clear
                self.latch & 0x7F
            };

            if self.debug { println!("IWM: CPU Read Data {:02X}", result); }
            return result;
        }
        // floating bus or random?
        0
    }

    pub fn access(&mut self, addr: u16, val: u8, write: bool) -> u8 {
        // catch up state before changing switches
        self.catch_up();

        // Update state based on address
        match addr & 0xF {
            0x0 => self.set_phase(0, false),
            0x1 => self.set_phase(0, true),
            0x2 => self.set_phase(1, false),
            0x3 => self.set_phase(1, true),
            0x4 => self.set_phase(2, false),
            0x5 => self.set_phase(2, true),
            0x6 => self.set_phase(3, false),
            0x7 => self.set_phase(3, true),
            0x8 => self.set_motor(false),
            0x9 => self.set_motor(true),
            0xA => self.drive_select = false,
            0xB => self.drive_select = true,
            0xC => self.q6 = false,
            0xD => self.q6 = true,
            0xE => self.q7 = false,
            0xF => self.q7 = true,
            _ => {}
        }

        if self.debug && write {
             let reg_name = match addr & 0xF {
                 0x0 => "PH0 OFF", 0x1 => "PH0 ON",
                 0x2 => "PH1 OFF", 0x3 => "PH1 ON",
                 0x4 => "PH2 OFF", 0x5 => "PH2 ON",
                 0x6 => "PH3 OFF", 0x7 => "PH3 ON",
                 0x8 => "MOTOR OFF", 0x9 => "MOTOR ON",
                 0xA => "DRIVE 1", 0xB => "DRIVE 2",
                 0xC => "Q6 OFF", 0xD => "Q6 ON",
                 0xE => "Q7 OFF", 0xF => "Q7 ON",
                 _ => "UNKNOWN"
             };
             println!("IWM Write: {} ({:04X}) = {:02X}", reg_name, addr, val);
        }

        if write {
             if (addr & 1) != 0 {
                 // write allowed on odd addresses
                 if self.q6 && self.q7 {
                     // mode register write (Q6=1, Q7=1)
                     self.mode = val;
                     if self.debug { println!("IWM Mode set to: {:02X}", self.mode); }
                 } else if self.q6 && !self.q7 {
                     // write data register (load) (Q6=1, Q7=0)
                     if self.motor_on {
                         // write data logic...
                         if !self.track_data.is_empty() {
                             let byte_index = self.bit_index / 8;
                             if byte_index < self.track_data.len() {
                                 self.track_data[byte_index] = val;
                                 self.dirty = true;
                                 self.nibbles_valid = false;
                                 self.bit_index += 8;
                                 if self.bit_index / 8 >= self.track_data.len() {
                                     self.bit_index = 0;
                                 }
                             }
                         }
                     }
                 }
             }
             0
        } else {
             if (addr & 1) == 0 {
                 // read allowed on even addresses
                 match (self.q7, self.q6) {
                     (false, false) => { // Q7=0, Q6=0: Read Data
                         self.read_data()
                     },
                     (false, true) => { // Q7=0, Q6=1: Read Status
                         let mut status = self.mode & 0x1F;
                         if self.motor_on { status |= 0x20; }
                         // Sense / Write Protect (Bit 7)
                         // 0 = Write Enabled, 1 = Write Protected
                         // TODO: implement write protect sensing, hardcoded to Write Protected for now
                         status &= 0x7F; 
                         if self.debug { println!("IWM Read Status: {:02X}", status); }
                         status
                     },
                     (true, false) => { // Q7=1, Q6=0: Read Handshake
                         // if self.debug { println!("IWM Read Handshake: 80"); }
                         0x80 // IWM Ready
                     },
                     (true, true) => { // Q7=1, Q6=1: Write Mode (Read)
                         if self.debug { println!("IWM Read Write Mode: 00"); }
                         0
                     }
                 }
             } else {
                 0
             }
        }
    }
}




