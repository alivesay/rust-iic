//! Apple IIc Memory Expansion Card (Slinky)
//!
//! Provides up to 1MB of additional RAM accessible through Slot 4 I/O space ($C0C0-$C0CF).
//!
//! Register map:
//! - $C0C0: Address low byte
//! - $C0C1: Address middle byte
//! - $C0C2: Address high byte (bits 0-3 for 1MB)
//! - $C0C3: Data port (read/write, auto-increments address)
//!
//! The 24-bit address register allows addressing up to 16MB, but only 1MB is implemented.
//! Accessing beyond installed RAM returns $FF on reads and ignores writes.

/// Memory Expansion Card state
pub struct MemoryExpansion {
    /// Expansion RAM (1MB = 1,048,576 bytes)
    ram: Vec<u8>,
    
    /// 24-bit address register (auto-incrementing)
    addr_lo: u8,
    addr_mid: u8,
    addr_hi: u8,
    
    /// Card enabled
    enabled: bool,
}

impl MemoryExpansion {
    /// Create a new Memory Expansion Card with 1MB RAM
    pub fn new() -> Self {
        Self {
            ram: vec![0x00; 1024 * 1024], // 1MB
            addr_lo: 0,
            addr_mid: 0,
            addr_hi: 0,
            enabled: true,
        }
    }
    
    /// Get the current 24-bit address
    fn address(&self) -> usize {
        (self.addr_lo as usize) 
            | ((self.addr_mid as usize) << 8)
            | ((self.addr_hi as usize) << 16)
    }
    
    /// Set the 24-bit address
    fn set_address(&mut self, addr: usize) {
        self.addr_lo = (addr & 0xFF) as u8;
        self.addr_mid = ((addr >> 8) & 0xFF) as u8;
        self.addr_hi = ((addr >> 16) & 0xFF) as u8;
    }
    
    /// Increment the address register (wraps at 24 bits)
    fn increment_address(&mut self) {
        let addr = self.address().wrapping_add(1) & 0xFFFFFF;
        self.set_address(addr);
    }
    
    /// Read from a Slot 4 I/O register ($C0C0-$C0CF)
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
                // Data port - read from RAM and auto-increment
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
    
    /// Write to a Slot 4 I/O register ($C0C0-$C0CF)
    pub fn write(&mut self, addr: u8, value: u8) {
        if !self.enabled {
            return;
        }
        
        match addr & 0x0F {
            0x00 => self.addr_lo = value,
            0x01 => self.addr_mid = value,
            0x02 => self.addr_hi = value & 0x0F, // Only 4 bits for 1MB
            0x03 => {
                // Data port - write to RAM and auto-increment
                let mem_addr = self.address();
                if mem_addr < self.ram.len() {
                    self.ram[mem_addr] = value;
                }
                self.increment_address();
            }
            _ => {} // Unused registers
        }
    }
    
    /// Enable or disable the card
    #[allow(dead_code)]
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    
    /// Check if card is enabled
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    
    /// Get installed RAM size in bytes
    #[allow(dead_code)]
    pub fn ram_size(&self) -> usize {
        self.ram.len()
    }
    
    /// Reset the card state (address registers only, RAM preserved)
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.addr_lo = 0;
        self.addr_mid = 0;
        self.addr_hi = 0;
    }
}

impl Default for MemoryExpansion {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_address_register() {
        let mut card = MemoryExpansion::new();
        
        // Set address to $012345
        card.write(0x00, 0x45);
        card.write(0x01, 0x23);
        card.write(0x02, 0x01);
        
        assert_eq!(card.read(0x00), 0x45);
        assert_eq!(card.read(0x01), 0x23);
        // High byte reads with upper nibble set (1MB size indicator)
        assert_eq!(card.read(0x02), 0xF1);
        assert_eq!(card.address(), 0x012345);
    }
    
    #[test]
    fn test_data_read_write() {
        let mut card = MemoryExpansion::new();
        
        // Set address to 0
        card.write(0x00, 0x00);
        card.write(0x01, 0x00);
        card.write(0x02, 0x00);
        
        // Write some data
        card.write(0x03, 0xAA);
        card.write(0x03, 0xBB);
        card.write(0x03, 0xCC);
        
        // Reset address and read back
        card.write(0x00, 0x00);
        card.write(0x01, 0x00);
        card.write(0x02, 0x00);
        
        assert_eq!(card.read(0x03), 0xAA);
        assert_eq!(card.read(0x03), 0xBB);
        assert_eq!(card.read(0x03), 0xCC);
    }
    
    #[test]
    fn test_auto_increment() {
        let mut card = MemoryExpansion::new();
        
        // Set address to 0
        card.set_address(0);
        
        // Write 3 bytes
        card.write(0x03, 0x11);
        card.write(0x03, 0x22);
        card.write(0x03, 0x33);
        
        // Address should now be 3
        assert_eq!(card.address(), 3);
    }
    
    #[test]
    fn test_beyond_ram() {
        let mut card = MemoryExpansion::new();
        
        // Set address beyond 1MB
        card.write(0x00, 0x00);
        card.write(0x01, 0x00);
        card.write(0x02, 0x0F); // High nibble ignored, but address is still > 1MB due to wrap
        
        // Writes should be ignored, reads return 0xFF beyond installed RAM
        // With addr_hi = 0x0F, address = 0x0F0000 = 983,040, still within 1MB
        // Let's test with the full 24-bit range
        card.set_address(0x100000); // Exactly at 1MB boundary
        assert_eq!(card.read(0x03), 0xFF);
    }
}
