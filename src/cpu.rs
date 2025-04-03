use crate::bus::Bus;
use crate::disassembler::{Disassembler, SymbolTable};
use crate::interrupts::InterruptType;
use crate::rom::ROM;
use bitflags::bitflags;
use core::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemType {
    Generic,
    AppleIIc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuType {
    NMOS6502,
    CMOS65C02,
    WDC65C02S,
}

#[derive(Default)]
pub struct Registers {
    pub a: u8,  // Accumulator
    pub x: u8,  // X Register
    pub y: u8,  // Y Register
    pub sp: u8, // Stack Pointer
}

bitflags! {
    #[derive(Default, Copy, Clone)]
    pub struct Flags: u8 {
        const CARRY       = 0b0000_0001;
        const ZERO        = 0b0000_0010;
        const IRQ_DISABLE = 0b0000_0100;
        const DECIMAL     = 0b0000_1000;
        const BREAK       = 0b0001_0000;
        const UNUSED      = 0b0010_0000;
        const OVERFLOW    = 0b0100_0000;
        const NEGATIVE    = 0b1000_0000;
    }
}

impl fmt::Debug for Flags {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", format_flags(self.bits()))
    }
}

fn format_flags(flags: u8) -> String {
    let flag_map = [
        ('N', Flags::NEGATIVE),
        ('V', Flags::OVERFLOW),
        ('-', Flags::UNUSED),
        ('B', Flags::BREAK),
        ('D', Flags::DECIMAL),
        ('I', Flags::IRQ_DISABLE),
        ('Z', Flags::ZERO),
        ('C', Flags::CARRY),
    ];

    flag_map
        .iter()
        .map(|&(ch, flag)| if flags & flag.bits() != 0 { ch } else { '.' })
        .collect()
}

pub struct CPU {
    pub system_type: SystemType,
    pub cpu_type: CpuType,

    pub bus: Bus,
    pub regs: Registers,
    pub pc: u16,
    pub p: Flags,

    symbol_table: SymbolTable,

    // target_hz: u32,
    last_frame_time: Instant,

    pub entry_point_override: Option<u16>,
}

impl CPU {
    pub fn new(system_type: SystemType, cpu_type: CpuType, _target_hz: u32) -> Self {
        Self {
            system_type,
            cpu_type,
            bus: Bus::new(system_type, cpu_type),
            pc: 0,
            // target_hz,
            p: Flags::from_bits_truncate(0b00110110),
            regs: Registers::default(),
            entry_point_override: None,
            last_frame_time: Instant::now(),
            symbol_table: SymbolTable::new(),
        }
    }

    pub fn resolve_entry_point(&self) -> u16 {
        if let Some(entry) = self.entry_point_override {
            println!("Using manually set entry point: {:#06X}", entry);
            return entry;
        }

        let reset_vector = self.bus.read_word(0xFFFC);
        if reset_vector != 0xFFFF {
            println!("Using Reset Vector entry point: {:#06X}", reset_vector);
            return reset_vector;
        }

        let default_entry = match self.system_type {
            SystemType::AppleIIc => 0xC800,
            SystemType::Generic => 0x0400,
        };

        println!(
            "No valid reset vector, defaulting to {:#06X}",
            default_entry
        );
        default_entry
    }

    pub fn load_rom(&mut self, rom: ROM) {
        self.bus.load_rom(rom);
    }

    pub fn init(&mut self) {
        println!("CPU INIT: Performing cold boot...");

        self.symbol_table.load_symbols();

        self.bus.init_mmu();

        self.bus.interrupts.clear_all();

        if self.system_type == SystemType::AppleIIc {
            self.pc = 0xFF59; // OLDRST
                              // self.pc = 0xFF65; // MON
                              // self.pc = 0xFF69; // MONZ
                              // self.pc = self.bus.read_word(0xFFFC);
            println!("Apple IIc Cold Boot: Entry point set to {:#06X}", self.pc);
        } else {
            self.pc = self.resolve_entry_point();
        }

        self.initialize_registers();
        self.initialize_flags();

        if self.system_type == SystemType::AppleIIc {
            self.initialize_soft_switches();
        }

        println!(
            "Initialization Complete: PC={:#06X}, SP={:#04X}, P={:08b}",
            self.pc,
            self.regs.sp,
            self.p.bits()
        );
    }

    pub fn reset(&mut self) {
        println!("CPU RESET: Performing warm reset...");

        // self.symbol_table.load_symbols();
        self.bus.interrupts.clear_all();

        self.pc = self.resolve_entry_point();

        self.initialize_registers();
        self.initialize_flags();

        if self.system_type == SystemType::AppleIIc {
            self.initialize_soft_switches();
        }

        println!(
            "Reset Complete: PC={:#06X}, SP={:#04X}, P={:08b}",
            self.pc,
            self.regs.sp,
            self.p.bits()
        );
    }

    fn initialize_registers(&mut self) {
        self.regs.a = 0xFF;
        self.regs.x = 0xFF;
        self.regs.y = 0xFF;
        self.regs.sp = 0xFF;
    }

    fn initialize_flags(&mut self) {
        self.p = Flags::from_bits_truncate(0b00110100); // I=1, B=0, U=1
        if self.cpu_type != CpuType::NMOS6502 {
            self.p.remove(Flags::DECIMAL);
        }
    }

    fn initialize_soft_switches(&mut self) {
        println!("Apple IIc: Initializing soft switches...");

        // Reset Soft Switches to Apple IIc Default State
        self.bus.handle_iic_write(0xC000, 0); // 80STORE OFF (Default)
        self.bus.handle_iic_write(0xC054, 0); // Page2 OFF (Default)
        self.bus.handle_iic_write(0xC051, 0); // TEXT ON
                                              //self.bus.handle_iic_write(0xC052, 0); // MIXED OFF
                                              //self.bus.handle_iic_write(0xC057, 0); // PAGE1 OFF

        self.bus.handle_iic_write(0xC028, 0); // ROM Bank 0
        self.bus.handle_iic_write(0xC008, 0); // Ensure ZP/Main Stack is active

        println!("Apple IIc Soft Switches Initialized");
    }

    fn handle_interrupt(&mut self) -> bool {
        let nmi_vector = self.bus.read_word(0xFFFA);
        let reset_vector = self.bus.read_word(0xFFFC);
        let irq_vector = self.bus.read_word(0xFFFE);

        if let Some((interrupt_type, target_pc)) = self
            .bus
            .interrupts
            .handle_interrupt_with_vectors(nmi_vector, reset_vector, irq_vector)
        {
            if self.p.contains(Flags::IRQ_DISABLE) && interrupt_type == InterruptType::IRQ {
                return false;
            }

            if interrupt_type == InterruptType::RST {
                println!("Handling CPU Reset...");
                self.pc = target_pc;
                return true;
            }

            let pushed_pc = match interrupt_type {
                InterruptType::BRK => match self.cpu_type {
                    CpuType::NMOS6502 => self.pc.wrapping_add(1),
                    CpuType::CMOS65C02 | CpuType::WDC65C02S => self.pc.wrapping_add(1),
                },
                _ => self.pc,
            };

            self.push_stack((pushed_pc >> 8) as u8);
            self.push_stack((pushed_pc & 0xFF) as u8);

            let mut pushed_p = self.p;
            pushed_p.insert(Flags::UNUSED);
            pushed_p.set(Flags::BREAK, interrupt_type == InterruptType::BRK);

            self.push_stack(pushed_p.bits());

            self.p.insert(Flags::IRQ_DISABLE);
            self.p.remove(Flags::BREAK);

            if self.cpu_type != CpuType::NMOS6502 {
                self.p.remove(Flags::DECIMAL);
            }

            self.pc = target_pc;

            if interrupt_type == InterruptType::IRQ {
                self.bus.interrupts.irq = false;
                self.bus.interrupts.leave_wait();
                return false;
            }

            if interrupt_type == InterruptType::NMI {
                self.bus.interrupts.nmi = false;
                self.bus.interrupts.leave_wait();
                return false;
            }

            self.bus.interrupts.leave_wait();

            return true;
        }
        false
    }

    fn fetch_byte(&mut self) -> u8 {
        let byte = self.bus.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    fn fetch_word(&mut self) -> u16 {
        let lo = self.fetch_byte() as u16;
        let hi = self.fetch_byte() as u16;
        let word = (hi << 8) | lo;

        word
    }

    fn fetch_indirect_x(&mut self) -> u16 {
        let zp_base = self.fetch_byte().wrapping_add(self.regs.x) as u16;
        let lo = self.bus.read_byte(zp_base) as u16;
        let hi = self.bus.read_byte(zp_base.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    fn fetch_indirect_y(&mut self) -> u16 {
        let zp_addr = self.fetch_byte() as u16;
        let low_byte = self.bus.read_byte(zp_addr) as u16;
        let high_byte = self.bus.read_byte(zp_addr.wrapping_add(1) & 0xFF) as u16;
        let base_addr = (high_byte << 8) | low_byte;

        let addr = base_addr.wrapping_add(self.regs.y as u16);

        // handle page-crossing penalty...
        // if check_page_crossing && (base_addr & 0xFF00) != (addr & 0xFF00) {
        //     self.cycle_count += 1;
        // }

        addr
    }

    fn push_stack(&mut self, value: u8) {
        self.bus.write_byte(0x0100 | self.regs.sp as u16, value);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
    }

    fn pop_stack(&mut self) -> u8 {
        self.regs.sp = self.regs.sp.wrapping_add(1);
        self.bus.read_byte(0x0100 | self.regs.sp as u16)
    }

    fn execute_bit(&mut self, value: u8) {
        // Z Flag: Set if (A & M) == 0
        self.p.set(Flags::ZERO, (self.regs.a & value) == 0);

        // N Flag: Set if Bit 7 of M is 1
        self.p.set(Flags::NEGATIVE, (value & 0x80) != 0);

        // V Flag: Set if Bit 6 of M is 1
        self.p.set(Flags::OVERFLOW, (value & 0x40) != 0);
    }

    pub fn tick(&mut self) {
        self.step();
        // if self.bus.vbl_interrupt.get() & 0x80 != 0 {
        //     println!("VBL Interrupt Detected");
        // }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame_time);

        if elapsed >= Duration::from_millis(16) {
            self.bus.video_update();
            self.last_frame_time = now;
        }

        // self.bus.vbl_interrupt.set(0x00);
    }

    pub fn step(&mut self) {
        if self.handle_interrupt() {
            return;
        }

        if self.bus.interrupts.halted {
            println!("CPU Halted! Exiting...");
            return;
        }

        if self.bus.interrupts.waiting {
            if self.bus.interrupts.irq || self.bus.interrupts.nmi {
                println!(
                    "IRQ/NMI TRIGGERED: IRQ={} NMI={} I={} PC={:#06X}",
                    self.bus.interrupts.irq,
                    self.bus.interrupts.nmi,
                    self.p.contains(Flags::IRQ_DISABLE),
                    self.pc
                );

                self.bus.interrupts.leave_wait();
            } else {
                return;
            }
        }

        let pc = self.pc;

        let instruction = Disassembler::disassemble(&self.bus, pc);

        let opcode = self.fetch_byte();

        self.decode_execute(opcode);

        println!(
            "{} A:{:02X} X:{:02X} Y:{:02X} P:{}[{:02X}] SP:{:02X}[{:02X}] {} {}{}",
            instruction,
            self.regs.a,
            self.regs.x,
            self.regs.y,
            format_flags(self.p.bits()),
            self.p.bits(),
            self.regs.sp,
            self.bus
                .read_byte(0x0100 | ((self.regs.sp.wrapping_add(1)) as u16)),
            self.bus.mmu_mem_state_to_string(),
            self.bus.interrupts.status_string(),
            self.symbol_table.append_symbol(instruction.clone()),
        );
    }

    fn update_zero_and_negative_flags(&mut self, value: u8) {
        self.p.set(Flags::ZERO, value == 0);
        self.p.set(Flags::NEGATIVE, (value & 0b1000_0000) != 0);
    }

    fn adc(&mut self, value: u8) {
        let carry_in = if self.p.contains(Flags::CARRY) { 1 } else { 0 };
        let a_before = self.regs.a;
        let sum_16 = a_before as u16 + value as u16 + carry_in as u16;
        let mut a_after = (sum_16 & 0xFF) as u8;
        let mut carry_out = sum_16 > 0xFF;

        if self.p.contains(Flags::DECIMAL) {
            let mut low_nibble = (a_before & 0x0F)
                .wrapping_add(value & 0x0F)
                .wrapping_add(carry_in);
            let mut high_nibble = (a_before >> 4).wrapping_add(value >> 4);

            if low_nibble > 9 {
                low_nibble = low_nibble.wrapping_sub(10);
                high_nibble = high_nibble.wrapping_add(1);
            }

            if high_nibble > 9 {
                high_nibble = high_nibble.wrapping_sub(10);
                carry_out = true;
            }

            a_after = (high_nibble << 4) | (low_nibble & 0x0F);
        }

        self.regs.a = a_after;

        self.p.set(Flags::CARRY, carry_out);

        let overflow = ((a_before ^ value) & 0x80 == 0) && ((a_before ^ a_after) & 0x80 != 0);
        self.p.set(Flags::OVERFLOW, overflow);

        self.p.set(Flags::ZERO, self.regs.a == 0);
        self.p.set(Flags::NEGATIVE, (self.regs.a & 0x80) != 0);
    }

    fn sbc(&mut self, value: u8) {
        let carry_in = if self.p.contains(Flags::CARRY) { 1 } else { 0 };
        let a_before = self.regs.a;
        let value_complement = !value;

        let binary_result = (a_before as u16) + (value_complement as u16) + carry_in as u16;
        let mut temp_result = (binary_result & 0xFF) as u8;
        let mut did_borrow = binary_result < 0x100;

        if self.p.contains(Flags::DECIMAL) {
            let mut low_nibble = (a_before & 0x0F)
                .wrapping_sub(value & 0x0F)
                .wrapping_sub(1 - carry_in);
            let mut high_nibble = (a_before >> 4).wrapping_sub(value >> 4);

            if (low_nibble & 0x10) != 0 {
                low_nibble = low_nibble.wrapping_sub(6) & 0x0F;
                high_nibble = high_nibble.wrapping_sub(1);
            }

            if high_nibble > 9 {
                high_nibble = high_nibble.wrapping_sub(6) & 0x0F;
                did_borrow = true;
            }

            temp_result = (high_nibble << 4) | (low_nibble & 0x0F);
        }

        self.regs.a = temp_result;

        let carry_set = !did_borrow;
        self.p.set(Flags::CARRY, carry_set);

        let overflow = ((a_before ^ value) & 0x80 != 0) && ((a_before ^ temp_result) & 0x80 != 0);
        self.p.set(Flags::OVERFLOW, overflow);

        self.p.set(Flags::ZERO, self.regs.a == 0);
        self.p.set(Flags::NEGATIVE, self.regs.a & 0x80 != 0);
    }

    fn asl(&mut self, value: u8) -> u8 {
        self.p.set(Flags::CARRY, value & 0b1000_0000 != 0);
        let result = value << 1;
        self.update_zero_and_negative_flags(result);
        result
    }

    fn lsr(&mut self, value: u8) -> u8 {
        self.p.set(Flags::CARRY, value & 0b0000_0001 != 0);
        let result = value >> 1;
        self.update_zero_and_negative_flags(result);
        result
    }

    fn rol(&mut self, value: u8) -> u8 {
        let carry_in = if self.p.contains(Flags::CARRY) { 1 } else { 0 };
        self.p.set(Flags::CARRY, value & 0b1000_0000 != 0);
        let result = (value << 1) | carry_in;
        self.update_zero_and_negative_flags(result);
        result
    }

    fn ror(&mut self, value: u8) -> u8 {
        let carry_in = if self.p.contains(Flags::CARRY) {
            0b1000_0000
        } else {
            0
        };
        self.p.set(Flags::CARRY, value & 0b0000_0001 != 0);
        let result = (value >> 1) | carry_in;
        self.update_zero_and_negative_flags(result);
        result
    }

    fn compare(&mut self, reg: u8, value: u8) {
        let result = reg.wrapping_sub(value);

        self.p.set(Flags::CARRY, reg >= value);
        self.p.set(Flags::ZERO, result == 0);
        self.p.set(Flags::NEGATIVE, (result & 0x80) != 0);
    }

    fn decode_execute(&mut self, opcode: u8) {
        match opcode {
            0xA9 => {
                let value = self.fetch_byte();
                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xA5 => {
                let addr = self.fetch_byte() as u16;
                self.regs.a = self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xAD => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xA2 => {
                let value = self.fetch_byte();
                self.regs.x = value;
                self.update_zero_and_negative_flags(self.regs.x);
            }
            0xA6 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.regs.x = value;
                self.update_zero_and_negative_flags(self.regs.x);
            }
            0xAE => {
                let addr = self.fetch_word();
                self.regs.x = self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0xA0 => {
                let value = self.fetch_byte();
                self.regs.y = value;
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0xA4 => {
                let addr = self.fetch_byte() as u16;
                self.regs.y = self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0xAC => {
                let addr = self.fetch_word();
                self.regs.y = self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0x85 => {
                let addr = self.fetch_byte() as u16;
                self.bus.write_byte(addr, self.regs.a);
            }

            0x8D => {
                let addr = self.fetch_word();
                let value = self.regs.a;
                self.bus.write_byte(addr, value);
            }

            0x86 => {
                let addr = self.fetch_byte() as u16;
                self.bus.write_byte(addr, self.regs.x);
            }

            0x8E => {
                let addr = self.fetch_word();
                self.bus.write_byte(addr, self.regs.x);
            }

            0x84 => {
                let addr = self.fetch_byte() as u16;
                self.bus.write_byte(addr, self.regs.y);
            }

            0x8C => {
                let addr = self.fetch_word();
                self.bus.write_byte(addr, self.regs.y);
            }

            0x29 => {
                let value = self.fetch_byte();
                self.regs.a &= value;
                self.update_zero_and_negative_flags(self.regs.a);
            }
            0x25 => {
                let addr = self.fetch_byte() as u16;
                self.regs.a &= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }
            0x2D => {
                let addr = self.fetch_word();
                self.regs.a &= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x49 => {
                let value = self.fetch_byte();
                self.regs.a ^= value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x45 => {
                let addr = self.fetch_byte() as u16;
                self.regs.a ^= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }
            0x4D => {
                let addr = self.fetch_word();
                self.regs.a ^= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x08 => {
                let mut pushed_p = self.p;
                pushed_p.insert(Flags::UNUSED);
                pushed_p.insert(Flags::BREAK);

                self.push_stack(pushed_p.bits());
            }

            0x05 => {
                let addr = self.fetch_byte() as u16;
                self.regs.a |= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x0D => {
                let addr = self.fetch_word();
                self.regs.a |= self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x24 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.execute_bit(value);
            }

            0x2C => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.execute_bit(value);
            }

            0x34 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(addr);
                self.execute_bit(value);
            }

            0x3C => {
                let addr = self.fetch_word().wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.execute_bit(value);
            }

            0x69 => {
                let value = self.fetch_byte();
                self.adc(value);
            }

            0x65 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0x6D => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0xE9 => {
                let value = self.fetch_byte();
                self.sbc(value);
            }

            0xE5 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0xED => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0xC9 => {
                let value = self.fetch_byte();
                self.compare(self.regs.a, value);
            }

            0xC5 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.a, value);
            }

            0xCD => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.a, value);
            }

            0xE0 => {
                let value = self.fetch_byte();
                self.compare(self.regs.x, value);
            }

            0xE4 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.x, value);
            }

            0xEC => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.x, value);
            }

            0xC0 => {
                let value = self.fetch_byte();
                let result = self.regs.y.wrapping_sub(value);
                self.p.set(Flags::ZERO, self.regs.y == value);
                self.p.set(Flags::NEGATIVE, result & 0x80 != 0);
                self.p.set(Flags::CARRY, self.regs.y >= value);
            }

            0xC4 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.y, value);
            }

            0xCC => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.y, value);
            }

            0x0A => {
                self.regs.a = self.asl(self.regs.a);
            }

            0x06 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = self.asl(value);
                self.bus.write_byte(addr, result);
            }

            0x0E => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                let result = self.asl(value);
                self.bus.write_byte(addr, result);
            }

            0x4A => {
                self.regs.a = self.lsr(self.regs.a);
            }

            0x46 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = self.lsr(value);
                self.bus.write_byte(addr, result);
            }

            0x4E => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                let result = self.lsr(value);
                self.bus.write_byte(addr, result);
            }

            0x2A => {
                self.regs.a = self.rol(self.regs.a);
            }

            0x26 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = self.rol(value);
                self.bus.write_byte(addr, result);
            }

            0x2E => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                let result = self.rol(value);
                self.bus.write_byte(addr, result);
            }

            0x6A => {
                self.regs.a = self.ror(self.regs.a);
            }

            0x66 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = self.ror(value);
                self.bus.write_byte(addr, result);
            }

            0x6E => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                let result = self.ror(value);
                self.bus.write_byte(addr, result);
            }

            0x4C => {
                let target = self.fetch_word();
                self.pc = target;
            }

            0x6C => {
                let addr = self.fetch_word();
                let lo = self.bus.read_byte(addr) as u16;
                let hi = match self.cpu_type {
                    CpuType::NMOS6502 if addr & 0x00FF == 0xFF => {
                        // NMOS 6502 bug: Reads high byte from addr & 0xFF00
                        self.bus.read_byte(addr & 0xFF00) as u16
                    }
                    _ => {
                        // reads high byte from addr + 1
                        self.bus.read_byte(addr.wrapping_add(1)) as u16
                    }
                };

                self.pc = (hi << 8) | lo;
            }

            0x20 => {
                let addr = self.fetch_word();

                self.pc = self.pc.wrapping_sub(1);
                self.push_stack((self.pc >> 8) as u8);
                self.push_stack((self.pc & 0xFF) as u8);

                self.pc = addr;
            }

            0x60 => {
                let low = self.pop_stack() as u16;
                let high = self.pop_stack() as u16;
                let return_addr = (high << 8) | low;
                self.pc = return_addr.wrapping_add(1);
            }

            0x40 => {
                let mut restored_p = Flags::from_bits_truncate(self.pop_stack());
                restored_p.insert(Flags::UNUSED);
                restored_p.remove(Flags::BREAK);
                self.p = restored_p;

                let lo = self.pop_stack() as u16;
                let hi = self.pop_stack() as u16;
                self.pc = (hi << 8) | lo;
            }

            0x80 => {
                let offset = self.fetch_byte() as i8;
                self.pc = self.pc.wrapping_add_signed(offset as i16);
            }

            0x9C => {
                let addr = self.fetch_word();
                self.bus.write_byte(addr, 0x00);
            }

            0x9E => {
                let addr = self.fetch_word() + self.regs.x as u16;
                self.bus.write_byte(addr, 0x00);
            }

            0x64 => {
                let addr = self.fetch_byte() as u16;
                self.bus.write_byte(addr, 0x00);
            }

            0x74 => {
                let addr = (self.fetch_byte() as u16 + self.regs.x as u16) & 0xFF;
                self.bus.write_byte(addr, 0x00);
            }

            0x1C => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.p.set(Flags::ZERO, (value & self.regs.a) == 0);
                self.bus.write_byte(addr, value & !self.regs.a);
            }

            0x14 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.p.set(Flags::ZERO, (value & self.regs.a) == 0);
                self.bus.write_byte(addr, value & !self.regs.a);
            }

            0x0C => {
                let addr = self.fetch_word();
                let value = self.bus.read_byte(addr);
                self.p.set(Flags::ZERO, (value & self.regs.a) == 0);
                self.bus.write_byte(addr, value | self.regs.a);
            }

            0x04 => {
                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                self.p.set(Flags::ZERO, (value & self.regs.a) == 0);
                self.bus.write_byte(addr, value | self.regs.a);
            }

            0xDA => {
                self.push_stack(self.regs.x);
            }

            0xFA => {
                self.regs.x = self.pop_stack();

                if matches!(self.cpu_type, CpuType::CMOS65C02 | CpuType::WDC65C02S) {
                    self.update_zero_and_negative_flags(self.regs.x);
                }
            }

            0x5A => {
                self.push_stack(self.regs.y);
            }

            0x7A => {
                self.regs.y = self.pop_stack();

                if matches!(self.cpu_type, CpuType::CMOS65C02 | CpuType::WDC65C02S) {
                    self.update_zero_and_negative_flags(self.regs.y);
                }
            }

            0x89 => {
                let value = self.fetch_byte();
                self.p.set(Flags::ZERO, (self.regs.a & value) == 0);
            }

            0xCB => {
                if self.cpu_type == CpuType::WDC65C02S {
                    self.bus.interrupts.enter_wait();
                }
            }

            0xDB => {
                if self.cpu_type == CpuType::WDC65C02S {
                    self.bus.interrupts.enter_halt();
                }
            }

            0xD8 => {
                self.p.remove(Flags::DECIMAL);
            }

            0xF8 => {
                self.p.insert(Flags::DECIMAL);
            }

            0x00 => {
                self.bus.interrupts.request_brk();
            }

            0xEA => {
                // NOP
            }

            0x10 => {
                let offset = self.fetch_byte() as i8;

                if !self.p.contains(Flags::NEGATIVE) {
                    self.pc = self.pc.wrapping_add_signed(offset.into());
                }
            }

            0x48 => {
                self.push_stack(self.regs.a);
            }

            0x30 => {
                let offset = self.fetch_byte() as i8;
                if self.p.contains(Flags::NEGATIVE) {
                    self.pc = self.pc.wrapping_add(offset as u16);
                }
            }

            0xBC => {
                let addr = self.fetch_word().wrapping_add(self.regs.x as u16);
                self.regs.y = self.bus.read_byte(addr);
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0xF0 => {
                let offset = self.fetch_byte() as i8;
                if self.p.contains(Flags::ZERO) {
                    self.pc = self.pc.wrapping_add_signed(offset as i16);
                }
            }

            0x38 => {
                self.p.insert(Flags::CARRY);
            }

            0x68 => {
                self.regs.a = self.pop_stack();
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xFE => {
                let addr = self.fetch_word().wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr).wrapping_add(1);
                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0x88 => {
                self.regs.y = self.regs.y.wrapping_sub(1);
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0x9A => {
                self.regs.sp = self.regs.x;
            }

            0xD0 => {
                let offset = self.fetch_byte() as i8;
                if !self.p.contains(Flags::ZERO) {
                    self.pc = self.pc.wrapping_add_signed(offset as i16);
                }
            }

            0xCA => {
                self.regs.x = self.regs.x.wrapping_sub(1);
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0x98 => {
                self.regs.a = self.regs.y;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xAA => {
                self.regs.x = self.regs.a;
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0x18 => {
                self.p.remove(Flags::CARRY);
            }

            0x90 => {
                let offset = self.fetch_byte() as i8;
                if !self.p.contains(Flags::CARRY) {
                    self.pc = self.pc.wrapping_add(offset as u16);
                }
            }

            0xB0 => {
                let offset = self.fetch_byte() as i8;

                if self.p.contains(Flags::CARRY) {
                    self.pc = self.pc.wrapping_add(offset as u16);

                    // page-crossing penalty...
                    // if (old_pc & 0xFF00) != (self.pc & 0xFF00) {
                    //     self.cycle_count += 1;
                    // }
                }
            }

            0xA8 => {
                self.regs.y = self.regs.a;
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0xBA => {
                self.regs.x = self.regs.sp;
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0x8A => {
                self.regs.a = self.regs.x;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x28 => {
                let popped = self.pop_stack();
                let mut restored_p = Flags::from_bits(popped).unwrap_or(Flags::empty());

                restored_p.insert(Flags::UNUSED);
                restored_p.remove(Flags::BREAK);

                self.p = restored_p;
            }

            0x50 => {
                let offset = self.fetch_byte() as i8;
                if !self.p.contains(Flags::OVERFLOW) {
                    self.pc = self.pc.wrapping_add(offset as u16);
                }
            }

            0x70 => {
                let offset = self.fetch_byte() as i8;
                if self.p.contains(Flags::OVERFLOW) {
                    self.pc = self.pc.wrapping_add(offset as u16);
                }
            }

            0xE8 => {
                self.regs.x = self.regs.x.wrapping_add(1);
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0xC8 => {
                self.regs.y = self.regs.y.wrapping_add(1);
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0xBD => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);

                let value = self.bus.read_byte(addr);
                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x58 => {
                self.p.remove(Flags::IRQ_DISABLE);
            }

            0x78 => {
                self.p.insert(Flags::IRQ_DISABLE);
            }

            0xB8 => {
                self.p.remove(Flags::OVERFLOW);
            }

            0xB6 => {
                let base_addr = self.fetch_byte() as u16;
                let addr = (base_addr.wrapping_add(self.regs.y as u16) & 0x00FF) as u16;
                let value = self.bus.read_byte(addr);

                self.regs.x = value;
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0x99 => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.y as u16);

                let addr = if base_addr < 0x100 { addr & 0xFF } else { addr };

                self.bus.write_byte(addr, self.regs.a);
            }

            0xD9 => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.a, value);
            }

            0xBE => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.x = value;
                self.update_zero_and_negative_flags(self.regs.x);
            }

            0x96 => {
                let base_addr = self.fetch_byte() as u16;
                let addr = (base_addr.wrapping_add(self.regs.y as u16)) & 0x00FF;

                self.bus.write_byte(addr, self.regs.x);
            }

            0xB9 => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xB4 => {
                let base = self.fetch_byte();
                let addr = base.wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(addr);
                self.regs.y = value;
                self.update_zero_and_negative_flags(self.regs.y);
            }

            0x9D => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);

                self.bus.write_byte(addr, self.regs.a);
            }

            0xDD => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);

                self.compare(self.regs.a, value);
            }

            0x94 => {
                let base = self.fetch_byte();
                let addr = base.wrapping_add(self.regs.x) as u16;
                self.bus.write_byte(addr, self.regs.y);
            }

            0xD5 => {
                let base = self.fetch_byte();
                let addr = (base.wrapping_add(self.regs.x) & 0x00FF) as u16;
                let value = self.bus.read_byte(addr);
                self.compare(self.regs.a, value);
            }

            0xB5 => {
                let base = self.fetch_byte();
                let addr = (base.wrapping_add(self.regs.x) & 0x00FF) as u16;
                let value = self.bus.read_byte(addr);
                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xC1 => {
                let base_addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let addr = self.bus.read_word(base_addr);
                let value = self.bus.read_byte(addr);

                self.compare(self.regs.a, value);
            }

            0xD1 => {
                let base_addr = self.fetch_byte() as u16;
                let addr = self
                    .bus
                    .read_word(base_addr)
                    .wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);

                self.compare(self.regs.a, value);
            }

            0x95 => {
                let base_address = self.fetch_byte() as u16;
                let effective_address = (base_address.wrapping_add(self.regs.x as u16)) & 0x00FF;
                self.bus.write_byte(effective_address, self.regs.a);
            }

            0xB1 => {
                let base_address = self.fetch_byte();
                let pointer_address = self.bus.read_word(base_address as u16 & 0xFF);

                let effective_address = pointer_address.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(effective_address);

                self.regs.a = value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x91 => {
                let base_address = self.fetch_byte();
                let pointer_address = self.bus.read_word(base_address as u16 & 0xFF);

                let effective_address = pointer_address.wrapping_add(self.regs.y as u16);
                self.bus.write_byte(effective_address, self.regs.a);
            }

            0xA1 => {
                let base_address = self.fetch_byte();
                let zp_addr = base_address.wrapping_add(self.regs.x) & 0xFF;
                let effective_address = self.bus.read_word(zp_addr as u16);
                self.regs.a = self.bus.read_byte(effective_address);
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x81 => {
                let base_address = self.fetch_byte();
                let zp_addr = (base_address.wrapping_add(self.regs.x)) & 0xFF;
                let effective_address = self.bus.read_word(zp_addr as u16);

                self.bus.write_byte(effective_address, self.regs.a);
            }

            0x16 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let mut value = self.bus.read_byte(addr);

                let carry = value & 0x80 != 0;
                value <<= 1;

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, carry);
                self.update_zero_and_negative_flags(value);
            }

            0x56 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let mut value = self.bus.read_byte(addr);

                let carry = value & 0x01 != 0;
                value >>= 1;

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, carry);
                self.update_zero_and_negative_flags(value);
            }

            0x36 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let mut value = self.bus.read_byte(addr);

                let old_carry = self.p.contains(Flags::CARRY);
                let new_carry = value & 0x80 != 0;

                value = (value << 1) | if old_carry { 1 } else { 0 };

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0x76 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let mut value = self.bus.read_byte(addr);

                let old_carry = self.p.contains(Flags::CARRY);
                let new_carry = value & 0x01 != 0;

                value = (value >> 1) | if old_carry { 0x80 } else { 0 };

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0x1E => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let mut value = self.bus.read_byte(addr);

                let new_carry = value & 0x80 != 0;

                value <<= 1;

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0x5E => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let mut value = self.bus.read_byte(addr);

                let new_carry = value & 0x01 != 0;

                value >>= 1;

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0x3E => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let mut value = self.bus.read_byte(addr);

                let new_carry = (value & 0x80) != 0;

                value = (value << 1) | (self.p.contains(Flags::CARRY) as u8);

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0x7E => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let mut value = self.bus.read_byte(addr);

                let new_carry = (value & 0x01) != 0;

                value = (value >> 1) | ((self.p.contains(Flags::CARRY) as u8) << 7);

                self.bus.write_byte(addr, value);

                self.p.set(Flags::CARRY, new_carry);
                self.update_zero_and_negative_flags(value);
            }

            0xE6 => {
                let addr = self.fetch_byte() as u16;
                let mut value = self.bus.read_byte(addr);

                value = value.wrapping_add(1);

                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0xC6 => {
                let addr = self.fetch_byte() as u16;
                let mut value = self.bus.read_byte(addr);

                value = value.wrapping_sub(1);

                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0xEE => {
                let addr = self.fetch_word();
                let mut value = self.bus.read_byte(addr);

                value = value.wrapping_add(1);

                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0xCE => {
                let addr = self.fetch_word();
                let mut value = self.bus.read_byte(addr);

                value = value.wrapping_sub(1);

                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0xF6 => {
                let base_addr = self.fetch_byte();
                let addr = (base_addr.wrapping_add(self.regs.x)) as u8 as u16;

                let mut value = self.bus.read_byte(addr);
                value = value.wrapping_add(1);

                self.bus.write_byte(addr, value);
                self.update_zero_and_negative_flags(value);
            }

            0x09 => {
                let value = self.fetch_byte();
                self.regs.a |= value;
                self.update_zero_and_negative_flags(self.regs.a);
            }

            0xD6 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(addr).wrapping_sub(1);
                self.bus.write_byte(addr, value);

                self.update_zero_and_negative_flags(value);
            }

            0xDE => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr).wrapping_sub(1);
                self.bus.write_byte(addr, value);

                self.update_zero_and_negative_flags(value);
            }

            0x35 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x);
                let value = self.bus.read_byte(addr as u16);
                self.regs.a &= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x3D => {
                let addr = self.fetch_word().wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a &= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x39 => {
                let addr = self.fetch_word().wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a &= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x21 => {
                let zp_addr = self.fetch_byte().wrapping_add(self.regs.x);
                let addr = self.bus.read_word(zp_addr as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a &= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x31 => {
                let zp_addr = self.fetch_byte() as u16;
                let base_addr = self.bus.read_word(zp_addr);
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a &= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x55 => {
                let zp_addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(zp_addr);
                self.regs.a ^= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x5D => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a ^= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x59 => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a ^= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x41 => {
                let zero_page_addr = self.fetch_byte().wrapping_add(self.regs.x);
                let addr = self.bus.read_word(zero_page_addr as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a ^= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x51 => {
                let zero_page_addr = self.fetch_byte() as u16;
                let base_addr = self.bus.read_word(zero_page_addr);
                let addr = base_addr.wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a ^= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x15 => {
                let addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(addr);
                self.regs.a |= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x1D => {
                let addr = self.fetch_word().wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a |= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x19 => {
                let addr = self.fetch_word().wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a |= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x01 => {
                let zp_addr = self.fetch_byte().wrapping_add(self.regs.x);
                let addr = self.bus.read_word(zp_addr as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a |= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x11 => {
                let zp_addr = self.fetch_byte() as u16;
                let addr = self.bus.read_word(zp_addr).wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.regs.a |= value;

                self.update_zero_and_negative_flags(self.regs.a);
            }

            0x75 => {
                let zp_addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(zp_addr);
                self.adc(value);
            }

            0xF5 => {
                let zp_addr = self.fetch_byte().wrapping_add(self.regs.x) as u16;
                let value = self.bus.read_byte(zp_addr);
                self.sbc(value);
            }

            0x7D => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0xFD => {
                let base_addr = self.fetch_word();
                let addr = base_addr.wrapping_add(self.regs.x as u16);
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0x79 => {
                let addr = self.fetch_word().wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0xF9 => {
                let addr = self.fetch_word().wrapping_add(self.regs.y as u16);
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0x61 => {
                let addr = self.fetch_indirect_x();
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0xF1 => {
                let addr = self.fetch_indirect_y();
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0xE1 => {
                let addr = self.fetch_indirect_x();
                let value = self.bus.read_byte(addr);
                self.sbc(value);
            }

            0x71 => {
                let addr = self.fetch_indirect_y();
                let value = self.bus.read_byte(addr);
                self.adc(value);
            }

            0x0F | 0x1F | 0x2F | 0x3F | 0x4F | 0x5F | 0x6F | 0x7F => {
                let zp_addr = self.fetch_byte() as u16;
                let rel_offset = self.fetch_byte() as i8 as i16;
                let value = self.bus.read_byte(zp_addr);
                let bit = 1 << ((opcode.wrapping_sub(0x0F)) / 0x10);

                if (value & bit) == 0 {
                    self.pc = self.pc.wrapping_add_signed(rel_offset);
                }
            }

            0x8F | 0x9F | 0xAF | 0xBF | 0xCF | 0xDF | 0xEF | 0xFF => {
                let zp_addr = self.fetch_byte() as u16;
                let rel_offset = self.fetch_byte() as i8 as i16;
                let value = self.bus.read_byte(zp_addr);
                let bit = 1 << ((opcode.wrapping_sub(0x8F)) / 0x10);

                if (value & bit) != 0 {
                    self.pc = self.pc.wrapping_add_signed(rel_offset);
                }
            }

            0x7C => {
                let base = self.fetch_word();
                let addr = base.wrapping_add(self.regs.x as u16);
                let lo = self.bus.read_byte(addr) as u16;
                let hi = self.bus.read_byte(addr.wrapping_add(1)) as u16;
                self.pc = (hi << 8) | lo;
            }

            0x03 | 0x13 | 0x23 | 0x33 | 0x43 | 0x53 | 0x63 | 0x73 | 0x83 | 0x93 | 0xA3 | 0xB3
            | 0xC3 | 0xD3 | 0xE3 | 0xF3 | 0x0B | 0x1B | 0x2B | 0x3B | 0x4B | 0x5B | 0x6B | 0x7B
            | 0x8B | 0x9B | 0xAB | 0xBB | 0xEB | 0xFB => {}

            0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xC2 | 0xE2 => {
                self.pc = self.pc.wrapping_add(1);
            }

            0x44 => {
                self.pc = self.pc.wrapping_add(1);
            }

            0x54 | 0xD4 | 0xF4 => {
                self.pc = self.pc.wrapping_add(1);
            }

            0x5C => {
                self.pc = self.pc.wrapping_add(2);
            }

            0xDC | 0xFC => {
                self.pc = self.pc.wrapping_add(2);
            }

            0x1A => {
                if self.cpu_type != CpuType::NMOS6502 {
                    self.regs.a = self.regs.a.wrapping_add(1);
                    self.update_zero_and_negative_flags(self.regs.a);
                }
            }

            0x3A => {
                if self.cpu_type != CpuType::NMOS6502 {
                    self.regs.a = self.regs.a.wrapping_sub(1);
                    self.update_zero_and_negative_flags(self.regs.a);
                }
            }

            0xB2 => {
                if self.cpu_type != CpuType::NMOS6502 {
                    let zp_addr = self.fetch_byte() as u16;
                    let indirect_addr = self.bus.read_word(zp_addr);
                    self.regs.a = self.bus.read_byte(indirect_addr);
                    self.update_zero_and_negative_flags(self.regs.a);
                }
            }

            0x92 => {
                if self.cpu_type != CpuType::NMOS6502 {
                    let zp_addr = self.fetch_byte() as u16;
                    let indirect_addr = self.bus.read_word(zp_addr);
                    self.bus.write_byte(indirect_addr, self.regs.a);
                }
            }

            0x07 | 0x17 | 0x27 | 0x37 | 0x47 | 0x57 | 0x67 | 0x77 => {
                let bit_n = (opcode >> 4) & 0b111;
                let mask = !(1 << bit_n);

                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = value & mask;
                self.bus.write_byte(addr, result);
            }

            0x87 | 0x97 | 0xA7 | 0xB7 | 0xC7 | 0xD7 | 0xE7 | 0xF7 => {
                let bit_n = (opcode >> 4) & 0b111;
                let mask = 1 << bit_n;

                let addr = self.fetch_byte() as u16;
                let value = self.bus.read_byte(addr);
                let result = value | mask;
                self.bus.write_byte(addr, result);
            }

            0xD2 => {
                let addr_ptr = self.fetch_byte() as u16;
                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                let result = self.regs.a.wrapping_sub(value);

                self.p.set(Flags::CARRY, self.regs.a >= value);
                self.p.set(Flags::ZERO, self.regs.a == value);
                self.p.set(Flags::NEGATIVE, (result & 0x80) != 0);
            }

            0x32 => {
                let addr_ptr = self.fetch_byte() as u16;
                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                self.regs.a &= value;

                self.p.set(Flags::ZERO, self.regs.a == 0);
                self.p.set(Flags::NEGATIVE, (self.regs.a & 0x80) != 0);
            }

            0x52 => {
                let addr_ptr = self.fetch_byte() as u16;

                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                self.regs.a ^= value;

                self.p.set(Flags::ZERO, self.regs.a == 0);
                self.p.set(Flags::NEGATIVE, (self.regs.a & 0x80) != 0);
            }

            0x12 => {
                let addr_ptr = self.fetch_byte() as u16;

                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                self.regs.a |= value;

                self.p.set(Flags::ZERO, self.regs.a == 0);
                self.p.set(Flags::NEGATIVE, (self.regs.a & 0x80) != 0);
            }

            0x72 => {
                let addr_ptr = self.fetch_byte() as u16;

                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                self.adc(value);
            }

            0xF2 => {
                let addr_ptr = self.fetch_byte() as u16;

                let addr = self.bus.read_word(addr_ptr);
                let value = self.bus.read_byte(addr);

                self.sbc(value);
            }
        }
    }
}
