pub struct Memory {
    data: Vec<u8>
}

impl Memory {
    pub fn new(size: usize, id: String) -> Self {
        println!("mem   {:>12} {:>8}    {:>8} bytes", id, "ONLINE", size);
        
        Self { data: vec![0x00; size] }
    }

    pub fn randomize_power_on(&mut self) {
        for (i, byte) in self.data.iter_mut().enumerate() {
            let base = if (i / 128) & 1 == 0 { 0x00u8 } else { 0xFFu8 };
            let noise = fastrand::u8(..) & fastrand::u8(..) & fastrand::u8(..);
            *byte = base ^ noise;
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        let byte = self.data.get(addr as usize).copied().unwrap_or(0x00);
        // #[cfg(feature = "debug-mode")]
        // println!(
        //     "memory[{:>8}] read_byte({:#06X}) => {:#04X}, bankid: {}",
        //     self.id, addr, byte, self.id
        // );
        byte
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) -> u8 {
        self.data[addr as usize] = value;
        // if let Some(byte) = self.data.get_mut(addr as usize) {
        //     *byte = value;
        // }
        // #[cfg(feature = "debug-mode")]
        // println!(
        //     "memory[{:>8}] write_byte({:#06X}, {:#04X}), bankid: {}",
        //     self.id, addr, value, self.id
        // );
        0x00
    }

    pub fn load_bytes(&mut self, offset: u16, bytes: &[u8]) {
        let start = offset as usize;
        let end = start + bytes.len();
        if end <= self.data.len() {
            self.data[start..end].copy_from_slice(bytes);
        }
    }
}
