use std::path::Path;
use std::time::Instant;
use a2kit::img::DiskImage;

#[derive(Clone, Copy, PartialEq, Debug)]
enum WozFormat { Woz1, Woz2, Unknown }

/// Per-drive state
struct DriveState {
    disk: Option<Box<dyn DiskImage>>,
    disk_path: Option<String>,
    woz_format: WozFormat,
    woz_raw: Vec<u8>,     // Raw WOZ file bytes for direct patching on save
    woz_tmap: [u8; 160],  // TMAP entries (quarter-track -> TRKS index)
    woz_bit_counts: [u32; 35], // Per-track bit counts from WOZ TRKS chunk
    dirty: bool,
    last_save: Instant,

    head_pos: u8, // 0-160 quarter tracks (track = head_pos / 4)

    track_data: Vec<u8>,
    track_bit_count: usize, // Actual valid bits in track_data (may be less than track_data.len()*8 due to block-alignment padding)
    loaded_track: Option<u8>,

    bit_index: usize,
    shift_register: u8, // Bits shift in here from disk
    data_latch: u8,     // CPU reads from here; loaded when shift_register MSB=1
    bit_cycle: u8, // 0-3: cycles within current bit period (4 cycles = 1 bit)

    write_protect: bool,

    // Pre-decoded latch state at each bit position for O(1) reads
    // (Currently unused - prepared for future optimization)
    #[allow(dead_code)]
    nibble_latch: Vec<u8>,
    #[allow(dead_code)]
    nibble_epoch: Vec<u16>,
    #[allow(dead_code)]
    next_epoch_bit: Vec<u32>,
    nibbles_valid: bool,
    consumed_epoch: u16,
    data_ready: bool,

    // Write state (IWM double-buffer: data register + shift register)
    write_data_reg: u8,       // Data register: CPU writes here via write_load
    write_data_pending: bool, // True when data_reg holds unshifted data
    write_shift: u8,          // Shift register: actively clocked out to disk
    write_bits_left: u8,      // Bits remaining in shift register
    was_writing: bool,

    cycles_since_save_check: u64,
}

impl DriveState {
    fn new() -> Self {
        Self {
            disk: None,
            disk_path: None,
            woz_format: WozFormat::Unknown,
            woz_raw: Vec::new(),
            woz_tmap: [0xFF; 160],
            woz_bit_counts: [0; 35],
            dirty: false,
            last_save: Instant::now(),
            head_pos: 0,
            track_data: Vec::new(),
            track_bit_count: 0,
            loaded_track: None,
            bit_index: 0,
            shift_register: 0,
            data_latch: 0,
            bit_cycle: 0,
            write_protect: false,
            nibble_latch: Vec::new(),
            nibble_epoch: Vec::new(),
            next_epoch_bit: Vec::new(),
            nibbles_valid: false,
            consumed_epoch: 0,
            data_ready: false,
            write_data_reg: 0,
            write_data_pending: false,
            write_shift: 0,
            write_bits_left: 0,
            was_writing: false,
            cycles_since_save_check: 0,
        }
    }

    fn has_disk(&self) -> bool {
        !self.woz_raw.is_empty()
    }
}

pub struct Iwm {
    pub motor_on: bool,
    q6: bool,
    q7: bool,
    write_mode: bool,  // Set by Q7H access, cleared by Q7L access
    latch: u8,         // Data latch loaded by STA to odd Q6H address
    pub debug: bool,

    phases: u8,

    pub mode: u8,
    pub drive_select: bool, // false = Drive 1, true = Drive 2
    pub fast_disk: bool,
    pub writes_enabled: bool,
    cycles_since_last_read: u64,
    motor_off_pending: bool,       // True when motor-off timer is counting down
    motor_off_timer: u64,          // Cycles remaining before motor actually turns off
    motor_on_cycles: u64,          // Cycles since motor turned on (for MZ status bit)

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
            write_mode: false,
            latch: 0,
            debug: false,
            phases: 0,

            mode: 0,
            drive_select: false,
            fast_disk: true,
            writes_enabled: true,
            cycles_since_last_read: 0,
            motor_off_pending: false,
            motor_off_timer: 0,
            motor_on_cycles: 0,

            drives: [DriveState::new(), DriveState::new()],

            bytes_read_counter: 0,
            revolutions_counter: 0,
            current_track_revolutions: 0,
            data_overrun_counter: 0,
        }
    }

    /// Reset IWM chip state as if the hardware reset line was asserted.
    /// Disk contents and head positions are preserved.
    pub fn reset(&mut self) {
        self.motor_on = false;
        self.q6 = false;
        self.q7 = false;
        self.write_mode = false;
        self.latch = 0;
        self.phases = 0;
        self.mode = 0;
        self.drive_select = false;
        self.motor_off_pending = false;
        self.motor_off_timer = 0;
        self.motor_on_cycles = 0;
        self.cycles_since_last_read = 0;
        for drive in &mut self.drives {
            drive.was_writing = false;
            drive.write_data_reg = 0;
            drive.write_data_pending = false;
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
        // Eject any existing disk first so stale track data is flushed
        if self.drives[drive].has_disk() {
            self.eject_disk(drive);
        }
        let path_str = path.as_ref().to_str().ok_or(anyhow::anyhow!("Invalid path"))?;

        // Parse WOZ bit_counts from raw file before a2kit takes ownership
        self.drives[drive].woz_bit_counts = [0; 35];
        self.drives[drive].woz_tmap = [0xFF; 160];
        self.drives[drive].woz_format = WozFormat::Unknown;
        self.drives[drive].woz_raw.clear();
        if let Ok(raw) = std::fs::read(path_str) {
            if raw.len() > 256 && &raw[0..4] == b"WOZ1" {
                self.drives[drive].woz_format = WozFormat::Woz1;
                // WOZ1: TMAP at offset 88 (80+8), TRKS at offset 256 (248+8)
                // Each Trk is 6656 bytes: 6646 bits + bytes_used(2) + bit_count(2) + 6 more
                let tmap_offset = 88;
                let trks_offset = 256;
                let trk_size: usize = 6656;
                if tmap_offset + 160 <= raw.len() {
                    self.drives[drive].woz_tmap.copy_from_slice(&raw[tmap_offset..tmap_offset + 160]);
                }
                for track in 0..35u8 {
                    let qt = (track * 4) as usize;
                    if qt < 160 {
                        let tmap_idx = raw[tmap_offset + qt] as usize;
                        if tmap_idx != 0xFF {
                            let bc_offset = trks_offset + tmap_idx * trk_size + 6648; // bit_count at +6648
                            if bc_offset + 2 <= raw.len() {
                                let bit_count = u16::from_le_bytes([raw[bc_offset], raw[bc_offset + 1]]) as u32;
                                self.drives[drive].woz_bit_counts[track as usize] = bit_count;
                            }
                        }
                    }
                }
                self.drives[drive].woz_raw = raw;
            } else if raw.len() > 1536 && &raw[0..4] == b"WOZ2" {
                self.drives[drive].woz_format = WozFormat::Woz2;
                // WOZ2: TMAP at offset 96 (88+8), TRKS records at offset 264 (256+8)
                // Each Trk record is 8 bytes: starting_block(2) + block_count(2) + bit_count(4)
                let tmap_offset = 96;
                let trks_offset = 264;
                if tmap_offset + 160 <= raw.len() {
                    self.drives[drive].woz_tmap.copy_from_slice(&raw[tmap_offset..tmap_offset + 160]);
                }
                for track in 0..35u8 {
                    let qt = (track * 4) as usize;
                    if qt < 160 {
                        let tmap_idx = raw[tmap_offset + qt] as usize;
                        if tmap_idx != 0xFF {
                            let bc_offset = trks_offset + tmap_idx * 8 + 4;
                            if bc_offset + 4 <= raw.len() {
                                let bit_count = u32::from_le_bytes([
                                    raw[bc_offset], raw[bc_offset + 1],
                                    raw[bc_offset + 2], raw[bc_offset + 3],
                                ]);
                                self.drives[drive].woz_bit_counts[track as usize] = bit_count;
                            }
                        }
                    }
                }
                self.drives[drive].woz_raw = raw;
            }
        }

        // BOOT_DIAG: Print WOZ format detection result
        println!("IWM: Loaded drive {} disk '{}' woz_format={:?} woz_raw_len={}", 
            drive + 1, path_str, self.drives[drive].woz_format, self.drives[drive].woz_raw.len());

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
                let d = self.di();
                if self.debug {
                    println!("IWM MOTOR ON: drive={} has_disk={} woz_format={:?} head_pos={} loaded_track={:?}",
                        d + 1, self.drives[d].has_disk(), self.drives[d].woz_format, 
                        self.drives[d].head_pos, self.drives[d].loaded_track);
                }
                self.motor_on_cycles = 0;
            }
            self.motor_on = true;
            self.cycles_since_last_read = 0;
        } else if self.motor_on {
            // Motor OFF request — check mode bit 2 for delay behavior
            if (self.mode & 0x04) != 0 {
                // Mode bit 2 set: immediate motor off (no timer)
                let d = self.di();
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }
                self.motor_on = false;
                if self.debug { println!("IWM MOTOR: ON → OFF immediate (drive={})", d + 1); }
            } else {
                // Mode bit 2 clear (default): start ~1 second motor-off timer
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
                        // Flush dirty track before changing tracks
                        if self.drives[d].dirty {
                            self.flush_track(d);
                            self.save_disk(d);
                            self.drives[d].was_writing = false;
                        }
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

        // Track how long motor has been on (for MZ status bit)
        self.motor_on_cycles = self.motor_on_cycles.saturating_add(cycles);

        let d = self.di();
        if !self.drives[d].has_disk() {
            return;
        }

        self.drives[d].cycles_since_save_check += cycles;

        // Check if we need to load track
        if self.drives[d].head_pos % 4 == 0 {
            let track_num = self.drives[d].head_pos / 4;

            if track_num < 35 && self.drives[d].loaded_track != Some(track_num) {
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }

                // Load track directly from woz_raw (bypasses a2kit's stale cache)
                if let Some(data) = self.load_track_from_raw(d, track_num) {
                    let bit_count = {
                        let woz_bc = self.drives[d].woz_bit_counts[track_num as usize] as usize;
                        if woz_bc > 0 && woz_bc <= data.len() * 8 {
                            woz_bc
                        } else {
                            data.iter().rposition(|&b| b != 0)
                                .map(|pos| (pos + 1) * 8)
                                .unwrap_or(data.len() * 8)
                        }
                    };
                    self.drives[d].track_data = data;
                    self.drives[d].track_bit_count = bit_count;
                    self.drives[d].bit_index = 0;
                    self.drives[d].loaded_track = Some(track_num);
                    self.drives[d].dirty = false;
                    self.drives[d].nibbles_valid = false;
                    if self.debug {
                        println!("IWM: Drive {} loaded track {} (buf_len={}, bit_count={})",
                            d + 1, track_num, self.drives[d].track_data.len(), self.drives[d].track_bit_count);
                        // Dump first 32 bytes of track data for debugging
                        let dump_len = std::cmp::min(32, self.drives[d].track_data.len());
                        let hex: String = self.drives[d].track_data[..dump_len].iter()
                            .map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        println!("IWM: Track {} first {} bytes: {}", track_num, dump_len, hex);
                    }
                } else {
                    // BOOT_DIAG: Track load failed - print diagnostic info
                    if self.debug {
                        let qt = (track_num * 4) as usize;
                        let tmap_idx = if qt < 160 { self.drives[d].woz_tmap[qt] } else { 0xFF };
                        println!("IWM BOOT_DIAG: Track {} load FAILED! drive={} woz_format={:?} woz_raw_len={} tmap[{}]={:02X}",
                            track_num, d + 1, self.drives[d].woz_format, self.drives[d].woz_raw.len(), qt, tmap_idx);
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

        // === BIT-LEVEL PROCESSING ===
        // Process bits continuously as cycles elapse (4 cycles = 1 bit for 5.25" drives)
        // IWM spec: 4µs per bit in slow mode = ~4 CPU cycles at 1.023 MHz
        let cycles_per_bit: u64 = 4;
        
        let track_bits = self.drives[d].track_bit_count;
        if track_bits == 0 || self.drives[d].track_data.is_empty() {
            return;
        }

        // Calculate bits elapsed this tick
        let total_cycles = self.drives[d].bit_cycle as u64 + cycles;
        let bits_elapsed = (total_cycles / cycles_per_bit) as usize;
        self.drives[d].bit_cycle = (total_cycles % cycles_per_bit) as u8;

        if bits_elapsed == 0 {
            return;
        }

        let writing = self.write_mode && self.drives[d].was_writing && self.writes_enabled;

        // Process each bit (typically only 1-2 per tick() call)
        for _ in 0..bits_elapsed {
            let bit_idx = self.drives[d].bit_index;
            let byte_idx = bit_idx / 8;
            let bit_offset = 7 - (bit_idx % 8);

            if writing {
                // WRITE: shift out bit from shift register to track
                if byte_idx < self.drives[d].track_data.len() {
                    if self.drives[d].write_bits_left > 0 {
                        let bit = (self.drives[d].write_shift >> 7) & 1;
                        if bit == 1 {
                            self.drives[d].track_data[byte_idx] |= 1 << bit_offset;
                        } else {
                            self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                        }
                        self.drives[d].write_shift <<= 1;
                        self.drives[d].write_bits_left -= 1;
                        
                        // If shift register just emptied and data is pending, transfer it
                        if self.drives[d].write_bits_left == 0 && self.drives[d].write_data_pending {
                            self.drives[d].write_shift = self.drives[d].write_data_reg;
                            self.drives[d].write_bits_left = 8;
                            self.drives[d].write_data_pending = false;
                        }
                    } else if self.drives[d].write_data_pending {
                        // Late transfer: shift register was empty but we have pending data
                        self.drives[d].write_shift = self.drives[d].write_data_reg;
                        self.drives[d].write_bits_left = 8;
                        self.drives[d].write_data_pending = false;
                        // Write the first bit
                        let bit = (self.drives[d].write_shift >> 7) & 1;
                        if bit == 1 {
                            self.drives[d].track_data[byte_idx] |= 1 << bit_offset;
                        } else {
                            self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                        }
                        self.drives[d].write_shift <<= 1;
                        self.drives[d].write_bits_left -= 1;
                    } else {
                        // Underrun: shift register empty and no pending data, write zero
                        // (Normal for self-sync bytes which intentionally produce 10-bit patterns)
                        self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                    }
                    self.drives[d].dirty = true;
                }
            } else {
                // READ: shift in bit from track to shift register
                if byte_idx < self.drives[d].track_data.len() {
                    let bit = (self.drives[d].track_data[byte_idx] >> bit_offset) & 1;
                    self.drives[d].shift_register = (self.drives[d].shift_register << 1) | bit;
                    
                    // When MSB is set, we have a complete nibble - latch it
                    if self.drives[d].shift_register & 0x80 != 0 {
                        self.drives[d].data_latch = self.drives[d].shift_register;
                        self.drives[d].shift_register = 0;
                        self.drives[d].data_ready = true;
                    }
                }
            }

            // Advance bit index (disk always rotates)
            self.drives[d].bit_index += 1;
            if self.drives[d].bit_index >= track_bits {
                self.drives[d].bit_index = 0;
                self.revolutions_counter += 1;
                self.current_track_revolutions += 1;
            }
        }
    }

    /// Load track data directly from woz_raw bytes (avoids a2kit's stale FluxCells cache).
    fn load_track_from_raw(&self, d: usize, track_num: u8) -> Option<Vec<u8>> {
        let qt = (track_num * 4) as usize;
        if qt >= 160 { return None; }
        let tmap_idx = self.drives[d].woz_tmap[qt] as usize;
        if tmap_idx == 0xFF { return None; }

        match self.drives[d].woz_format {
            WozFormat::Woz1 => {
                let trk_offset = 256 + tmap_idx * 6656;
                let data_end = trk_offset + 6646;
                if data_end <= self.drives[d].woz_raw.len() {
                    Some(self.drives[d].woz_raw[trk_offset..data_end].to_vec())
                } else {
                    None
                }
            },
            WozFormat::Woz2 => {
                let rec_offset = 264 + tmap_idx * 8;
                if rec_offset + 8 > self.drives[d].woz_raw.len() { return None; }
                let start_block = u16::from_le_bytes([
                    self.drives[d].woz_raw[rec_offset],
                    self.drives[d].woz_raw[rec_offset + 1],
                ]) as usize;
                let block_count = u16::from_le_bytes([
                    self.drives[d].woz_raw[rec_offset + 2],
                    self.drives[d].woz_raw[rec_offset + 3],
                ]) as usize;
                let data_offset = start_block * 512;
                let data_len = block_count * 512;
                let data_end = data_offset + data_len;
                if data_end <= self.drives[d].woz_raw.len() {
                    Some(self.drives[d].woz_raw[data_offset..data_end].to_vec())
                } else {
                    None
                }
            },
            WozFormat::Unknown => None,
        }
    }

    /// Decode and verify all sectors on the current track from the raw bitstream.
    /// Prints detailed status for each sector found — useful for diagnosing write corruption.
    fn verify_track_sectors(&self, d: usize) {
        let track_data = &self.drives[d].track_data;
        let total_bits = self.drives[d].track_bit_count;
        let track_num = self.drives[d].loaded_track.unwrap_or(255);
        if total_bits == 0 || track_data.is_empty() { return; }

        // Helper: read one bit from bitstream
        let get_bit = |pos: usize| -> u8 {
            let p = pos % total_bits;
            (track_data[p / 8] >> (7 - (p % 8))) & 1
        };

        // Helper: read a decoded nibble (wait for bit 7 set) starting from a bit position.
        // Returns (nibble_value, next_bit_position, bits_consumed).
        let read_nibble = |start: usize| -> (u8, usize) {
            let mut shift: u8 = 0;
            let mut pos = start;
            for _ in 0..64 {  // safety limit
                shift = (shift << 1) | get_bit(pos);
                pos += 1;
                if shift & 0x80 != 0 {
                    return (shift, pos);
                }
            }
            (0, pos) // failed
        };

        // Scan for address fields (D5 AA 96) and data fields (D5 AA AD)
        if self.debug { println!("=== VERIFY TRACK {} (drive {}, {} bits) ===", track_num, d + 1, total_bits); }

        // Decode 4-and-4 encoded byte pair
        let decode_44 = |a: u8, b: u8| -> u8 {
            ((a << 1) | 1) & b
        };

        let mut sectors_found = 0;
        let mut data_fields_ok = 0;
        let mut data_fields_bad = 0;
        let mut bit_pos: usize = 0;
        let mut scanned = 0;
        let mut d5_count = 0;

        while scanned < total_bits + 20 {
            let (nib, next) = read_nibble(bit_pos);
            scanned += next - bit_pos;
            bit_pos = next;

            if nib != 0xD5 { continue; }
            d5_count += 1;
            let saved_pos = bit_pos;

            let (nib2, next2) = read_nibble(bit_pos);
            bit_pos = next2;
            if nib2 != 0xAA { continue; }

            let (nib3, next3) = read_nibble(bit_pos);
            bit_pos = next3;

            if nib3 == 0x96 {
                // Address field
                let (v1, p) = read_nibble(bit_pos);
                let (v2, p) = read_nibble(p);
                let vol = decode_44(v1, v2);
                let (t1, p) = read_nibble(p);
                let (t2, p) = read_nibble(p);
                let trk = decode_44(t1, t2);
                let (s1, p) = read_nibble(p);
                let (s2, p) = read_nibble(p);
                let sec = decode_44(s1, s2);
                let (c1, p) = read_nibble(p);
                let (c2, p) = read_nibble(p);
                let cksum = decode_44(c1, c2);
                let expected = vol ^ trk ^ sec;
                let (e1, p) = read_nibble(p);
                let (e2, p) = read_nibble(p);
                let (e3, p) = read_nibble(p);
                bit_pos = p;

                let cksum_ok = cksum == expected;
                let epilog_ok = e1 == 0xDE && e2 == 0xAA && e3 == 0xEB;
                let epilog_str = if epilog_ok { "OK".to_string() } else { format!("{:02X} {:02X} {:02X}", e1, e2, e3) };
                if self.debug { println!("  ADDR @bit {}: vol={} trk={} sec={:2} cksum={} epilog={}",
                    saved_pos - 8, vol, trk, sec,
                    if cksum_ok { "OK" } else { "BAD" },
                    epilog_str); }
                sectors_found += 1;
            } else if nib3 == 0xAD {
                // Data field: read 342 data nibbles + 1 checksum = 343 bytes
                let data_start_bit = bit_pos;
                let mut checksum: u8 = 0;
                let mut ok = true;
                let mut nibbles_read = 0;
                let mut p = bit_pos;
                for i in 0..343 {
                    let (nib, np) = read_nibble(p);
                    p = np;
                    nibbles_read += 1;
                    if i < 342 {
                        checksum ^= nib;
                    } else {
                        // Last byte is the checksum; after XOR it should be 0
                        checksum ^= nib;
                    }
                }
                // Read epilogue
                let (e1, np) = read_nibble(p);
                let (e2, np) = read_nibble(np);
                let (e3, np) = read_nibble(np);
                bit_pos = np;

                let cksum_ok = checksum == 0;
                let epilog_ok = e1 == 0xDE && e2 == 0xAA && e3 == 0xEB;
                if cksum_ok && epilog_ok {
                    data_fields_ok += 1;
                } else {
                    data_fields_bad += 1;
                    ok = false;
                }
                let bits_span = bit_pos - data_start_bit;
                let cksum_str = if cksum_ok { "OK".to_string() } else { format!("BAD({:02X})", checksum) };
                let epilog_str = if epilog_ok { "OK".to_string() } else { format!("{:02X} {:02X} {:02X}", e1, e2, e3) };
                if self.debug { println!("  DATA @bit {}: {} nibbles, cksum={} epilog={} span={} bits{}",
                    data_start_bit, nibbles_read,
                    cksum_str, epilog_str, bits_span,
                    if !ok { " *** FAIL ***" } else { "" }); }
            }
        }
        if self.debug { println!("=== VERIFY RESULT: {} addr fields, {} data OK, {} data BAD (total D5s found: {}) ===",
            sectors_found, data_fields_ok, data_fields_bad, d5_count); }
    }

    fn flush_track(&mut self, d: usize) {
        if let Some(track_num) = self.drives[d].loaded_track {
            // Patch track data directly into the raw WOZ bytes
            let qt = (track_num * 4) as usize;
            if qt >= 160 { return; }
            let tmap_idx = self.drives[d].woz_tmap[qt] as usize;
            if tmap_idx == 0xFF { return; }

            match self.drives[d].woz_format {
                WozFormat::Woz1 => {
                    // WOZ1: TRKS data starts at offset 256
                    // Each track record: 6646 bytes data + 2 bytes bytes_used + 2 bytes bit_count + 6 padding = 6656
                    let trk_offset = 256 + tmap_idx * 6656;
                    let data_len = self.drives[d].track_data.len().min(6646);
                    if trk_offset + data_len <= self.drives[d].woz_raw.len() {
                        self.drives[d].woz_raw[trk_offset..trk_offset + data_len]
                            .copy_from_slice(&self.drives[d].track_data[..data_len]);
                        if self.debug {
                            println!("IWM: Flushed track {} to WOZ1 ({} bytes)", track_num, data_len);
                        }
                    }
                },
                WozFormat::Woz2 => {
                    // WOZ2: TRKS records at offset 264, each 8 bytes: starting_block(2) + block_count(2) + bit_count(4)
                    let rec_offset = 264 + tmap_idx * 8;
                    if rec_offset + 4 <= self.drives[d].woz_raw.len() {
                        let start_block = u16::from_le_bytes([
                            self.drives[d].woz_raw[rec_offset],
                            self.drives[d].woz_raw[rec_offset + 1],
                        ]) as usize;
                        let data_offset = start_block * 512;
                        let data_len = self.drives[d].track_data.len();
                        if data_offset + data_len <= self.drives[d].woz_raw.len() {
                            self.drives[d].woz_raw[data_offset..data_offset + data_len]
                                .copy_from_slice(&self.drives[d].track_data);
                            if self.debug {
                                println!("IWM: Flushed track {} to WOZ2 ({} bytes)", track_num, data_len);
                            }
                        }
                    }
                },
                WozFormat::Unknown => {
                    eprintln!("IWM Error: Cannot flush track {} - unknown WOZ format", track_num);
                }
            }

            // Verify all sectors on the flushed track (debug only - expensive scan)
            if self.debug {
                self.verify_track_sectors(d);
            }
        }
    }

    fn save_disk(&mut self, d: usize) {
        if let Some(path) = &self.drives[d].disk_path {
            if !self.drives[d].woz_raw.is_empty() && self.drives[d].woz_raw.len() > 12 {
                // Update CRC32 (bytes 8-11, computed over everything from byte 12 onward)
                let crc = crc32fast::hash(&self.drives[d].woz_raw[12..]);
                self.drives[d].woz_raw[8..12].copy_from_slice(&crc.to_le_bytes());
                if let Err(e) = std::fs::write(path, &self.drives[d].woz_raw) {
                    eprintln!("IWM Error: Failed to save disk: {}", e);
                } else if self.debug {
                    println!("IWM: Saved drive {} disk to {}", d + 1, path);
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
    #[allow(dead_code)]
    fn ensure_nibbles(&mut self) {
        let d = self.di();
        if self.drives[d].nibbles_valid { return; }
        
        let total_bits = self.drives[d].track_bit_count;
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
        // Sync consumed_epoch to current bit position so we don't
        // immediately see a stale nibble as "new data"
        let bi = self.drives[d].bit_index;
        self.drives[d].consumed_epoch = if bi < self.drives[d].nibble_epoch.len() {
            self.drives[d].nibble_epoch[bi]
        } else {
            0
        };
        self.drives[d].data_ready = false;
    }

    fn write_load(&mut self, val: u8) {
        let d = self.di();
        if self.drives[d].write_protect { return; }
        if !self.writes_enabled { return; }
        
        let track_bits = self.drives[d].track_bit_count;
        if track_bits == 0 { return; }

        // IWM double-buffering: data register + shift register.
        // When shift register is empty, data transfers immediately.
        // Otherwise, data waits until shift register empties (in tick()).
        
        if !self.drives[d].was_writing {
            // First write - enter write mode
            self.drives[d].was_writing = true;
            self.drives[d].write_shift = val;
            self.drives[d].write_bits_left = 8;
            self.drives[d].write_data_pending = false;
        } else if self.drives[d].write_bits_left == 0 {
            // Shift register empty - transfer immediately
            self.drives[d].write_shift = val;
            self.drives[d].write_bits_left = 8;
            self.drives[d].write_data_pending = false;
        } else {
            // Shift register busy - buffer in data register
            self.drives[d].write_data_reg = val;
            self.drives[d].write_data_pending = true;
        }
        
        self.latch = val;
        self.drives[d].nibbles_valid = false;
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
                self.drives[d].shift_register = (self.drives[d].shift_register << 1) | 1;
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
            // No disk: OLD behavior returned 0
            if self.debug { println!("IWM: read_data() NO DISK on drive {}", d + 1); }
            return 0;
        }

        if self.motor_on {
            self.cycles_since_last_read = 0;

            let result = if self.drives[d].data_ready {
                // New nibble is available in the data latch
                self.drives[d].data_ready = false;
                self.bytes_read_counter += 1;
                // Return data_latch (shift_register was already cleared when MSB was set)
                self.drives[d].data_latch
            } else if self.fast_disk && !self.drives[d].track_data.is_empty() {
                // Fast disk: skip ahead to find next complete nibble
                let total_bits = self.drives[d].track_bit_count;
                let mut bits_checked = 0;
                while bits_checked < total_bits {
                    let byte_idx = self.drives[d].bit_index / 8;
                    let bit_offset = 7 - (self.drives[d].bit_index % 8);
                    if byte_idx < self.drives[d].track_data.len() {
                        let bit = (self.drives[d].track_data[byte_idx] >> bit_offset) & 1;
                        self.drives[d].shift_register = (self.drives[d].shift_register << 1) | bit;
                        if self.drives[d].shift_register & 0x80 != 0 {
                            self.drives[d].bit_index = (self.drives[d].bit_index + 1) % total_bits;
                            self.bytes_read_counter += 1;
                            // Per IWM spec: latch the nibble and clear shift register
                            let nibble = self.drives[d].shift_register;
                            self.drives[d].shift_register = 0;
                            if self.debug { println!("IWM: Drive {} CPU Read Data {:02X} (fast)", d + 1, nibble); }
                            return nibble;
                        }
                    }
                    self.drives[d].bit_index = (self.drives[d].bit_index + 1) % total_bits;
                    bits_checked += 1;
                }
                // No complete nibble found in entire track - return shift register with MSB cleared
                self.drives[d].shift_register & 0x7F
            } else {
                // BOOT_DIAG: Track data not loaded - this would cause boot to hang!
                if self.debug && self.drives[d].track_data.is_empty() && self.bytes_read_counter < 100 {
                    println!("IWM BOOT_DIAG: read_data() with EMPTY track_data! drive={} head_pos={} loaded_track={:?} fast_disk={}",
                        d + 1, self.drives[d].head_pos, self.drives[d].loaded_track, self.fast_disk);
                }
                // No new data ready yet - return shift register with MSB cleared.
                // Software polls until MSB is set (new nibble arrived).
                self.drives[d].shift_register & 0x7F
            };

            if self.debug { println!("IWM: Drive {} CPU Read Data {:02X}", d + 1, result); }
            return result;
        }
        if self.debug { println!("IWM: read_data() MOTOR OFF, drive={}", d + 1); }
        0
    }

    pub fn access(&mut self, addr: u16, val: u8, write: bool) -> u8 {
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
                if self.motor_on && self.drives[d].was_writing {
                    if self.drives[d].dirty {
                        self.flush_track(d);
                        self.save_disk(d);
                    }
                    self.drives[d].was_writing = false;
                }
                self.set_motor(false);
            },
            0x9 => self.set_motor(true),
            0xA => self.drive_select = false,
            0xB => self.drive_select = true,
            0xC => self.q6 = false,
            0xD => {
                // L6 going ON.
                // Per IWM spec: register is written when both L6 and L7 are set
                // (or are being set) to 1 and A0 is 1 (write access).
                // Motor-On=0 + L6=1+L7=1 → mode register
                // Motor-On=1 + L6=1+L7=1 → write data register (write load)
                self.q6 = true;
                if write {
                    if self.q7 {
                        if self.motor_on {
                            // Write Load: load data into write buffer
                            self.write_load(val);
                        } else {
                            // Mode Set: write mode register
                            self.mode = val;
                            if self.debug { println!("IWM Mode set to: {:02X} (via Q6H)", self.mode); }
                        }
                    }
                }
            },
            0xE => {
                // L7 going OFF = leaving write mode.
                // Complete any in-progress byte in the shift register.
                if self.write_mode {
                    let d = self.di();
                    let track_bits = self.drives[d].track_bit_count;
                    
                    // Finish writing any bits remaining in the shift register
                    if self.writes_enabled && track_bits > 0 && self.drives[d].write_bits_left > 0 {
                        while self.drives[d].write_bits_left > 0 {
                            let bit_idx = self.drives[d].bit_index;
                            let byte_idx = bit_idx / 8;
                            let bit_offset = 7 - (bit_idx % 8);
                            
                            if byte_idx < self.drives[d].track_data.len() {
                                let bit = (self.drives[d].write_shift >> 7) & 1;
                                if bit == 1 {
                                    self.drives[d].track_data[byte_idx] |= 1 << bit_offset;
                                } else {
                                    self.drives[d].track_data[byte_idx] &= !(1 << bit_offset);
                                }
                                self.drives[d].dirty = true;
                            }
                            
                            self.drives[d].write_shift <<= 1;
                            self.drives[d].write_bits_left -= 1;
                            self.drives[d].bit_index += 1;
                            if self.drives[d].bit_index >= track_bits {
                                self.drives[d].bit_index = 0;
                            }
                        }
                    }
                    
                    // Abandon any buffered data (was never transferred to shift register)
                    self.drives[d].write_data_pending = false;
                    self.drives[d].was_writing = false;
                    
                    if self.drives[d].dirty {
                        self.flush_track(d);
                        self.save_disk(d);
                    }
                }
                self.write_mode = false;
                self.q7 = false;
            },
            0xF => {
                // L7 going ON.
                // Per IWM spec: register is written when both L6 and L7 are set
                // (or are being set) to 1 and A0 is 1 (write access).
                self.q7 = true;
                if write && self.q6 {
                    if self.motor_on {
                        // Write Load: load data into write buffer
                        self.write_load(val);
                    } else {
                        // Mode Set: write mode register 
                        self.mode = val;
                        if self.debug { println!("IWM Mode set to: {:02X} (via Q7H)", self.mode); }
                    }
                }
                // Track write_mode state: L7=1 means we're in write mode
                self.write_mode = true;
            },
            _ => {}
        }

        let d = self.di();

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
             // Write register operations handled in match arms above:
             // 0xD/0xF with L6=1+L7=1: Mode Set (motor off) or Write Load (motor on)
             0
        } else if (addr & 0xF) < 0xC {
             // Addresses 0x0-0xB (phases, motor, drive select) don't return IWM data
             // Return floating bus / last value
             0xFF
        } else {
             // Read register based on current L7, L6, Motor-On per IWM spec page 7:
             //   L7=0, L6=0, Motor=0  →  all ones
             //   L7=0, L6=0, Motor=1  →  data register (Read)
             //   L7=0, L6=1, x        →  status register (Write-Protect Sense)
             //   L7=1, L6=0, x        →  write-handshake register (Write)
             //   L7=1, L6=1, Motor=0  →  (mode set state, not a normal read)
             //   L7=1, L6=1, Motor=1  →  (write load state, return buffer for verify)
             match (self.q7, self.q6) {
                 (false, false) => {
                     if self.motor_on {
                         let result = self.read_data();
                         if self.debug { println!("IWM DATA READ: q6=0 q7=0 motor=1 drive={} result={:02X} has_disk={}", d+1, result, self.drives[d].has_disk()); }
                         result
                     } else {
                         if self.debug { println!("IWM DATA READ: q6=0 q7=0 motor=0 -> 0"); }
                         0  // OLD behavior: motor off returns 0
                     }
                 },
                 (false, true) => {
                     // Status register (Q6=1, Q7=0)
                     // Bit 7: SENSE input - for 5.25" drives, this is the write-protect sensor
                     //        For an EMPTY drive, the write-protect sensor reads nothing (LOW/0)
                     //        Only set bit 7 if there IS a disk AND it's write-protected
                     // Bit 6: MZ (reserved, should always be 0 per IWM spec)
                     // Bit 5: motor on (either /ENBL1 or /ENBL2 active)
                     // Bits 0-4: mode register
                     let mut status = self.mode & 0x1F;
                     if self.motor_on {
                         status |= 0x20;
                     }
                     // Bit 6 (MZ) stays 0 - "reserved for future products, should always be read as zero"
                     // Bit 7: SENSE input
                     //   - For a drive with a disk: reflects write-protect sensor (1 = protected)
                     //   - For an EMPTY drive (no disk): set to 1 to signal "no drive/no disk"
                     //     This allows the Apple IIc boot ROM to detect empty Drive 2 and
                     //     fallback to Drive 1 for booting.
                     if !self.drives[d].has_disk() || self.drives[d].write_protect {
                         status |= 0x80;
                     }
                     if self.debug { println!("IWM Read Status: {:02X} (drive={}, has_disk={}, wp={})", status, d+1, self.drives[d].has_disk(), self.drives[d].write_protect); }
                     status
                 },
                 (true, false) => {
                     // Handshake register per IWM spec page 9:
                     // bits 0-5: ones (reserved)
                     // bit 6: 1 = write state active (0 if underrun)
                     // bit 7: 1 = ready to accept next byte from CPU
                     //
                     // With double-buffering: CPU can write if data register is empty.
                     // The data register will hold the byte until shift register empties.
                     // NOTE: Old code returned just 0x80, reserving bits 0-5 as 0.
                     // For compatibility, keep bits 0-5 as 0 for now.
                     let in_write_state = self.drives[d].was_writing;
                     // Ready when EITHER shift register is empty OR data register is empty (can buffer)
                     let ready = self.drives[d].write_bits_left == 0 || !self.drives[d].write_data_pending;
                     let mut handshake: u8 = 0x00;
                     if in_write_state { handshake |= 0x40; }
                     if ready { handshake |= 0x80; }
                     if self.debug { println!("IWM Read Handshake: {:02X} (bits_left={}, pending={})", 
                         handshake, self.drives[d].write_bits_left, self.drives[d].write_data_pending); }
                     handshake
                 },
                 (true, true) => {
                     // Write load/mode set state read - return buffer value
                     if self.debug { println!("IWM Read Write Buffer: {:02X}", self.latch); }
                     self.latch
                 }
             }
        }
    }
}
