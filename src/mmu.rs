use crate::{iou::IOU, memory::Memory, rom::ROM, video::VideoModeMask};

const RAM_SIZE: usize = 64 * 1024;
const ROM_SIZE: usize = 16 * 1024;
const LCRAM_SIZE: usize = 4 * 1024;

macro_rules! maybe_write_byte {
    ($write:expr, $ram:expr, $bank:expr, $addr:expr, $value:expr) => {
        if $write == 1 {
            $ram[$bank as usize].write_byte($addr, $value);
            0x00
        } else {
            println!("Attempted write to read-only memory at {:#06X}", $addr);
            0x00
        }
    };
}

pub struct MemStateMask;
#[rustfmt::skip]
impl MemStateMask {
    pub const INIT: u8         = 0b0010_0000; // Initial state
    pub const ALTZP: u8        = 0b0000_0001; // Read whether auxiliary (1) or main (0) bank
    pub const P280STORE: u8    = 0b0000_0010; // 80STORE + PAGE2
    pub const RAMRD: u8        = 0b0000_0100; // Read whether main (0) or aux (1)
    pub const RAMWRT: u8       = 0b0000_1000; // Write whether main (0) or aux (1)
    pub const LCRAM: u8        = 0b0001_0000; // Read RAM (1) or ROM (0)
    pub const RDBNK: u8        = 0b0010_0000; // Read whether $D000 bank 1 (0) or bank 2 (1)
    pub const WRITE: u8        = 0b0100_0000; // Write enabled (1) or read-only (0)
    pub const ALTROM: u8       = 0b1000_0000; // Read whether ROM bank 2 (1) or bank 1 (0)
}

pub const LCRAMMODEMASK: u8 = 0b0111_0000;

pub struct LcRamMode;
#[rustfmt::skip]
impl LcRamMode {
    // Read RAM; no write; use $D000 bank 2
    pub const C080: u8 = MemStateMask::RDBNK | MemStateMask::LCRAM;
    // Read ROM; write RAM; use $D000 bank 2
    pub const C081: u8 = MemStateMask::RDBNK | MemStateMask::WRITE;
    // Read ROM; no write; use $D000 bank 2
    pub const C082: u8 = MemStateMask::RDBNK;
    // Read and write RAM; use $D000 bank 2
    pub const C083: u8 = MemStateMask::RDBNK | MemStateMask::LCRAM | MemStateMask::WRITE;
    // Read RAM; no write; use $D000 bank 1
    pub const C088: u8 = MemStateMask::LCRAM;
    // Read ROM; write RAM; use$D000 bank 1
    pub const C089: u8 = MemStateMask::WRITE;
    // Read ROM; no write; use $D000 bank 1
    pub const C08A: u8 = 0x00;
    // Read and write RAM; use $D000 bank 1
    pub const C08B: u8 = MemStateMask::LCRAM | MemStateMask::WRITE;
    // Read RAM bank 1; no write
    pub const C08C: u8 = MemStateMask::LCRAM;
    // Read ROM; write RAM bank 1
    pub const C08D: u8 = MemStateMask::WRITE;
    // Read ROM; no write
    pub const C08E: u8 = 0x00;
    //  Read/write RAM bank 1
    pub const C08F: u8 = MemStateMask::LCRAM | MemStateMask::WRITE;
}

pub struct MMU {
    rom: [Memory; 2],   // Two 16KB ROM banks | [ROM1, ROM2]
    ram: [Memory; 2],   // 64KB Main and Auxiliary RAM | [MAIN, AUX]
    lcram: [Memory; 4], // Four 4KB Language Card RAM banks | [MAIN1, MAIN2, AUX1, AUX2]
}

impl MMU {
    pub fn new() -> Self {
        Self {
            rom: [
                Memory::new(ROM_SIZE, "ROM1".into()),
                Memory::new(ROM_SIZE, "ROM2".into()),
            ],
            ram: [
                Memory::new(RAM_SIZE, "RAMMAIN".into()),
                Memory::new(RAM_SIZE, "RAMAUX".into()),
            ],
            lcram: [
                Memory::new(LCRAM_SIZE, "LCMAIN1".into()),
                Memory::new(LCRAM_SIZE, "LCMAIN2".into()),
                Memory::new(LCRAM_SIZE, "LCAUX1".into()),
                Memory::new(LCRAM_SIZE, "LCAUX2".into()),
            ],
        }
    }

    // pub fn init_mem_state(&self) {
    //         self.mem_state.set(MemStateMask::INIT);
    // }

    pub fn load_rom(&mut self, rom: ROM) {
        // v3 iic rom boundary $3fff
        self.rom[0].load_bytes(0, &rom.data[0..ROM_SIZE]);
        self.rom[1].load_bytes(0, &rom.data[ROM_SIZE..(ROM_SIZE << 1)]);

        // println!("ROM Bank 1:");
        // self.rom[0].dump_range(0x0000..=0x00FF);
        // println!("ROM Bank 2:");
        // self.rom[1].dump_range(0x0000..=0x00FF);
    }

    pub fn read_aux_byte(&self, addr: u16) -> u8 {
        self.ram[1].read_byte(addr)
    }

    pub fn read_byte(&self, iou: &IOU, addr: u16) -> u8 {
        let mem_state = iou.mem_state.get();
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        let altzp = check_bits_u8!(mem_state, MemStateMask::ALTZP) as usize;
        let altrom = check_bits_u8!(mem_state, MemStateMask::ALTROM) as usize;
        let lcram = check_bits_u8!(mem_state, MemStateMask::LCRAM) as usize;
        let bank = check_bits_u8!(mem_state, MemStateMask::RDBNK) as usize;
        let ramrd = check_bits_u8!(mem_state, MemStateMask::RAMRD) as usize;

        match addr {
            // **Zero Page & Stack (Main vs. Auxiliary)**
            0x0000..=0x01FF => self.ram[altzp].read_byte(addr),

            // **80STORE-affected Display Memory (Text & Graphics)**
            0x0400..=0x07FF | 0x2000..=0x3FFF if is_80store => {
                self.ram[is_page2 as usize].read_byte(addr)
            }

            // **General 48K RAM ($0200 - $BFFF)**
            0x0200..=0xBFFF => self.ram[ramrd].read_byte(addr),

            // // **Soft Switches ($C000 - $C0FF)**
            // 0xC000..=0xC0FF => {
            //     let result = self.handle_softswitch_read(addr);
            //     println!(
            //         "SoftSwitch Read at {:#06X} = {:#04X} {}",
            //         addr,
            //         result,
            //         mem_state_to_string(mem_state)
            //     );
            //     self.last_rd_addr.set(addr);
            //     return result;
            // }

            // **Language Card (LC) RAM / ROM ($C100 - $CFFF)**
            0xC100..=0xCFFF => {
                if lcram == 1 {
                    self.lcram[bank + (ramrd << 1)].read_byte(addr.wrapping_sub(0xC100))
                } else {
                    self.rom[altrom].read_byte(addr.wrapping_sub(0xC000))
                }
            }

            // **Language Card RAM ($D000 - $DFFF)**
            0xD000..=0xDFFF => {
                if lcram == 1 {
                    self.lcram[bank + (ramrd << 1)].read_byte(addr.wrapping_sub(0xD000))
                } else {
                    self.rom[altrom].read_byte(addr - 0xC000)
                }
            }

            // **High Memory RAM / ROM ($E000 - $FFFF)**
            0xE000..=0xFFFF => {
                if lcram == 1 {
                    self.ram[altzp].read_byte(addr)
                } else {
                    self.rom[altrom].read_byte(addr.wrapping_sub(0xC000))
                }
            } // // **Reset Slot ROM Mapping ($CFFF)**
            // 0xCFFF => {
            //     println!("Resetting C800 Slot ROM Mapping!");
            //     return 0x00;  // Custom logic for slot ROM reset if necessary
            // }
            _ => {
                println!("Unhandled Memory Read at {:#06X}", addr);
                0x00
            }
        }
    }

    pub fn write_byte(
        &mut self,
        addr: u16,
        value: u8,
        mem_state: u8,
        is_80store: bool,
        is_page2: bool,
    ) -> u8 {
        let altzp = check_bits_u8!(mem_state, MemStateMask::ALTZP) as usize;
        let bank = check_bits_u8!(mem_state, MemStateMask::RDBNK) as usize;
        let ramwrt = check_bits_u8!(mem_state, MemStateMask::RAMWRT) as usize;
        let write = check_bits_u8!(mem_state, MemStateMask::WRITE) as usize;

        match addr {
            // **Zero Page & Stack (Main vs. Auxiliary)**
            0x0000..=0x01FF => self.ram[altzp].write_byte(addr, value),

            // **80STORE-affected Display Memory (Text & Graphics)**
            0x0400..=0x07FF | 0x2000..=0x3FFF if is_80store => {
                self.ram[is_page2 as usize].write_byte(addr, value)
            }

            // **General 48K RAM ($0200 - $BFFF)**
            0x0200..=0xBFFF => self.ram[ramwrt].write_byte(addr, value),

            // // **Soft Switch Writes ($C000 - $C0FF)**
            // 0xC000..=0xC0FF => {
            //     let result = self.handle_softswitch_write(addr, value, is_80store);
            //     println!(
            //         "SoftSwitch Write at {:#06X} = {:#04X} {}",
            //         addr,
            //         value,
            //         mem_state_to_string(mem_state)
            //     );
            //     return result;
            // }

            // **Language Card (LC) RAM / ROM ($C100 - $CFFF)**
            0xC100..=0xCFFF => maybe_write_byte!(
                write,
                self.lcram,
                bank + (ramwrt << 1),
                addr - 0xC100,
                value
            ),

            // **Language Card RAM ($D000 - $DFFF)**
            0xD000..=0xDFFF => maybe_write_byte!(
                write,
                self.lcram,
                bank + (ramwrt << 1),
                addr - 0xD000,
                value
            ),

            // **High Memory RAM / ROM ($E000 - $FFFF)**
            0xE000..=0xFFFF => maybe_write_byte!(write, self.ram, altzp, addr, value),
            // // **Reset Slot ROM Mapping ($CFFF)**
            // 0xCFFF => {
            //     println!("Resetting C800 Slot ROM Mapping!");
            //     return 0x00;  // Custom logic for slot ROM reset if necessary
            // }
            _ => {
                println!("Unhandled Memory Write at {:#06X}", addr);
                0x00
            }
        }
    }
}
