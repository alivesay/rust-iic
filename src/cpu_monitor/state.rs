// Plain data types used across the CPU monitor: trace entries, watches,
// and the per-frame CPU state snapshot the host passes in.

use crate::cpu::Flags;

pub const MAX_TRACE_ENTRIES: usize = 2000;
pub const MAX_WATCHES: usize = 16;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct CpuTraceEntry {
    pub pc: u16,
    pub opcode: u8,
    pub operand1: u8,
    pub operand2: u8,
    pub instruction_len: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub cycles: u64,
}

impl Default for CpuTraceEntry {
    fn default() -> Self {
        Self {
            pc: 0,
            opcode: 0,
            operand1: 0,
            operand2: 0,
            instruction_len: 1,
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFF,
            p: 0x34,
            cycles: 0,
        }
    }
}

impl CpuTraceEntry {
    // Format flags as `NV-BDIZC` style (caps = set, lower = clear).
    pub fn format_flags(&self) -> String {
        let flags = Flags::from_bits_truncate(self.p);
        format!(
            "{}{}{}{}{}{}{}{}",
            if flags.contains(Flags::NEGATIVE) { 'N' } else { 'n' },
            if flags.contains(Flags::OVERFLOW) { 'V' } else { 'v' },
            '-',
            if flags.contains(Flags::BREAK) { 'B' } else { 'b' },
            if flags.contains(Flags::DECIMAL) { 'D' } else { 'd' },
            if flags.contains(Flags::IRQ_DISABLE) { 'I' } else { 'i' },
            if flags.contains(Flags::ZERO) { 'Z' } else { 'z' },
            if flags.contains(Flags::CARRY) { 'C' } else { 'c' },
        )
    }

    pub fn format_bytes(&self) -> String {
        match self.instruction_len {
            1 => format!("{:02X}      ", self.opcode),
            2 => format!("{:02X} {:02X}   ", self.opcode, self.operand1),
            3 => format!("{:02X} {:02X} {:02X}", self.opcode, self.operand1, self.operand2),
            _ => format!("{:02X}      ", self.opcode),
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct MemoryWatch {
    pub address: u16,
    pub size: u8,
    label: [u8; 16],
    label_len: u8,
}

impl Default for MemoryWatch {
    fn default() -> Self {
        Self {
            address: 0,
            size: 1,
            label: [0; 16],
            label_len: 0,
        }
    }
}

impl MemoryWatch {
    pub fn new(address: u16, size: u8, label: &str) -> Self {
        let mut watch = Self {
            address,
            size: size.min(4),
            label: [0; 16],
            label_len: 0,
        };
        let bytes = label.as_bytes();
        let len = bytes.len().min(16);
        watch.label[..len].copy_from_slice(&bytes[..len]);
        watch.label_len = len as u8;
        watch
    }

    pub fn label_str(&self) -> &str {
        std::str::from_utf8(&self.label[..self.label_len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuState {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub cycles: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct IouSnapshot {
    pub mem_state: u8,
    pub video_mode: u8,
    pub is_80store: bool,
    pub ioudis: bool,
    pub col80_switch: bool,
    pub disk35_mode: bool,
    pub self_test: bool,
    pub scan_cycle: u64,
    pub floating_bus: u8,
    pub irq_pending: bool,
    pub nmi_pending: bool,
    pub mouse_x_int: bool,
    pub mouse_y_int: bool,
    pub mouse_vbl_int: bool,
    pub mouse_xy_mask: bool,
    pub mouse_vbl_mask: bool,
    pub mouse_x: u16,
    pub mouse_y: u16,
    pub mouse_button0: bool,
    pub mouse_button1: bool,
    pub kbd_last_key: u8,
    pub kbd_strobe: bool,
    pub kbd_queued: u16,
    pub kbd_held: u16,

    pub scc_crossloop: bool,
    pub scc_a: crate::device::scc::SccChannelSnap,
    pub scc_b: crate::device::scc::SccChannelSnap,

    pub softswitches: [Option<u8>; 256],

    pub recent_accesses: [IouAccessSample; 32],
    pub recent_access_count: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IouAccessSample {
    pub addr: u16,
    pub pc: u16,
    pub cycle: u64,
    pub value: u8,
    pub write: bool,
}

impl Default for IouSnapshot {
    fn default() -> Self {
        Self {
            mem_state: 0,
            video_mode: 0,
            is_80store: false,
            ioudis: false,
            col80_switch: false,
            disk35_mode: false,
            self_test: false,
            scan_cycle: 0,
            floating_bus: 0,
            irq_pending: false,
            nmi_pending: false,
            mouse_x_int: false,
            mouse_y_int: false,
            mouse_vbl_int: false,
            mouse_xy_mask: false,
            mouse_vbl_mask: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button0: false,
            mouse_button1: false,
            kbd_last_key: 0,
            kbd_strobe: false,
            kbd_queued: 0,
            kbd_held: 0,
            scc_crossloop: false,
            scc_a: crate::device::scc::SccChannelSnap::default(),
            scc_b: crate::device::scc::SccChannelSnap::default(),
            softswitches: [None; 256],
            recent_accesses: [IouAccessSample {
                addr: 0,
                pc: 0,
                cycle: 0,
                value: 0,
                write: false,
            }; 32],
            recent_access_count: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DevicesSnapshot {
    pub drive_active: [bool; 4],
    pub drive_present: [bool; 4],
    pub drive_write_protect: [bool; 4],
    pub drive_head_qt: [u16; 4],
    pub iwm_motor_on: bool,
    pub iwm_motor_on35: bool,
    pub iwm_drive_select: u8,
    pub iwm_phases: u8,
    pub iwm_write_mode: bool,
    pub iwm_head35: u8,
    pub speaker_scope: Vec<f32>,
    pub mockingboard1_scope: Vec<f32>,
    pub mockingboard2_scope: Vec<f32>,
    pub mockingboard1_enabled: bool,
    pub mockingboard2_enabled: bool,
}
