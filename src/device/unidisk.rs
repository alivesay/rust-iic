use super::drive_audio::DriveEvent;
use super::smartport::SmartPortDevice;

pub struct CommandResult {
    // SmartPort status byte (0x00 = OK, nonzero = error code)
    pub status: u8,
    // Response payload (after status byte).  For READ_BLOCK this is 512
    // bytes of block data; for most other commands it is a short status.
    pub payload: Vec<u8>,
    // Drive-audio events to queue (motor, step, eject, etc.)
    pub audio_events: Vec<DriveEvent>,
}

impl CommandResult {
    fn ok(payload: &[u8]) -> Self {
        Self { status: 0x00, payload: payload.to_vec(), audio_events: Vec::new() }
    }

    fn ok_with_audio(payload: &[u8], events: Vec<DriveEvent>) -> Self {
        Self { status: 0x00, payload: payload.to_vec(), audio_events: events }
    }

    fn error(code: u8) -> Self {
        Self { status: code, payload: vec![code], audio_events: Vec::new() }
    }
}

// UniDisk 3.5 physical drive emulation.
//
// Wraps a `SmartPortDevice` (block I/O backend) with physical drive state
// that is meaningful for a 3.5" floppy: motor, head, track position, and
// write-protect.  Provides `execute()` to handle decoded SmartPort commands.
pub struct UniDisk35 {
    // Block I/O backend (the disk image file)
    pub device: SmartPortDevice,

    // -- physical drive state --
    // Motor spinning
    pub motor_on: bool,
    // Current quarter-track position (0–319 for 80 tracks × 2 sides × 2)
    pub cur_qtr_track: u16,
    // Total quarter-tracks (usually 320 for an 800K disk)
    pub num_qtr_tracks: u16,
    // Disk-switched flag (cleared by OS after acknowledging swap)
    pub just_ejected: bool,
}

impl UniDisk35 {
    fn status_info_byte(&self) -> u8 {
        let mut info = 0x80 | 0x20 | 0x08;

        if self.has_disk() {
            info |= 0x10;
            if !self.device.write_protected {
                info |= 0x40;
            } else {
                info |= 0x04;
            }
        }

        info
    }

    fn standard_status_payload(&self) -> [u8; 4] {
        let blocks = self.device.block_count;
        [
            self.status_info_byte(),
            (blocks & 0xFF) as u8,
            ((blocks >> 8) & 0xFF) as u8,
            ((blocks >> 16) & 0xFF) as u8,
        ]
    }

    fn dib_status_payload(&self) -> [u8; 25] {
        let blocks = self.device.block_count;
        let mut payload = [0u8; 25];

        payload[0] = self.status_info_byte();
        payload[1] = (blocks & 0xFF) as u8;
        payload[2] = ((blocks >> 8) & 0xFF) as u8;
        payload[3] = ((blocks >> 16) & 0xFF) as u8;
        payload[4] = 11;
        payload[5..16].copy_from_slice(b"UniDisk 3.5");
        payload[21] = 0x02;
        payload[22] = 0x00;
        payload[23] = 0x01;
        payload[24] = 0x00;

        payload
    }

    pub fn new() -> Self {
        Self {
            device: SmartPortDevice::new(),
            motor_on: false,
            cur_qtr_track: 0,
            num_qtr_tracks: 0,
            just_ejected: false,
        }
    }

    // Load a 3.5" disk image (.po / 2IMG)
    pub fn load_disk(&mut self, path: &str) -> Result<(), String> {
        self.device.load_disk_image(path)?;
        self.num_qtr_tracks = 320; // 80 tracks × 2 sides × 2
        self.cur_qtr_track = 0;
        self.just_ejected = false;
        Ok(())
    }

    // Is a disk loaded?
    pub fn has_disk(&self) -> bool {
        self.device.has_disk()
    }

    // UI-facing status triple: (has_disk, motor_on, write_protected)
    pub fn drive_status(&self) -> (bool, bool, bool) {
        (self.has_disk(), self.motor_on, self.device.write_protected)
    }

    pub fn toggle_write_protect(&mut self) {
        self.device.write_protected = !self.device.write_protected;
    }

    pub fn eject(&mut self) {
        self.just_ejected = true;
        self.motor_on = false;
        self.device = SmartPortDevice::new();
        self.num_qtr_tracks = 0;
        self.cur_qtr_track = 0;
    }

    // Execute a decoded SmartPort command.
    //
    // `cmd`     -> command byte ($00=STATUS .. $05=INIT)
    // `decoded` -> full decoded payload (cmd, unit, params…)
    //
    // Returns a `CommandResult` that the bus controller will encode into
    // a wire-protocol response packet.
    pub fn execute(&mut self, cmd: u8, decoded: &[u8]) -> CommandResult {
        match cmd {
            0x00 => self.cmd_status(decoded),
            0x01 => self.cmd_read_block(decoded),
            0x02 => self.cmd_write_block(decoded),
            0x03 => self.cmd_format(),
            0x04 => self.cmd_control(decoded),
            0x05 => CommandResult::ok(&[0x00]),   // INIT, just ACK
            _ => {
                log::debug!("UniDisk35: unknown cmd {:02X}", cmd);
                CommandResult::error(0x21)
            }
        }
    }

    fn cmd_status(&mut self, decoded: &[u8]) -> CommandResult {
        // Status code is at decoded[4] (after cmd, unit, list_ptr_lo, list_ptr_hi)
        let code = if decoded.len() > 4 { decoded[4] } else { 0 };

        match code {
            0x00 => {
                let payload = self.standard_status_payload();
                CommandResult::ok(&payload)
            }
            0x03 => {
                let payload = self.dib_status_payload();
                CommandResult::ok(&payload)
            }
            _ => {
                log::debug!("UniDisk35: STATUS code {:02X} not implemented", code);
                CommandResult::error(0x21)
            }
        }
    }

    fn cmd_read_block(&mut self, decoded: &[u8]) -> CommandResult {
        let block = if decoded.len() >= 7 {
            (decoded[4] as u32) | ((decoded[5] as u32) << 8) | ((decoded[6] as u32) << 16)
        } else {
            0
        };

        // Spin up motor if needed
        let mut audio = Vec::new();
        if !self.motor_on {
            self.motor_on = true;
            audio.push(DriveEvent::MotorOn35);
        }
        audio.push(DriveEvent::Step35);

        let mut buf = [0u8; 512];
        match self.device.read_block(block, &mut buf) {
            Ok(()) => {
                log::debug!("UniDisk35: READ_BLOCK #{} OK", block);
                CommandResult::ok_with_audio(&buf, audio)
            }
            Err(e) => {
                log::warn!("UniDisk35: READ_BLOCK #{} error: {}", block, e);
                CommandResult::error(0x27) // I/O error
            }
        }
    }

    fn cmd_write_block(&mut self, decoded: &[u8]) -> CommandResult {
        let block = if decoded.len() >= 7 {
            (decoded[4] as u32) | ((decoded[5] as u32) << 8) | ((decoded[6] as u32) << 16)
        } else {
            0
        };

        if decoded.len() < 7 + 512 {
            log::warn!("UniDisk35: WRITE_BLOCK short payload ({} bytes)", decoded.len());
            return CommandResult::error(0x27);
        }

        let mut audio = Vec::new();
        if !self.motor_on {
            self.motor_on = true;
            audio.push(DriveEvent::MotorOn35);
        }
        audio.push(DriveEvent::Step35);

        let mut buf = [0u8; 512];
        buf.copy_from_slice(&decoded[7..7 + 512]);
        match self.device.write_block(block, &buf) {
            Ok(()) => {
                log::debug!("UniDisk35: WRITE_BLOCK #{} OK", block);
                CommandResult::ok_with_audio(&[0x00], audio)
            }
            Err(e) => {
                log::warn!("UniDisk35: WRITE_BLOCK #{} error: {}", block, e);
                CommandResult::error(0x27)
            }
        }
    }

    fn cmd_format(&mut self) -> CommandResult {
        log::debug!("UniDisk35: FORMAT (no-op)");
        CommandResult::ok(&[0x00])
    }

    fn cmd_control(&mut self, decoded: &[u8]) -> CommandResult {
        let code = if decoded.len() > 4 { decoded[4] } else { 0 };
        log::debug!("UniDisk35: CONTROL code={:02X}", code);
        match code {
            0x00 => {
                self.just_ejected = false;
                CommandResult::ok(&[0x00])
            }
            0x04 => {
                self.eject();
                CommandResult::ok_with_audio(&[0x00], vec![DriveEvent::Eject35])
            }
            _ => CommandResult::ok(&[0x00]),
        }
    }
}

impl Default for UniDisk35 {
    fn default() -> Self { Self::new() }
}
