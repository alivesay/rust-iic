//! SmartPort / Hard Drive Emulation for Apple IIc
//!
//! Provides support for HDV (raw ProDOS block device) images as virtual hard drives.
//! 
//! SmartPort is Apple's block device protocol used by:
//! - UniDisk 3.5 (800K)
//! - Hard drives (SCSI, IDE via adapters)
//! - RAM disks
//! - Network drives
//!
//! HDV files are raw 512-byte block images. Block N starts at offset N * 512.
//!
//! Implementation approach:
//! We hook the ProDOS MLI entry point and intercept READ_BLOCK/WRITE_BLOCK calls
//! for our virtual hard drive unit. This is cleaner than full SmartPort hardware
//! emulation and works with all ProDOS software.
//!
//! SmartPort Commands (for reference):
//! $00 - STATUS     : Get device status/info
//! $01 - READ_BLOCK : Read a 512-byte block
//! $02 - WRITE_BLOCK: Write a 512-byte block
//! $03 - FORMAT     : Format device
//! $04 - CONTROL    : Device-specific control
//! $05 - INIT       : Initialize device
//! $06 - OPEN       : Open (for character devices)
//! $07 - CLOSE      : Close (for character devices)
//! $08 - READ       : Read bytes (character devices)
//! $09 - WRITE      : Write bytes (character devices)

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Block size for ProDOS/SmartPort devices
pub const BLOCK_SIZE: usize = 512;

/// Maximum supported blocks (32MB limit for ProDOS)
pub const MAX_BLOCKS: u32 = 65535;

/// SmartPort device status
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    Ready,
    NotReady,
    WriteProtected,
    Error,
}

/// A SmartPort block device (hard drive image)
pub struct SmartPortDevice {
    /// Path to the HDV file
    path: String,
    /// File handle (None if not loaded)
    file: Option<File>,
    /// Total number of blocks
    pub block_count: u32,
    /// Whether the device is write-protected
    pub write_protected: bool,
    /// Whether the device is enabled/present
    pub enabled: bool,
    /// Dirty blocks needing flush (for write caching)
    dirty: bool,
    /// Debug logging
    pub debug: bool,
}

impl Default for SmartPortDevice {
    fn default() -> Self {
        Self {
            path: String::new(),
            file: None,
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

    /// Load an HDV file as a hard drive image
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

    /// Read a block from the device
    pub fn read_block(&mut self, block: u32, buffer: &mut [u8; BLOCK_SIZE]) -> Result<(), String> {
        if !self.enabled {
            return Err("Device not ready".to_string());
        }
        
        if block >= self.block_count {
            return Err(format!("Block {} out of range (max {})", block, self.block_count - 1));
        }

        let file = self.file.as_mut().ok_or("No file loaded")?;
        let offset = block as u64 * BLOCK_SIZE as u64;
        
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("Seek error: {}", e))?;
        
        file.read_exact(buffer)
            .map_err(|e| format!("Read error at block {}: {}", block, e))?;

        if self.debug {
            log::debug!("SmartPort: Read block {} (offset 0x{:X})", block, offset);
        }

        Ok(())
    }

    /// Write a block to the device
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
        let offset = block as u64 * BLOCK_SIZE as u64;
        
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

    /// Flush any pending writes to disk
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

    /// Get device status information
    pub fn status(&self) -> DeviceStatus {
        if !self.enabled {
            DeviceStatus::NotReady
        } else if self.write_protected {
            DeviceStatus::WriteProtected
        } else {
            DeviceStatus::Ready
        }
    }

    /// Check if the device is loaded and ready
    pub fn is_ready(&self) -> bool {
        self.enabled && self.file.is_some()
    }

    /// Get the device path
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for SmartPortDevice {
    fn drop(&mut self) {
        if let Err(e) = self.flush() {
            log::error!("Failed to flush SmartPort device on drop: {}", e);
        }
    }
}

/// SmartPort controller managing multiple devices
/// ProDOS supports up to 14 units (2 per slot, slots 1-7)
/// For IIc, we typically use slot 5 (external drive port)
pub struct SmartPort {
    /// Hard drive devices (up to 2 for IIc external port)
    pub devices: [SmartPortDevice; 2],
    /// Debug logging
    pub debug: bool,
}

impl Default for SmartPort {
    fn default() -> Self {
        Self {
            devices: [SmartPortDevice::default(), SmartPortDevice::default()],
            debug: false,
        }
    }
}

impl SmartPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load an HDV file as device 0 (first hard drive)
    pub fn load_hdv<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        self.devices[0].load(path)
    }

    /// Load a second HDV file as device 1
    pub fn load_hdv2<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        self.devices[1].load(path)
    }

    /// Check if any device is loaded
    pub fn has_device(&self) -> bool {
        self.devices.iter().any(|d| d.is_ready())
    }

    /// Flush all devices
    pub fn flush_all(&mut self) {
        for device in &mut self.devices {
            if let Err(e) = device.flush() {
                log::error!("Failed to flush SmartPort device: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_hdv() {
        // Create a test HDV file (1 block = 512 bytes)
        let mut file = NamedTempFile::new().unwrap();
        let block = [0xAAu8; BLOCK_SIZE];
        file.write_all(&block).unwrap();
        file.flush().unwrap();

        let mut device = SmartPortDevice::new();
        device.load(file.path()).unwrap();

        assert!(device.is_ready());
        assert_eq!(device.block_count, 1);
    }

    #[test]
    fn test_read_write_block() {
        // Create a test HDV file with 2 blocks
        let mut file = NamedTempFile::new().unwrap();
        let block0 = [0x00u8; BLOCK_SIZE];
        let block1 = [0xFFu8; BLOCK_SIZE];
        file.write_all(&block0).unwrap();
        file.write_all(&block1).unwrap();
        file.flush().unwrap();

        let mut device = SmartPortDevice::new();
        device.load(file.path()).unwrap();

        // Read block 0
        let mut buffer = [0u8; BLOCK_SIZE];
        device.read_block(0, &mut buffer).unwrap();
        assert_eq!(buffer[0], 0x00);

        // Read block 1
        device.read_block(1, &mut buffer).unwrap();
        assert_eq!(buffer[0], 0xFF);

        // Write block 0
        let new_data = [0x42u8; BLOCK_SIZE];
        device.write_block(0, &new_data).unwrap();

        // Read back and verify
        device.read_block(0, &mut buffer).unwrap();
        assert_eq!(buffer[0], 0x42);
    }
}
