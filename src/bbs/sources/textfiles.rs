use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context};

use crate::bbs::source::{Blob, Entry, FileKind, Listing, Source};
use crate::config;

#[allow(dead_code)]
const BASE_URL: &str = "http://textfiles.com/";
const ZIP_BASE_URL: &str = "http://archives.textfiles.com/";
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const ZIP_MAX_BYTES: u64 = 64 * 1024 * 1024;
const TEXT_MAX_BYTES: u64 = 1024 * 1024;

const SECTIONS: &[(&str, &str)] = &[
    ("100",        "100 favorites"),
    ("adventure",  "Adventure walkthrus"),
    ("anarchy",    "Anarchy"),
    ("apple",      "Apple II"),
    ("art",        "ASCII art"),
    ("bbs",        "BBS history"),
    ("computers",  "Computers"),
    ("conspiracy", "Conspiracy"),
    ("drugs",      "Drugs"),
    ("etext",      "E-Texts"),
    ("food",       "Food"),
    ("fun",        "Fun"),
    ("games",      "Games"),
    ("groups",     "Groups"),
    ("hacking",    "Hacking"),
    ("hamradio",   "Ham Radio"),
    ("holiday",    "Holiday"),
    ("humor",      "Humor"),
    ("internet",   "Internet"),
    ("law",        "Law"),
    ("magazines",  "Magazines / e-zines"),
    ("media",      "Mass Media"),
    ("messages",   "BBS messages"),
    ("music",      "Music"),
    ("news",       "News"),
    ("occult",     "Occult"),
    ("phreak",     "Phreaking"),
    ("piracy",     "Piracy / warez"),
    ("politics",   "Politics"),
    ("programming","Programming"),
    ("reports",    "School reports"),
    ("rpg",        "Role Playing Games"),
    ("science",    "Science"),
    ("sex",        "Sex / sexuality"),
    ("sf",         "Science Fiction"),
    ("stories",    "BBS stories"),
    ("survival",   "Survival"),
    ("ufo",        "UFO"),
    ("uploads",    "Uploads"),
    ("virus",      "Viruses"),
];

pub struct TextfilesSource {
    cache_root: PathBuf,
    memo: Mutex<Option<(String, Listing)>>,
}

impl TextfilesSource {
    pub fn new() -> Self {
        let cache_root = config::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cache")
            .join("textfiles");
        let _ = fs::create_dir_all(&cache_root);
        Self {
            cache_root,
            memo: Mutex::new(None),
        }
    }

    fn section_dir(&self, slug: &str) -> PathBuf {
        self.cache_root.join(slug)
    }

    fn local_path(&self, path: &str) -> Option<PathBuf> {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        Some(self.cache_root.join(trimmed))
    }

    fn section_of<'a>(&self, path: &'a str) -> Option<&'a str> {
        let trimmed = path.trim_start_matches('/').trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        Some(match trimmed.find('/') {
            Some(i) => &trimmed[..i],
            None => trimmed,
        })
    }

    fn ensure_section(
        &self,
        slug: &str,
        progress: &mut dyn FnMut(&str, u64, Option<u64>),
    ) -> anyhow::Result<()> {
        let dir = self.section_dir(slug);

        let sentinel = dir.join(".extracted");
        if sentinel.exists() {
            return Ok(());
        }

        if !SECTIONS.iter().any(|(s, _)| *s == slug) {
            return Err(anyhow!("unknown section: {}", slug));
        }

        log::info!("textfiles: fetching section {}", slug);
        let url = format!("{}{}.zip", ZIP_BASE_URL, slug);
        progress(&format!("downloading {}.zip", slug), 0, None);
        let bytes = http_get_bytes_progress(&url, ZIP_MAX_BYTES, progress)
            .with_context(|| format!("downloading {}", url))?;

        progress(&format!("extracting {}.zip", slug), 0, Some(bytes.len() as u64));
        fs::create_dir_all(&dir).context("create section dir")?;

        extract_zip(&bytes, &self.cache_root)
            .with_context(|| format!("extract {}.zip", slug))?;

        let _ = fs::write(&sentinel, b"ok");
        progress(&format!("ready: {}", slug), bytes.len() as u64, Some(bytes.len() as u64));
        log::info!(
            "textfiles: extracted section {} ({} bytes)",
            slug,
            bytes.len()
        );
        Ok(())
    }

    fn list_root(&self) -> Listing {
        let mut entries = Vec::with_capacity(SECTIONS.len());
        for (slug, title) in SECTIONS {
            entries.push(Entry::Dir {
                name: format!("{:<11}  {}", slug, title),
                path: format!("{}/", slug),
            });
        }
        Listing { entries }
    }

    fn list_dir(&self, path: &str) -> anyhow::Result<Listing> {
        let local = self
            .local_path(path)
            .ok_or_else(|| anyhow!("internal: empty path passed to list_dir"))?;
        let mut entries = Vec::new();
        let mut items: Vec<_> = fs::read_dir(&local)
            .with_context(|| format!("read_dir {}", local.display()))?
            .filter_map(|r| r.ok())
            .collect();
        items.sort_by_key(|e| e.file_name());

        for it in items {
            let name = it.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let kind = it.file_type();
            let child_path = format!("{}{}", normalize_dir(path), name);
            match kind {
                Ok(t) if t.is_dir() => entries.push(Entry::Dir {
                    name,
                    path: format!("{}/", child_path),
                }),
                Ok(t) if t.is_file() => {
                    let size = it.metadata().ok().map(|m| m.len());
                    let kind = classify(&name);
                    entries.push(Entry::File {
                        name,
                        path: child_path,
                        size,
                        kind,
                    });
                }
                _ => {}
            }
        }
        Ok(Listing { entries })
    }
}

impl Source for TextfilesSource {
    fn title(&self) -> &str {
        "TEXTZ"
    }
    fn description(&self) -> &str {
        "textfiles.com archive"
    }
    fn hotkey(&self) -> char {
        'T'
    }

    fn list(&self, path: &str) -> anyhow::Result<Listing> {
        if let Ok(memo) = self.memo.lock() {
            if let Some((cached_path, cached_listing)) = memo.as_ref() {
                if cached_path == path {
                    return Ok(cached_listing.clone());
                }
            }
        }

        let listing = if path.is_empty() || path == "/" {
            self.list_root()
        } else {
            if let Some(slug) = self.section_of(path) {
                self.ensure_section(slug, &mut |_, _, _| {})?;
            }
            self.list_dir(path)?
        };

        if let Ok(mut memo) = self.memo.lock() {
            *memo = Some((path.to_string(), listing.clone()));
        }
        Ok(listing)
    }

    fn fetch(&self, path: &str) -> anyhow::Result<Blob> {
        let slug = self
            .section_of(path)
            .ok_or_else(|| anyhow!("cannot fetch root"))?;
        self.ensure_section(slug, &mut |_, _, _| {})?;

        let local = self
            .local_path(path)
            .ok_or_else(|| anyhow!("invalid path"))?;
        if !local.is_file() {
            return Err(anyhow!("not a file: {}", local.display()));
        }

        let raw = fs::read(&local).with_context(|| format!("read {}", local.display()))?;
        if raw.len() as u64 > TEXT_MAX_BYTES {
            return Err(anyhow!(
                "file too large for terminal display: {} bytes",
                raw.len()
            ));
        }

        let s = String::from_utf8_lossy(&raw);
        let mut out = String::with_capacity(raw.len());
        let mut prev_cr = false;
        for ch in s.chars() {
            match ch {
                '\0' => {}
                '\r' => {
                    out.push('\r');
                    prev_cr = true;
                }
                '\n' => {
                    if !prev_cr {
                        out.push('\r');
                    }
                    prev_cr = false;
                }
                c if (c as u32) < 0x20 && c != '\t' => {
                    prev_cr = false;
                }
                c if c.is_ascii() => {
                    out.push(c);
                    prev_cr = false;
                }
                _ => {
                    out.push('?');
                    prev_cr = false;
                }
            }
        }
        Ok(Blob::Text(out))
    }

    fn prepare(
        &self,
        path: &str,
        progress: &mut dyn FnMut(&str, u64, Option<u64>),
    ) -> anyhow::Result<()> {
        if let Some(slug) = self.section_of(path) {
            self.ensure_section(slug, progress)?;
        }
        Ok(())
    }
}

fn normalize_dir(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{}/", path)
    }
}

fn classify(name: &str) -> FileKind {
    let lower = name.to_ascii_lowercase();
    if let Some(
        "zip" | "gz" | "tgz" | "tar" | "shk" | "sit" | "bin" | "exe" | "com" | "gif" | "jpg"
        | "jpeg" | "png" | "bmp" | "pdf" | "mp3" | "wav",
    ) = lower.rsplit('.').next()
    {
        return FileKind::Binary;
    }
    FileKind::Text
}

fn http_get_bytes_progress(
    url: &str,
    max: u64,
    progress: &mut dyn FnMut(&str, u64, Option<u64>),
) -> anyhow::Result<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(HTTP_TIMEOUT)
        .timeout_read(HTTP_TIMEOUT)
        .user_agent("rust-iic-bbs/0.1")
        .build();
    let resp = agent.get(url).call().with_context(|| format!("GET {}", url))?;

    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());

    let mut reader = resp.into_reader().take(max + 1);
    let mut buf: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut chunk = [0u8; 8 * 1024];
    let mut last_reported: u64 = 0;
    progress("downloading", 0, total);
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                let len = buf.len() as u64;
                if len > max {
                    return Err(anyhow!("response exceeds {} bytes", max));
                }
                if len - last_reported >= 16 * 1024 {
                    progress("downloading", len, total);
                    last_reported = len;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    progress("downloading", buf.len() as u64, total);
    Ok(buf)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("open zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("read zip entry")?;
        let raw_name = file.name().to_string();
        let rel = match file.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                log::warn!("textfiles: skipping unsafe entry {}", raw_name);
                continue;
            }
        };
        let mut out_path = dest.join(&rel);
        if file.is_dir() {
            match fs::symlink_metadata(&out_path) {
                Ok(meta) if !meta.is_dir() => {
                    out_path.set_file_name(format!(
                        "{}_dir",
                        out_path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    log::warn!(
                        "textfiles: case-insensitive collision, renamed dir {} -> {}",
                        rel.display(),
                        out_path.display()
                    );
                }
                _ => {}
            }
            fs::create_dir_all(&out_path)
                .with_context(|| format!("mkdir {}", out_path.display()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        if let Ok(meta) = fs::symlink_metadata(&out_path) {
            if meta.is_dir() {
                out_path.set_file_name(format!(
                    "{}_file",
                    out_path.file_name().unwrap_or_default().to_string_lossy()
                ));
                log::warn!(
                    "textfiles: case-insensitive collision, renamed file {} -> {}",
                    rel.display(),
                    out_path.display()
                );
            }
        }
        let mut out = fs::File::create(&out_path)
            .with_context(|| format!("create {}", out_path.display()))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("write {}", out_path.display()))?;
    }
    Ok(())
}
