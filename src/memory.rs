#[allow(dead_code)]
pub struct Memory {
    data: Vec<u8>,
    id: String,
}

impl Memory {
    pub fn new(size: usize, id: String) -> Self {
        println!("memory {:>8} {:>8} {} KB", id, "ONLINE", size);
        Self {
            data: vec![0x00; size],
            id,
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

    #[allow(dead_code)]
    pub fn dump_range(&self, range: std::ops::RangeInclusive<u16>) {
        use crate::util::hexdump;

        let start = *range.start() as usize;
        let end = *range.end() as usize;
        let length = (end - start) + 1;

        if start >= self.data.len() {
            println!("DEBUG: Start out of range, skipping hexdump!");
            return;
        }

        let slice_end = (start + length).min(self.data.len());
        let slice = &self.data[start..slice_end];

        hexdump(slice, Some(start as u16), Some(slice.len()));
    }

    pub fn load_bytes(&mut self, offset: u16, bytes: &[u8]) {
        let start = offset as usize;
        let end = start + bytes.len();
        if end <= self.data.len() {
            self.data[start..end].copy_from_slice(bytes);
        }
    }
}
