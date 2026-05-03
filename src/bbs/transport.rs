use std::io::{self, Write};

pub struct AsciiWriter<W: Write> {
    inner: W,
}

impl<W: Write> AsciiWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn writeln(&mut self, s: &str) -> io::Result<()> {
        self.write_str(s)?;
        self.inner.write_all(b"\r")
    }

    pub fn write_str(&mut self, s: &str) -> io::Result<()> {
        for ch in s.bytes() {
            let b = match ch {
                b'\n' => b'\r',
                _ => ch,
            };
            self.inner.write_all(&[b])?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
