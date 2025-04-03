use crate::cpu::{CpuType, SystemType};
use crate::interrupts::InterruptController;
use crate::iou::IOU;
use crate::memory::Memory;
use crate::mmu::MMU;
use crate::rom::ROM;
use crate::util::mem_state_to_string;
use crate::video::Video;

const MEMORY_SIZE: usize = 64 * 1024;
const RAM_BANK_SIZE: usize = 48 * 1024;

pub struct Bus {
    system_type: SystemType,
    pub iou: IOU,
    mmu: MMU,
    bus_ram: Memory,
    pub interrupts: InterruptController,

    pub video: Video,

    // pub vblint: Cell<u8>,   // VBL Interrupt Status
    // iou_disabled: Cell<u8>, // IOU Disable Flag

    // button_0: Cell<u8>,     // Button 0 (Open Apple Key)
    // button_1: Cell<u8>,     // Button 1 (Closed Apple Key)
    // paddle_timer: Cell<u8>, // Paddle Timer
    // mouse_x: Cell<u8>,      // Mouse X Position
    // mouse_y: Cell<u8>,      // Mouse Y Position
    // mouse_ack: Cell<u8>,    // Mouse Acknowledge

    //#[cfg(feature = "klauss-interrupt-test")]
    pub i_port: u8, // Klauss IRQ/NMI Feedback Register
}

impl Bus {
    pub fn new(system_type: SystemType, _cpu_type: CpuType) -> Self {
        let memory_size = match system_type {
            SystemType::Generic => MEMORY_SIZE,
            SystemType::AppleIIc => RAM_BANK_SIZE * 2,
        };

        Self {
            system_type,
            iou: IOU::new(),
            mmu: MMU::new(),
            interrupts: InterruptController::default(),

            video: Video::new(),

            // vblint: Cell::new(0),
            // iou_disabled: Cell::new(0),

            // button_0: Cell::new(0),
            // button_1: Cell::new(0),
            // mouse_x: Cell::new(0),
            // mouse_y: Cell::new(0),
            // paddle_timer: Cell::new(0),
            // mouse_ack: Cell::new(0),
            bus_ram: Memory::new(memory_size, "BUSRAM".into()),

            // #[cfg(feature = "klauss-interrupt-test")]
            i_port: 0,
        }
    }

    pub fn init_mmu(&mut self) {
        //self.mmu.init_mem_state();
    }

    pub fn mmu_mem_state_to_string(&self) -> String {
        mem_state_to_string(self.iou.mem_state.get())
    }

    pub fn video_update(&mut self) {
        self.video.update(&self.iou, &self.mmu);
    }

    pub fn load_rom(&mut self, rom: ROM) {
        if self.system_type == SystemType::AppleIIc {
            self.mmu.load_rom(rom);
        } else {
            self.bus_ram.load_bytes(0, &rom.data[0..MEMORY_SIZE]);
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if addr >= 0xC000 && addr <= 0xC0FF {
                let result = self.handle_iic_read(addr);
                // println!("SoftSwitch Read: {:#06X} = {:#04X}", addr, result);
                result
            } else {
                self.handle_iic_read(addr)
            }
        } else {
            // #[cfg(feature = "klauss-interrupt-test")]
            // match addr {
            //     0xBFFC => {
            //         println!("Reading $BFFC: {:#04X}", self.i_port);
            //         self.i_port
            //     }
            //     _ => self.testmem.read_byte(addr),
            // }
            self.bus_ram.read_byte(addr)
        }
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        let lo = self.read_byte(addr) as u16;
        let hi = self.read_byte(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if addr >= 0xC000 && addr <= 0xC0FF {
                let result = self.handle_iic_write(addr, value);
                println!("SoftSwitch Write: {:#06X} = {:#04X}", addr, value);
                result
            } else {
                self.handle_iic_write(addr, value)
            }
        } else {
            match addr {
                0xBFFC => {
                    println!(
                        "⚡ Writing to IRQ/NMI feedback register at $BFFC: {:#04X}",
                        value
                    );
                    self.i_port = value;

                    let irq_triggered = value & (1 << 0) != 0;
                    let nmi_triggered = value & (1 << 1) != 0;

                    if irq_triggered {
                        println!("Triggering IRQ from $BFFC!");
                        self.interrupts.request_irq();
                    }

                    if nmi_triggered {
                        println!("Triggering NMI from $BFFC!");
                        self.interrupts.request_nmi();
                    }

                    0x00
                }
                _ => self.bus_ram.write_byte(addr, value),
            }
        }
    }

    pub fn write_bytes(&mut self, start: u16, bytes: &[u8]) {
        for (i, &byte) in bytes.iter().enumerate() {
            self.write_byte(start.wrapping_add(i as u16), byte);
        }
    }

    // #[allow(dead_code)]
    // pub fn dump_stack(&self) {
    //     self.memory().dump_range(0x0100..=0x01FF);
    // }

    // pub fn memory(&self) -> &Memory {
    //     &self.mmu.active_ram()
    // }

    pub fn handle_iic_read(&self, addr: u16) -> u8 {
        match addr {
            0xC000..=0xC0FF => self.iou.ss_read(addr),
            _ => self.mmu.read_byte(&self.iou, addr),
        }
    }

    pub fn handle_iic_write(&mut self, addr: u16, value: u8) -> u8 {
        match addr {
            0xC000..=0xC0FF => self.iou.ss_write(addr),
            _ => self.mmu.write_byte(
                addr,
                value,
                self.iou.mem_state.get(),
                self.iou.is_80store.get(),
                false,
            ),
        }
    }
}
