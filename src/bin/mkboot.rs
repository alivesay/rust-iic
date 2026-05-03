// mkboot -- splice a raw 6502 boot sector into a blank DOS 3.3 .dsk image.
//
// Usage: mkboot <input.bin> <output.dsk>
//
// The input must be exactly 256 bytes (one DOS sector).  We write a
// 143_360-byte (35 tracks * 16 sectors * 256 bytes) .dsk where T0/S0
// contains the boot sector and every other sector is zero-filled.
// This is enough for the Apple //c boot ROM to load and execute the
// boot1 payload at $0801.

use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("usage: mkboot <input.bin> <output.dsk>");
    std::process::exit(2);
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        usage();
    }
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);

    let boot = std::fs::read(&input)?;
    if boot.len() != 256 {
        eprintln!(
            "error: boot sector must be 256 bytes, got {} bytes from {}",
            boot.len(),
            input.display()
        );
        std::process::exit(1);
    }

    const DSK_SIZE: usize = 35 * 16 * 256; // 143_360
    let mut img = vec![0u8; DSK_SIZE];
    img[..256].copy_from_slice(&boot);

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&output, &img)?;
    println!(
        "mkboot: {} ({} bytes) -> {} ({} bytes)",
        input.display(),
        boot.len(),
        output.display(),
        img.len()
    );
    Ok(())
}
