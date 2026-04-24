use crate::cpu::{CpuType, SystemType};
use crate::device::speaker::AudioProducer;
use crate::interrupts::InterruptController;
use crate::iou::IOU;
use crate::memory::Memory;
use crate::mmu::MMU;
use crate::rom::ROM;
use crate::util::mem_state_to_string;
use crate::video::{Video, VideoModeMask};

const MEMORY_SIZE: usize = 64 * 1024;
const RAM_BANK_SIZE: usize = 48 * 1024;

pub struct Bus {
    system_type: SystemType,
    pub iou: IOU,
    mmu: MMU,
    bus_ram: Memory,
    pub interrupts: InterruptController,

    pub video: Video,
    pub i_port: u8, // Klauss IRQ/NMI Feedback Register

    pub debug: bool,
}

impl Bus {
    pub fn new(system_type: SystemType, _cpu_type: CpuType, self_test: bool, audio_producer: AudioProducer, sample_rate: u32) -> Self {
        let memory_size = match system_type {
            SystemType::Generic => MEMORY_SIZE,
            SystemType::AppleIIc => RAM_BANK_SIZE * 2,
        };

        Self {
            system_type,
            iou: IOU::new(self_test, audio_producer, sample_rate),
            mmu: MMU::new(),
            interrupts: InterruptController::default(),

            video: Video::new(),

            bus_ram: Memory::new(memory_size, "BUSRAM".into()),

            // #[cfg(feature = "klauss-interrupt-test")]
            i_port: 0,
            debug: false,
        }
    }

    pub fn randomize_ram(&mut self) {
        self.mmu.randomize_ram();
    }

    pub fn mmu_mem_state_to_string(&self) -> String {
        mem_state_to_string(self.iou.mem_state.get())
    }

    pub fn video_update(&mut self) {
        self.video.update(&self.iou, &self.mmu);
    }

    pub fn video_begin_frame(&mut self) {
        self.video.begin_frame();
    }

    pub fn video_snapshot_scanline(&mut self, scanline: usize) {
        self.video.snapshot_scanline(
            scanline,
            self.iou.video_mode.get(),
            self.iou.is_80store.get(),
        );
    }

    pub fn load_rom(&mut self, rom: ROM) {
        if self.system_type == SystemType::AppleIIc {
            self.mmu.load_rom(rom);
        } else {
            self.bus_ram.load_bytes(0, &rom.data[0..MEMORY_SIZE]);
        }
    }

    pub fn peek_byte(&mut self, addr: u16) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if addr >= 0xC000 && addr <= 0xC0FF {
                // TODO: For now, return 0 for soft switches to avoid side effects
                0x00
            } else {
                self.mmu.read_byte(&mut self.iou, addr)
            }
        } else {
            self.bus_ram.read_byte(addr)
        }
    }

    pub fn update_interrupts(&mut self) {
        if self.system_type == SystemType::AppleIIc {
            self.interrupts.irq = self.iou.check_interrupts();
            if self.interrupts.irq {
                self.interrupts.waiting = false;
            }
        }
    }

    pub fn read_byte(&mut self, addr: u16) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if addr >= 0xC000 && addr <= 0xC0FF {
                let result = self.handle_iic_read(addr);
                if self.debug {
                    println!("SoftSwitch Read: {:#06X} = {:#04X}", addr, result);
                }
                result
            } else {
                self.mmu.read_byte(&mut self.iou, addr)
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

    pub fn read_word(&mut self, addr: u16) -> u16 {
        let lo = self.read_byte(addr) as u16;
        let hi = self.read_byte(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if addr >= 0xC000 && addr <= 0xC0FF {
                let result = self.handle_iic_write(addr, value);
                if self.debug {
                    println!("SoftSwitch Write: {:#06X} = {:#04X}", addr, value);
                }
                result
            } else {
                let video_mode = self.iou.video_mode.get();
                let is_page2 = (video_mode & crate::video::VideoModeMask::PAGE2) != 0;
                let is_hires = (video_mode & crate::video::VideoModeMask::HIRES) != 0;
                let is_80store = self.iou.is_80store.get();
                let mem_state = self.iou.mem_state.get();

                self.mmu.write_byte(
                    &mut self.iou,
                    addr,
                    value,
                    mem_state,
                    is_80store,
                    is_page2,
                    is_hires,
                )
            }
        } else {
            match addr {
                0xBFFC => {
                    self.i_port = value;

                    let irq_triggered = value & (1 << 0) != 0;
                    let nmi_triggered = value & (1 << 1) != 0;

                    if irq_triggered {
                        self.interrupts.request_irq();
                    }

                    if nmi_triggered {
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

    pub fn handle_iic_read(&mut self, addr: u16) -> u8 {
        match addr {
            0xC000..=0xC0FF => {
                self.iou.zip.io_access();  // ZIP Chip: slow down for I/O
                self.iou.ss_read(addr)
            },
            _ => self.mmu.read_byte(&mut self.iou, addr),
        }
    }

    pub fn handle_iic_write(&mut self, addr: u16, value: u8) -> u8 {
        match addr {
            0xC000..=0xC0FF => {
                self.iou.zip.io_access();  // ZIP Chip: slow down for I/O
                self.iou.ss_write(addr, value)
            },
            _ => {
                let video_mode = self.iou.video_mode.get();
                let is_page2 = (video_mode & VideoModeMask::PAGE2) != 0;
                let is_hires = (video_mode & VideoModeMask::HIRES) != 0;
                let is_80store = self.iou.is_80store.get();
                let mem_state = self.iou.mem_state.get();

                self.mmu.write_byte(
                    &mut self.iou,
                    addr,
                    value,
                    mem_state,
                    is_80store,
                    is_page2,
                    is_hires,
                )
            }
        }
    }

    pub fn tick(&mut self, cycles: u64) {
        self.iou.cycles += cycles;

        // Track position within NTSC frame for VBL timing
        // 262 scanlines × 65 cycles = 17030 cycles/frame
        let old_scan = self.iou.scan_cycle;

        // Per-cycle floating bus update.
        // The Apple II video hardware fetches one byte per cycle during active
        // display. Software that reads the floating bus ($C000-$C07F with no
        // switch selected, or certain unconnected addresses) sees whatever the
        // video circuitry last put on the data bus.
        let video_mode = self.iou.video_mode.get();
        let is_hires = (video_mode & VideoModeMask::HIRES) != 0;
        let is_page2 = (video_mode & VideoModeMask::PAGE2) != 0;
        let is_80store = self.iou.is_80store.get();

        for _ in 0..cycles {
            self.iou.scan_cycle += 1;
            if self.iou.scan_cycle >= 17030 {
                self.iou.scan_cycle -= 17030;
            }

            let scan = self.iou.scan_cycle;
            let scanline = (scan / 65) as u16;
            let col = (scan % 65) as u16;

            if scanline < 192 && col < 40 {
                let row = scanline / 8;
                let group = row / 8;
                let offset = row % 8;

                if is_hires {
                    // HiRes: video fetches from $2000/$4000 page
                    // Address = base + scanline_row*1024 + (group%8)*128 + (group/8)*40 + col
                    // where scanline_row = scanline % 8, group = scanline / 8
                    let s_group = scanline / 8;
                    let s_row = scanline % 8;
                    let base: u16 = if !is_80store && is_page2 { 0x4000 } else { 0x2000 };
                    let addr = base + s_row * 1024 + (s_group % 8) * 128 + (s_group / 8) * 40 + col;
                    self.iou.floating_bus = self.mmu.read_main_byte(addr);
                } else {
                    // Text/LoRes: video fetches from $0400/$0800 page
                    let base: u16 = if !is_80store && is_page2 { 0x0800 } else { 0x0400 };
                    let addr = base + group * 0x80 + offset * 0x28 + col;
                    self.iou.floating_bus = self.mmu.read_main_byte(addr);
                }
            }
        }

        // Set VBL interrupt when entering VBL region (scanline 192+)
        if old_scan < 12480 && self.iou.scan_cycle >= 12480 {
            self.iou.mouse.vbl_int.set(true);
        }

        self.iou.mouse.tick(cycles);

        // Per-cycle IWM ticking for precise bit timing (4 cycles = 1 bit).
        // When the motor is active, tick one cycle at a time so the data latch
        // becomes ready on the exact cycle.
        if self.iou.iwm.motor_on {
            for _ in 0..cycles {
                self.iou.iwm.tick(1);
            }
        } else {
            self.iou.iwm.tick(cycles);
        }
        
        self.iou.scc.tick(cycles);
        self.iou.zip.tick();
        
        self.iou.mockingboard.tick_n(cycles as u32);
        self.iou.mockingboard2.tick_n(cycles as u32);
        
        if self.iou.check_interrupts() {
            self.interrupts.request_irq();
        }
    }
}
