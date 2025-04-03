use regex::Regex;
use std::collections::HashMap;

use crate::bus::Bus;

pub struct SymbolTable {
    symbols: HashMap<u16, String>,
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            symbols: HashMap::new(),
        }
    }

    pub fn append_symbol(&self, disassembly: String) -> String {
        let re = Regex::new(r"\$([0-9A-F]{4})").unwrap();
        let mut updated_disassembly = disassembly.clone();

        for cap in re.captures_iter(&disassembly) {
            if let Some(hex_str) = cap.get(1) {
                if let Ok(addr) = u16::from_str_radix(hex_str.as_str(), 16) {
                    if let Some(symbol) = self.symbols.get(&addr) {
                        updated_disassembly = format!(" ; {}", symbol);
                    } else {
                        updated_disassembly = "".to_string();
                    }
                } else {
                    println!("Invalid hex conversion: {}", hex_str.as_str());
                }
            }
        }

        updated_disassembly
    }

    pub fn load_symbols(&mut self) {
        let data = include_str!("../APPLE2E.SYM").replace("\r", "");

        for line in data.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(address) = u16::from_str_radix(parts[0], 16) {
                    self.symbols.remove(&address);
                    self.symbols.insert(address, parts[1].to_string());
                }
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum AddressingMode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    ZeroPageIndirect,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndirectX,
    IndirectY,
    Relative,
    IndirectAbsolute,
}

impl AddressingMode {
    pub fn operand_bytes(&self) -> usize {
        match self {
            AddressingMode::Implied | AddressingMode::Accumulator => 0,
            AddressingMode::Immediate
            | AddressingMode::ZeroPage
            | AddressingMode::ZeroPageX
            | AddressingMode::ZeroPageY
            | AddressingMode::ZeroPageIndirect
            | AddressingMode::Relative
            | AddressingMode::IndirectX
            | AddressingMode::IndirectY => 1,
            AddressingMode::Absolute
            | AddressingMode::AbsoluteX
            | AddressingMode::AbsoluteY
            | AddressingMode::Indirect
            | AddressingMode::IndirectAbsolute => 2,
        }
    }
}

const OPCODES: [(u8, &str, AddressingMode); 256] = [
    (0x00, "BRK", AddressingMode::Implied),
    (0x01, "ORA", AddressingMode::IndirectX),
    (0x02, "KIL", AddressingMode::Implied),
    (0x03, "KIL", AddressingMode::Implied),
    (0x04, "TSB", AddressingMode::ZeroPage),
    (0x05, "ORA", AddressingMode::ZeroPage),
    (0x06, "ASL", AddressingMode::ZeroPage),
    (0x07, "RMB0", AddressingMode::ZeroPage),
    (0x08, "PHP", AddressingMode::Implied),
    (0x09, "ORA", AddressingMode::Immediate),
    (0x0A, "ASL", AddressingMode::Accumulator),
    (0x0B, "NOP", AddressingMode::Implied),
    (0x0C, "TSB", AddressingMode::Absolute),
    (0x0D, "ORA", AddressingMode::Absolute),
    (0x0E, "ASL", AddressingMode::Absolute),
    (0x0F, "BBR0", AddressingMode::ZeroPage),
    (0x10, "BPL", AddressingMode::Relative),
    (0x11, "ORA", AddressingMode::IndirectY),
    (0x12, "ORA", AddressingMode::ZeroPageIndirect),
    (0x13, "NOP", AddressingMode::Implied),
    (0x14, "TRB", AddressingMode::ZeroPage),
    (0x15, "ORA", AddressingMode::ZeroPageX),
    (0x16, "ASL", AddressingMode::ZeroPageX),
    (0x17, "RMB1", AddressingMode::ZeroPage),
    (0x18, "CLC", AddressingMode::Implied),
    (0x19, "ORA", AddressingMode::AbsoluteY),
    (0x1A, "INA", AddressingMode::Implied),
    (0x1B, "NOP", AddressingMode::Implied),
    (0x1C, "TRB", AddressingMode::Absolute),
    (0x1D, "ORA", AddressingMode::AbsoluteX),
    (0x1E, "ASL", AddressingMode::AbsoluteX),
    (0x1F, "BBR1", AddressingMode::ZeroPage),
    (0x20, "JSR", AddressingMode::Absolute),
    (0x21, "AND", AddressingMode::IndirectX),
    (0x22, "KIL", AddressingMode::Implied),
    (0x23, "NOP", AddressingMode::Implied),
    (0x24, "BIT", AddressingMode::ZeroPage),
    (0x25, "AND", AddressingMode::ZeroPage),
    (0x26, "ROL", AddressingMode::ZeroPage),
    (0x27, "RMB2", AddressingMode::ZeroPage),
    (0x28, "PLP", AddressingMode::Implied),
    (0x29, "AND", AddressingMode::Immediate),
    (0x2A, "ROL", AddressingMode::Accumulator),
    (0x2B, "NOP", AddressingMode::Implied),
    (0x2C, "BIT", AddressingMode::Absolute),
    (0x2D, "AND", AddressingMode::Absolute),
    (0x2E, "ROL", AddressingMode::Absolute),
    (0x2F, "BBR2", AddressingMode::ZeroPage),
    (0x30, "BMI", AddressingMode::Relative),
    (0x31, "AND", AddressingMode::IndirectY),
    (0x32, "AND", AddressingMode::ZeroPageIndirect),
    (0x33, "NOP", AddressingMode::Implied),
    (0x34, "BIT", AddressingMode::ZeroPageX),
    (0x35, "AND", AddressingMode::ZeroPageX),
    (0x36, "ROL", AddressingMode::ZeroPageX),
    (0x37, "RMB3", AddressingMode::ZeroPage),
    (0x38, "SEC", AddressingMode::Implied),
    (0x39, "AND", AddressingMode::AbsoluteY),
    (0x3A, "DEA", AddressingMode::Implied),
    (0x3B, "NOP", AddressingMode::Implied),
    (0x3C, "BIT", AddressingMode::AbsoluteX),
    (0x3D, "AND", AddressingMode::AbsoluteX),
    (0x3E, "ROL", AddressingMode::AbsoluteX),
    (0x3F, "BBR3", AddressingMode::ZeroPage),
    (0x40, "RTI", AddressingMode::Implied),
    (0x41, "EOR", AddressingMode::IndirectX),
    (0x42, "KIL", AddressingMode::Implied),
    (0x43, "NOP", AddressingMode::Implied),
    (0x44, "NOP", AddressingMode::Implied),
    (0x45, "EOR", AddressingMode::ZeroPage),
    (0x46, "LSR", AddressingMode::ZeroPage),
    (0x47, "RMB4", AddressingMode::ZeroPage),
    (0x48, "PHA", AddressingMode::Implied),
    (0x49, "EOR", AddressingMode::Immediate),
    (0x4A, "LSR", AddressingMode::Accumulator),
    (0x4B, "NOP", AddressingMode::Implied),
    (0x4C, "JMP", AddressingMode::Absolute),
    (0x4D, "EOR", AddressingMode::Absolute),
    (0x4E, "LSR", AddressingMode::Absolute),
    (0x4F, "BBR4", AddressingMode::ZeroPage),
    (0x50, "BVC", AddressingMode::Relative),
    (0x51, "EOR", AddressingMode::IndirectY),
    (0x52, "EOR", AddressingMode::ZeroPageIndirect),
    (0x53, "NOP", AddressingMode::Implied),
    (0x54, "NOP", AddressingMode::Implied),
    (0x55, "EOR", AddressingMode::ZeroPageX),
    (0x56, "LSR", AddressingMode::ZeroPageX),
    (0x57, "RMB5", AddressingMode::ZeroPage),
    (0x58, "CLI", AddressingMode::Implied),
    (0x59, "EOR", AddressingMode::AbsoluteY),
    (0x5A, "PHY", AddressingMode::Implied),
    (0x5B, "NOP", AddressingMode::Implied),
    (0x5C, "JMP", AddressingMode::IndirectAbsolute),
    (0x5D, "EOR", AddressingMode::AbsoluteX),
    (0x5E, "LSR", AddressingMode::AbsoluteX),
    (0x5F, "BBR5", AddressingMode::ZeroPage),
    (0x60, "RTS", AddressingMode::Implied),
    (0x61, "ADC", AddressingMode::IndirectX),
    (0x62, "KIL", AddressingMode::Implied),
    (0x63, "NOP", AddressingMode::Implied),
    (0x64, "STZ", AddressingMode::ZeroPage),
    (0x65, "ADC", AddressingMode::ZeroPage),
    (0x66, "ROR", AddressingMode::ZeroPage),
    (0x67, "RMB6", AddressingMode::ZeroPage),
    (0x68, "PLA", AddressingMode::Implied),
    (0x69, "ADC", AddressingMode::Immediate),
    (0x6A, "ROR", AddressingMode::Accumulator),
    (0x6B, "NOP", AddressingMode::Implied),
    (0x6C, "JMP", AddressingMode::Indirect),
    (0x6D, "ADC", AddressingMode::Absolute),
    (0x6E, "ROR", AddressingMode::Absolute),
    (0x6F, "BBR6", AddressingMode::ZeroPage),
    (0x70, "BVS", AddressingMode::Relative),
    (0x71, "ADC", AddressingMode::IndirectY),
    (0x72, "ADC", AddressingMode::ZeroPageIndirect),
    (0x73, "NOP", AddressingMode::Implied),
    (0x74, "STZ", AddressingMode::ZeroPageX),
    (0x75, "ADC", AddressingMode::ZeroPageX),
    (0x76, "ROR", AddressingMode::ZeroPageX),
    (0x77, "RMB7", AddressingMode::ZeroPage),
    (0x78, "SEI", AddressingMode::Implied),
    (0x79, "ADC", AddressingMode::AbsoluteY),
    (0x7A, "PLY", AddressingMode::Implied),
    (0x7B, "NOP", AddressingMode::Implied),
    (0x7C, "JMP", AddressingMode::IndirectAbsolute),
    (0x7D, "ADC", AddressingMode::AbsoluteX),
    (0x7E, "ROR", AddressingMode::AbsoluteX),
    (0x7F, "BBR7", AddressingMode::ZeroPage),
    (0x80, "BRA", AddressingMode::Relative),
    (0x81, "STA", AddressingMode::IndirectX),
    (0x82, "NOP", AddressingMode::Implied),
    (0x83, "NOP", AddressingMode::Implied),
    (0x84, "STY", AddressingMode::ZeroPage),
    (0x85, "STA", AddressingMode::ZeroPage),
    (0x86, "STX", AddressingMode::ZeroPage),
    (0x87, "SMB0", AddressingMode::ZeroPage),
    (0x88, "DEY", AddressingMode::Implied),
    (0x89, "BIT", AddressingMode::Immediate),
    (0x8A, "TXA", AddressingMode::Implied),
    (0x8B, "NOP", AddressingMode::Implied),
    (0x8C, "STY", AddressingMode::Absolute),
    (0x8D, "STA", AddressingMode::Absolute),
    (0x8E, "STX", AddressingMode::Absolute),
    (0x8F, "BBS0", AddressingMode::ZeroPage),
    (0x90, "BCC", AddressingMode::Relative),
    (0x91, "STA", AddressingMode::IndirectY),
    (0x92, "STA", AddressingMode::ZeroPageIndirect),
    (0x93, "NOP", AddressingMode::Implied),
    (0x94, "STY", AddressingMode::ZeroPageX),
    (0x95, "STA", AddressingMode::ZeroPageX),
    (0x96, "STX", AddressingMode::ZeroPageY),
    (0x97, "SMB1", AddressingMode::ZeroPage),
    (0x98, "TYA", AddressingMode::Implied),
    (0x99, "STA", AddressingMode::AbsoluteY),
    (0x9A, "TXS", AddressingMode::Implied),
    (0x9B, "NOP", AddressingMode::Implied),
    (0x9C, "STZ", AddressingMode::Absolute),
    (0x9D, "STA", AddressingMode::AbsoluteX),
    (0x9E, "STZ", AddressingMode::AbsoluteX),
    (0x9F, "BBS1", AddressingMode::ZeroPage),
    (0xA0, "LDY", AddressingMode::Immediate),
    (0xA1, "LDA", AddressingMode::IndirectX),
    (0xA2, "LDX", AddressingMode::Immediate),
    (0xA3, "NOP", AddressingMode::Implied),
    (0xA4, "LDY", AddressingMode::ZeroPage),
    (0xA5, "LDA", AddressingMode::ZeroPage),
    (0xA6, "LDX", AddressingMode::ZeroPage),
    (0xA7, "SMB2", AddressingMode::ZeroPage),
    (0xA8, "TAY", AddressingMode::Implied),
    (0xA9, "LDA", AddressingMode::Immediate),
    (0xAA, "TAX", AddressingMode::Implied),
    (0xAB, "NOP", AddressingMode::Implied),
    (0xAC, "LDY", AddressingMode::Absolute),
    (0xAD, "LDA", AddressingMode::Absolute),
    (0xAE, "LDX", AddressingMode::Absolute),
    (0xAF, "BBS2", AddressingMode::ZeroPage),
    (0xB0, "BCS", AddressingMode::Relative),
    (0xB1, "LDA", AddressingMode::IndirectY),
    (0xB2, "LDA", AddressingMode::ZeroPageIndirect),
    (0xB3, "NOP", AddressingMode::Implied),
    (0xB4, "LDY", AddressingMode::ZeroPageX),
    (0xB5, "LDA", AddressingMode::ZeroPageX),
    (0xB6, "LDX", AddressingMode::ZeroPageY),
    (0xB7, "SMB3", AddressingMode::ZeroPage),
    (0xB8, "CLV", AddressingMode::Implied),
    (0xB9, "LDA", AddressingMode::AbsoluteY),
    (0xBA, "TSX", AddressingMode::Implied),
    (0xBB, "NOP", AddressingMode::Implied),
    (0xBC, "LDY", AddressingMode::AbsoluteX),
    (0xBD, "LDA", AddressingMode::AbsoluteX),
    (0xBE, "LDX", AddressingMode::AbsoluteY),
    (0xBF, "BBS3", AddressingMode::ZeroPage),
    (0xC0, "CPY", AddressingMode::Immediate),
    (0xC1, "CMP", AddressingMode::IndirectX),
    (0xC2, "NOP", AddressingMode::Implied),
    (0xC3, "NOP", AddressingMode::Implied),
    (0xC4, "CPY", AddressingMode::ZeroPage),
    (0xC5, "CMP", AddressingMode::ZeroPage),
    (0xC6, "DEC", AddressingMode::ZeroPage),
    (0xC7, "SMB4", AddressingMode::ZeroPage),
    (0xC8, "INY", AddressingMode::Implied),
    (0xC9, "CMP", AddressingMode::Immediate),
    (0xCA, "DEX", AddressingMode::Implied),
    (0xCB, "WAI", AddressingMode::Implied),
    (0xCC, "CPY", AddressingMode::Absolute),
    (0xCD, "CMP", AddressingMode::Absolute),
    (0xCE, "DEC", AddressingMode::Absolute),
    (0xCF, "BBS4", AddressingMode::ZeroPage),
    (0xD0, "BNE", AddressingMode::Relative),
    (0xD1, "CMP", AddressingMode::IndirectY),
    (0xD2, "CMP", AddressingMode::ZeroPageIndirect),
    (0xD3, "NOP", AddressingMode::Implied),
    (0xD4, "NOP", AddressingMode::Implied),
    (0xD5, "CMP", AddressingMode::ZeroPageX),
    (0xD6, "DEC", AddressingMode::ZeroPageX),
    (0xD7, "SMB5", AddressingMode::ZeroPage),
    (0xD8, "CLD", AddressingMode::Implied),
    (0xD9, "CMP", AddressingMode::AbsoluteY),
    (0xDA, "PHX", AddressingMode::Implied),
    (0xDB, "STP", AddressingMode::Implied),
    (0xDC, "NOP", AddressingMode::Implied),
    (0xDD, "CMP", AddressingMode::AbsoluteX),
    (0xDE, "DEC", AddressingMode::AbsoluteX),
    (0xDF, "BBS5", AddressingMode::ZeroPage),
    (0xE0, "CPX", AddressingMode::Immediate),
    (0xE1, "SBC", AddressingMode::IndirectX),
    (0xE2, "NOP", AddressingMode::Implied),
    (0xE3, "NOP", AddressingMode::Implied),
    (0xE4, "CPX", AddressingMode::ZeroPage),
    (0xE5, "SBC", AddressingMode::ZeroPage),
    (0xE6, "INC", AddressingMode::ZeroPage),
    (0xE7, "SMB6", AddressingMode::ZeroPage),
    (0xE8, "INX", AddressingMode::Implied),
    (0xE9, "SBC", AddressingMode::Immediate),
    (0xEA, "NOP", AddressingMode::Implied),
    (0xEB, "NOP", AddressingMode::Implied),
    (0xEC, "CPX", AddressingMode::Absolute),
    (0xED, "SBC", AddressingMode::Absolute),
    (0xEE, "INC", AddressingMode::Absolute),
    (0xEF, "BBS6", AddressingMode::ZeroPage),
    (0xF0, "BEQ", AddressingMode::Relative),
    (0xF1, "SBC", AddressingMode::IndirectY),
    (0xF2, "SBC", AddressingMode::ZeroPageIndirect),
    (0xF3, "NOP", AddressingMode::Implied),
    (0xF4, "NOP", AddressingMode::Implied),
    (0xF5, "SBC", AddressingMode::ZeroPageX),
    (0xF6, "INC", AddressingMode::ZeroPageX),
    (0xF7, "SMB7", AddressingMode::ZeroPage),
    (0xF8, "SED", AddressingMode::Implied),
    (0xF9, "SBC", AddressingMode::AbsoluteY),
    (0xFA, "PLX", AddressingMode::Implied),
    (0xFB, "NOP", AddressingMode::Implied),
    (0xFC, "NOP", AddressingMode::Implied),
    (0xFD, "SBC", AddressingMode::AbsoluteX),
    (0xFE, "INC", AddressingMode::AbsoluteX),
    (0xFF, "NOP", AddressingMode::Implied),
];

pub struct Disassembler;

impl Disassembler {
    pub fn disassemble(bus: &Bus, addr: u16) -> String {
        let opcode = bus.read_byte(addr);
        let (mnemonic, mode) = Disassembler::lookup_opcode(opcode);
        let operand_bytes = mode.operand_bytes();

        let operand1 = bus.read_byte(addr.wrapping_add(1));
        let operand2 = bus.read_byte(addr.wrapping_add(2));

        let formatted_operand = match operand_bytes {
            0 => String::new(),
            1 => Disassembler::format_operands(addr, mode, operand1, 0x00),
            2 => Disassembler::format_operands(addr, mode, operand1, operand2),
            _ => String::new(),
        };

        let mut byte_dump = format!("{:02X}", opcode);
        if operand_bytes >= 1 {
            byte_dump.push_str(&format!(" {:02X}", operand1));
        }
        if operand_bytes == 2 {
            byte_dump.push_str(&format!(" {:02X}", operand2));
        }

        format!(
            "${:04X}  {:<8}  -  {:<4} {:<8}",
            addr, byte_dump, mnemonic, formatted_operand
        )
    }

    fn lookup_opcode(opcode: u8) -> (&'static str, AddressingMode) {
        OPCODES
            .iter()
            .find(|&&(code, _, _)| code == opcode)
            .map(|&(_, mnemonic, mode)| (mnemonic, mode))
            .unwrap_or(("???", AddressingMode::Implied))
    }

    pub fn format_operands(addr: u16, mode: AddressingMode, operand1: u8, operand2: u8) -> String {
        match mode {
            AddressingMode::Implied => String::new(),
            AddressingMode::Accumulator => "A".to_string(),
            AddressingMode::Immediate => format!("#${:02X}", operand1),
            AddressingMode::ZeroPage => format!("${:02X}", operand1),
            AddressingMode::ZeroPageX => format!("${:02X},X", operand1),
            AddressingMode::ZeroPageY => format!("${:02X},Y", operand1),
            AddressingMode::ZeroPageIndirect => format!("(${:02X})", operand1),
            AddressingMode::Absolute => {
                format!("${:04X}", (operand2 as u16) << 8 | operand1 as u16)
            }
            AddressingMode::AbsoluteX => {
                format!("${:04X},X", (operand2 as u16) << 8 | operand1 as u16)
            }
            AddressingMode::AbsoluteY => {
                format!("${:04X},Y", (operand2 as u16) << 8 | operand1 as u16)
            }
            AddressingMode::Indirect => {
                format!("(${:04X})", (operand2 as u16) << 8 | operand1 as u16)
            }
            AddressingMode::IndirectX => format!("(${:02X},X)", operand1),
            AddressingMode::IndirectY => format!("(${:02X}),Y", operand1),
            AddressingMode::IndirectAbsolute => {
                format!("(${:04X})", (operand2 as u16) << 8 | operand1 as u16)
            }
            AddressingMode::Relative => {
                let offset = operand1 as i8;
                let target = addr.wrapping_add(2).wrapping_add(offset as u16);
                format!("${:04X}", target)
            }
        }
    }
}
