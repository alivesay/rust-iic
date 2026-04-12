use a2kit::img::DiskImage;
use a2kit::img::Track;

fn main() {
    let woz_path = "disks/appleiic_atplay.woz";
    let dsk_path = "/Users/alivesay/Downloads/Apple IIc - At Play.dsk";
    
    let mut woz = a2kit::create_img_from_file(woz_path).expect("Failed to load WOZ");
    let dsk_data = std::fs::read(dsk_path).expect("Failed to read DSK file");
    
    println!("DSK file: {} bytes ({} sectors)", dsk_data.len(), dsk_data.len() / 256);
    
    // Read raw track 0 bitstream
    let track = woz.get_track_buf(Track::Num(0)).expect("track read failed");
    let bits: Vec<u8> = track.iter()
        .flat_map(|byte| (0..8).rev().map(move |i| (byte >> i) & 1))
        .collect();
    
    println!("Track 0: {} bytes, {} bits\n", track.len(), bits.len());
    
    // Denibbilize table
    let valid_nibbles: [u8; 64] = [
        0x96, 0x97, 0x9A, 0x9B, 0x9D, 0x9E, 0x9F, 0xA6,
        0xA7, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF, 0xB2, 0xB3,
        0xB4, 0xB5, 0xB6, 0xB7, 0xB9, 0xBA, 0xBB, 0xBC,
        0xBD, 0xBE, 0xBF, 0xCB, 0xCD, 0xCE, 0xCF, 0xD3,
        0xD6, 0xD7, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE,
        0xDF, 0xE5, 0xE6, 0xE7, 0xE9, 0xEA, 0xEB, 0xEC,
        0xED, 0xEE, 0xEF, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6,
        0xF7, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF,
    ];
    let mut denib = [0u8; 256];
    for (i, &nib) in valid_nibbles.iter().enumerate() {
        denib[nib as usize] = i as u8;
    }
    
    // DOS 3.3 interleave: physical -> DOS logical
    // formula: logical = (phys == 15) ? 15 : ((phys * 7) % 15)
    // DSK file is DOS 3.3 order, so DSK offset = logical_sector * 256
    
    let mut pos = 0;
    while pos + 24 < bits.len() {
        if read_byte(&bits, pos) != 0xD5 { pos += 1; continue; }
        if read_byte(&bits, pos+8) != 0xAA { pos += 1; continue; }
        if read_byte(&bits, pos+16) != 0x96 { pos += 1; continue; }
        
        // Decode address field
        let mut decoded = [0u8; 4];
        for i in 0..4 {
            let off = pos + 24 + i * 16;
            let hi = read_byte(&bits, off);
            let lo = read_byte(&bits, off + 8);
            decoded[i] = (hi << 1 | 1) & lo;
        }
        let _vol = decoded[0];
        let _trk = decoded[1];
        let phys_sec = decoded[2];
        
        // Find data prologue
        let mut dpos = pos + 24 + 64 + 24;
        let mut found = false;
        for _ in 0..400 {
            if dpos + 24 >= bits.len() { break; }
            if read_byte(&bits, dpos) == 0xD5 
                && read_byte(&bits, dpos+8) == 0xAA 
                && read_byte(&bits, dpos+16) == 0xAD {
                dpos += 24;
                found = true;
                break;
            }
            dpos += 1;
        }
        
        if found {
            let mut nibbles = [0u8; 343];
            for i in 0..343 {
                nibbles[i] = read_byte(&bits, dpos + i * 8);
            }
            let result = denibbilize(&nibbles, &denib);
            
            // What DOS logical sector does this physical sector map to?
            let dos_logical = if phys_sec == 15 { 15 } else { ((phys_sec as u32 * 7) % 15) as u8 };
            let dsk_offset = dos_logical as usize * 256;
            
            let matches_dos = if dsk_offset + 256 <= dsk_data.len() {
                result[..256] == dsk_data[dsk_offset..dsk_offset+256]
            } else { false };
            
            // Also check ProDOS mapping in case dsk2woz used wrong interleave
            let prodos_logical = if phys_sec == 15 { 15 } else { ((phys_sec as u32 * 8) % 15) as u8 };
            let po_offset = prodos_logical as usize * 256;
            let matches_prodos = if po_offset + 256 <= dsk_data.len() {
                result[..256] == dsk_data[po_offset..po_offset+256]
            } else { false };
            
            print!("Phys {:2} -> DOS log {:2} (off {:5}): ", phys_sec, dos_logical, dsk_offset);
            for b in &result[..8] { print!("{:02X} ", b); }
            print!("  dos_match={} prodos_match={}", matches_dos, matches_prodos);
            if phys_sec == 0 { print!("  <- boot sector"); }
            println!();
        }
        
        pos += 24;
    }
    
    println!("\n=== DSK file sector 0 (boot sector) ===");
    print!("  ");
    for i in 0..32 { print!("{:02X} ", dsk_data[i]); }
    println!();
}

fn read_byte(bits: &[u8], pos: usize) -> u8 {
    let mut val = 0u8;
    for i in 0..8 { val = (val << 1) | bits[pos + i]; }
    val
}

fn denibbilize(nibbles: &[u8; 343], denib: &[u8; 256]) -> [u8; 256] {
    let mut buf = [0u8; 343];
    let mut prev = 0u8;
    for i in 0..343 {
        let val = denib[nibbles[i] as usize];
        buf[i] = val ^ prev;
        prev = buf[i];
    }
    let mut result = [0u8; 256];
    for i in 0..256 {
        let main_val = buf[86 + i];
        let aux_idx = i % 86;
        let aux_val = buf[aux_idx];
        let shift = (i / 86) * 2;
        let low_bits = (aux_val >> shift) & 0x03;
        result[i] = (main_val << 2) | low_bits;
    }
    result
}
