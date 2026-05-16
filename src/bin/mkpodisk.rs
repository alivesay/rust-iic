//! mkpodisk -- pack ProDOS WOZ disks for the rust-iic tooling.
//!
//! Creates a 140K ProDOS-formatted WOZ image with:
//!   * PRODOS         (kernel, SYS)
//!   * BASIC.SYSTEM   (interpreter, SYS)
//!   * STARTUP        (Applesoft) -- synthesised entry script
//!   * <NAME>         (main payload binary, BLOAD'd at --load-addr)
//!   * any number of --extra-file entries (BLOAD'd by STARTUP before CALL)
//!   * any number of --save-file entries (BSAVE'd by STARTUP after CALL
//!     when location $0BFF != 0; cleared and re-CALL'd in a loop)
//!
//! Boot path:
//!   ProDOS auto-runs PRODOS.SYS, which chains BASIC.SYSTEM, which runs
//!   STARTUP. STARTUP BLOADs every --extra-file then BLOADs and CALLs
//!   the main payload. With --save-loop, the BASIC then watches $0BFF
//!   and re-saves the listed files whenever the program asks.

use std::path::PathBuf;
use clap::Parser;

use a2kit::commands::ItemType;
use a2kit::fs::prodos;
use a2kit::img::names::A2_DOS33_KIND;
use a2kit::img::woz1::Woz1;
use a2kit::img::DiskImage;

#[derive(Parser, Debug)]
#[command(name = "mkpodisk", about = "Build a ProDOS WOZ image for rust-iic")]
struct Cli {
    /// Output disk image path (.woz)
    #[arg(short, long)]
    output: PathBuf,

    /// Main payload binary (e.g. cart_edit.bin)
    #[arg(short, long)]
    input: PathBuf,

    /// BLOAD address for the payload
    #[arg(long, default_value = "0x8000")]
    load_addr: String,

    /// Filename on disk for the payload (max 15 chars, ProDOS rules)
    #[arg(long, default_value = "GAME")]
    name: String,

    /// Volume name (no leading slash)
    #[arg(long, default_value = "RUSTIIC")]
    volume: String,

    /// PRODOS kernel binary path
    #[arg(long, default_value = "assets/prodos/PRODOS.bin")]
    prodos_bin: PathBuf,

    /// BASIC.SYSTEM binary path
    #[arg(long, default_value = "assets/prodos/BASIC.SYSTEM.bin")]
    basic_system_bin: PathBuf,

    /// Applesoft STARTUP source file (text). When omitted, one is
    /// synthesised from --name / --extra-file / --save-file.
    #[arg(long)]
    basic: Option<PathBuf>,

    /// Extra binary to copy onto the disk *and* BLOAD before CALL.
    /// Format: PATH:NAME:ADDR  (ADDR may be 0x.., $.., or decimal).
    /// Repeatable.
    #[arg(long, value_name = "PATH:NAME:ADDR")]
    extra_file: Vec<String>,

    /// File to BSAVE in the editor save-loop. Format: NAME:ADDR:LEN.
    /// Repeatable. Only meaningful with --save-loop.
    #[arg(long, value_name = "NAME:ADDR:LEN")]
    save_file: Vec<String>,

    /// Editor mode: STARTUP loops, BSAVE'ing every --save-file when the
    /// program returns with $0BFF (3071) != 0. The program POKEs 3071,1
    /// then RTS to request a save.
    #[arg(long)]
    save_loop: bool,
}

#[derive(Debug, Clone)]
struct Extra {
    path: PathBuf,
    name: String,
    addr: u16,
}

#[derive(Debug, Clone)]
struct SaveSpec {
    name: String,
    addr: u16,
    len: u16,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let load_addr = parse_u16(&cli.load_addr)
        .ok_or_else(|| anyhow::anyhow!("bad --load-addr '{}'", cli.load_addr))?;

    let extras: Vec<Extra> = cli.extra_file.iter()
        .map(|s| parse_extra(s))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let saves: Vec<SaveSpec> = cli.save_file.iter()
        .map(|s| parse_savespec(s))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let payload = std::fs::read(&cli.input)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.input.display(), e))?;
    let prodos_bin = std::fs::read(&cli.prodos_bin)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.prodos_bin.display(), e))?;
    let basic_system_bin = std::fs::read(&cli.basic_system_bin)
        .map_err(|e| anyhow::anyhow!("read {}: {}", cli.basic_system_bin.display(), e))?;

    // 5.25" 140K WOZ + ProDOS filesystem.
    let woz = Woz1::create(254, A2_DOS33_KIND, None)
        .map_err(|e| anyhow::anyhow!("woz create: {}", e))?;

    let mut fs = prodos::Disk::from_img(Box::new(woz) as Box<dyn DiskImage>)
        .map_err(|e| anyhow::anyhow!("prodos from img: {}", e))?;
    fs.format(&cli.volume, true, None)
        .map_err(|e| anyhow::anyhow!("format: {}", e))?;

    let mut fs: Box<dyn a2kit::fs::DiskFS> = Box::new(fs);

    save_sys(&mut *fs, "PRODOS", &prodos_bin, 0x2000)?;
    save_sys(&mut *fs, "BASIC.SYSTEM", &basic_system_bin, 0x2000)?;

    fs.bsave(&cli.name, &payload, Some(load_addr as usize), None)
        .map_err(|e| anyhow::anyhow!("bsave {}: {}", cli.name, e))?;

    for e in &extras {
        let buf = std::fs::read(&e.path)
            .map_err(|err| anyhow::anyhow!("read {}: {}", e.path.display(), err))?;
        fs.bsave(&e.name, &buf, Some(e.addr as usize), None)
            .map_err(|err| anyhow::anyhow!("bsave {}: {}", e.name, err))?;
    }

    let startup = match &cli.basic {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?,
        None => synth_startup(&cli.name, load_addr, &extras, &saves, cli.save_loop),
    };
    let toks = tokenize_applesoft(&startup)?;
    fs.save("STARTUP", &toks, ItemType::ApplesoftTokens, None)
        .map_err(|e| anyhow::anyhow!("save STARTUP: {}", e))?;

    a2kit::save_img(&mut fs, &cli.output.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("save {}: {}", cli.output.display(), e))?;

    println!(
        "mkpodisk: wrote {} ({} byte payload @ ${:04X}, vol /{}, name {}, +{} extras, +{} save specs)",
        cli.output.display(),
        payload.len(),
        load_addr,
        cli.volume,
        cli.name,
        extras.len(),
        saves.len(),
    );
    Ok(())
}

fn synth_startup(
    name: &str,
    load_addr: u16,
    extras: &[Extra],
    saves: &[SaveSpec],
    save_loop: bool,
) -> String {
    let mut s = String::new();
    let mut ln = 10u16;
    s.push_str(" 0  HOME\n");
    for e in extras {
        s.push_str(&format!(
            "{:>2}  PRINT  CHR$ (4)\"BLOAD {},A${:04X}\"\n",
            ln, e.name, e.addr
        ));
        ln += 10;
    }
    s.push_str(&format!(
        "{:>2}  PRINT  CHR$ (4)\"BLOAD {},A${:04X}\"\n",
        ln, name, load_addr
    ));
    ln += 10;
    if save_loop {
        let loop_top = ln;
        s.push_str(&format!("{:>2}  POKE 3071,0\n", ln)); ln += 10;
        s.push_str(&format!("{:>2}  CALL {}\n", ln, load_addr)); ln += 10;
        s.push_str(&format!("{:>2}  IF  PEEK (3071) = 0  THEN  END \n", ln)); ln += 10;
        for sp in saves {
            s.push_str(&format!(
                "{:>2}  PRINT  CHR$ (4)\"BSAVE {},A${:04X},L${:04X}\"\n",
                ln, sp.name, sp.addr, sp.len
            ));
            ln += 10;
        }
        s.push_str(&format!("{:>2}  GOTO {}\n", ln, loop_top));
    } else {
        s.push_str(&format!("{:>2}  CALL {}\n", ln, load_addr));
    }
    s
}

fn parse_extra(spec: &str) -> anyhow::Result<Extra> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 3 {
        anyhow::bail!("--extra-file '{}' must be PATH:NAME:ADDR", spec);
    }
    Ok(Extra {
        path: PathBuf::from(parts[0]),
        name: parts[1].to_string(),
        addr: parse_u16(parts[2])
            .ok_or_else(|| anyhow::anyhow!("bad ADDR in --extra-file '{}'", spec))?,
    })
}

fn parse_savespec(spec: &str) -> anyhow::Result<SaveSpec> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 3 {
        anyhow::bail!("--save-file '{}' must be NAME:ADDR:LEN", spec);
    }
    Ok(SaveSpec {
        name: parts[0].to_string(),
        addr: parse_u16(parts[1])
            .ok_or_else(|| anyhow::anyhow!("bad ADDR in --save-file '{}'", spec))?,
        len: parse_u16(parts[2])
            .ok_or_else(|| anyhow::anyhow!("bad LEN in --save-file '{}'", spec))?,
    })
}

fn save_sys(
    fs: &mut dyn a2kit::fs::DiskFS,
    path: &str,
    data: &[u8],
    load_addr: usize,
) -> anyhow::Result<()> {
    fs.bsave(path, data, Some(load_addr), None)
        .map_err(|e| anyhow::anyhow!("bsave {}: {}", path, e))?;
    let mut fimg = fs.get(path)
        .map_err(|e| anyhow::anyhow!("get {}: {}", path, e))?;
    fimg.fs_type = vec![0xFF]; // ProDOS SYS
    fs.delete(path)
        .map_err(|e| anyhow::anyhow!("delete {}: {}", path, e))?;
    fs.put(&fimg)
        .map_err(|e| anyhow::anyhow!("put {}: {}", path, e))?;
    Ok(())
}

fn tokenize_applesoft(src: &str) -> anyhow::Result<Vec<u8>> {
    let mut tok = a2kit::lang::applesoft::tokenizer::Tokenizer::new();
    tok.tokenize(src, 0x0801)
        .map_err(|e| anyhow::anyhow!("applesoft tokenize: {}", e))
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
