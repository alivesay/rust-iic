use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use crate::config;

const ROM_FILENAME: &str = "iic.bin";

const ROM_URL: &str =
    "https://downloads.reactivemicro.com/Apple%20II%20Items/ROM_and_JEDEC/IIc/IIc%20-%20ROM4x%20-%20v2018-10-01%20-%2027256.bin";

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
        "rom   {:>12} {:>8}    not found, fetching from ReactiveMicro",
        "FIRMWARE", "MISSING"
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let rom = download_with_retry(ROM_URL, RETRY_ATTEMPTS)?;
    if rom.len() != 16_384 && rom.len() != 32_768 {
        bail!(
            "unexpected rom size {} bytes (expected 16384 or 32768)",
            rom.len()
        );
    }

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
