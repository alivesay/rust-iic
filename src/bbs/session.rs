use std::io::{self, Read, Write};use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

use crate::bbs::server::BbsEvent;
use crate::bbs::source::{Blob, Entry, FileKind, Listing, SourceRef};
use crate::bbs::transport::AsciiWriter;

const READ_TIMEOUT: Duration = Duration::from_secs(120);

pub fn run(
    mut stream: TcpStream,
    sources: Vec<SourceRef>,
    events: mpsc::Sender<BbsEvent>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_nodelay(true)?;

    let mut tx = AsciiWriter::new(stream.try_clone()?);

    dialup_prelude(&mut tx)?;

    banner(&mut tx)?;
    main_menu(&mut tx, &sources)?;
    tx.flush()?;
    let mut input = [0u8; 1];
    loop {
        match stream.read(&mut input) {
            Ok(0) => break, // peer closed
            Ok(_) => {}
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
        let key = input[0].to_ascii_uppercase();
        // echo hotkey + CR
        if key.is_ascii_graphic() {
            let _ = tx.write_str(std::str::from_utf8(&[key]).unwrap_or(""));
            let _ = tx.writeln("");
            let _ = tx.flush();
        }
        match key {
            b'\r' | b'\n' => {
                main_menu(&mut tx, &sources)?;
            }
            b'G' | 0x03 /* ctrl-c */ => {
                tx.writeln("")?;
                tx.flush()?;
                break;
            }
            _ => {
                if let Some(src) = sources.iter().find(|s| s.hotkey() == key as char) {
                    if src.is_interactive() {
                        if let Err(e) = src.interactive(&mut stream) {
                            tx.writeln("")?;
                            tx.writeln(&format!("? error: {:#}", e))?;
                            log::warn!("bbs: interactive error: {:?}", e);
                        }
                    } else if let Err(e) = browse(&mut tx, &mut stream, src.clone(), "", &events) {
                        tx.writeln("")?;
                        tx.writeln(&format!("? error: {:#}", e))?;
                        log::warn!("bbs: browse error: {:?}", e);
                    }
                    main_menu(&mut tx, &sources)?;
                } else {
                    tx.writeln("")?;
                    tx.writeln("? UNKNOWN COMMAND")?;
                    main_menu(&mut tx, &sources)?;
                }
                tx.flush()?;
            }
        }
    }
    Ok(())
}


fn dialup_prelude<W: Write>(tx: &mut AsciiWriter<W>) -> io::Result<()> {
    use std::thread::sleep;
    use std::time::Duration as StdDuration;

    tx.writeln("")?;
    tx.writeln("ATZ")?;
    tx.flush()?;
    sleep(StdDuration::from_millis(120));
    tx.writeln("OK")?;
    tx.flush()?;
    sleep(StdDuration::from_millis(900));

    tx.writeln("CONNECT 9600")?;
    tx.flush()?;
    sleep(StdDuration::from_millis(150));
    Ok(())
}

fn banner<W: Write>(tx: &mut AsciiWriter<W>) -> io::Result<()> {    let lines = [
        "",
        "+--------------------------------------+",
        "|       R U S T - I I C   B B S        |",
        "|       =======================        |",
        "|          the back door v0.1          |",
        "+--------------------------------------+",
        "",
    ];
    for line in lines {
        tx.writeln(line)?;
    }
    Ok(())
}

fn main_menu<W: Write>(tx: &mut AsciiWriter<W>, sources: &[SourceRef]) -> io::Result<()> {
    tx.writeln("")?;
    tx.writeln("MAIN MENU")?;
    tx.writeln("---------")?;
    for s in sources {
        tx.writeln(&format!(" [{}] {:<10}  {}", s.hotkey(), s.title(), s.description()))?;
    }
    tx.writeln(" [G] GOODBYE     hang up")?;
    tx.writeln("")?;
    tx.write_str("CMD> ")?;
    tx.flush()
}

// Result of a `browse` recursion level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowseExit {
    Back,
    MainMenu,
}

fn browse<W: Write>(
    tx: &mut AsciiWriter<W>,
    rx: &mut TcpStream,
    source: SourceRef,
    path: &str,
    events: &mpsc::Sender<BbsEvent>,
) -> anyhow::Result<BrowseExit> {

    run_prepare(tx, &source, path)?;

    let listing: Listing = source.list(path)?;
    let mut preselect: Option<usize> = None;
    let mut redraw = true;

    loop {
        if preselect.is_none() {
            if redraw {
                if let Some(n) = render_listing(tx, rx, &source, path, &listing)? {
                    preselect = Some(n);
                } else {
                    tx.writeln("")?;
                    tx.write_str("(L)IST  (B)ACK  (M)ENU  [#]> ")?;
                    tx.flush()?;
                }
                redraw = false;
            } else {
                tx.writeln("")?;
                tx.write_str("(L)IST  (B)ACK  (M)ENU  [#]> ")?;
                tx.flush()?;
            }
        }

        let idx = if let Some(n) = preselect.take() {
            n
        } else {
            let pick = read_line(tx, rx)?;
            let trimmed = pick.trim();
            if trimmed.is_empty() {
                continue;
            }
            let upper = trimmed.to_ascii_uppercase();
            if upper == "B" {
                return Ok(BrowseExit::Back);
            }
            if upper == "M" || upper == "Q" {
                return Ok(BrowseExit::MainMenu);
            }
            if upper == "L" {
                redraw = true;
                continue;
            }
            let Ok(idx) = trimmed.parse::<usize>() else {
                tx.writeln("? unknown command")?;
                continue;
            };
            idx
        };

        if idx == 0 || idx > listing.entries.len() {
            tx.writeln("? out of range")?;
            continue;
        }

        match &listing.entries[idx - 1] {
            Entry::Dir { path: sub, .. } => {
                let sub = sub.clone();
                match browse(tx, rx, source.clone(), &sub, events)? {
                    BrowseExit::Back => {
                        redraw = true;
                        continue;
                    }
                    BrowseExit::MainMenu => return Ok(BrowseExit::MainMenu),
                }
            }
            Entry::File { path: file_path, kind, name, .. } => {
                run_prepare(tx, &source, file_path)?;
                match source.fetch(file_path)? {
                    Blob::Text(s) => match view_text(tx, rx, name, &s, events)? {
                        TextExit::Listing => {}
                        TextExit::MainMenu => return Ok(BrowseExit::MainMenu),
                        TextExit::Pick(n) => {
                            preselect = Some(n);
                        }
                    },
                    Blob::Binary(bytes) if matches!(kind, FileKind::Binary) => {
                        deliver_binary(tx, rx, name, bytes, events)?;
                    }
                    Blob::Binary(bytes) => {
                        deliver_binary(tx, rx, name, bytes.clone(), events)?;
                        tx.writeln("")?;
                        tx.write_str("View it now? (Y/n) ")?;
                        tx.flush()?;
                        let ans = read_line(tx, rx)?;
                        let yes = ans.trim().is_empty()
                            || ans.trim().eq_ignore_ascii_case("y")
                            || ans.trim().eq_ignore_ascii_case("yes");
                        if yes {
                            let text = String::from_utf8_lossy(&bytes).into_owned();
                            match view_text(tx, rx, name, &text, events)? {
                                TextExit::Listing => {}
                                TextExit::MainMenu => return Ok(BrowseExit::MainMenu),
                                TextExit::Pick(n) => {
                                    preselect = Some(n);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_listing<W: Write>(
    tx: &mut AsciiWriter<W>,
    rx: &mut TcpStream,
    source: &SourceRef,
    path: &str,
    listing: &Listing,
) -> anyhow::Result<Option<usize>> {
    drain_input(rx);
    let mut pager = Pager::new();
    pager.writeln(tx, rx, "")?;
    pager.writeln(
        tx,
        rx,
        &format!(
            "== {} :: {} ==",
            source.title(),
            if path.is_empty() { "/" } else { path }
        ),
    )?;
    for (i, entry) in listing.entries.iter().enumerate() {
        if pager.aborted {
            break;
        }
        let line = match entry {
            Entry::Dir { name, .. } => format!(" {:>3}  <DIR>  {}", i + 1, name),
            Entry::File { name, size, kind, .. } => {
                let size = size.map(format_size).unwrap_or_else(|| "     ?".into());
                let tag = match kind {
                    FileKind::Binary => "BIN",
                    FileKind::Text => "TXT",
                };
                format!(" {:>3}  {} {}  {}", i + 1, tag, size, name)
            }
        };
        pager.writeln(tx, rx, &line)?;
    }
    Ok(pager.picked)
}

const VIEW_ROWS: usize = 22; // 24 rows total - 2 for header/footer

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextExit {
    Listing,
    MainMenu,
    Pick(usize),
}

fn view_text<W: Write>(
    tx: &mut AsciiWriter<W>,
    rx: &mut TcpStream,
    name: &str,
    body: &str,
    events: &mpsc::Sender<BbsEvent>,
) -> anyhow::Result<TextExit> {
    drain_input(rx);
    let lines = wrap_text(body, SCREEN_COLS);
    let total = lines.len().max(1);
    let pages = (total + VIEW_ROWS - 1) / VIEW_ROWS;
    let mut page: usize = 0;

    loop {
        // Header
        tx.writeln("")?;
        tx.writeln(&format!(
            "--- {}  [page {}/{}] ---",
            truncate(name, 50),
            page + 1,
            pages.max(1)
        ))?;

        let start = page * VIEW_ROWS;
        let end = (start + VIEW_ROWS).min(total);
        for line in &lines[start..end] {
            tx.writeln(line)?;
        }
        // Pad short last page so the prompt always sits at the bottom.
        for _ in (end - start)..VIEW_ROWS {
            tx.writeln("")?;
        }

        // Footer / prompt
        let at_end = page + 1 >= pages;
        let at_top = page == 0;
        tx.write_str(&format!(
            "{} (SPACE=fwd  B=back  T=top  G=end  S=save  Q=quit  M=menu  [#]=pick)> ",
            if at_end {
                "[END]"
            } else if at_top {
                "[TOP]"
            } else {
                "     "
            }
        ))?;
        tx.flush()?;

        let key = read_key(rx)?;
        match key {
            None => return Ok(TextExit::Listing),
            Some(b'M') => return Ok(TextExit::MainMenu),
            Some(b'Q') | Some(0x03) | Some(b'X') => return Ok(TextExit::Listing),
            Some(b'S') => {
                // save the rendered text to disks/bbs/
                deliver_binary(tx, rx, name, body.as_bytes().to_vec(), events)?;
            }
            Some(b' ') | Some(b'F') | Some(b'N') | Some(b'\r') | Some(b'\n') => {
                if !at_end {
                    page += 1;
                }
            }
            Some(b'B') | Some(b'-') | Some(b'P') => {
                if !at_top {
                    page -= 1;
                }
            }
            Some(b'T') | Some(b'H') => page = 0,
            Some(b'G') | Some(b'E') => page = pages.saturating_sub(1),
            Some(c) if c.is_ascii_digit() => {
                tx.writeln("")?;
                tx.write_str("PICK> ")?;
                tx.write_str(std::str::from_utf8(&[c]).unwrap_or(""))?;
                tx.flush()?;
                let mut buf = String::new();
                buf.push(c as char);
                let rest = read_line(tx, rx)?;
                buf.push_str(rest.trim());
                if let Ok(n) = buf.trim().parse::<usize>() {
                    return Ok(TextExit::Pick(n));
                }
            }
            _ => {} // redraw
        }
    }
}

fn format_size(n: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = 1024 * 1024;
    const G: u64 = 1024 * 1024 * 1024;
    if n < K * 10 {
        format!("{:>6}", n)
    } else if n < M {
        format!("{:>5}K", n / K)
    } else if n < M * 10 {
        format!("{:>5.1}M", n as f64 / M as f64)
    } else if n < G {
        format!("{:>5}M", n / M)
    } else {
        format!("{:>5.1}G", n as f64 / G as f64)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}

fn wrap_text(body: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in body.split(|c| c == '\r' || c == '\n') {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        // remove tabs, expand to 4 spaces
        let line: String = raw_line
            .chars()
            .flat_map(|c| {
                if c == '\t' {
                    "    ".chars().collect::<Vec<_>>()
                } else if c.is_ascii() {
                    vec![c]
                } else {
                    vec!['?']
                }
            })
            .collect();

        if line.len() <= cols {
            out.push(line);
            continue;
        }

        // word-wrap.
        let mut start = 0usize;
        let bytes = line.as_bytes();
        while start < bytes.len() {
            let remaining = bytes.len() - start;
            if remaining <= cols {
                out.push(line[start..].to_string());
                break;
            }
            let mut end = start + cols;
            // Walk back to a space if we're mid-word.
            let mut split = end;
            while split > start && bytes[split - 1] != b' ' {
                split -= 1;
            }
            if split == start {
                // No space found; hard-break.
                split = end;
            } else {
                end = split;
            }
            out.push(line[start..end].trim_end().to_string());
            start = split;
            // Skip a single leading space carried into the next line.
            while start < bytes.len() && bytes[start] == b' ' {
                start += 1;
            }
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn read_key(rx: &mut TcpStream) -> anyhow::Result<Option<u8>> {
    let mut buf = [0u8; 1];
    let key = loop {
        match rx.read(&mut buf) {
            Ok(0) => return Ok(None),
            Ok(_) => break buf[0].to_ascii_uppercase(),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    };
    drain_input(rx);
    Ok(Some(key))
}

fn drain_input(rx: &mut TcpStream) {
    let prev = rx.read_timeout().ok().flatten();
    let _ = rx.set_read_timeout(Some(Duration::from_millis(80)));
    let mut scratch = [0u8; 64];

    loop {
        match rx.read(&mut scratch) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    let _ = rx.set_read_timeout(prev);
}

fn deliver_binary<W: Write>(
    tx: &mut AsciiWriter<W>,
    rx: &mut TcpStream,
    name: &str,
    bytes: Vec<u8>,
    events: &mpsc::Sender<BbsEvent>,
) -> anyhow::Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let dir = PathBuf::from("disks/bbs");
    fs::create_dir_all(&dir)?;

    // Sanitise the filename.
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let path = dir.join(if safe.is_empty() { "download.bin" } else { &safe });

    let kind_label = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_ascii_uppercase();

    if path.exists() {
        let existing_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tx.writeln("")?;
        tx.writeln(&format!(
            "*** EXISTS {} ({} bytes) -> {}",
            kind_label,
            existing_size,
            path.display()
        ))?;
        tx.write_str("Overwrite? (y/N) ")?;
        tx.flush()?;
        let ans = read_line(tx, rx)?;
        let yes = ans.trim().eq_ignore_ascii_case("y") || ans.trim().eq_ignore_ascii_case("yes");
        if !yes {
            tx.writeln(&format!("*** KEPT {}", path.display()))?;
            tx.flush()?;
            let _ = events.send(BbsEvent::DownloadCompleted {
                name: name.to_string(),
                path,
                kind: FileKind::Binary,
            });
            return Ok(());
        }
    }

    fs::write(&path, &bytes)?;

    tx.writeln("")?;
    tx.writeln(&format!(
        "*** SAVED {} ({} bytes) -> {}",
        kind_label,
        bytes.len(),
        path.display()
    ))?;
    tx.flush()?;

    let _ = events.send(BbsEvent::DownloadCompleted {
        name: name.to_string(),
        path,
        kind: FileKind::Binary,
    });

    Ok(())
}

fn run_prepare<W: Write>(
    tx: &mut AsciiWriter<W>,
    source: &SourceRef,
    path: &str,
) -> anyhow::Result<()> {
    use std::time::Instant;
    let mut started = false;
    let mut last_paint = Instant::now();
    let mut last_label = String::new();
    let mut dot_count = 0usize;
    source.prepare(path, &mut |label, bytes, total| {
        if !started {
            let _ = tx.writeln("");
            last_label = label.to_string();
            let _ = tx.write_str(&format!("{}: ", &last_label));
            let _ = tx.flush();
            started = true;
            last_paint = Instant::now();
            return;
        }
        if label != last_label {
            let _ = tx.writeln("");
            last_label = label.to_string();
            let _ = tx.write_str(&format!("{}: ", &last_label));
            let _ = tx.flush();
            dot_count = 0;
            last_paint = Instant::now();
        }
        if last_paint.elapsed().as_millis() < 750 && total.is_some() {
            return;
        }
        last_paint = Instant::now();
        if let Some(total) = total {
            const BAR: usize = 30;
            let pct = if total == 0 { 0.0 } else { bytes as f32 / total as f32 };
            let filled = ((pct * BAR as f32).round() as usize).min(BAR);
            let mut line = String::with_capacity(BAR + 48);
            line.push('\r');
            line.push_str(&last_label);
            line.push_str(": [");
            for i in 0..BAR {
                line.push(if i < filled { '#' } else { '.' });
            }
            line.push_str(&format!(
                "] {:>4}/{:<4} KB",
                bytes / 1024,
                total / 1024
            ));
            let _ = tx.write_str(&line);
            let _ = tx.flush();
        } else {
            if last_paint.elapsed().as_millis() < 750 {
                return;
            }
            let _ = tx.write_str(".");
            dot_count += 1;
            if dot_count % 30 == 0 {
                let _ = tx.write_str(&format!(" {} KB\r{}: ", bytes / 1024, &last_label));
            }
            let _ = tx.flush();
        }
    })?;
    if started {
        let _ = tx.writeln("");
        let _ = tx.flush();
    }
    Ok(())
}

fn read_line<W: Write>(
    tx: &mut AsciiWriter<W>,
    rx: &mut TcpStream,
) -> anyhow::Result<String> {
    let mut buf = String::new();
    let mut byte = [0u8; 1];
    loop {
        match rx.read(&mut byte) {
            Ok(0) => return Ok(buf),
            Ok(_) => {}
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
        let c = byte[0];
        match c {
            b'\r' | b'\n' => {
                tx.writeln("")?;
                tx.flush()?;
                return Ok(buf);
            }
            0x08 | 0x7f => {
                if buf.pop().is_some() {
                    // BS, space, BS to erase last char on the screen
                    tx.write_str("\x08 \x08")?;
                    tx.flush()?;
                }
            }
            0x03 => return Ok(String::new()),
            c if c.is_ascii_graphic() || c == b' ' => {
                buf.push(c as char);
                tx.write_str(std::str::from_utf8(&[c]).unwrap_or(""))?;
                tx.flush()?;
            }
            _ => {}
        }
    }
}

const SCREEN_COLS: usize = 80;
const PAGE_ROWS: usize = 22;

struct Pager {
    rows: usize,
    aborted: bool,
    picked: Option<usize>,
}

impl Pager {
    fn new() -> Self {
        Self { rows: 0, aborted: false, picked: None }
    }

    fn writeln<W: Write>(
        &mut self,
        tx: &mut AsciiWriter<W>,
        rx: &mut TcpStream,
        line: &str,
    ) -> anyhow::Result<()> {
        if self.aborted {
            return Ok(());
        }
        tx.writeln(line)?;
        // wrapped lines consume floor(len/40)+1 screen rows
        let wraps = (line.len() / SCREEN_COLS).max(0);
        self.rows += 1 + wraps;
        if self.rows >= PAGE_ROWS {
            self.prompt(tx, rx)?;
        }
        Ok(())
    }

    fn prompt<W: Write>(
        &mut self,
        tx: &mut AsciiWriter<W>,
        rx: &mut TcpStream,
    ) -> anyhow::Result<()> {
        tx.write_str("-- MORE -- (SPACE/Q/[#]) ")?;
        tx.flush()?;
        let mut buf = [0u8; 1];
        loop {
            match rx.read(&mut buf) {
                Ok(0) => {
                    self.aborted = true;
                    return Ok(());
                }
                Ok(_) => break,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e.into()),
            }
        }

        drain_input(rx);
        // erase the prompt with CRs+spaces
        tx.write_str("\r                                                                                \r")?;
        tx.flush()?;
        let raw = buf[0];
        let key = raw.to_ascii_uppercase();
        if key == b'Q' || key == 0x03 {
            self.aborted = true;
            tx.writeln("")?;
            tx.writeln("[ABORTED]")?;
        } else if raw.is_ascii_digit() {
            tx.writeln("")?;
            tx.write_str("PICK> ")?;
            tx.write_str(std::str::from_utf8(&[raw]).unwrap_or(""))?;
            tx.flush()?;
            let mut s = String::new();
            s.push(raw as char);
            let rest = read_line(tx, rx)?;
            s.push_str(rest.trim());
            if let Ok(n) = s.trim().parse::<usize>() {
                self.picked = Some(n);
            }
            self.aborted = true;
        }
        self.rows = 0;
        Ok(())
    }
}

