// SmartPort Bus Controller for Apple IIc
//
// This module implements:
// - `SmartPortDevice` — block I/O backend (HDV, .po, 2IMG files)
// - `SmartPort`       — bus controller: wire protocol state machine,
//   7-bit packet encode/decode, command dispatch, and device chain management
//
// Device chain (unit numbers are 1-based):
//   devices[0] = 3.5" floppy (unit 1)  — loaded via `load_disk()`
//   devices[1] = hard drive 1 (unit 2) — loaded via `load_hdv()`
//   devices[2] = hard drive 2 (unit 3) — loaded via `load_hdv()`
//
// SmartPort Commands:
// $00 - STATUS      : Get device status/info
// $01 - READ_BLOCK  : Read a 512-byte block
// $02 - WRITE_BLOCK : Write a 512-byte block
// $03 - FORMAT      : Format device
// $04 - CONTROL     : Device-specific control/eject
// $05 - INIT        : Initialize device
// $06 - OPEN        : Open (character devices only)
// $07 - CLOSE       : Close (character devices only)
// $08 - READ        : Read bytes (character devices)
// $09 - WRITE       : Write bytes (character devices)

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::drive_audio::DriveEvent;
use super::unidisk::UniDisk35;

// Block size for ProDOS/SmartPort devices
pub const BLOCK_SIZE: usize = 512;

// Maximum supported blocks (32MB limit for ProDOS)
pub const MAX_BLOCKS: u32 = 65535;

// SmartPort block device (hard drive image)
pub struct SmartPortDevice {
    // Path to the image file
    pub path: String,
    // File handle (None if not loaded)
    file: Option<File>,
    // Byte offset to the start of block data (e.g. 64 for 2IMG header)
    data_offset: u64,
    // Total number of blocks
    pub block_count: u32,
    // Whether the device is write-protected
    pub write_protected: bool,
    // Whether the device is enabled/present
    pub enabled: bool,
    // Dirty blocks needing flush (for write caching)
    dirty: bool,
    // Debug logging
    pub debug: bool,
}

impl Default for SmartPortDevice {
    fn default() -> Self {
        Self {
            path: String::new(),
            file: None,
            data_offset: 0,
            block_count: 0,
            write_protected: false,
            enabled: false,
            dirty: false,
            debug: false,
        }
    }
}

impl SmartPortDevice {
    pub fn new() -> Self {
        Self::default()
    }

    // Load an HDV file as a hard drive image
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        
        // Open file for read/write
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Failed to open HDV file '{}': {}", path_str, e))?;

        // Get file size and calculate block count
        let metadata = file.metadata()
            .map_err(|e| format!("Failed to get HDV file metadata: {}", e))?;
        
        let file_size = metadata.len();
        if file_size == 0 {
            return Err("HDV file is empty".to_string());
        }
        
        if file_size % BLOCK_SIZE as u64 != 0 {
            log::warn!("HDV file size {} is not a multiple of block size {}", file_size, BLOCK_SIZE);
        }

        let block_count = (file_size / BLOCK_SIZE as u64) as u32;
        if block_count > MAX_BLOCKS {
            return Err(format!("HDV file too large: {} blocks (max {})", block_count, MAX_BLOCKS));
        }

        // Check if file is read-only
        let write_protected = metadata.permissions().readonly();

        self.path = path_str.clone();
        self.file = Some(file);
        self.block_count = block_count;
        self.write_protected = write_protected;
        self.enabled = true;
        self.dirty = false;

        log::info!("Loaded HDV: {} ({} blocks, {} MB{})", 
            path_str,
            block_count,
            (block_count as u64 * BLOCK_SIZE as u64) / (1024 * 1024),
            if write_protected { ", read-only" } else { "" }
        );

        Ok(())
    }

    // Read a block from the device
    pub fn read_block(&mut self, block: u32, buffer: &mut [u8; BLOCK_SIZE]) -> Result<(), String> {
        if !self.enabled {
            return Err("Device not ready".to_string());
        }
        
        if block >= self.block_count {
            return Err(format!("Block {} out of range (max {})", block, self.block_count - 1));
        }

        let file = self.file.as_mut().ok_or("No file loaded")?;
        let offset = self.data_offset + block as u64 * BLOCK_SIZE as u64;
        
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("Seek error: {}", e))?;
        
        file.read_exact(buffer)
            .map_err(|e| format!("Read error at block {}: {}", block, e))?;

        if self.debug {
            log::debug!("SmartPort: Read block {} (offset 0x{:X})", block, offset);
        }

        Ok(())
    }

    // Write a block to the device
    pub fn write_block(&mut self, block: u32, buffer: &[u8; BLOCK_SIZE]) -> Result<(), String> {
        if !self.enabled {
            return Err("Device not ready".to_string());
        }
        
        if self.write_protected {
            return Err("Device is write-protected".to_string());
        }
        
        if block >= self.block_count {
            return Err(format!("Block {} out of range (max {})", block, self.block_count - 1));
        }

        let file = self.file.as_mut().ok_or("No file loaded")?;
        let offset = self.data_offset + block as u64 * BLOCK_SIZE as u64;
        
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("Seek error: {}", e))?;
        
        file.write_all(buffer)
            .map_err(|e| format!("Write error at block {}: {}", block, e))?;

        self.dirty = true;

        if self.debug {
            log::debug!("SmartPort: Write block {} (offset 0x{:X})", block, offset);
        }

        Ok(())
    }

    // Flush any pending writes to disk
    pub fn flush(&mut self) -> Result<(), String> {
        if self.dirty {
            if let Some(file) = self.file.as_mut() {
                file.flush()
                    .map_err(|e| format!("Flush error: {}", e))?;
                self.dirty = false;
            }
        }
        Ok(())
    }

    // Convenience alias: true when a disk/image is loaded with blocks available
    pub fn has_disk(&self) -> bool {
        self.enabled && self.block_count > 0
    }

    // Load a 3.5" disk image (.po raw or 2IMG with 64-byte header)
    pub fn load_disk_image<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Failed to open 3.5\" disk image '{}': {}", path_str, e))?;

        let metadata = file.metadata()
            .map_err(|e| format!("Failed to get file metadata: {}", e))?;

        let file_size = metadata.len();

        // 800K = 819200 bytes, 2IMG adds a 64-byte header
        let (data_offset, data_size) = if file_size == 819200 {
            (0u64, 819200u64)
        } else if file_size == 819200 + 64 {
            let mut header = [0u8; 64];
            file.read_exact(&mut header)
                .map_err(|e| format!("Failed to read 2IMG header: {}", e))?;
            if &header[0..4] != b"2IMG" {
                return Err("Invalid 2IMG header".to_string());
            }
            (64u64, 819200u64)
        } else if file_size >= 819200 {
            (0u64, 819200u64)
        } else {
            return Err(format!("Invalid 3.5\" disk image size: {} bytes", file_size));
        };

        if file_size < data_offset + data_size {
            return Err("Disk image too small".to_string());
        }

        let write_protected = metadata.permissions().readonly();

        self.path = path_str.clone();
        self.file = Some(file);
        self.data_offset = data_offset;
        self.block_count = (data_size / BLOCK_SIZE as u64) as u32;
        self.write_protected = write_protected;
        self.enabled = true;
        self.dirty = false;

        log::info!("Loaded 3.5\" disk: {} ({} blocks{})", 
            path_str, self.block_count,
            if data_offset > 0 { ", 2IMG" } else { "" });

        Ok(())
    }
}

impl Drop for SmartPortDevice {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            log::error!("Failed to flush SmartPort device on drop: {}", e);
        }
    }
}

// Maximum hard-drive (HDV) devices on the SmartPort chain
const MAX_HDV_DEVICES: usize = 2;
// Maximum 3.5" floppy drives on the SmartPort chain
const MAX_FLOPPY_DEVICES: usize = 2;
const SMARTPORT_RESPONSE_DELAY_CYCLES: u64 = 32;

// Wire protocol state machine
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProtocolState {
    WaitingForSync,
    ReceivingCommand,
    ResponsePending,
    SendingResponse,
    ResponseDone,
    Idle,
}

// SmartPort bus controller
//
// Owns the device chain and the wire protocol state machine.
// The IWM delegates SmartPort byte I/O here; this struct handles
// sync detection, packet decode, command dispatch, response encoding,
// and ACK handshaking.
pub struct SmartPort {
    // 3.5" floppy drives
    pub floppies: [UniDisk35; MAX_FLOPPY_DEVICES],
    // Hard-drive devices
    pub hdv_devices: [SmartPortDevice; MAX_HDV_DEVICES],

    // -- wire protocol state --
    state: ProtocolState,
    cmd_buffer: Vec<u8>,
    resp_buffer: Vec<u8>,
    resp_index: usize,
    sync_count: u8,
    header_count: u8,
    header: [u8; 7],
    bsy: bool,
    response_delay_cycles: u64,
    response_waiting_for_req_low: bool,
    req_high: bool,
    // Unit offset from INIT: the ROM's global unit number for the last
    // non-SmartPort device (e.g. 2 for two internal 5.25" drives).
    // Our local unit 1 (floppy) = ROM unit (unit_offset + 1).
    unit_offset: u8,
    // How many devices have been initialized via INIT commands
    devices_initialized: u8,
    // DEST from the current command (used as SRC in responses)
    current_dest: u8,
    cmd_count: u32,
    pub debug: bool,

    // Audio events pending delivery to the IWM/DriveAudio system.
    // Filled by command handlers, drained by the IWM after each command.
    pending_audio: Vec<DriveEvent>,
}

impl Default for SmartPort {
    fn default() -> Self { Self::new() }
}

impl SmartPort {
    fn make_response_ready(&mut self) {
        self.bsy = true;
        log::debug!(
            "SmartPort: BSY HIGH, response ready ({} bytes)",
            self.resp_buffer.len()
        );
    }

    fn maybe_start_sending_response(&mut self) {
        if self.state != ProtocolState::ResponsePending || !self.bsy || !self.req_high {
            return;
        }

        self.state = ProtocolState::SendingResponse;
        self.resp_index = 0;
        log::debug!(
            "SmartPort: REQ HIGH, sending response ({} bytes)",
            self.resp_buffer.len()
        );
    }

    pub fn new() -> Self {
        println!("disk  {:>12} {:>8}", "SMARTPORT", "ONLINE");
        Self {
            floppies: [UniDisk35::new(), UniDisk35::new()],
            hdv_devices: [SmartPortDevice::new(), SmartPortDevice::new()],
            state: ProtocolState::WaitingForSync,
            cmd_buffer: Vec::with_capacity(64),
            resp_buffer: Vec::with_capacity(1024),
            resp_index: 0,
            sync_count: 0,
            header_count: 0,
            header: [0; 7],
            bsy: true,
            response_delay_cycles: 0,
            response_waiting_for_req_low: false,
            req_high: true,
            unit_offset: 0,
            devices_initialized: 0,
            current_dest: 0,
            cmd_count: 0,
            debug: false,
            pending_audio: Vec::new(),
        }
    }

    // Load a 3.5" disk image into a specific floppy slot
    pub fn load_floppy(&mut self, slot: usize, path: &str) -> Result<(), String> {
        if slot >= MAX_FLOPPY_DEVICES {
            return Err(format!("Invalid floppy slot {} (max {})", slot, MAX_FLOPPY_DEVICES - 1));
        }
        self.floppies[slot].load_disk(path)
    }

    // Load an HDV hard-drive image into the next free slot (units 2+)
    pub fn load_hdv(&mut self, path: &str) -> Result<(), String> {
        for (i, dev) in self.hdv_devices.iter_mut().enumerate() {
            if !dev.has_disk() {
                dev.load(path)?;
                log::info!("HDV loaded as SmartPort unit {} (hdv_devices[{}])", i + 2, i);
                return Ok(());
            }
        }
        Err("No free HDV device slots".to_string())
    }

    // True if any device (floppy or HDV) is loaded
    pub fn has_any_device(&self) -> bool {
        self.floppies.iter().any(|f| f.has_disk()) || self.hdv_devices.iter().any(|d| d.has_disk())
    }

    // Number of active devices on the chain
    fn device_count(&self) -> u8 {
        let mut count: u8 = 0;
        for f in &self.floppies {
            if f.has_disk() { count += 1; }
        }
        for d in &self.hdv_devices {
            if d.has_disk() { count += 1; }
        }
        count
    }

    // Get bare SmartPortDevice by 1-based unit number.
    // Maps unit numbers to active (has_disk) devices dynamically:
    // floppies first (in order), then HDVs.
    fn get_device(&mut self, unit: u8) -> Option<&mut SmartPortDevice> {
        let mut current = 0u8;
        for i in 0..self.floppies.len() {
            if self.floppies[i].has_disk() {
                current += 1;
                if current == unit { return Some(&mut self.floppies[i].device); }
            }
        }
        for i in 0..self.hdv_devices.len() {
            if self.hdv_devices[i].has_disk() {
                current += 1;
                if current == unit { return Some(&mut self.hdv_devices[i]); }
            }
        }
        None
    }

    // Check if a 1-based unit number maps to a floppy drive.
    // Returns the floppy index if so.
    fn floppy_index_for_unit(&self, unit: u8) -> Option<usize> {
        let mut current = 0u8;
        for (i, f) in self.floppies.iter().enumerate() {
            if f.has_disk() {
                current += 1;
                if current == unit { return Some(i); }
            }
        }
        None
    }

    // Drain any pending drive-audio events (called by IWM after command processing)
    pub fn drain_audio_events(&mut self) -> Vec<DriveEvent> {
        std::mem::take(&mut self.pending_audio)
    }

    // Reset SmartPort bus state (called when phase lines signal bus reset: ph0+ph2)
    pub fn bus_reset(&mut self) {
        log::debug!("SmartPort: bus reset");
        self.state = ProtocolState::WaitingForSync;
        self.cmd_buffer.clear();
        self.resp_buffer.clear();
        self.resp_index = 0;
        self.sync_count = 0;
        self.header_count = 0;
        self.bsy = true;
        self.unit_offset = 0;
        self.devices_initialized = 0;
        self.current_dest = 0;
        self.response_delay_cycles = 0;
        self.response_waiting_for_req_low = false;
    }

    pub fn tick(&mut self, cycles: u64) {
        if self.state != ProtocolState::ResponsePending
            || self.response_waiting_for_req_low
            || self.response_delay_cycles == 0
        {
            return;
        }

        self.response_delay_cycles = self.response_delay_cycles.saturating_sub(cycles);
        if self.response_delay_cycles == 0 {
            self.make_response_ready();
            self.maybe_start_sending_response();
        }
    }

    pub fn is_wire_active(&self) -> bool {
        !matches!(self.state, ProtocolState::WaitingForSync | ProtocolState::Idle)
    }

    // Get BSY line state for the wire protocol handshake.
    // BSY LOW (false) = device acknowledges command receipt
    // BSY HIGH (true) = device has data / is idle
    pub fn get_bsy(&self) -> bool {
        self.bsy
    }

    // Notify SmartPort that REQ (phase 0) has changed.
    // This advances the handshake state machine:
    //   ResponsePending + REQ drops  → host has seen ACK-low; arm reply-ready
    //   ResponseDone    + REQ drops  → BSY stays LOW, return to idle
    pub fn notify_req_change(&mut self, req_high: bool) {
        self.req_high = req_high;

        if req_high {
            self.maybe_start_sending_response();
            return;
        }

        match self.state {
            ProtocolState::ResponsePending => {
                self.response_waiting_for_req_low = false;
                if self.response_delay_cycles > 0 {
                    return;
                }
                self.make_response_ready();
                self.maybe_start_sending_response();
            }
            ProtocolState::ResponseDone => {
                // Transfer complete
                self.bsy = true;
                self.state = ProtocolState::Idle;
                log::debug!("SmartPort: REQ dropped after response done, back to idle");
            }
            _ => {}
        }
    }

    pub fn take_response_done(&mut self) -> bool {
        if self.state == ProtocolState::ResponseDone {
            self.bsy = true; // Back to idle = BSY HIGH (ready for next command)
            self.state = ProtocolState::Idle;
            true
        } else {
            false
        }
    }

    // Process a byte written by the host (command direction)
    pub fn write_byte(&mut self, val: u8) {
        if self.debug {
            log::debug!("SP write_byte: val={:02X} state={:?} sync_count={} hdr_count={} buf_len={}", 
                val, self.state, self.sync_count, self.header_count, self.cmd_buffer.len());
        }
        match self.state {
            ProtocolState::WaitingForSync => self.handle_sync_byte(val),
            ProtocolState::ReceivingCommand => self.handle_command_byte(val),
            ProtocolState::ResponsePending
            | ProtocolState::SendingResponse
            | ProtocolState::ResponseDone => {
                // Track sync in case ROM times out and starts a new command
                if val == 0xFF {
                    if self.sync_count == 0 { self.sync_count = 1; }
                } else if matches!(val, 0x3F | 0xCF | 0xF3 | 0xFC) && self.sync_count >= 1 {
                    self.sync_count += 1;
                } else if val == 0xC3 && self.sync_count >= 3 {
                    log::debug!("SmartPort: New command during response (ROM timeout?)");
                    self.state = ProtocolState::ReceivingCommand;
                    self.cmd_buffer.clear();
                    self.header_count = 0;
                    self.sync_count = 0;
                } else {
                    self.sync_count = 0;
                }
            }
            ProtocolState::Idle => {
                self.state = ProtocolState::WaitingForSync;
                self.handle_sync_byte(val);
            }
        }
    }

    // Read the next response byte (device→host direction)
    pub fn read_byte(&mut self) -> Option<u8> {
        match self.state {
            ProtocolState::SendingResponse => self.read_next_response_byte(),
            _ => None,
        }
    }

    fn read_next_response_byte(&mut self) -> Option<u8> {
        if self.resp_index < self.resp_buffer.len() {
            let byte = self.resp_buffer[self.resp_index];
            self.resp_index += 1;

            if self.resp_index <= 30 || self.resp_index > self.resp_buffer.len() - 10 {
                log::debug!("SmartPort read [{}]: {:02X}", self.resp_index - 1, byte);
            }

            if self.resp_index >= self.resp_buffer.len() {
                log::debug!("SmartPort: Response fully read ({} bytes)", self.resp_buffer.len());
                self.bsy = false; // BSY LOW signals transfer complete
                self.state = ProtocolState::ResponseDone;
            }
            Some(byte)
        } else {
            log::debug!("SmartPort: read_next but buffer exhausted! idx={} len={}",
                self.resp_index, self.resp_buffer.len());
            None
        }
    }

    fn handle_sync_byte(&mut self, val: u8) {
        if val == 0xFF {
            // 0xFF is a self-sync byte. Start sync if not started,
            // but do NOT reset count if already in the middle of the
            // sync header (3F CF F3 FC). The ROM sends an extra 0xFF
            // between the header and the C3 packet-begin marker.
            if self.sync_count == 0 {
                self.sync_count = 1;
            }
            // If already >= 1, keep current count (don't reset)
        } else if matches!(val, 0x3F | 0xCF | 0xF3 | 0xFC) {
            if self.sync_count >= 1 { self.sync_count += 1; }
        } else if val == 0xC3 && self.sync_count >= 3 {
            self.state = ProtocolState::ReceivingCommand;
            self.cmd_buffer.clear();
            self.header_count = 0;
            self.sync_count = 0;
        } else if val >= 0x80 && self.sync_count >= 3 {
            self.state = ProtocolState::ReceivingCommand;
            self.cmd_buffer.clear();
            self.cmd_buffer.push(val);
            self.header_count = 1;
            self.header[0] = val & 0x7F;
            self.sync_count = 0;
        } else {
            self.sync_count = 0;
        }
    }

    fn handle_command_byte(&mut self, val: u8) {
        self.cmd_buffer.push(val);
        if self.header_count < 7 {
            self.header[self.header_count as usize] = val & 0x7F;
            self.header_count += 1;
        } else if self.cmd_buffer.len() >= self.expected_command_len() || self.cmd_buffer.len() > 64 {
            self.try_process_command();
        }
    }

    fn expected_command_len(&self) -> usize {
        if self.header_count < 7 {
            return usize::MAX;
        }

        let odd_cnt = self.header[5] as usize;
        let grp_cnt = self.header[6] as usize;
        let odd_section_len = if odd_cnt > 0 { 1 + odd_cnt } else { 0 };
        let group_section_len = grp_cnt * 8;

        // cmd_buffer starts after the C3 packet-begin byte and contains:
        //   7-byte header + encoded payload + 2 checksum bytes + 1 end byte.
        7 + odd_section_len + group_section_len + 3
    }

    fn try_process_command(&mut self) {
        if self.header_count < 7 { return; }

        let dest = self.header[0];
        let mut decoded = self.decode_payload();
        let cmd  = decoded.first().copied().unwrap_or(0);
        let raw_unit = if decoded.len() > 1 { decoded[1] } else { 1 };

        // dest=0 → broadcast (STATUS unit=0 device-count query)
        // dest=1..N → addressed to a specific device in our chain
        let max_dest = self.device_count();

        self.current_dest = dest;

        if dest > max_dest {
            self.state = ProtocolState::Idle;
            if self.debug {
                log::debug!("SmartPort: Ignoring cmd {:02X} dest={} (we have {} devices)",
                    cmd, dest, max_dest);
            }
            return;
        }

        self.cmd_count = self.cmd_count.wrapping_add(1);

        // Handle INIT first, sets unit_offset
        if cmd == 0x05 {
            self.handle_init(&decoded);
            return;
        }

        // Remap unit: Apple IIc ROM sends global unit numbers where
        // units 1..unit_offset are internal 5.25" drives.
        // Our local unit 1 (floppy) = ROM unit (unit_offset + 1).
        let unit = if self.unit_offset > 0 && raw_unit > 0 {
            raw_unit.saturating_sub(self.unit_offset)
        } else {
            raw_unit
        };

        // Update decoded payload so handlers see the remapped unit
        if decoded.len() > 1 {
            decoded[1] = unit;
        }

        log::debug!("SmartPort: CMD={:02X} dest={} raw_unit={} local_unit={} (devices={})",
            cmd, dest, raw_unit, unit, max_dest);

        // Floppy drives get priority unit numbering;
        // check if this unit maps to a floppy device.
        let floppy_idx = self.floppy_index_for_unit(unit);
        if floppy_idx.is_some() || (unit == 0 && cmd == 0x00) {
            if let Some(idx) = floppy_idx {
                let _block = if decoded.len() >= 7 {
                    (decoded[4] as u32) | ((decoded[5] as u32) << 8) | ((decoded[6] as u32) << 16)
                } else { 0 };
                
                let result = self.floppies[idx].execute(cmd, &decoded);

                self.pending_audio.extend(result.audio_events);
                self.build_response(0x00, dest, if cmd == 0x01 { 0x02 } else { 0x01 }, result.status, &result.payload);
                return;
            }
            // unit=0 STATUS: device count query, handle below
        }

        // Unit 0 STATUS (device count), INIT, or HDV units
        match cmd {
            0x00 => self.handle_status(&decoded),
            0x01 => self.handle_read_block(&decoded),
            0x02 => self.handle_write_block(&decoded),
            0x03 => self.handle_format(&decoded),
            0x04 => self.handle_control(&decoded),
            _ => {
                log::debug!("SmartPort: Unknown cmd {:02X}", cmd);
                self.generate_error_response(0x21);
            }
        }
    }

    fn handle_status(&mut self, decoded: &[u8]) {
        let unit = if decoded.len() > 1 { decoded[1] } else { 0 };
        let code = if decoded.len() > 4 { decoded[4] } else { 0 };
        log::debug!("SmartPort: STATUS unit={} code={:02X}", unit, code);

        if unit == 0 && code == 0 {
            self.generate_device_count_response();
        } else {
            self.generate_status_response_for_unit(unit);
        }
    }

    fn handle_read_block(&mut self, decoded: &[u8]) {
        let unit = if decoded.len() > 1 { decoded[1] } else { 1 };
        let block = if decoded.len() >= 7 {
            (decoded[4] as u32) | ((decoded[5] as u32) << 8) | ((decoded[6] as u32) << 16)
        } else { 0 };

        // Units 2+ are HDV (get_device handles mapping)
        if let Some(dev) = self.get_device(unit) {
            let mut payload = [0u8; 512];
            match dev.read_block(block, &mut payload) {
                Ok(()) => {
                    self.build_response(0x00, 0x01, 0x02, 0x00, &payload);
                    // log::debug!("SmartPort: READ_BLOCK #{} unit={} OK", block, unit);
                }
                Err(_e) => {
                    // log::warn!("SmartPort: READ_BLOCK #{} unit={} error: {}", block, unit, e);
                    self.generate_error_response(0x27);
                }
            }
        } else {
            // log::warn!("SmartPort: READ_BLOCK to invalid unit {}", unit);
            self.generate_error_response(0x28);
        }
    }

    fn handle_write_block(&mut self, decoded: &[u8]) {
        let unit = if decoded.len() > 1 { decoded[1] } else { 1 };
        let block = if decoded.len() >= 7 {
            (decoded[4] as u32) | ((decoded[5] as u32) << 8) | ((decoded[6] as u32) << 16)
        } else { 0 };

        if decoded.len() >= 7 + 512 {
            let mut buf = [0u8; 512];
            buf.copy_from_slice(&decoded[7..7 + 512]);
            if let Some(dev) = self.get_device(unit) {
                match dev.write_block(block, &buf) {
                    Ok(()) => {
                        log::debug!("SmartPort: WRITE_BLOCK #{} unit={} OK", block, unit);
                        self.generate_success_response();
                    }
                    Err(e) => {
                        log::warn!("SmartPort: WRITE_BLOCK #{} unit={} error: {}", block, unit, e);
                        self.generate_error_response(0x27);
                    }
                }
            } else {
                log::warn!("SmartPort: WRITE_BLOCK to invalid unit {}", unit);
                self.generate_error_response(0x28);
            }
        } else {
            log::warn!("SmartPort: WRITE_BLOCK unit={} short payload ({} bytes)", unit, decoded.len());
            self.generate_error_response(0x27);
        }
    }

    fn handle_init(&mut self, decoded: &[u8]) {
        let raw_unit = if decoded.len() > 1 { decoded[1] } else { 0 };
        log::debug!("SmartPort: INIT raw_unit={} (current offset={}, initialized={})",
            raw_unit, self.unit_offset, self.devices_initialized);
        if self.unit_offset == 0 && raw_unit > 0 {
            self.unit_offset = raw_unit;
        }
        self.devices_initialized += 1;
        let is_last = self.devices_initialized >= self.device_count();
        self.generate_init_response(raw_unit, is_last);
    }

    fn handle_format(&mut self, decoded: &[u8]) {
        let unit = if decoded.len() > 1 { decoded[1] } else { 1 };
        log::debug!("SmartPort: FORMAT unit={}", unit);

        if self.get_device(unit).is_some() {
            self.generate_success_response();
        } else {
            log::warn!("SmartPort: FORMAT for invalid unit {}", unit);
            self.generate_error_response(0x28); // NoDrive
        }
    }

    fn handle_control(&mut self, decoded: &[u8]) {
        let unit = if decoded.len() > 1 { decoded[1] } else { 1 };
        let code = if decoded.len() > 4 { decoded[4] } else { 0 };
        log::debug!("SmartPort: CONTROL unit={} code={:02X}", unit, code);

        match code {
            0x00 => {
                self.generate_success_response();
            }
            _ => {
                log::debug!("SmartPort: CONTROL code {:02X} not supported", code);
                self.generate_error_response(0x21); // BadCtl
            }
        }
    }

    fn decode_payload(&self) -> Vec<u8> {
        if self.cmd_buffer.len() < 8 { return Vec::new(); }

        let odd_cnt  = self.header[5] as usize;
        let even_cnt = self.header[6] as usize;
        let data = &self.cmd_buffer[7..];
        let mut decoded = Vec::new();
        let mut pos = 0;

        // Odd section
        if odd_cnt > 0 && pos < data.len() {
            let prefix = data[pos]; pos += 1;
            for i in 0..odd_cnt {
                if pos < data.len() {
                    let wire = data[pos]; pos += 1;
                    let msb = if (prefix >> (6 - i)) & 1 != 0 { 0x80 } else { 0 };
                    decoded.push((wire & 0x7F) | msb);
                }
            }
        }

        // Groups of 7
        for _ in 0..even_cnt {
            if pos >= data.len() { break; }
            let prefix = data[pos]; pos += 1;
            for i in 0..7 {
                if pos < data.len() {
                    let wire = data[pos]; pos += 1;
                    let msb = if (prefix >> (6 - i)) & 1 != 0 { 0x80 } else { 0 };
                    decoded.push((wire & 0x7F) | msb);
                }
            }
        }
        decoded
    }

    fn decode_wire_payload_section(encoded: &[u8], odd_cnt: usize, group_cnt: usize) -> Vec<u8> {
        let mut decoded = Vec::with_capacity(odd_cnt + group_cnt * 7);
        let mut pos = 0;

        if odd_cnt > 0 && pos < encoded.len() {
            let prefix = encoded[pos] & 0x7F;
            pos += 1;
            for i in 0..odd_cnt {
                if pos >= encoded.len() {
                    break;
                }
                let wire = encoded[pos] & 0x7F;
                pos += 1;
                let msb = if (prefix >> (6 - i)) & 1 != 0 { 0x80 } else { 0 };
                decoded.push(wire | msb);
            }
        }

        for _ in 0..group_cnt {
            if pos >= encoded.len() {
                break;
            }
            let prefix = encoded[pos] & 0x7F;
            pos += 1;
            for i in 0..7 {
                if pos >= encoded.len() {
                    break;
                }
                let wire = encoded[pos] & 0x7F;
                pos += 1;
                let msb = if (prefix >> (6 - i)) & 1 != 0 { 0x80 } else { 0 };
                decoded.push(wire | msb);
            }
        }

        decoded
    }

    fn build_response(&mut self, dest: u8, src: u8, pkt_type: u8, status: u8, payload: &[u8]) {
        self.resp_buffer.clear();
        self.resp_index = 0;

        // Sync pattern (matches SmartportSD: FF 3F CF F3 FC FF then C3)
        self.resp_buffer.extend_from_slice(&[0xFF, 0x3F, 0xCF, 0xF3, 0xFC, 0xFF]);
        self.resp_buffer.push(0xC3);

        let odd_bytes = payload.len() % 7;
        let grp_count = payload.len() / 7;

        // Header (7 bytes, each | 0x80)
        let header = [dest, src, pkt_type, 0x00, status, odd_bytes as u8, grp_count as u8];
        for &b in &header { self.resp_buffer.push(b | 0x80); }

        // Odd section
        if odd_bytes > 0 {
            let mut topbits = 0x80u8;
            for i in 0..odd_bytes {
                if payload[i] & 0x80 != 0 { topbits |= 0x40 >> i; }
            }
            self.resp_buffer.push(topbits);
            for i in 0..odd_bytes { self.resp_buffer.push(payload[i] | 0x80); }
        }

        // Groups of 7
        for g in 0..grp_count {
            let base = odd_bytes + g * 7;
            let mut topbits = 0x80u8;
            for i in 0..7 {
                if payload[base + i] & 0x80 != 0 { topbits |= 0x40 >> i; }
            }
            self.resp_buffer.push(topbits);
            for i in 0..7 { self.resp_buffer.push(payload[base + i] | 0x80); }
        }

        // Checksum: XOR all raw payload bytes, then XOR wire header bytes.
        // Reference: SmartportSD encode_data_packet by Robert Justice
        let mut checksum: u8 = 0;
        for &b in payload { checksum ^= b; }
        for &b in &header { checksum ^= b | 0x80; }
        let chk_even = checksum | 0xAA;
        let chk_odd  = (checksum >> 1) | 0xAA;
        log::debug!("SmartPort: checksum {:02X} -> {:02X} {:02X}", checksum, chk_even, chk_odd);
        self.resp_buffer.push(chk_even);
        self.resp_buffer.push(chk_odd);
        self.resp_buffer.push(0xC8); // packet end

        // Always verify encoding for data packets (512-byte payloads)
        if payload.len() >= 512 || self.debug {
            let encoded_start = 14; // 7 sync + 7 header
            let encoded_end = self.resp_buffer.len().saturating_sub(3);
            let decoded = Self::decode_wire_payload_section(
                &self.resp_buffer[encoded_start..encoded_end],
                odd_bytes,
                grp_count,
            );
            if decoded != payload {
                let mismatch = decoded
                    .iter()
                    .zip(payload.iter())
                    .position(|(decoded_byte, payload_byte)| decoded_byte != payload_byte)
                    .unwrap_or_else(|| decoded.len().min(payload.len()));
                eprintln!(
                    "SmartPort: ENCODING MISMATCH at byte {} decoded={:02X?} expected={:02X?} payload_len={} decoded_len={}",
                    mismatch,
                    decoded.get(mismatch).copied(),
                    payload.get(mismatch).copied(),
                    payload.len(),
                    decoded.len(),
                );
            }
        }

        self.bsy = false; // BSY LOW signals command acknowledged
        self.response_delay_cycles = SMARTPORT_RESPONSE_DELAY_CYCLES;
        self.response_waiting_for_req_low = true;
        self.state = ProtocolState::ResponsePending;
    }

    fn generate_device_count_response(&mut self) {
        let count = self.device_count();
        let payload = [count, 0x00, 0x00, 0x00];
        self.build_response(0x00, self.current_dest, 0x01, 0x00, &payload);
        log::debug!("SmartPort: device count = {}", count);
    }

    fn generate_status_response_for_unit(&mut self, unit: u8) {
        if let Some(dev) = self.get_device(unit) {
            let blocks = if dev.has_disk() { dev.block_count } else { 0 };
            // General status byte: bit7=block, bit6=write, bit5=read,
            // bit4=online, bit3=format (same as SmartportSD 0xF8)
            let info: u8 = 0xF8;
            let payload = [
                info,
                (blocks & 0xFF) as u8,
                ((blocks >> 8) & 0xFF) as u8,
                ((blocks >> 16) & 0xFF) as u8,
            ];
            self.build_response(0x00, self.current_dest, 0x01, 0x00, &payload);
            log::debug!("SmartPort: STATUS unit={} blocks={} info=${:02X}", unit, blocks, info);
        } else {
            log::warn!("SmartPort: STATUS for invalid unit {}", unit);
            self.generate_error_response(0x28);
        }
    }

    fn generate_init_response(&mut self, unit: u8, is_last: bool) {
        let payload = [0x00];
        // SmartportSD: status=0x00 for "more devices", 0x7F for "last device"
        let status = if is_last { 0x7F } else { 0x00 };
        self.build_response(0x00, unit, 0x01, status, &payload);
        log::debug!("SmartPort: INIT response unit={} is_last={} ({} bytes)", unit, is_last, self.resp_buffer.len());
    }

    fn generate_success_response(&mut self) {
        self.build_response(0x00, self.current_dest, 0x01, 0x00, &[0x00]);
    }

    fn generate_error_response(&mut self, code: u8) {
        self.build_response(0x00, self.current_dest, 0x01, code, &[code]);
    }

    pub fn flush_all(&mut self) {
        for (i, floppy) in self.floppies.iter_mut().enumerate() {
            if let Err(e) = floppy.device.flush() {
                log::error!("Failed to flush SmartPort floppy {}: {}", i, e);
            }
        }
        for device in &mut self.hdv_devices {
            if let Err(e) = device.flush() {
                log::error!("Failed to flush SmartPort HDV device: {}", e);
            }
        }
    }
}