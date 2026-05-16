//! pomvfile -- pull named files out of a ProDOS WOZ image.
//!
//! Used by `make cart_edit` to round-trip in-editor V-saves back into
//! `assets/cartedit/` before repacking the disk.
//!
//! Usage:
//!     pomvfile --input DISK.po.woz --out NAME:OUTPATH [--out NAME:OUTPATH ...]
//!
//! No-op (exit 0) if --input does not exist, so the Makefile can depend
//! on it unconditionally for the first build.

use std::path::PathBuf;
use clap::Parser;

use a2kit::fs::prodos;
use a2kit::img::woz1::Woz1;
use a2kit::img::DiskImage;

#[derive(Parser, Debug)]
#[command(name = "pomvfile", about = "Extract files from a ProDOS WOZ image")]
struct Cli {
    /// Input ProDOS .po.woz disk
    #[arg(short, long)]
    input: PathBuf,

    /// NAME:OUTPATH - extract NAME from disk to OUTPATH. Repeatable.
    #[arg(long, value_name = "NAME:OUTPATH")]
    out: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if !cli.input.exists() {
        eprintln!("pomvfile: {} not present, skipping", cli.input.display());
        return Ok(());
    }

    let bytes = std::fs::read(&cli.input)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.input.display(), e))?;
    let woz = Woz1::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("parse woz: {}", e))?;

    let fs = prodos::Disk::from_img(Box::new(woz) as Box<dyn DiskImage>)
        .map_err(|e| anyhow::anyhow!("prodos from img: {}", e))?;
    let mut fs: Box<dyn a2kit::fs::DiskFS> = Box::new(fs);

    for spec in &cli.out {
        let (name, path) = spec.split_once(':')
            .ok_or_else(|| anyhow::anyhow!("--out '{}' must be NAME:OUTPATH", spec))?;
        let (_addr, data) = fs.bload(name)
            .map_err(|e| anyhow::anyhow!("bload {}: {}", name, e))?;
        std::fs::write(path, &data)
            .map_err(|e| anyhow::anyhow!("write {}: {}", path, e))?;
        println!("pomvfile: {} -> {} ({} B)", name, path, data.len());
    }
    Ok(())
}
