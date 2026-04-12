use std::path::Path;
use std::time::Instant;
use a2kit::img::{DiskImage, Track};

/// Per-drive state
struct DriveState {
    disk: Option<Box<dyn DiskImage>>,
    disk_path: Option<String>,
    dirty: bool,
    last_save: Instant,

    head_pos: u8, // 0-160 quarter tracks (track = head_pos / 4)

    track_data: Vec<u8>,
    loaded_track: Option<u8>,

    bit_index: usize,
    latch: u8,
    pending_cycles: u64,

    write_protect: bool,

    // Pre-decoded latch state at each bit position for O(1) reads
    nibble_latch: Vec<u8>,
    nibble_epoch: Vec<u16>,
    next_epoch_bit: Vec<u32>,
    nibbles_valid: bool,
    consumed_epoch: u16,
    data_ready: bool,

    // Write state
    write_shift: u8,
    write_bits_left: u8,
    was_writing: bool,

    cycles_since_save_check: u64,
}

impl DriveState {
    fn new() -> Self {
        Self {
            disk: None,
            disk_path: None,
            dirty: false,
            last_save: Instant::now(),
            head_pos: 0,
            track_data: Vec::new(),
            loaded_track: None,
            bit_index: 0,
            latch: 0,
            pending_cycles: 0,
            write_protect: false,
            nibble_latch: Vec::new(),
            nibble_epoch: Vec::new(),
            next_epoch_bit: Vec::new(),
            nibbles_valid: false,
            consumed_epoch: 0,
            data_ready: false,
            write_shift: 0,
            write_bits_left: 0,
            was_writing: false,
            cycles_since_save_check: 0,
        }
    }

    fn has_disk(&self) -> bool {
        self.disk.is_some()
    }
}

pub struct Iwm {
    pub motor_on: bool,
    q6: bool,
    q7: bool,
    pub debug: bool,

    phases: u8,

    pub mode: u8,
    pub drive_select: bool, // false = Drive 1, true = Drive 2
    pub fast_disk: bool,
    cycles_since_last_read: u64,
    motor_off_pending: bool,       // True when motor-off timer is counting down
    motor_off_timer: u64,          // Cycles remaining before motor actually turns off

    drives: [DriveState; 2],

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
            debug: false,
            phases: 0,

            mode: 0,
            drive_select: false,
            fast_disk: true,
            cycles_since_last_read: 0,
            motor_off_pending: false,
            motor_off_timer: 0,

            drives: [DriveState::new(), DriveState::new()],

            bytes_read_counter: 0,
            revolutions_counter: 0,
            current_track_revolutions: 0,
            data_overrun_counter: 0,
        }
    }

    /// Reset IWM chip state as if the hardware reset line was asserted.
    /// Disk contents and head positions are preserved (like a real power cycle).
    pub fn reset(&mut self) {
        self.motor_on = false;
        self.q6 = false;
        self.q7 = false;
        self.phases = 0;
        self.mode = 0;
        self.drive_select = false;
        self.motor_off_pending = false;
        self.motor_off_timer = 0;
        self.cycles_since_last_read = 0;
        for drive in &mut self.drives {
            drive.was_writing = false;
            drive.write_shift = 0;
            drive.write_bits_left = 0;
            drive.data_ready = false;
            drive.consumed_epoch = 0;
        }
    }

    /// Index of the currently selected drive (0 or 1).
    #[inline]
    fn di(&self) -> usize {
        self.drive_select as usize
    }

    pub fn get_and_reset_metrics(&mut self) -> (u64, bool, u8, u64, u64) {
        let bytes = self.bytes_read_counter;
        let revs = self.revolutions_counter;
        let overruns = self.data_overrun_counter;
        self.bytes_read_counter = 0;
        self.revolutions_counter = 0;
        self.data_overrun_counter = 0;
        let d = self.di();
        (bytes, self.motor_on, self.drives[d].head_pos / 2, revs, overruns)
    }

    /// Drive UI status for rendering the status bar.
    /// Returns (has_disk, is_active, is_write_protected) for the given drive (0 or 1).
    pub fn drive_status(&self, drive: usize) -> (bool, bool, bool) {
        let has_disk = self.drives[drive].has_disk();
        let is_active = self.motor_on && self.di() == drive;
        let wp = self.drives[drive].write_protect;
        (has_disk, is_active, wp)
    }

    /// Toggle write protect for the given drive.
    pub fn toggle_write_protect(&mut self, drive: usize) {
        self.drives[drive].write_protect = !self.drives[drive].write_protect;
    }

    /// Eject the disk from the given drive.
    #[allow(dead_code)]
    pub fn eject_disk(&mut self, drive: usize) {
        if self.drives[drive].dirty {
            self.flush_track(drive);
            self.save_disk(drive);
        }
        self.drives[drive].disk = None;
        self.drives[drive].disk_path = None;
        self.drives[drive].track_data.clear();
        self.drives[drive].loaded_track = None;
        self.drives[drive].nibbles_valid = false;
        self.drives[drive].dirty = false;
    }


    pub fn load_disk<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        self.load_disk_drive(0, path)
    }

    pub fn load_disk2<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        self.load_disk_drive(1, path)
    }

    fn load_disk_drive<P: AsRef<Path>>(&mut self, drive: usize, path: P) -> anyhow::Result<()> {
        let path_str = path.as_ref().to_str().ok_or(anyhow::anyhow!("Invalid path"))?;
        self.drives[drive].disk = Some(a2kit::create_img_from_file(path_str).map_err(|e| anyhow::anyhow!(e.to_string()))?);
        self.drives[drive].disk_path = Some(path_str.to_string());
        self.drives[drive].dirty = false;
        Ok(())
    }

    pub fn set_motor(&mut self, on: bool) {
        if on {
            // Motor ON cancels any pending motor-off timer
            self.motor_off_pending = false;
            self.motor_off_timer = 0;
            if !self.motor_on {
                if self.debug { println!("IWM MOTOR: OFF → ON (drive={})", self.di() + 1); }
            }
            self.motor_on = true;
            self.cycles_since_last_read = 0;
        } else if self.motor_on {
            // Motor OFF request — check mode bit 4 for delay behavior
            if (self.mode & 0x10) != 0 {
                // Mode bit 4 set: immediate motor off (no timer)
                let d = self.di();
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }
                self.motor_on = false;
                if self.debug { println!("IWM MOTOR: ON → OFF immediate (drive={})", d + 1); }
            } else {
                // Mode bit 4 clear (default): start ~1 second motor-off timer
                self.motor_off_pending = true;
                self.motor_off_timer = 1_023_000; // ~1 second at 1.023 MHz
                if self.debug { println!("IWM MOTOR: ON → OFF pending (drive={}, 1s timer)", self.di() + 1); }
            }
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
        let target_angle = match self.phases & 0xF {
            0x1 => Some(0),
            0x3 => Some(1),
            0x2 => Some(2),
            0x6 => Some(3),
            0x4 => Some(4),
            0xC => Some(5),
            0x8 => Some(6),
            0x9 => Some(7),
            _ => None,
        };

        if let Some(target) = target_angle {
            let d = self.di();
            let current_angle = (self.drives[d].head_pos % 8) as i32;
            let mut delta = target - current_angle;

            if delta > 4 { delta -= 8; }
            else if delta <= -4 { delta += 8; }

            if delta != 0 {
                let new_pos = self.drives[d].head_pos as i32 + delta;
                if new_pos >= 0 && new_pos <= 160 {
                    if self.drives[d].head_pos != new_pos as u8 {
                        self.drives[d].head_pos = new_pos as u8;
                        self.current_track_revolutions = 0;
                        if self.debug {
                            println!("IWM: Drive {} head moved to {} (Delta: {})", d + 1, self.drives[d].head_pos, delta);
                        }
                    }
                }
            }
        }
    }

    pub fn tick(&mut self, cycles: u64) {
        // Process motor-off timer even if motor appears on
        if self.motor_off_pending {
            if cycles >= self.motor_off_timer {
                self.motor_off_timer = 0;
                self.motor_off_pending = false;
                let d = self.di();
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }
                self.drives[d].was_writing = false;
                self.motor_on = false;
                if self.debug { println!("IWM MOTOR: delayed OFF fired (drive={})", d + 1); }
            } else {
                self.motor_off_timer -= cycles;
            }
        }

        if !self.motor_on {
            return;
        }

        let d = self.di();
        if !self.drives[d].has_disk() {
            return;
        }

        self.drives[d].pending_cycles += cycles;
        self.drives[d].cycles_since_save_check += cycles;

        // Check if we need to load track
        if self.drives[d].head_pos % 4 == 0 {
            let track_num = self.drives[d].head_pos / 4;

            if track_num < 35 && self.drives[d].loaded_track != Some(track_num) {
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }

                if let Some(disk) = &mut self.drives[d].disk {
                    if let Ok(data) = disk.get_track_buf(Track::Num(track_num as usize)) {
                        self.drives[d].track_data = data;
                        self.drives[d].bit_index = 0;
                        self.drives[d].loaded_track = Some(track_num);
                        self.drives[d].dirty = false;
                        self.drives[d].nibbles_valid = false;
                        if self.debug {
                            println!("IWM: Drive {} loaded track {} (len {})", d + 1, track_num, self.drives[d].track_data.len());
                        }
                    }
                }
            }
        }

        // Auto-off motor if no IWM access for ~10 seconds
        self.cycles_since_last_read += cycles;
        if self.cycles_since_last_read > 10_230_000 {
            if self.drives[d].dirty {
                self.flush_track(d);
                self.save_disk(d);
            }
            self.drives[d].was_writing = false;
            self.motor_on = false;
            self.cycles_since_last_read = 0;
            if self.debug { println!("IWM: Motor auto-off (no access for ~10s)"); }
        }

        // Auto-save every 5 seconds if dirty
        if self.drives[d].cycles_since_save_check > 1_000_000 {
            self.drives[d].cycles_since_save_check = 0;
            if self.drives[d].dirty && self.drives[d].last_save.elapsed().as_secs() >= 5 {
                self.flush_track(d);
                self.save_disk(d);
            }
        }
    }

    fn flush_track(&mut self, d: usize) {
        if let Some(track_num) = self.drives[d].loaded_track {
            if let Some(disk) = &mut self.drives[d].disk {
                if let Err(e) = disk.set_track_buf(Track::Num(track_num as usize), &self.drives[d].track_data) {
                    println!("IWM Error: Failed to flush drive {} track {}: {}", d + 1, track_num, e);
                } else {
                    if self.debug { println!("IWM: Drive {} flushed track {}", d + 1, track_num); }
                }
            }
        }
    }

    fn save_disk(&mut self, d: usize) {
        if let Some(path) = &self.drives[d].disk_path {
            if let Some(disk) = &mut self.drives[d].disk {
                let bytes = disk.to_bytes();
                if let Err(e) = std::fs::write(path, bytes) {
                    println!("IWM Error: Failed to save disk: {}", e);
                } else {
                    if self.debug { println!("IWM: Saved drive {} disk to {}", d + 1, path); }
                }
            }
        }
        self.drives[d].last_save = Instant::now();
        self.drives[d].dirty = false;
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
        let d = self.di();
        if self.drives[d].nibbles_valid { return; }
        
        let total_bits = self.drives[d].track_data.len() * 8;
        if total_bits == 0 {
            self.drives[d].nibble_latch.clear();
            self.drives[d].nibble_epoch.clear();
            self.drives[d].next_epoch_bit.clear();
            self.drives[d].nibbles_valid = true;
            return;
        }
        
        self.drives[d].nibble_latch.resize(total_bits, 0);
        self.drives[d].nibble_epoch.resize(total_bits, 0);
        self.drives[d].next_epoch_bit.resize(total_bits, 0);
        let mut shift_reg: u8 = 0;
        let mut latch: u8 = 0;
        let mut epoch: u16 = 0;
        
        for rev in 0..2u8 {
            if rev == 1 { epoch = 0; }
            for i in 0..total_bits {
                let bit = (self.drives[d].track_data[i >> 3] >> (7 - (i & 7))) & 1;
                
                shift_reg = (shift_reg << 1) | bit;
                if shift_reg & 0x80 != 0 {
                    latch = shift_reg;
                    shift_reg = 0;
                    if rev == 1 {
                        epoch = epoch.wrapping_add(1);
                    }
                }
                
                if rev == 1 {
                    self.drives[d].nibble_latch[i] = latch;
                    self.drives[d].nibble_epoch[i] = epoch;
                }
            }
        }
        
        let wrap_boundary = {
            let mut b = 0u32;
            for i in 0..total_bits {
                if i > 0 && self.drives[d].nibble_epoch[i] != self.drives[d].nibble_epoch[i - 1] {
                    b = i as u32;
                    break;
                }
            }
            b
        };
        let mut next_boundary = wrap_boundary;
        for i in (0..total_bits).rev() {
            self.drives[d].next_epoch_bit[i] = next_boundary;
            if i > 0 && self.drives[d].nibble_epoch[i] != self.drives[d].nibble_epoch[i - 1] {
                next_boundary = i as u32;
            }
        }
        
        self.drives[d].nibbles_valid = true;
        self.drives[d].consumed_epoch = 0;
        self.drives[d].data_ready = false;
    }

    fn catch_up(&mut self) {
        let d = self.di();
        if !self.motor_on || self.drives[d].track_data.is_empty() {
            self.drives[d].pending_cycles = 0;
            return;
        }

        if self.drives[d].was_writing && self.drives[d].write_bits_left > 0 {
            self.catch_up_write();
        } else {
            self.catch_up_read();
        }
    }

    fn catch_up_read(&mut self) {
        self.ensure_nibbles();

        let d = self.di();
        let cycles_per_bit: u64 = if (self.mode & 0x10) != 0 { 2 } else { 4 };
        let track_bits = self.drives[d].track_data.len() as u64 * 8;
        if track_bits == 0 {
            self.drives[d].pending_cycles = 0;
            return;
        }

        let bits_elapsed = self.drives[d].pending_cycles / cycles_per_bit;
        self.drives[d].pending_cycles %= cycles_per_bit;

        if bits_elapsed == 0 { return; }

        let new_pos = self.drives[d].bit_index as u64 + bits_elapsed;
        let revolutions = new_pos / track_bits;
        let target_bit = (new_pos % track_bits) as usize;

        self.revolutions_counter += revolutions;
        self.current_track_revolutions += revolutions;
        self.drives[d].bit_index = target_bit;

        let current_epoch = self.drives[d].nibble_epoch[target_bit];
        if current_epoch != self.drives[d].consumed_epoch || revolutions > 0 {
            self.drives[d].latch = self.drives[d].nibble_latch[target_bit];
            self.drives[d].data_ready = true;
        }
    }

    fn catch_up_write(&mut self) {
        let d = self.di();
        let cycles_per_bit: u64 = if (self.mode & 0x10) != 0 { 2 } else { 4 };
        let track_bits = self.drives[d].track_data.len() * 8;
        if track_bits == 0 {
            self.drives[d].pending_cycles = 0;
            return;
        }

        let mut bits_to_write = self.drives[d].pending_cycles / cycles_per_bit;
        self.drives[d].pending_cycles %= cycles_per_bit;

        while bits_to_write > 0 {
            if self.drives[d].write_bits_left > 0 {
                let bit = (self.drives[d].write_shift >> 7) & 1;
                let byte_idx = self.drives[d].bit_index / 8;
                let bit_offset = 7 - (self.drives[d].bit_index % 8);

                if byte_idx < self.drives[d].track_data.len() {
                    if bit == 1 {
                        self.drives[d].track_data[byte_idx] |= 1 << bit_offset;
                    } else {
                        self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                    }
                }

                self.drives[d].write_shift <<= 1;
                self.drives[d].write_bits_left -= 1;

                self.drives[d].bit_index += 1;
                if self.drives[d].bit_index >= track_bits {
                    self.drives[d].bit_index = 0;
                    self.revolutions_counter += 1;
                    self.current_track_revolutions += 1;
                }
            } else {
                let byte_idx = self.drives[d].bit_index / 8;
                let bit_offset = 7 - (self.drives[d].bit_index % 8);
                if byte_idx < self.drives[d].track_data.len() {
                    self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                }

                self.drives[d].bit_index += 1;
                if self.drives[d].bit_index >= track_bits {
                    self.drives[d].bit_index = 0;
                    self.revolutions_counter += 1;
                    self.current_track_revolutions += 1;
                }
            }

            bits_to_write -= 1;
        }
    }

    fn write_load(&mut self, val: u8) {
        self.catch_up();
        let d = self.di();
        if !self.drives[d].was_writing {
            self.drives[d].was_writing = true;
            if self.debug { println!("IWM: Drive {} entering write mode at bit {}", d + 1, self.drives[d].bit_index); }
        }
        self.drives[d].write_shift = val;
        self.drives[d].write_bits_left = 8;
        self.drives[d].dirty = true;
        self.drives[d].nibbles_valid = false;
        if self.debug { println!("IWM: Drive {} write load {:02X} at bit {}", d + 1, val, self.drives[d].bit_index); }
    }

    #[allow(dead_code)]
    fn fast_forward_zeros(&mut self) {
        let d = self.di();
        if self.drives[d].track_data.is_empty() { return; }

        let mut bits_checked = 0;
        while bits_checked < 10000 {
            let byte_index = self.drives[d].bit_index / 8;
            let bit_offset = 7 - (self.drives[d].bit_index % 8);
            
            if byte_index >= self.drives[d].track_data.len() {
                self.drives[d].bit_index = 0;
                self.revolutions_counter += 1;
                self.current_track_revolutions += 1;
                continue;
            }

            let bit = (self.drives[d].track_data[byte_index] >> bit_offset) & 1;
            
            if bit == 1 {
                self.drives[d].latch = (self.drives[d].latch << 1) | 1;
                self.drives[d].bit_index += 1;
                return;
            }

            self.drives[d].bit_index += 1;
            bits_checked += 1;
        }
    }

    pub fn read_data(&mut self) -> u8 {
        let d = self.di();
        if !self.drives[d].has_disk() {
            // No disk: return random noise so RWTS can fail gracefully
            // (bit 7 set occasionally lets the read loop exit and report I/O error)
            return fastrand::u8(..);
        }

        if self.motor_on {
            self.catch_up();
            self.cycles_since_last_read = 0;

            let result = if self.drives[d].data_ready {
                self.drives[d].consumed_epoch = if !self.drives[d].nibble_epoch.is_empty() {
                    self.drives[d].nibble_epoch[self.drives[d].bit_index]
                } else {
                    0
                };
                self.drives[d].data_ready = false;
                self.bytes_read_counter += 1;
                self.drives[d].latch
            } else if self.fast_disk && !self.drives[d].next_epoch_bit.is_empty() {
                let next_bit = self.drives[d].next_epoch_bit[self.drives[d].bit_index] as usize;
                let total_bits = self.drives[d].track_data.len() * 8;
                if next_bit < total_bits {
                    self.drives[d].bit_index = next_bit;
                    self.drives[d].latch = self.drives[d].nibble_latch[next_bit];
                    self.drives[d].consumed_epoch = self.drives[d].nibble_epoch[next_bit];
                    self.drives[d].pending_cycles = 0;
                    self.bytes_read_counter += 1;
                    self.drives[d].latch
                } else {
                    self.drives[d].latch & 0x7F
                }
            } else {
                self.drives[d].latch & 0x7F
            };

            if self.debug { println!("IWM: Drive {} CPU Read Data {:02X}", d + 1, result); }
            return result;
        }
        0
    }

    pub fn access(&mut self, addr: u16, val: u8, write: bool) -> u8 {
        self.catch_up();
        self.cycles_since_last_read = 0;

        match addr & 0xF {
            0x0 => self.set_phase(0, false),
            0x1 => self.set_phase(0, true),
            0x2 => self.set_phase(1, false),
            0x3 => self.set_phase(1, true),
            0x4 => self.set_phase(2, false),
            0x5 => self.set_phase(2, true),
            0x6 => self.set_phase(3, false),
            0x7 => self.set_phase(3, true),
            0x8 => {
                let d = self.di();
                if self.motor_on && self.drives[d].was_writing && self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                    self.drives[d].was_writing = false;
                }
                self.set_motor(false);
            },
            0x9 => self.set_motor(true),
            0xA => self.drive_select = false,
            0xB => self.drive_select = true,
            0xC => self.q6 = false,
            0xD => self.q6 = true,
            0xE => self.q7 = false,
            0xF => self.q7 = true,
            _ => {}
        }

        // Detect leaving write mode
        let d = self.di();
        if self.drives[d].was_writing {
            let still_in_write_pos = self.q6 && !self.q7 && self.motor_on;
            if !still_in_write_pos {
                self.drives[d].was_writing = false;
                self.drives[d].write_bits_left = 0;
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                    if self.debug { println!("IWM: Drive {} left write mode, flushed", d + 1); }
                }
            }
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
                 if self.q6 && self.q7 {
                     self.mode = val;
                     if self.debug { println!("IWM Mode set to: {:02X}", self.mode); }
                 } else if self.q6 && !self.q7 {
                     if self.motor_on && !self.drives[d].write_protect {
                         self.write_load(val);
                     }
                 }
             }
             0
        } else {
             if (addr & 1) == 0 {
                 match (self.q7, self.q6) {
                     (false, false) => {
                         self.read_data()
                     },
                     (false, true) => {
                         let mut status = self.mode & 0x1F;
                         if self.motor_on { status |= 0x20; }
                         if self.drives[d].write_protect {
                             status |= 0x80;
                         }
                         if self.debug { println!("IWM Read Status: {:02X}", status); }
                         status
                     },
                     (true, false) => {
                         let ready = self.drives[d].write_bits_left == 0;
                         let handshake = if ready { 0x80 } else { 0x00 };
                         if self.debug { println!("IWM Read Handshake: {:02X}", handshake); }
                         handshake
                     },
                     (true, true) => {
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
