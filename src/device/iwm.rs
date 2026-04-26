use std::path::Path;
use std::time::Instant;
use a2kit::img::DiskImage;

use crate::timing;
use super::drive_audio::{DriveAudio, DriveEvent, AudioProducer};
use super::smartport::SmartPort;

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

    head_pos: u16, // Quarter tracks (track = head_pos / 4)

    track_data: Vec<u8>,
    track_bit_count: usize, // Actual valid bits in track_data (may be less than track_data.len()*8 due to block-alignment padding)
    loaded_track: Option<u8>,

    bit_index: usize,
    shift_register: u8, // Bits shift in here from disk
    data_latch: u8,     // CPU reads from here; loaded when shift_register MSB=1
    bit_cycle: u8, // 0-3: cycles within current bit period (4 cycles = 1 bit)

    write_protect: bool,

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

    fn max_head_pos(&self) -> u16 {
        34 * 4
    }
}

pub struct Iwm {
    pub motor_on: bool,
    q6: bool,
    q7: bool,
    pub write_mode: bool,  // Set by Q7H access, cleared by Q7L access
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

    drives: [DriveState; 2],       // 5.25" drives (internal + external)
    
    // SmartPort bus controller (wire protocol + device chain: floppy, HDV)
    pub smartport: SmartPort,

    // 3.5" drive motor state (for audio events)
    motor_on35: bool,
    // 3.5" head selection (from $C031 bit 7, used by IOU)
    head35: u8,

    // SmartPort wire protocol state (only timing/cooldown state needed by IWM)
    smartport_idle_counter: u8,         // Counter for idle state timeout simulation
    smartport_response_cooldown: u16,   // Cooldown cycles after response to avoid phantom reads

    // Drive audio synthesis
    pub drive_audio: DriveAudio,
    audio_cycle: u64,  // Current cycle count for audio events

    // Metrics
    pub bytes_read_counter: u64,
    pub revolutions_counter: u64,
    pub current_track_revolutions: u64,
    pub data_overrun_counter: u64,
}

impl Iwm {
    pub fn new() -> Self {
        println!("disk  {:>12} {:>8}", "IWM", "ONLINE");
        println!("disk  {:>12} {:>8}", "5.25_D1", "ONLINE");
        println!("disk  {:>12} {:>8}", "5.25_D2", "ONLINE");
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
            fast_disk: false,
            writes_enabled: true,
            cycles_since_last_read: 0,
            motor_off_pending: false,
            motor_off_timer: 0,
            motor_on_cycles: 0,

            drives: [DriveState::new(), DriveState::new()],
            
            smartport: SmartPort::new(),
            
            motor_on35: false,
            head35: 0,

            smartport_idle_counter: 0,
            smartport_response_cooldown: 0,

            drive_audio: DriveAudio::new(),

            audio_cycle: 0,

            bytes_read_counter: 0,
            revolutions_counter: 0,
            current_track_revolutions: 0,
            data_overrun_counter: 0,
        }
    }
    
    /// Get current 3.5" head selection (0=lower/side0, 1=upper/side1)
    pub fn get_head35(&self) -> u8 {
        self.head35
    }
    
    /// Set 3.5" head selection (from $C031 bit 7)
    pub fn set_head35(&mut self, head: u8) {
        self.head35 = head & 1;
    }

    /// Initialize drive audio with an audio producer
    pub fn init_audio(&mut self, producer: AudioProducer, sample_rate: u32) {
        self.drive_audio = DriveAudio::with_audio(producer, sample_rate);
    }

    /// Update drive audio synthesis (call once per frame)
    pub fn update_audio(&mut self) {
        self.drive_audio.update(self.audio_cycle);
        // Tick down 3.5" drive activity indicators
        for floppy in &mut self.smartport.floppies {
            floppy.tick_activity();
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
        // Reset 3.5" drive state
        if self.motor_on35 {
            self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOff35);
        }
        self.motor_on35 = false;
        self.head35 = 0;
        // Reset SmartPort timing state
        self.smartport_idle_counter = 0;
    }

    fn has_smartport_device(&self) -> bool {
        self.smartport.has_any_device()
    }

    fn smartport_route_reason(&self, disk35_mode: bool) -> &'static str {
        if disk35_mode {
            "disk35_mode"
        } else if self.smartport.is_wire_active() {
            "wire_active"
        } else if self.has_smartport_device() {
            "device_present_idle"
        } else {
            "no_device"
        }
    }

    /// True when IWM reads should expose SmartPort semantics instead of the
    /// legacy 5.25" controller path.
    ///
    /// A present device is not enough on its own. Otherwise any attached
    /// SmartPort device alters ordinary C0Ex probing even when the guest is
    /// not in an active SmartPort exchange.
    fn is_smartport_data_active(&self, disk35_mode: bool) -> bool {
        disk35_mode || self.smartport.is_wire_active()
    }

    fn is_smartport_control_visible(&self, disk35_mode: bool) -> bool {
        disk35_mode || self.smartport.is_wire_active()
    }

    fn is_smartport_write_routed(&self, disk35_mode: bool) -> bool {
        disk35_mode || self.smartport.is_wire_active() || self.has_smartport_device()
    }

    fn is_smartport_bootstrap_visible(&self) -> bool {
        self.has_smartport_device()
    }

    fn log_route_decision(&self, op: &str, addr: u16, disk35_mode: bool) {
        if !self.debug {
            return;
        }

        log::debug!(
            "IWM route {} @{:04X}: smartport_active={} reason={} has_device={} disk35_mode={} wire_active={} q6={} q7={} motor={} motor35={}",
            op,
            addr,
            self.is_smartport_data_active(disk35_mode),
            self.smartport_route_reason(disk35_mode),
            self.has_smartport_device(),
            disk35_mode,
            self.smartport.is_wire_active(),
            self.q6,
            self.q7,
            self.motor_on,
            self.motor_on35,
        );
    }

    /// Process a byte written to SmartPort device
    fn smartport_write_byte(&mut self, val: u8) {
        if !self.has_smartport_device() {
            return;
        }

        self.smartport.write_byte(val);

        // Drain audio events generated by UniDisk35 command execution
        let events = self.smartport.drain_audio_events();
        for event in events {
            self.drive_audio.queue_event(self.audio_cycle, event);
        }
    }
    
    /// Get next byte from SmartPort response with ACK handling
    fn smartport_read_byte(&mut self) -> Option<u8> {
        self.smartport.read_byte()
    }

    fn read_smartport_data(&mut self, floating_bus: u8) -> u8 {
        if let Some(byte) = self.smartport_read_byte() {
            if self.debug {
                log::debug!("IWM SmartPort read_data byte={:02X}", byte);
            }
            return byte;
        }

        if self.smartport.take_response_done() {
            self.smartport_response_cooldown = 1;
        }

        if self.smartport_response_cooldown > 0 {
            self.smartport_response_cooldown -= 1;
            if self.debug {
                log::debug!("IWM SmartPort read_data cooldown -> 00");
            }
            return 0x00;
        }

        if self.debug {
            log::debug!("IWM SmartPort read_data idle -> floating_bus {:02X}", floating_bus);
        }
        floating_bus
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
        (bytes, self.motor_on, (self.drives[d].head_pos / 2) as u8, revs, overruns)
    }

    /// Drive UI status for rendering the status bar.
    /// Returns (has_disk, is_active, is_write_protected) for the given 5.25" drive (0 or 1).
    pub fn drive_status(&self, drive: usize) -> (bool, bool, bool) {
        let has_disk = self.drives[drive].has_disk();
        // When drive_select selects drive 2 and SmartPort devices are present,
        // the IWM motor activity belongs to the SmartPort bus, not 5.25" drive B.
        let is_active = self.motor_on && self.di() == drive
            && !(drive == 1 && self.has_smartport_device());
        let wp = self.drives[drive].write_protect;
        (has_disk, is_active, wp)
    }

    /// Toggle write protect for the given drive.
    pub fn toggle_write_protect(&mut self, drive: usize) {
        self.drives[drive].write_protect = !self.drives[drive].write_protect;
    }

    /// Get the disk image filename (not full path) for a 5.25" drive.
    pub fn disk_filename(&self, drive: usize) -> Option<String> {
        self.drives[drive].disk_path.as_ref().map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone())
        })
    }

    /// Get the disk image filename (not full path) for a 3.5" SmartPort floppy.
    pub fn disk_filename_35(&self, drive: usize) -> Option<String> {
        if drive < self.smartport.floppies.len() && self.smartport.floppies[drive].has_disk() {
            let path = &self.smartport.floppies[drive].device.path;
            if path.is_empty() {
                None
            } else {
                Some(std::path::Path::new(path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone()))
            }
        } else {
            None
        }
    }

    /// Eject the disk from the given drive.
    pub fn eject_disk(&mut self, drive: usize) {
        if self.drives[drive].dirty {
            self.flush_track(drive);
            self.save_disk(drive);
        }
        self.drives[drive].disk = None;
        self.drives[drive].disk_path = None;
        self.drives[drive].woz_format = WozFormat::Unknown;
        self.drives[drive].woz_raw.clear();
        self.drives[drive].woz_tmap = [0xFF; 160];
        self.drives[drive].woz_bit_counts = [0; 35];
        self.drives[drive].track_data.clear();
        self.drives[drive].track_bit_count = 0;
        self.drives[drive].loaded_track = None;
        self.drives[drive].nibbles_valid = false;
        self.drives[drive].dirty = false;
        // NOTE: Do NOT reset head_pos, real Apple IIc preserves head position
        // across disk changes, and some programs (like disk copiers) rely on this
        self.drives[drive].bit_index = 0;
        self.drives[drive].shift_register = 0;
        self.drives[drive].data_latch = 0;
        self.drives[drive].data_ready = false;
    }


    pub fn load_disk<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        self.load_disk_drive(0, path)
    }

    pub fn load_disk2<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        self.load_disk_drive(1, path)
    }
    
    /// Load a 3.5" disk image (.po, .2mg) into a SmartPort floppy slot
    pub fn load_disk35<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        self.load_disk35_drive(0, path)
    }

    /// Load a 3.5" disk image into a specific SmartPort floppy slot (0 or 1)
    pub fn load_disk35_drive<P: AsRef<Path>>(&mut self, slot: usize, path: P) -> anyhow::Result<()> {
        let path_str = path.as_ref().to_str().ok_or(anyhow::anyhow!("Invalid path"))?;
        self.smartport.load_floppy(slot, path_str).map_err(|e| anyhow::anyhow!(e))
    }
    
    /// Returns (has_disk, is_active, is_write_protected) for the given SmartPort floppy slot.
    pub fn drive_status_35(&self, drive: usize) -> (bool, bool, bool) {
        if drive < self.smartport.floppies.len() {
            let f = &self.smartport.floppies[drive];
            let has_disk = f.has_disk();
            // The firmware toggles the IWM motor on/off during SmartPort bus
            // exchanges (drive_select=1 selects the SmartPort port).  Use that
            // hardware signal combined with recent-access tracking so the icon
            // blinks naturally like 5.25" drives.
            let is_active = has_disk
                && f.active_frames > 0
                && self.motor_on
                && self.drive_select;
            let wp = f.device.write_protected;
            (has_disk, is_active, wp)
        } else {
            (false, false, false)
        }
    }
    
    /// Toggle write protect for the given SmartPort floppy slot.
    pub fn toggle_write_protect_35(&mut self, drive: usize) {
        if drive < self.smartport.floppies.len() {
            self.smartport.floppies[drive].toggle_write_protect();
        }
    }

    /// Eject the disk from the given SmartPort floppy slot.
    pub fn eject_disk_35(&mut self, drive: usize) {
        if drive < self.smartport.floppies.len() {
            self.smartport.floppies[drive].eject();
        }
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

        log::debug!("IWM: Loaded drive {} disk '{}' woz_format={:?} woz_raw_len={}", 
            drive + 1, path_str, self.drives[drive].woz_format, self.drives[drive].woz_raw.len());

        self.drives[drive].disk = Some(a2kit::create_img_from_file(path_str).map_err(|e| anyhow::anyhow!(e.to_string()))?);
        self.drives[drive].disk_path = Some(path_str.to_string());
        self.drives[drive].dirty = false;
        
        // Clear stale track data so new disk is read fresh
        self.drives[drive].loaded_track = None;
        self.drives[drive].track_data.clear();
        self.drives[drive].track_bit_count = 0;
        self.drives[drive].bit_index = 0;
        self.drives[drive].shift_register = 0;
        self.drives[drive].data_latch = 0;
        self.drives[drive].data_ready = false;
        self.drives[drive].nibbles_valid = false;
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
                    log::debug!("IWM MOTOR ON: drive={} has_disk={} woz_format={:?} head_pos={} loaded_track={:?}",
                        d + 1, self.drives[d].has_disk(), self.drives[d].woz_format, 
                        self.drives[d].head_pos, self.drives[d].loaded_track);
                }
                self.motor_on_cycles = 0;
                // Queue motor on audio event
                self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOn);
            }
            self.motor_on = true;
            self.cycles_since_last_read = 0;
        } else if self.motor_on {
            // Motor OFF request, check mode bit 2 for delay behavior
            if (self.mode & 0x04) != 0 {
                // Mode bit 2 set: immediate motor off (no timer)
                let d = self.di();
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }
                self.motor_on = false;
                // Queue motor off audio event
                self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOff);
                if self.debug { println!("IWM MOTOR: ON → OFF immediate (drive={})", d + 1); }
            } else {
                // Mode bit 2 clear (default): start ~1 second motor-off timer
                self.motor_off_pending = true;
                self.motor_off_timer = timing::CYCLES_PER_SECOND as u64; // ~1 second
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
            // On real Apple IIc hardware, phase signals are connected to BOTH drives.
            for d in 0..2 {
                let current_angle = (self.drives[d].head_pos % 8) as i32;
                let mut delta = target - current_angle;

                if delta > 4 { delta -= 8; }
                else if delta <= -4 { delta += 8; }

                if delta != 0 {
                    let new_pos = self.drives[d].head_pos as i32 + delta;
                    let max_head_pos = self.drives[d].max_head_pos() as i32;
                    if new_pos >= 0 && new_pos <= max_head_pos {
                        if self.drives[d].head_pos != new_pos as u16 {
                            // Flush dirty track before changing tracks
                            if self.drives[d].dirty {
                                self.flush_track(d);
                                self.save_disk(d);
                                self.drives[d].was_writing = false;
                            }
                            self.drives[d].head_pos = new_pos as u16;
                            self.current_track_revolutions = 0;
                            
                            // Queue stepper audio event only for selected drive
                            if d == self.di() {
                                self.drive_audio.queue_event(
                                    self.audio_cycle,
                                    DriveEvent::Step { quarter_track: new_pos as u8 }
                                );
                            }
                            
                            if self.debug {
                                log::debug!("IWM: Drive {} head moved to {} (Delta: {})", d + 1, self.drives[d].head_pos, delta);
                            }
                        }
                    } else if new_pos < 0 && self.drives[d].head_pos > 0 {
                        // Trying to step below track 0
                        self.drives[d].head_pos = 0;
                        if self.debug {
                            log::debug!("IWM: Drive {} hit track 0 stop", d + 1);
                        }
                    } else if new_pos < 0 {
                        // Already at track 0, just clamp
                        self.drives[d].head_pos = 0;
                    }
                }
            }
        }
    }

    pub fn tick(&mut self, cycles: u64) {
        // Update audio cycle counter
        self.audio_cycle += cycles;

        if self.has_smartport_device() {
            self.smartport.tick(cycles);
        }

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
                // Queue motor off audio event
                self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOff);
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
            let track_num = (self.drives[d].head_pos / 4) as u16;

            if track_num < 35 && self.drives[d].loaded_track != Some(track_num as u8) {
                if self.drives[d].dirty {
                    self.flush_track(d);
                    self.save_disk(d);
                }

                if let Some((data, bit_count)) = self.load_track_data(d, track_num as u8) {
                    let bit_count = {
                        let woz_bc = if (track_num as usize) < self.drives[d].woz_bit_counts.len() {
                            self.drives[d].woz_bit_counts[track_num as usize] as usize
                        } else {
                            0
                        };
                        if woz_bc > 0 && woz_bc <= data.len() * 8 {
                            woz_bc
                        } else {
                            bit_count
                        }
                    };
                    self.drives[d].track_data = data;
                    self.drives[d].track_bit_count = bit_count;
                    self.drives[d].bit_index = 0;
                    self.drives[d].loaded_track = Some(track_num as u8);
                    self.drives[d].dirty = false;
                    self.drives[d].nibbles_valid = false;
                    if self.debug {
                        log::debug!("IWM: Drive {} loaded track {} (buf_len={}, bit_count={})",
                            d + 1, track_num, self.drives[d].track_data.len(), self.drives[d].track_bit_count);
                        // Dump first 32 bytes of track data for debugging
                        let dump_len = std::cmp::min(32, self.drives[d].track_data.len());
                        let hex: String = self.drives[d].track_data[..dump_len].iter()
                            .map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                        log::debug!("IWM: Track {} first {} bytes: {}", track_num, dump_len, hex);
                    }
                } else {
                    if self.debug {
                        let qt = (track_num * 4) as usize;
                        let tmap_idx = if qt < 160 { self.drives[d].woz_tmap[qt] } else { 0xFF };
                        log::debug!("IWM BOOT_DIAG: Track {} load FAILED! drive={} woz_format={:?} woz_raw_len={} tmap[{}]={:02X}",
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
            // Queue motor off audio event
            self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOff);
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

        // BIT-LEVEL PROCESSING
        // Process bits continuously as cycles elapse (4 cycles = 1 bit for 5.25" drives)
        // IWM spec: 4 CPU cycles per bit ≈ 3.92µs at effective clock
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
                    
                    // When MSB is set, we have a complete nibble: latch it
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

    fn load_track_data(&self, d: usize, track_num: u8) -> Option<(Vec<u8>, usize)> {
        self.load_track_from_raw(d, track_num).map(|data| {
            let bit_count = data.iter().rposition(|&b| b != 0)
                .map(|pos| (pos + 1) * 8)
                .unwrap_or(data.len() * 8);
            (data, bit_count)
        })
    }

    /// Load track data directly from woz_raw bytes
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
                            log::debug!("IWM: Flushed track {} to WOZ1 ({} bytes)", track_num, data_len);
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
                                log::debug!("IWM: Flushed track {} to WOZ2 ({} bytes)", track_num, data_len);
                            }
                        }
                    }
                },
                WozFormat::Unknown => {
                    log::warn!("IWM Error: Cannot flush track {} - unknown WOZ format", track_num);
                }
            }

            // Verify all sectors on the flushed track
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
                    log::warn!("IWM Error: Failed to save disk: {}", e);
                } else if self.debug {
                    log::debug!("IWM: Saved drive {} disk to {}", d + 1, path);
                }
            }
        }
        self.drives[d].last_save = Instant::now();
        self.drives[d].dirty = false;
    }

    fn disk_write_load(&mut self, val: u8) {
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

    fn smartport_write_load(&mut self, val: u8) {
        if !self.has_smartport_device() {
            return;
        }

        self.log_route_decision("smartport_write_load", 0xC0EF, true);
        self.latch = val;
        self.smartport_write_byte(val);
    }

    pub fn read_data(&mut self, floating_bus: u8, disk35_mode: bool) -> u8 {
        // SmartPort wire protocol: intercept reads when 3.5" disk is in external slot
        if self.is_smartport_data_active(disk35_mode) {
            self.log_route_decision("read_data", 0xC0EC, disk35_mode);
            return self.read_smartport_data(floating_bus);
        }
        
        let d = self.di();
        if !self.drives[d].has_disk() {
            // No disk: return floating bus value (video RAM data at current scan position).
            // Real hardware: read head picks up noise; floating bus is a reasonable approximation.
            // Bit 7 will randomly be set, allowing BPL loops to eventually exit.
            if self.debug { println!("IWM: read_data() NO DISK on drive {} -> floating_bus {:02X}", d + 1, floating_bus); }
            return floating_bus;
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
                    log::debug!("IWM BOOT_DIAG: read_data() with EMPTY track_data! drive={} head_pos={} loaded_track={:?} fast_disk={}",
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

    /// Handle IWM access in 3.5"/SmartPort mode ($C031 bit 6 = 1)
    /// In this mode, phase signals encode status queries and actions
    /// instead of stepper motor control.
    fn access_35(&mut self, addr: u16, val: u8, write: bool, floating_bus: u8) -> u8 {
        let loc = addr & 0xF;
        let on = (loc & 1) != 0;
        
        // Log 3.5" mode accesses (first time only per session to avoid spam)
        static LOGGED_35_ACCESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOGGED_35_ACCESS.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::debug!("*** IWM: Entering 3.5\" mode access (addr={:04X})", addr);
        }
        
        // Handle phase changes, in 3.5" mode these encode status/commands
        if loc < 8 {
            let phase = (loc >> 1) as u8;
            if on {
                self.phases |= 1 << phase;
            } else {
                self.phases &= !(1 << phase);
            }
            
            // Phase 0 = REQ signal for SmartPort wire protocol
            if phase == 0 && self.is_smartport_bootstrap_visible() {
                self.smartport.notify_req_change(on);
            }

            // SmartPort bus reset: ph0=1 ph2=1 (phases = 0x05)
            if self.is_smartport_bootstrap_visible() && (self.phases & 0x0F) == 0x05 {
                self.smartport.bus_reset();
            }
        } else {
            // Non-phase switches (motor, drive select, Q6/Q7) work the same
            match loc {
                0x8 => {
                    self.set_motor(false);
                    if self.motor_on35 {
                        self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOff35);
                    }
                    self.motor_on35 = false;  // Also update 3.5" motor state
                },
                0x9 => {
                    self.set_motor(true);
                    if !self.motor_on35 {
                        self.drive_audio.queue_event(self.audio_cycle, DriveEvent::MotorOn35);
                    }
                    self.motor_on35 = true;   // Also update 3.5" motor state
                },
                0xA => self.drive_select = false,
                0xB => self.drive_select = true,
                0xC => self.q6 = false,
                0xD => {
                    self.q6 = true;
                    if write && self.q7 {
                        if self.motor_on {
                            self.smartport_write_load(val);
                        } else {
                            self.mode = val;
                        }
                    }
                },
                0xE => {
                    self.write_mode = false;
                    self.q7 = false;
                },
                0xF => {
                    self.q7 = true;
                    if write && self.q6 {
                        if self.motor_on {
                            self.smartport_write_load(val);
                        } else {
                            self.mode = val;
                        }
                    }
                    self.write_mode = true;
                },
                _ => {}
            }
        }
        
        // Return value based on Q6/Q7/motor state
        if write {
            return 0;
        }
        
        if loc < 0xC {
            return floating_bus;
        }
        
        match (self.q7, self.q6) {
            (false, false) => {
                // SmartPort wire protocol: use same data path as 5.25" mode
                // which handles response bytes, cooldown after response,
                // and idle patterns between commands
                self.read_smartport_data(floating_bus)
            },
            (false, true) => {
                // Status register - bit 7 is SENSE input
                // During SmartPort wire protocol, SENSE reflects BSY from the device:
                //   BSY LOW (bit 7 = 0) after send = device acknowledged command
                //   BSY HIGH (bit 7 = 1) after ack = device ready to send response
                let bsy = if self.is_smartport_control_visible(true) {
                    self.smartport.get_bsy()
                } else {
                    true
                };
                (bsy as u8) << 7 | (self.motor_on as u8) << 5 | (self.mode & 0x1F)
            },
            (true, false) => {
                // Write handshake register: bit 7 = underrun (ready), bit 6 = data register full
                // SmartPort consumes bytes immediately, so always ready
                0x80  // bit 7=1 (ready), bit 6=0 (buffer empty)
            },
            (true, true) => {
                0xFF
            },
        }
    }

    pub fn access(&mut self, addr: u16, val: u8, write: bool, floating_bus: u8, disk35_mode: bool) -> u8 {
        self.cycles_since_last_read = 0;

        // Use disk35_mode only when explicitly enabled via $C031 bit 6.
        // Auto-detection was breaking normal 5.25" boot.
        let effective_disk35_mode = disk35_mode;

        if self.debug && (0xC0E0..=0xC0EF).contains(&addr) {
            self.log_route_decision(if write { "access-write" } else { "access-read" }, addr, effective_disk35_mode);
        }
        
        // In 3.5"/SmartPort mode, phase signals have different meanings
        if effective_disk35_mode {
            return self.access_35(addr, val, write, floating_bus);
        }

        match addr & 0xF {
            0x0 => {
                self.set_phase(0, false);
                // SmartPort: phase 0 = REQ line
                if self.is_smartport_bootstrap_visible() {
                    self.smartport.notify_req_change(false);
                }
            },
            0x1 => {
                self.set_phase(0, true);
                if self.is_smartport_bootstrap_visible() {
                    self.smartport.notify_req_change(true);
                }
            },
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
                            if self.is_smartport_write_routed(effective_disk35_mode) {
                                self.smartport_write_load(val);
                            } else {
                                self.disk_write_load(val);
                            }
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
                        if self.is_smartport_write_routed(effective_disk35_mode) {
                            self.smartport_write_load(val);
                        } else {
                            self.disk_write_load(val);
                        }
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
             log::debug!("IWM Write: {} ({:04X}) = {:02X}", reg_name, addr, val);
        }

        let result = if write {
            // Write register operations handled in match arms above:
            // 0xD/0xF with L6=1+L7=1: Mode Set (motor off) or Write Load (motor on)
            0
        } else if (addr & 0xF) < 0xC {
            // Addresses 0x0-0xB (phases, motor, drive select) don't return IWM data
            // Return floating bus value
            floating_bus
        } else {
            // Read register based on current L7, L6, Motor-On per IWM spec page 7:
            //   L7=0, L6=0, Motor=0  →  floating bus
            //   L7=0, L6=0, Motor=1  →  data register (Read)
            //   L7=0, L6=1, x        →  status register (Write-Protect Sense)
            //   L7=1, L6=0, x        →  write-handshake register (Write)
            //   L7=1, L6=1, Motor=0  →  (mode set state, not a normal read)
            //   L7=1, L6=1, Motor=1  →  (write load state, return buffer for verify)
            match (self.q7, self.q6) {
                (false, false) => {
                     if self.is_smartport_data_active(effective_disk35_mode) {
                         self.read_data(floating_bus, effective_disk35_mode)
                     } else if self.motor_on {
                         self.read_data(floating_bus, effective_disk35_mode)
                     } else {
                         floating_bus  // Motor off returns floating bus
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
                     // Bit 7: SENSE input / SmartPort BSY
                     //   - For a drive with a disk: reflects write-protect sensor (1 = protected)
                     //   - For SmartPort: bit 7 = BSY line state
                     //     BSY HIGH (1) = device idle/ready or ready to send response
                     //     BSY LOW  (0) = device acknowledges command / transfer complete
                     //
                     // ROM checks:
                     //   ubsy1:  BMI = wait for BSY HIGH (bit 7 = 1) before sending
                     //   sd9:    BMI → loop while HIGH, exits when LOW (command ack)
                     //   rdh1:   BPL → loop while LOW, exits when HIGH (ready to send)
                     //   rdh45:  BMI → loop while HIGH, exits when LOW (transfer done)
                    let smartport_active = self.is_smartport_control_visible(effective_disk35_mode);
                    if smartport_active {
                         let bsy = self.smartport.get_bsy();
                         if bsy {
                             status |= 0x80;
                         }
                     } else if !self.drives[d].has_disk() {
                         // No disk at all, signal "no device"
                         status |= 0x80;
                     } else if self.drives[d].write_protect {
                         // Has disk and write protected
                         status |= 0x80;
                     }
                     status
                 },
                 (true, false) => {
                     // Handshake register per IWM spec page 9:
                     // bits 0-5: ones (reserved)
                     // bit 6: 1 = write state active (0 if underrun)
                     // bit 7: 1 = ready to accept next byte from CPU
                     //
                     // SmartPort: ROM's sendbyte ($CA50) reads l6clr,X (Q6=0,Q7=1)
                     // and loops while bit 7 = 0. Return 0x80 = always ready.
                     if self.is_smartport_control_visible(effective_disk35_mode) {
                         0x80
                     } else {
                         let in_write_state = self.drives[d].was_writing;
                         let ready = self.drives[d].write_bits_left == 0 || !self.drives[d].write_data_pending;
                         let mut handshake: u8 = 0x00;
                         if in_write_state { handshake |= 0x40; }
                         if ready { handshake |= 0x80; }
                         handshake
                     }
                 },
                 (true, true) => {
                     // Write load/mode set state read, return buffer value
                     if self.debug { println!("IWM Read Write Buffer: {:02X}", self.latch); }
                     self.latch
                 }
            }
        };

        result
    }
}
