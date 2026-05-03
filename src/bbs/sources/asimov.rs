use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context};

use crate::bbs::source::{Blob, Entry, FileKind, Listing, Source};
use crate::config;

const BASE_URL: &str = "https://www.apple.asimov.net/images/";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const INDEX_TTL: Duration = Duration::from_secs(60 * 60 * 24); // 1 day
const BLOB_MAX_BYTES: u64 = 128 * 1024 * 1024;

const BINARY_EXTS: &[&str] = &[
    "dsk", "do", "po", "nib", "woz", "2mg", "hdv", "shk", "sdk",
    "zip", "gz", "tgz", "tar", "bz2", "7z", "rar",
    "bin", "rom", "img", "iso",
];

const TEXT_EXTS: &[&str] = &["txt", "md", "readme", "nfo", "diz", "doc", "asc", "log"];

pub struct AsimovSource {
    cache_root: PathBuf,

    memo: Mutex<Option<(String, Listing)>>,
}

impl AsimovSource {
    pub fn new() -> Self {
        let cache_root = config::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cache")
            .join("asimov");
        let _ = fs::create_dir_all(cache_root.join("index"));
        let _ = fs::create_dir_all(cache_root.join("blob"));
        Self {
            cache_root,
            memo: Mutex::new(None),
        }
    }

    fn url_for(&self, path: &str) -> String {
        if path.is_empty() {
            BASE_URL.to_string()
        } else {
            // path is already relative to BASE_URL; preserve trailing slash
            format!("{}{}", BASE_URL, path)
        }
    }

    fn cache_path(&self, kind: &str, path: &str) -> PathBuf {
        let safe = path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();
        let key = if safe.is_empty() { "root".into() } else { safe };
        self.cache_root.join(kind).join(format!("{}.bin", key))
    }

    fn http_get_text(&self, url: &str) -> anyhow::Result<String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(HTTP_TIMEOUT)
            .timeout_read(HTTP_TIMEOUT)
            .user_agent("rust-iic-bbs/0.1")
            .build();
        let resp = agent.get(url).call().with_context(|| format!("GET {}", url))?;
        let body = resp.into_string().context("read body")?;
        Ok(body)
    }

    fn http_get_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(HTTP_TIMEOUT)
            .timeout_read(HTTP_TIMEOUT)
            .user_agent("rust-iic-bbs/0.1")
            .build();
        let resp = agent.get(url).call().with_context(|| format!("GET {}", url))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(BLOB_MAX_BYTES + 1)
            .read_to_end(&mut buf)?;
        if buf.len() as u64 > BLOB_MAX_BYTES {
            return Err(anyhow!("blob exceeds {} bytes", BLOB_MAX_BYTES));
        }
        Ok(buf)
    }

    fn fresh_enough(path: &std::path::Path, ttl: Duration) -> bool {
        if let Ok(meta) = fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                if let Ok(age) = SystemTime::now().duration_since(modified) {
                    return age < ttl;
                }
            }
        }
        false
    }
}

impl Source for AsimovSource {
    fn title(&self) -> &str {
        "FILEZ"
    }
    fn description(&self) -> &str {
        "asimov.net disk archive"
    }
    fn hotkey(&self) -> char {
        'F'
    }

    fn list(&self, path: &str) -> anyhow::Result<Listing> {
        if let Ok(memo) = self.memo.lock() {
            if let Some((cached_path, cached_listing)) = memo.as_ref() {
                if cached_path == path {
                    return Ok(cached_listing.clone());
                }
            }
        }

        // disk cache
        let cache = self.cache_path("index", path);
        let html = if Self::fresh_enough(&cache, INDEX_TTL) {
            fs::read_to_string(&cache).unwrap_or_default()
        } else {
            let body = self.http_get_text(&self.url_for(path))?;
            let _ = fs::write(&cache, &body);
            body
        };

        let listing = parse_index(&html, path);

        if let Ok(mut memo) = self.memo.lock() {
            *memo = Some((path.to_string(), listing.clone()));
        }
        Ok(listing)
    }

    fn fetch(&self, path: &str) -> anyhow::Result<Blob> {
        let cache = self.cache_path("blob", path);
        let bytes = if cache.exists() {
            fs::read(&cache).context("read cached blob")?
        } else {
            let body = self.http_get_bytes(&self.url_for(path))?;
            let _ = fs::write(&cache, &body);
            body
        };
        Ok(Blob::Binary(bytes))
    }
}

// `take(...).read_to_end`
use std::io::Read;

fn parse_index(html: &str, parent: &str) -> Listing {
    let mut entries = Vec::new();
    let re = regex::Regex::new(r#"(?i)<a\s+href="([^"]+)"\s*>([^<]*)</a>"#).unwrap();
    for cap in re.captures_iter(html) {
        let m = cap.get(0).unwrap();
        let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");

        if href.is_empty() {
            continue;
        }
        if href.starts_with('?') || href.starts_with('/') || href.starts_with("http") {
            continue; // sort links / absolute escapes
        }
        if href == "../" {
            continue;
        }

        let raw_name = url_decode(href.trim_end_matches('/'));

        let name = raw_name.trim_end_matches('/').to_string();
        let child = format!("{}{}", parent, href);
        if href.ends_with('/') {
            entries.push(Entry::Dir { name, path: child });
        } else {
            let kind = classify(&name);

            let after = &html[m.end()..];
            let line_end = after.find('\n').unwrap_or(after.len());
            let tail = &after[..line_end];
            let size = parse_size(tail);
            entries.push(Entry::File {
                name,
                path: child,
                size,
                kind,
            });
        }
    }
    Listing { entries }
}

fn parse_size(tail: &str) -> Option<u64> {
    let tok = tail.split_whitespace().last()?;
    if tok == "-" {
        return None;
    }
    let (num_str, mult): (&str, u64) = match tok.as_bytes().last() {
        Some(b'K') | Some(b'k') => (&tok[..tok.len() - 1], 1024),
        Some(b'M') | Some(b'm') => (&tok[..tok.len() - 1], 1024 * 1024),
        Some(b'G') | Some(b'g') => (&tok[..tok.len() - 1], 1024 * 1024 * 1024),
        _ => (tok, 1),
    };
    let n: f64 = num_str.parse().ok()?;
    Some((n * mult as f64) as u64)
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn classify(name: &str) -> FileKind {
    let lower = name.to_ascii_lowercase();
    if let Some(ext) = lower.rsplit('.').next() {
        if TEXT_EXTS.contains(&ext) {
            return FileKind::Text;
        }
        if BINARY_EXTS.contains(&ext) {
            return FileKind::Binary;
        }
    }

    FileKind::Binary
}
