use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Binary,
    Text,
}

pub enum Blob {
    Binary(Vec<u8>),
    Text(String),
}

#[derive(Debug, Clone)]
pub enum Entry {
    Dir { name: String, path: String },
    File {
        name: String,
        path: String,
        size: Option<u64>,
        kind: FileKind,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
}

pub trait Source: Send + Sync {
    fn title(&self) -> &str;
    fn description(&self) -> &str;
    fn hotkey(&self) -> char;
    fn list(&self, path: &str) -> anyhow::Result<Listing>;
    fn fetch(&self, path: &str) -> anyhow::Result<Blob>;
    fn prepare(
        &self,
        _path: &str,
        _progress: &mut dyn FnMut(&str, u64, Option<u64>),
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_interactive(&self) -> bool {
        false
    }

    fn interactive(
        &self,
        _stream: &mut std::net::TcpStream,
    ) -> anyhow::Result<()> {
        anyhow::bail!("source is not interactive")
    }
}

pub type SourceRef = Arc<dyn Source>;
