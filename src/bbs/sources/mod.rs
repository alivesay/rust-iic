// Source plugin registry.

use std::sync::Arc;

use crate::bbs::source::{Blob, Entry, FileKind, Listing, Source, SourceRef};

pub mod asimov;
pub mod textfiles;
pub mod telnet;

pub fn all() -> Vec<SourceRef> {
    vec![
        Arc::new(asimov::AsimovSource::new()) as SourceRef,
        Arc::new(textfiles::TextfilesSource::new()) as SourceRef,
        Arc::new(telnet::TelnetSource::new()) as SourceRef,
        Arc::new(WelcomeSource) as SourceRef,
    ]
}

struct WelcomeSource;

impl Source for WelcomeSource {
    fn title(&self) -> &str {
        "INFO"
    }
    fn description(&self) -> &str {
        "about this bbs"
    }
    fn hotkey(&self) -> char {
        'I'
    }
    fn list(&self, _path: &str) -> anyhow::Result<Listing> {
        Ok(Listing {
            entries: vec![
                Entry::File {
                    name: "README".into(),
                    path: "readme".into(),
                    size: None,
                    kind: FileKind::Text,
                },
                Entry::File {
                    name: "PHRACK".into(),
                    path: "phrack".into(),
                    size: None,
                    kind: FileKind::Text,
                },
            ],
        })
    }
    fn fetch(&self, path: &str) -> anyhow::Result<Blob> {
        match path {
            "readme" => Ok(Blob::Text(
                "                                          \r\n\
                 RUST-IIC BBS  -- a wrapper around public  \r\n\
                 archives, dialed from inside an emulated  \r\n\
                 Apple //c.  Type a hotkey at the main     \r\n\
                 menu to enter a section, B to back out.   \r\n\
                                                           \r\n"
                    .to_string(),
            )),
            "phrack" => Ok(Blob::Text(
                "                                          \r\n\
                    My name is Ozymandias, king of kings: \r\n\
                    Look on my works, ye mighty, and despair! \r\n\
                    Nothing beside remains. Round the decay \r\n\
                    Of that colossal wreck, boundless and bare \r\n\
                    The lone and level sands stretch far away.\r\n"
                    .to_string(),
            )),
            other => anyhow::bail!("no such file: {}", other),
        }
    }
}
