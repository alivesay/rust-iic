// Memory Expansion Card state
pub struct MemoryExpansion {
    // Expansion RAM (1MB = 1,048,576 bytes)
    ram: Vec<u8>,
    
    // 24-bit address register (auto-incrementing)
    addr_lo: u8,
    addr_mid: u8,
    addr_hi: u8,
    
    // Card enabled
    enabled: bool,

    // True when RAM has been written since the last successful flush.
    dirty: bool,
}

impl MemoryExpansion {
    // Create a new Memory Expansion Card with 1MB RAM
    pub fn new() -> Self {
        Self {
            ram: vec![0x00; 1024 * 1024], // 1MB
            addr_lo: 0,
            addr_mid: 0,
            addr_hi: 0,
            enabled: true,
            dirty: false,
        }
    }
    
    // Get the current 24-bit address
    fn address(&self) -> usize {
        (self.addr_lo as usize) 
            | ((self.addr_mid as usize) << 8)
            | ((self.addr_hi as usize) << 16)
    }
    
    // Set the 24-bit address
    fn set_address(&mut self, addr: usize) {
        self.addr_lo = (addr & 0xFF) as u8;
        self.addr_mid = ((addr >> 8) & 0xFF) as u8;
        self.addr_hi = ((addr >> 16) & 0xFF) as u8;
    }
    
    // Increment the address register (wraps at 24 bits)
    fn increment_address(&mut self) {
        let addr = self.address().wrapping_add(1) & 0xFFFFFF;
        self.set_address(addr);
    }
    
    // Read from a Slot 4 I/O register ($C0C0-$C0CF)
    pub fn read(&mut self, addr: u8) -> u8 {
        if !self.enabled {
            return 0xFF;
        }
        
        match addr & 0x0F {
            0x00 => self.addr_lo,
            0x01 => self.addr_mid,
            // High byte: bits 4-7 read as 1s to indicate 1MB size limit
            // (only bits 0-3 are valid address bits for 1MB)
            0x02 => self.addr_hi | 0xF0,
            0x03 => {
                // Data port: read from RAM and auto-increment
                let mem_addr = self.address();
                let value = if mem_addr < self.ram.len() {
                    self.ram[mem_addr]
                } else {
                    0xFF // Beyond installed RAM
                };
                self.increment_address();
                value
            }
            _ => 0xFF, // Unused registers
        }
    }
    
    // Write to a Slot 4 I/O register ($C0C0-$C0CF)
    pub fn write(&mut self, addr: u8, value: u8) {
        if !self.enabled {
            return;
        }
        
        match addr & 0x0F {
            0x00 => self.addr_lo = value,
            0x01 => self.addr_mid = value,
            0x02 => self.addr_hi = value & 0x0F, // Only 4 bits for 1MB
            0x03 => {
                // Data port: write to RAM and auto-increment
                let mem_addr = self.address();
                if mem_addr < self.ram.len() {
                    if self.ram[mem_addr] != value {
                        self.ram[mem_addr] = value;
                        self.dirty = true;
                    }
                }
                self.increment_address();
            }
            _ => {} // Unused registers
        }
    }
    
    // Enable or disable the card
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    // Load RAM contents from a file. Missing/short/oversize files are
    // tolerated: present bytes overwrite the start of RAM, the rest
    // stays zero. A non-existent path is a no-op (first launch).
    pub fn load_from_file(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let n = bytes.len().min(self.ram.len());
                self.ram[..n].copy_from_slice(&bytes[..n]);
                if n > 0 {
                    log::info!("memexp: loaded {} bytes from {}", n, path.display());
                }
                // Loaded image matches disk; nothing to flush yet.
                self.dirty = false;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!("memexp: failed to read {}: {}", path.display(), e);
            }
        }
    }

    // True if RAM has been written since the last successful flush.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    // Persist the full RAM image to disk only if dirty. Returns true if
    // a write actually occurred. Uses atomic write (tmp file + rename)
    // so a crash mid-write can't corrupt the existing image.
    pub fn flush_if_dirty(&mut self, path: &std::path::Path) -> bool {
        if !self.dirty {
            return false;
        }
        if self.write_atomic(path) {
            self.dirty = false;
            true
        } else {
            false
        }
    }

    // Persist the full RAM image to disk unconditionally (e.g. on exit).
    // Creates parent directories as needed. Errors are logged but not
    // propagated.
    pub fn save_to_file(&mut self, path: &std::path::Path) {
        if self.write_atomic(path) {
            self.dirty = false;
        }
    }

    fn write_atomic(&self, path: &std::path::Path) -> bool {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("memexp: failed to create {}: {}", parent.display(), e);
                return false;
            }
        }
        let tmp = path.with_extension("bin.tmp");
        if let Err(e) = std::fs::write(&tmp, &self.ram) {
            log::warn!("memexp: failed to write {}: {}", tmp.display(), e);
            return false;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            log::warn!("memexp: failed to rename {} -> {}: {}", tmp.display(), path.display(), e);
            // best-effort cleanup of the orphan tmp file.
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
        log::info!("memexp: saved {} bytes to {}", self.ram.len(), path.display());
        true
    }
}

impl Default for MemoryExpansion {
    fn default() -> Self {
        Self::new()
    }
}

