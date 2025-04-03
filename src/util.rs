use crate::mmu::MemStateMask;

pub fn hexdump(data: &[u8], start: Option<u16>, length: Option<usize>) {
    let start = start.unwrap_or(0x0000) as usize;
    let length = length.unwrap_or(data.len());

    let actual_length = length.min(data.len());
    let end = start + actual_length - 1;

    let chunk_size = 16;
    println!("hexdump: {:04X} - {:04X}", start, end);

    for chunk_start in (start..=end).step_by(chunk_size) {
        print!("{:04X}: ", chunk_start);

        for i in chunk_start..chunk_start + chunk_size {
            if i <= end {
                print!("{:02X} ", data[i - start]);
            } else {
                print!("   ");
            }
        }

        print!(" | ");

        for i in chunk_start..chunk_start + chunk_size {
            if i <= end {
                let byte = data[i - start];
                let ascii = if byte.is_ascii_graphic() || byte == b' ' {
                    byte as char
                } else {
                    '.'
                };
                print!("{}", ascii);
            }
        }

        println!();
    }
}

#[rustfmt::skip]
pub fn mem_state_to_string(mem_state: u8) -> String {
    let value = mem_state;
    format!(
        "[{}{}{}{}{}{}{}{}]",
        if value & MemStateMask::ALTZP != 0 { 'A' } else { '.' },     // A: Auxiliary Zero Page (1) or Main (0)
        if value & MemStateMask::P280STORE != 0 { '8' } else { '.' }, // 8: 80STORE + PAGE2
        if value & MemStateMask::RAMRD != 0 { 'r' } else { '.' },     // r: Read from Aux (1) or Main (0)
        if value & MemStateMask::RAMWRT != 0 { 'w' } else { '.' },    // w: Write to Aux (1) or Main (0)
        if value & MemStateMask::LCRAM != 0 { 'L' } else { '.' },     // L: Read RAM (1) or ROM (0)
        if value & MemStateMask::RDBNK != 0 { 'B' } else { '.' },     // B: Bank Selection ($D000, Bank 2)
        if value & MemStateMask::WRITE != 0 { 'W' } else { '.' },     // W: Write Enabled (1) or Read-Only (0)
        if value & MemStateMask::ALTROM != 0 { 'R' } else { '.' }     // R: ROM Bank 2 (1) or Bank 1 (0)
    )
}

#[inline]
pub fn ior(val: u8) -> u8 {
    if val != 0 {
        0x80
    } else {
        0x00
    }
}

pub fn ascii_to_apple_iic(ch: u8, is_altchar: bool) -> u8 {
    match ch {
        b'A'..=b'O' => (ch - b'A') + 0xC1, // 'A' (0x41) → 0xC1, 'B' (0x42) → 0xC2
        b'P'..=b'_' => (ch - b'P') + 0xD0, // 'P' (0x50) → 0xD0, includes [\]^_

        b'0'..=b'9' => (ch - b'0') + 0xB0,

        b' ' => 0x20,

        b'!'..=b'/' => (ch - b'!') + 0xA1, //  !"#$%&'()*+,-./ → 0xA1-0xAF
        b':'..=b'@' => (ch - b':') + 0xBA, //  :;<=>?@ → 0xBA-0xBF

        b'a'..=b'z' => (ch - 32 - b'A') + 0xC1, // Convert to uppercase VRAM range

        b'`'..=b'~' if is_altchar => (ch - b'`') + 0x40, // MouseText uses 0x40-0x5F

        _ => 0xA0,
    }
}

pub fn apple_iic_font_index(vram_code: u8, is_altchar: bool) -> usize {
    let char_code = (vram_code & 0x7F) as usize;

    if is_altchar && (0x40..=0x5F).contains(&char_code) {
        return ((char_code - 0x40) + 64 * 4) * 8;
    }

    let base_index = match char_code {
        0x00..=0x1F => char_code + 0x40, // Inverse Symbols
        0x20..=0x3F => char_code,        // Normal Symbols & Numbers
        0x40..=0x5F => char_code - 0x40, // Corrected Normal Uppercase
        0x60..=0x7F => char_code,        // Flashing Text
        _ => char_code,                  // Other Characters
    };

    let row_offset = match vram_code {
        0x40..=0x5F => 0,  // Normal Text
        0x60..=0x7F => 12, // Flashing Text
        0x80..=0x9F => 0,  // Normal Text
        0xA0..=0xBF => 0,  // Normal Text
        0xC0..=0xDF => 0,  // Normal Text
        0xE0..=0xFF => 12, // Flashing Text
        _ => 0,
    };

    ((base_index + 64 * row_offset) * 8).min(4096 - 8)
}
