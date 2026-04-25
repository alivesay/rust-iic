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

pub fn apple_iic_font_index(vram_code: u8, is_altchar: bool) -> (usize, bool) {
    // Returns (clean_font_index, invert_flag)
    // Clean Font Layout (0-127):
    // 0x00-0x1F: MouseText
    // 0x20-0x3F: Symbols/Numbers
    // 0x40-0x5F: Uppercase
    // 0x60-0x7F: Lowercase

    let (index, invert) = match vram_code {
        // 0x00-0x1F: Inverse Uppercase (@, A, B...)
        // Map to Clean Uppercase (0x40-0x5F)
        0x00..=0x1F => (vram_code + 0x40, true),

        // 0x20-0x3F: Inverse Symbols/Numbers
        // Map to Clean Symbols (0x20-0x3F)
        0x20..=0x3F => (vram_code, true),

        // 0x40-0x5F: Flashing Uppercase OR MouseText
        0x40..=0x5F => {
            if is_altchar {
                // MouseText (0x40-0x5F -> Clean 0x00-0x1F)
                (vram_code - 0x40, false)
            } else {
                // Flashing Uppercase (Treat as Inverse for now)
                (vram_code, true)
            }
        }

        // 0x60-0x7F: Flashing Symbols OR Inverse Lowercase
        0x60..=0x7F => {
            if is_altchar {
                // Inverse Lowercase (0x60-0x7F -> Clean 0x60-0x7F)
                (vram_code, true)
            } else {
                // Flashing Symbols (Treat as Inverse)
                (vram_code - 0x40, true)
            }
        }

        // 0x80-0x9F: Normal Uppercase (Control?) -> Map to Uppercase
        // Usually 0x80 is '@' (ASCII 0x40)
        0x80..=0x9F => (vram_code - 0x40, false),

        // 0xA0-0xBF: Normal Symbols/Numbers
        // 0xA0 is Space (ASCII 0x20)
        0xA0..=0xBF => (vram_code - 0x80, false),

        // 0xC0-0xDF: Normal Uppercase
        // 0xC1 is 'A' (ASCII 0x41)
        0xC0..=0xDF => (vram_code - 0x80, false),

        // 0xE0-0xFF: Normal Lowercase
        // 0xE1 is 'a' (ASCII 0x61)
        0xE0..=0xFF => (vram_code - 0x80, false),
    };

    (index as usize * 8, invert)
}
