use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};

use crate::cpu::SystemType;
use crate::util::hexdump;

pub struct ROM {
    pub data: Vec<u8>,
}

impl ROM {
    pub fn load_from_file(filename: &str, system_type: SystemType) -> io::Result<Self> {
        let mut file = File::open(filename)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Self::load_from_bytes(&data, system_type)
    }

    pub fn load_from_bytes(bytes: &[u8], system_type: SystemType) -> io::Result<Self> {
        if bytes.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ROM is empty"));
        }

        let max_size = match system_type {
            SystemType::AppleIIc => 0x8000,
            SystemType::Generic => 0x10000,
        };

        if bytes.len() > max_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ROM too large: {} bytes (Max allowed: {} bytes)",
                    bytes.len(),
                    max_size
                ),
            ));
        }

        let mut data = vec![0xFF; max_size];
        data[..bytes.len()].copy_from_slice(bytes);

        println!("ROM Loaded | {:?} | {} bytes", system_type, bytes.len());

        hexdump(&data, Some(0), Some(bytes.len().min(0x100)));

        Ok(Self { data })
    }

    pub fn load_from_intel(filename: &str, system_type: SystemType) -> io::Result<Self> {
        let file = File::open(filename)?;
        let reader = BufReader::new(file);

        let max_size = match system_type {
            SystemType::AppleIIc => 0x8000,
            SystemType::Generic => 0x10000,
        };

        let mut data = vec![0xFF; max_size];
        let mut address_offset: u32 = 0;

        for line in reader.lines() {
            let line = line?;
            if !line.starts_with(':') || line.len() < 11 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid Intel HEX format",
                ));
            }

            let byte_count = u8::from_str_radix(&line[1..3], 16).unwrap();
            let address = u16::from_str_radix(&line[3..7], 16).unwrap();
            let record_type = u8::from_str_radix(&line[7..9], 16).unwrap();

            let mut checksum: u8 = 0;
            for i in (1..line.len() - 2).step_by(2) {
                let byte = u8::from_str_radix(&line[i..i + 2], 16).unwrap();
                checksum = checksum.wrapping_add(byte);
            }
            checksum = checksum.wrapping_neg();

            let expected_checksum = u8::from_str_radix(&line[line.len() - 2..], 16).unwrap();
            if checksum != expected_checksum {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Checksum mismatch",
                ));
            }

            match record_type {
                0x00 => {
                    let addr = (address_offset + address as u32) as usize;
                    if addr + (byte_count as usize) > max_size {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "HEX file exceeds ROM size",
                        ));
                    }

                    for i in 0..byte_count {
                        let start = 9 + (i as usize) * 2;
                        let byte = u8::from_str_radix(&line[start..start + 2], 16).unwrap();
                        data[addr + i as usize] = byte;
                    }
                }
                0x01 => {
                    break;
                }
                0x02 => {
                    address_offset = u16::from_str_radix(&line[9..13], 16).unwrap() as u32 * 16;
                }
                _ => {
                    continue;
                }
            }
        }

        println!(
            "Intel HEX ROM Loaded | {:?} | {} bytes",
            system_type,
            data.len()
        );

        hexdump(&data, Some(0), Some(0x100));

        Ok(Self { data })
    }
}
