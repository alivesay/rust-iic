use crate::cpu::{CpuType, SystemType};
use crate::device::speaker::AudioProducer;
use crate::interrupts::InterruptController;
use crate::iou::Iou;
use crate::memory::Memory;
use crate::mmu::{Mmu, WriteCtx};
use crate::rom::Rom;
use crate::util::mem_state_to_string;
use crate::video::{Video, VideoModeMask};
use crate::timing;

const MEMORY_SIZE: usize = 64 * 1024;
const RAM_BANK_SIZE: usize = 48 * 1024;

pub struct Bus {
    system_type: SystemType,
    pub iou: Iou,
    mmu: Mmu,
    bus_ram: Memory,
    pub interrupts: InterruptController,

    pub video: Video,
    pub i_port: u8, // Klauss IRQ/NMI Feedback Register

    last_video_mode: u8,
    last_is_80store: bool,

    pub step_access_count: u64,

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
            iou: Iou::new(self_test, audio_producer, sample_rate),
            mmu: Mmu::new(),
            interrupts: InterruptController::default(),

            video: Video::new(),

            bus_ram: Memory::new(memory_size, "BUSRAM".into()),

            // #[cfg(feature = "klauss-interrupt-test")]
            i_port: 0,
            last_video_mode: 0,
            last_is_80store: false,
            step_access_count: 0,
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

        let mode = self.iou.video_mode.get();
        let store = self.iou.is_80store.get();
        self.video.seed_frame_start_mode(mode, store);
        self.last_video_mode = mode;
        self.last_is_80store = store;
    }

    pub fn video_snapshot_scanline(&mut self, scanline: usize) {
        self.video.snapshot_scanline(
            scanline,
            self.iou.video_mode.get(),
            self.iou.is_80store.get(),
        );
    }

    pub fn video_compose_monitor_partial(&mut self, up_to_scanline: usize) -> &[u8] {
        self.video.compose_monitor_partial(&self.iou, &self.mmu, up_to_scanline)
    }

    pub fn load_rom(&mut self, rom: Rom) {
        if self.system_type == SystemType::AppleIIc {
            self.mmu.load_rom(rom);
        } else {
            self.bus_ram.load_bytes(0, &rom.data[0..MEMORY_SIZE]);
        }
    }

    pub fn peek_byte(&mut self, addr: u16) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if (0xC000..=0xC0FF).contains(&addr) {
                // TODO: for now, return 0 for soft switches to avoid side effects
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
        let result = self.read_byte_inner(addr);
        self.step_access_count += 1;
        self.advance_scan_one();
        result
    }

    fn read_byte_inner(&mut self, addr: u16) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if (0xC000..=0xC0FF).contains(&addr) {
                let result = self.handle_iic_read(addr);
                if self.debug {
                    println!("SoftSwitch Read: {:#06X} = {:#04X}", addr, result);
                }
                result
            } else {
                self.mmu.read_byte(&mut self.iou, addr)
            }
        } else {
            self.bus_ram.read_byte(addr)
        }
    }

    pub fn read_word(&mut self, addr: u16) -> u16 {
        let lo = self.read_byte(addr) as u16;
        let hi = self.read_byte(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) -> u8 {
        let result = self.write_byte_inner(addr, value);
        self.step_access_count += 1;
        self.advance_scan_one();
        result
    }

    fn write_byte_inner(&mut self, addr: u16, value: u8) -> u8 {
        if self.system_type == SystemType::AppleIIc {
            if (0xC000..=0xC0FF).contains(&addr) {
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
                    WriteCtx { mem_state, is_80store, is_page2, is_hires },
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
                    WriteCtx { mem_state, is_80store, is_page2, is_hires },
                )
            }
        }
    }

    pub fn detect_video_mode_change(&mut self) {
        let cur_video_mode = self.iou.video_mode.get();
        let cur_is_80store = self.iou.is_80store.get();
        if cur_video_mode == self.last_video_mode && cur_is_80store == self.last_is_80store {
            return;
        }
        let scan = self.iou.scan_cycle;
        let scanline = (scan / timing::CYCLES_PER_SCANLINE) as usize;
        let raw_col = (scan % timing::CYCLES_PER_SCANLINE) as u8;

        const VIDEO_PIPELINE_DELAY: u8 = 2;
        let display_raw_col = raw_col.saturating_add(VIDEO_PIPELINE_DELAY);

        if display_raw_col >= 25 && (display_raw_col as u16) < timing::CYCLES_PER_SCANLINE as u16 {
            let active_col = display_raw_col - 25;
            if active_col < 40 && scanline < 192 {
                self.video
                    .record_mode_change(scanline, active_col, cur_video_mode, cur_is_80store);
            }
        } else if display_raw_col < 25 {
            // HBLANK flip
            if scanline < 192 {
                self.video
                    .set_scanline_start(scanline, cur_video_mode, cur_is_80store);
            }
        } else {
            // delay pushed past end of scanline...
            let next_line = scanline + 1;
            if next_line < 192 {
                self.video
                    .set_scanline_start(next_line, cur_video_mode, cur_is_80store);
            }
        }
        self.last_video_mode = cur_video_mode;
        self.last_is_80store = cur_is_80store;
    }

    pub fn tick(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.advance_scan_one();
        }
        self.tick_devices(cycles);
    }

    // advance scan_cycle by exactly one cycle, with per-cycle side effects
    pub fn advance_scan_one(&mut self) {
        self.detect_video_mode_change();

        let old_scan = self.iou.scan_cycle;
        self.iou.scan_cycle += 1;
        if self.iou.scan_cycle >= timing::CYCLES_PER_FRAME {
            self.iou.scan_cycle -= timing::CYCLES_PER_FRAME;
        }

        // VBL edge
        if old_scan < timing::VBL_START_CYCLE && self.iou.scan_cycle >= timing::VBL_START_CYCLE {
            self.iou.mouse.vbl_int.set(true);
        }

        // scanline-start hook
        let new_col = self.iou.scan_cycle % timing::CYCLES_PER_SCANLINE;
        if new_col == 0 {
            let new_line = (self.iou.scan_cycle / timing::CYCLES_PER_SCANLINE) as usize;
            if new_line < 192 {
                self.video.set_scanline_start(
                    new_line,
                    self.last_video_mode,
                    self.last_is_80store,
                );
            }
        }

        // per-cycle floating-bus update
        let video_mode = self.iou.video_mode.get();
        let is_text = (video_mode & VideoModeMask::TEXT) != 0;
        let is_hires = (video_mode & VideoModeMask::HIRES) != 0;
        let is_mixed = (video_mode & VideoModeMask::MIXED) != 0;
        let is_page2 = (video_mode & VideoModeMask::PAGE2) != 0;
        let is_80store = self.iou.is_80store.get();

        let scan = self.iou.scan_cycle;
        let scanline = (scan / timing::CYCLES_PER_SCANLINE) as u16;
        let col = (scan % timing::CYCLES_PER_SCANLINE) as u16;
        let byte_idx = if col >= 25 { col - 25 } else { col + 40 };

        let use_hires = is_hires && !is_text && !(is_mixed && scanline >= 160);

        if use_hires {
            let line = scanline % 192;
            let s_row = line % 8;
            let s_group = line / 8;
            let base: u16 = if !is_80store && is_page2 { 0x4000 } else { 0x2000 };
            let addr = base
                + s_row * 0x400
                + (s_group % 8) * 0x80
                + (s_group / 8) * 0x28
                + byte_idx;
            let addr = (addr & 0x1FFF) | base;
            self.iou.floating_bus = self.mmu.read_main_byte(addr);
        } else {
            let row = scanline / 8;
            let base: u16 = if !is_80store && is_page2 { 0x0800 } else { 0x0400 };
            let addr = base + (row / 8) * 0x28 + (row % 8) * 0x80 + byte_idx;
            let addr = (addr & 0x03FF) | base;
            self.iou.floating_bus = self.mmu.read_main_byte(addr);
        }
    }

    pub fn tick_devices(&mut self, cycles: u64) {
        self.iou.cycles += cycles;
        self.iou.mouse.tick(cycles);
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

    pub fn tick_scan_only(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.advance_scan_one();
        }
    }
}
