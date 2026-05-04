// mkdisk: wrap a raw 6502 binary into a bootable DOS 3.3 WOZ disk.
//
// Given a `.bin` produced by ld65 and a load address, this:
//   1. Creates a fresh DOS 3.3-formatted WOZ1 image (volume 254, bootable).
//   2. BSAVEs the binary on disk under a chosen filename (default GAME).
//   3. Writes a tokenized Applesoft `HELLO` that does `PRINT CHR$(4)"BRUN <name>"`,
//      so booting the disk auto-runs the binary.
//   4. Writes the result to the given output path.
//
// Used by the asm Makefile to produce game disks alongside the raw .bin.

use clap::Parser;
use std::path::PathBuf;

use a2kit::commands::ItemType;
use a2kit::fs::dos3x;
use a2kit::img::names::A2_DOS33_KIND;
use a2kit::img::woz1::Woz1;
use a2kit::img::DiskImage;

#[derive(Parser)]
#[command(name = "mkdisk", about = "Build a bootable DOS 3.3 WOZ from a raw 6502 binary")]
struct Cli {
    // Output WOZ path
    #[arg(short, long)]
    output: PathBuf,

    // Input raw binary (no DOS header; we add one)
    #[arg(short, long)]
    input: PathBuf,

    // Load address (e.g. 0x6000 or 24576)
    #[arg(long, default_value = "0x6000")]
    load_addr: String,

    // Filename on disk (max 30 chars)
    #[arg(long, default_value = "GAME")]
    name: String,

    // Volume number (1..254)
    #[arg(long, default_value = "254")]
    volume: u8,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let load_addr = parse_u16(&cli.load_addr)
        .ok_or_else(|| anyhow::anyhow!("bad --load-addr '{}'", cli.load_addr))?;

    if cli.volume == 0 || cli.volume == 255 {
        anyhow::bail!("volume must be 1..254");
    }

    let bin = std::fs::read(&cli.input)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.input.display(), e))?;
    if bin.is_empty() {
        anyhow::bail!("input is empty");
    }

    // Low-level DOS 3.3 layout WOZ.
    let woz = Woz1::create(cli.volume, A2_DOS33_KIND, None)
        .map_err(|e| anyhow::anyhow!("woz create: {}", e))?;

    // Wrap as DOS 3.3 filesystem and lay down the catalog + DOS image.
    let mut fs = dos3x::Disk::from_img(Box::new(woz) as Box<dyn DiskImage>)
        .map_err(|e| anyhow::anyhow!("fs from img: {}", e))?;
    fs.init33(cli.volume, true)
        .map_err(|e| anyhow::anyhow!("init33: {}", e))?;

    // Box it for the trait-based convenience API.
    let mut fs: Box<dyn a2kit::fs::DiskFS> = Box::new(fs);

    // Save the binary.
    fs.bsave(&cli.name, &bin, Some(load_addr as usize), None)
        .map_err(|e| anyhow::anyhow!("bsave {}: {}", cli.name, e))?;

    // Save Applesoft HELLO that BRUNs the binary on boot.
    let hello = build_hello_brun(&cli.name);
    fs.save("HELLO", &hello, ItemType::ApplesoftTokens, None)
        .map_err(|e| anyhow::anyhow!("save HELLO: {}", e))?;

    a2kit::save_img(&mut fs, &cli.output.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("save {}: {}", cli.output.display(), e))?;

    println!(
        "mkdisk: wrote {} ({} byte binary @ ${:04X}, BRUN {})",
        cli.output.display(),
        bin.len(),
        load_addr,
        cli.name
    );
    Ok(())
}

fn parse_u16(s: &str) -> Option<u16> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(rest, 16).ok()
    } else if let Some(rest) = s.strip_prefix('$') {
        u16::from_str_radix(rest, 16).ok()
    } else {
        s.parse().ok()
    }
}

// Build a tokenized Applesoft program:
//
//   10 PRINT CHR$(4)"BRUN <name>"
//
// Applesoft in-memory format (loaded at $0801 by RUN HELLO):
//   [next_link_lo, next_link_hi, line#_lo, line#_hi, tokens..., 0x00]
//   ... more lines ...
//   [0x00, 0x00]
fn build_hello_brun(name: &str) -> Vec<u8> {
    const LOAD_ADDR: u16 = 0x0801;
    const TOK_PRINT: u8 = 0xBA;
    const TOK_CHR: u8 = 0xE7;

    let mut line: Vec<u8> = vec![
      TOK_PRINT,
      b' ',
      TOK_CHR,
      b'(',
      b'4',
      b')',
      b'"',
    ];
    line.extend_from_slice(b"BRUN ");
    line.extend_from_slice(name.as_bytes());
    line.push(b'"');
    line.push(0x00); // end-of-line

    // 2 bytes link + 2 bytes line# + line bytes
    let next_link = LOAD_ADDR + 4 + line.len() as u16;

    let mut prog = Vec::with_capacity(6 + line.len());
    prog.extend_from_slice(&next_link.to_le_bytes());
    prog.extend_from_slice(&10u16.to_le_bytes());
    prog.extend_from_slice(&line);
    prog.extend_from_slice(&[0x00, 0x00]); // end of program
    prog
}
