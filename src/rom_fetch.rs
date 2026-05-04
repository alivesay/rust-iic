use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use crate::config;

const ROM_FILENAME: &str = "iic.bin";
const ROM_URL: &str =
    "https://www.apple.asimov.net/emulators/rom_images/apple_iic_rom.zip";
const ROM_MEMBER: &str = "APPLE2C 3.ROM";

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_millis(750);

pub fn rom_path() -> PathBuf {
    let mut p = config::config_path();
    p.set_file_name(ROM_FILENAME);
    p
}

pub fn load_or_fetch() -> Result<Vec<u8>> {
    let path = rom_path();
    if path.exists() {
        return fs::read(&path)
            .with_context(|| format!("reading {}", path.display()));
    }

    println!(
        "rom   {:>12} {:>8}    not found, fetching from Asimov",
        "FIRMWARE", "MISSING"
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let bytes = download_with_retry(ROM_URL, RETRY_ATTEMPTS)?;
    let rom = extract_member(&bytes, ROM_MEMBER)?;

    fs::write(&path, &rom)
        .with_context(|| format!("writing {}", path.display()))?;
    println!(
        "rom   {:>12} {:>8}    {} ({} bytes)",
        "FIRMWARE",
        "CACHED",
        path.display(),
        rom.len()
    );
    Ok(rom)
}

fn download_with_retry(url: &str, attempts: usize) -> Result<Vec<u8>> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=attempts {
        match download_once(url) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                eprintln!(
                    "rom   {:>12} {:>8}    attempt {}/{} failed: {}",
                    "FIRMWARE", "RETRY", attempt, attempts, e
                );
                last_err = Some(e);
                if attempt < attempts {
                    sleep(RETRY_DELAY);
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("download failed")))
}

fn download_once(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(HTTP_TIMEOUT)
        .timeout_read(HTTP_TIMEOUT)
        .build();
    let resp = agent.get(url).call().context("http get")?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .context("reading response body")?;
    if buf.is_empty() {
        bail!("empty response from {}", url);
    }
    Ok(buf)
}

fn extract_member(zip_bytes: &[u8], member: &str) -> Result<Vec<u8>> {
    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("open zip")?;

    if let Ok(mut f) = archive.by_name(member) {
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out).context("reading rom member")?;
        return Ok(out);
    }

    let want = Path::new(member)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(member);
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();
    for name in &names {
        let base = Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if base.eq_ignore_ascii_case(want) {
            let mut f = archive
                .by_name(name)
                .with_context(|| format!("opening {}", name))?;
            let mut out = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut out).context("reading rom member")?;
            return Ok(out);
        }
    }

    bail!(
        "{} not found in zip (members: {})",
        member,
        names.join(", ")
    )
}
