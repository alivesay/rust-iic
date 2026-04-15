use clap::Parser;
use std::path::PathBuf;

use a2kit::img::woz1::Woz1;
use a2kit::img::woz2::Woz2;
use a2kit::img::names::{A2_DOS33_KIND, A2_DOS32_KIND};
use a2kit::img::DiskImage;

#[derive(Parser)]
#[command(name = "mkwoz", about = "Create blank WOZ disk images")]
struct Cli {
    /// Output file path
    #[arg(default_value = "blank.woz")]
    output: PathBuf,

    /// WOZ format version (1 or 2)
    #[arg(short = 'w', long, default_value = "1")]
    woz_version: u8,

    /// Disk format: dos33 (16-sector) or dos32 (13-sector)
    #[arg(short, long, default_value = "dos33")]
    format: String,

    /// Volume number (1-254)
    #[arg(short, long, default_value = "254")]
    volume: u8,

    /// Create unformatted (blank) media instead of formatted empty disk
    #[arg(short, long)]
    blank: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let kind = match cli.format.as_str() {
        "dos33" | "16" => A2_DOS33_KIND,
        "dos32" | "13" => A2_DOS32_KIND,
        _ => anyhow::bail!("Unknown format '{}'. Use 'dos33' or 'dos32'.", cli.format),
    };

    if cli.volume == 0 {
        anyhow::bail!("Volume number must be 1-254");
    }

    let bytes = match (cli.woz_version, cli.blank) {
        (1, true) => {
            let mut disk = Woz1::blank(kind);
            disk.to_bytes()
        }
        (1, false) => {
            let mut disk = Woz1::create(cli.volume, kind, None)
                .map_err(|e| anyhow::anyhow!("Failed to create WOZ1: {}", e))?;
            disk.to_bytes()
        }
        (2, true) => {
            let mut disk = Woz2::blank(kind);
            disk.to_bytes()
        }
        (2, false) => {
            let mut disk = Woz2::create(cli.volume, kind, None, vec![])
                .map_err(|e| anyhow::anyhow!("Failed to create WOZ2: {}", e))?;
            disk.to_bytes()
        }
        _ => anyhow::bail!("WOZ version must be 1 or 2"),
    };

    std::fs::write(&cli.output, &bytes)?;
    let kind_name = if kind == A2_DOS33_KIND { "DOS 3.3 (16-sector)" } else { "DOS 3.2 (13-sector)" };
    let mode = if cli.blank { "blank/unformatted" } else { "formatted" };
    println!(
        "Created {} WOZ{} {} disk: {} ({} bytes)",
        mode,
        cli.woz_version,
        kind_name,
        cli.output.display(),
        bytes.len()
    );

    Ok(())
}
