use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::bbs::source::{Blob, Listing, Source};

const READ_TIMEOUT: Duration = Duration::from_millis(150);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

const PHONEBOOK: &[(&str, &str, u16)] = &[
    ("TELEHACK", "telehack.com", 23),
    ("ISCABBS", "bbs.iscabbs.com", 23),
    ("FOZZTEXX", "bbs.fozztexx.com", 23),
    ("A-NET", "bbs.a-net.online", 1337),
    ("DURA-BBS", "dura-bbs.net", 6359),
    ("WACBBS", "wacbbs.ddns.net", 6502),
    ("HEATWAVE", "heatwave.ddns.net", 9640),
    ("CAPTAINS", "cqbbs.ddns.net", 6800),
    ("PRO-KEGS", "proline.ksherlock.com", 6523)
];

pub struct TelnetSource;

impl TelnetSource {
    pub fn new() -> Self {
        Self
    }
}

impl Source for TelnetSource {
    fn title(&self) -> &str {
        "TELNET"
    }
    fn description(&self) -> &str {
        "dial out to vt100 hosts"
    }
    fn hotkey(&self) -> char {
        'N'
    }
    fn list(&self, _path: &str) -> anyhow::Result<Listing> {
        Ok(Listing::default())
    }
    fn fetch(&self, _path: &str) -> anyhow::Result<Blob> {
        anyhow::bail!("telnet source is interactive, not fetchable")
    }
    fn is_interactive(&self) -> bool {
        true
    }
    fn interactive(&self, stream: &mut TcpStream) -> anyhow::Result<()> {
        run_session(stream)
    }
}

fn run_session(stream: &mut TcpStream) -> anyhow::Result<()> {
    write_line(stream, "")?;
    write_line(stream, "== TELNET ==")?;
    write_line(stream, "")?;
    for (i, (label, host, port)) in PHONEBOOK.iter().enumerate() {
        write_line(
            stream,
            &format!("  [{}] {:<10} {}:{}", i + 1, label, host, port),
        )?;
    }
    write_line(stream, "  [C] custom host:port")?;
    write_line(stream, "  [Q] back to main menu")?;
    write_line(stream, "")?;
    write_str(stream, "select> ")?;

    let pick = read_line(stream)?;
    let trimmed = pick.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("q") {
        return Ok(());
    }

    let (host, port): (String, u16) = if trimmed.eq_ignore_ascii_case("c") {
        write_str(stream, "host:port> ")?;
        let raw = read_line(stream)?;
        parse_hostport(raw.trim())?
    } else {
        let n: usize = trimmed
            .parse()
            .map_err(|_| anyhow::anyhow!("bad selection"))?;
        let entry = PHONEBOOK
            .get(n.wrapping_sub(1))
            .ok_or_else(|| anyhow::anyhow!("out of range"))?;
        (entry.1.to_string(), entry.2)
    };

    write_str(stream, "DHIRES? (y/n) ")?;
    let mode = read_line(stream)?;
    let dhires = matches!(mode.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if dhires {
        write_str(stream, "80COL? (y/n) ")?;
        let cols_in = read_line(stream)?;
        let eighty = matches!(cols_in.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        write_line(stream, "")?;
        let (module, rows, cols, write_cols, label) = if eighty {
            (
                DHGR_MODULE,
                24usize,
                80usize,
                80usize,
                "80-col DHGR (text+color flicker)",
            )
        } else {
            (
                DHGR40_MODULE,
                24usize,
                40usize,
                40usize,
                "40-col DHGR true color",
            )
        };
        write_line(stream, &format!("*** uploading {} ...", label))?;
        upload_module(stream, module)?;

        // brief pause to let the //c module take over the SCC
        std::thread::sleep(Duration::from_millis(100));

        write_line(stream, &format!("Dialing {}:{} ...", host, port))?;

        let addr = (host.as_str(), port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("no address resolved"))?;
        let remote = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        remote.set_nodelay(true)?;
        remote.set_read_timeout(Some(READ_TIMEOUT))?;

        let result = pump(stream, remote, rows, cols, write_cols);

        // tell the module to exit back to text mode + boot terminal
        let _ = stream.write_all(&[0x1b, b'X']);
        let _ = stream.flush();
        return result;
    }

    write_line(stream, "")?;
    write_line(stream, &format!("Dialing {}:{} ...", host, port))?;

    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("no address resolved"))?;
    let remote = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    remote.set_nodelay(true)?;
    remote.set_read_timeout(Some(READ_TIMEOUT))?;

    write_line(stream, "Connected.  Press Ctrl-] to disconnect.")?;
    write_line(stream, "")?;

    // 80-col firmware on the //c auto-wraps if column 79 is written
    // skip the last column to keep diffs from corrupting other rows
    let result = pump(stream, remote, 24, 80, 79);

    write_line(stream, "")?;
    write_line(stream, "*** disconnected ***")?;
    result
}

// Drive a remote VT100 stream into a //c terminal that speaks the
// `ESC =` cursor address protocol.
//
// `rows`/`cols` size the virtual screen.  `write_cols` is the number
// of columns we actually emit per row
fn pump(
    local: &mut TcpStream,
    mut remote: TcpStream,
    rows: usize,
    cols: usize,
    write_cols: usize,
) -> anyhow::Result<()> {
    local.set_read_timeout(Some(READ_TIMEOUT))?;

    let mut parser = vt100::Parser::new(rows as u16, cols as u16, 0);
    // per-cell shadow grid: (ch, fg_dhgr, bg_dhgr).  fg/bg = 0xFF
    // means "vt100 reported Default", which we resolve at emit time.
    let mut shadow: Vec<(u8, u8, u8)> = vec![(b' ', 0xFF, 0xFF); rows * cols];
    let mut first_paint = true;
    let mut last_paint = std::time::Instant::now();
    let paint_every = Duration::from_millis(80);

    const LINE_BAUD: u32 = 9600;
    const BITS_PER_CHAR: u32 = 10;
    let bytes_per_sec = (LINE_BAUD / BITS_PER_CHAR) as f64;
    let mut last_read = std::time::Instant::now();
    let mut read_credit = 0.0_f64;

    let tx_bytes_per_sec = bytes_per_sec; // 960 B/s
    let mut last_tx = std::time::Instant::now();
    let mut tx_credit = 0.0_f64;

    let mut local_buf = [0u8; 256];
    let mut remote_buf = [0u8; 256];
    let mut iac = IacState::default();

    let mut utf8_carry: Vec<u8> = Vec::with_capacity(4);
    let mut shadow_cursor: (u16, u16) = (0, 0);

    log::info!(
        "telnet pump: entering ({}x{}, write_cols={})",
        rows,
        cols,
        write_cols
    );

    loop {
        // local -> remote
        match local.read(&mut local_buf) {
            Ok(0) => {
                log::info!("telnet pump: local EOF, exiting");
                break;
            }
            Ok(n) => {
                log::debug!("telnet local rx {}B: {}", n, debug_dump(&local_buf[..n]));
                for &b in &local_buf[..n] {
                    // Ctrl-] (GS, 0x1d) disconnects
                    if b == 0x1d {
                        log::info!("telnet pump: Ctrl-] received, exiting");
                        return Ok(());
                    }
                }
                remote.write_all(&local_buf[..n])?;
            }
            Err(ref e) if is_timeout(e) => {}
            Err(e) => {
                log::warn!("telnet pump: local read err: {}", e);
                return Err(e.into());
            }
        }

        // IAC stripping + vt100 parse
        let now = std::time::Instant::now();
        let dt = now.duration_since(last_read).as_secs_f64();
        last_read = now;
        read_credit = (read_credit + dt * bytes_per_sec).min(remote_buf.len() as f64);
        let max_read = read_credit as usize;
        if max_read > 0 {
            match remote.read(&mut remote_buf[..max_read]) {
                Ok(0) => {
                    log::info!("telnet pump: remote EOF, exiting");
                    return Ok(());
                }
                Ok(n) => {
                    read_credit -= n as f64;
                    let cleaned_now = iac.process(&mut remote, &remote_buf[..n])?;
                    // re-attach any UTF-8 bytes left over from the
                    // previous chunk so a multi-byte sequence split
                    // across TCP reads decodes cleanly
                    let cleaned: Vec<u8> = if utf8_carry.is_empty() {
                        cleaned_now
                    } else {
                        let mut v = std::mem::take(&mut utf8_carry);
                        v.extend_from_slice(&cleaned_now);
                        v
                    };
                    // stash trailing partial UTF-8 so a multi-byte
                    // sequence split across TCP reads decodes cleanly
                    // on the next pass
                    let stash_n = trailing_utf8_partial_len(&cleaned);
                    if stash_n > 0 && stash_n <= cleaned.len() {
                        utf8_carry.extend_from_slice(&cleaned[cleaned.len() - stash_n..]);
                    }
                    let cleaned = &cleaned[..cleaned.len() - stash_n];

                    let mut scrubbed: Vec<u8> = Vec::with_capacity(cleaned.len());
                    utf8_to_ascii(cleaned, &mut scrubbed);

                    parser.process(&scrubbed);
                }
                Err(ref e) if is_timeout(e) => {}
                Err(e) => {
                    log::warn!("telnet pump: remote read err: {}", e);
                    return Err(e.into());
                }
            }
        }

        // periodic repaint: build new 24x80 grid, diff against shadow,
        // emit minimal cursor-position + character runs.  The //c
        // 80-col firmware understands ESC = (32+row) (32+col) as a
        // VT52-style cursor address, so we can update only the cells
        // that changed instead of redrawing the whole frame.
        if last_paint.elapsed() >= paint_every {
            // replenish the //c-wire byte budget based on real time
            let now_tx = std::time::Instant::now();
            tx_credit = (tx_credit
                + now_tx.duration_since(last_tx).as_secs_f64() * tx_bytes_per_sec)
                .min(tx_bytes_per_sec * 0.5); // cap at 0.5s burst
            last_tx = now_tx;
            // out of budget?  Don't paint this round, but keep pumping
            let paint_now = tx_credit >= 1.0;
            if paint_now {
                last_paint = std::time::Instant::now();
            }
            if paint_now {
                let screen = parser.screen();

                // flatten current screen into a rows*cols grid of
                // (ch, fg_dhgr, bg_dhgr), per-cell color only
                let mut new_grid: Vec<(u8, u8, u8)> = Vec::with_capacity(rows * cols);
                for row in 0..rows {
                    for col in 0..cols {
                        let cell = screen.cell(row as u16, col as u16);
                        let ch = cell
                            .map(|c| c.contents().chars().next().unwrap_or(' '))
                            .unwrap_or(' ');
                        let b = if (0x20..=0x7e).contains(&(ch as u32)) {
                            ch as u8
                        } else if ch == '\0' {
                            b' '
                        } else {
                            b'?'
                        };
                        let (fg, bg) = cell
                            .map(|c| {
                                // promote `[1;30m` (bold + black) to grey (bright-black, idx 8)
                                let raw_fg = match c.fgcolor() {
                                    vt100::Color::Idx(0) if c.bold() => vt100::Color::Idx(8),
                                    other => other,
                                };
                                let mut f = match raw_fg {
                                    vt100::Color::Default => 0xFFu8,
                                    other => ansi_to_dhgr(other, true),
                                };
                                let mut b = match c.bgcolor() {
                                    vt100::Color::Default => 0xFFu8,
                                    other => ansi_to_dhgr(other, false),
                                };
                                // reverse-video (`ESC[7m`) swaps fg/bg
                                if c.inverse() {
                                    let resolved_f = if f == 0xFF { 15 } else { f };
                                    let resolved_b = if b == 0xFF { 0 } else { b };
                                    f = resolved_b;
                                    b = resolved_f;
                                }
                                (f, b)
                            })
                            .unwrap_or((0xFF, 0xFF));
                        new_grid.push((b, fg, bg));
                    }
                }

                let mut out: Vec<u8> = Vec::new();
                // track currently-active color, only emit ESC F when it changes
                let mut cur_color: u8 = 0xF0;

                if first_paint {
                    // home + clear, then paint every cell of every row.
                    // Don't stop at last-non-default: leftover DHGR or
                    // text-page memory from a previous module load can
                    // tint cells we never overwrite.  Painting the full
                    // grid (even default-bg spaces) forces the //c to
                    // re-plot each cell with our resolved color and
                    // wipes stale state.
                    out.extend_from_slice(b"\x0c");
                    for r in 0..rows {
                        let row_start = r * cols;
                        out.extend_from_slice(&[0x1b, b'=', 32 + r as u8, 32]);
                        for c in 0..write_cols {
                            let (ch, fg_raw, bg_raw) = new_grid[row_start + c];
                            let want = pack_color(ch, fg_raw, bg_raw, cols);
                            if want != cur_color {
                                out.extend_from_slice(&[0x1b, b'F', want]);
                                cur_color = want;
                            }
                            out.push(ch);
                        }
                    }
                    first_paint = false;
                } else {
                    // diff each row, emit changed runs only
                    for r in 0..rows {
                        let mut c = 0;
                        while c < write_cols {
                            let i = r * cols + c;
                            if new_grid[i] == shadow[i] {
                                c += 1;
                                continue;
                            }
                            let start = c;
                            while c < write_cols && new_grid[r * cols + c] != shadow[r * cols + c] {
                                c += 1;
                            }
                            out.extend_from_slice(&[
                                0x1b,
                                b'=',
                                (32 + r as u8),
                                (32 + start as u8),
                            ]);

                            for cc in start..c {
                                let (ch, fg_raw, bg_raw) = new_grid[r * cols + cc];
                                let want = pack_color(ch, fg_raw, bg_raw, cols);
                                if want != cur_color {
                                    out.extend_from_slice(&[0x1b, b'F', want]);
                                    cur_color = want;
                                }
                                out.push(ch);
                            }

                            shadow_cursor = (r as u16, c as u16);
                        }
                    }
                }

                // park the //c cursor where vt100 thinks it should be
                let (cr, cc) = screen.cursor_position();
                if (shadow_cursor.0, shadow_cursor.1) != (cr, cc)
                    && (cr as usize) < rows
                    && (cc as usize) < cols
                {
                    out.extend_from_slice(&[0x1b, b'=', 32 + cr as u8, 32 + cc as u8]);
                    shadow_cursor = (cr, cc);
                }

                if !out.is_empty() {
                    log::debug!("telnet tx -> //c [{}B]: {}", out.len(), debug_dump(&out));
                    local.write_all(&out)?;
                    local.flush()?;
                    tx_credit -= out.len() as f64;
                    shadow = new_grid;
                } else {
                    let nonblank = new_grid.iter().filter(|c| c.0 != b' ').count();
                    let (cr, cc) = screen.cursor_position();
                    log::trace!(
                        "telnet paint: no diff, grid nonblank={} cursor=({},{}) first_paint={}",
                        nonblank,
                        cr,
                        cc,
                        first_paint
                    );
                }
            } // end of `if paint_now`
        }

        // yield
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[derive(Default)]
struct IacState {
    in_iac: bool,
    cmd: Option<u8>,
    in_sb: bool,
    sb_buf: Vec<u8>,
}

impl IacState {
    // strip telnet IAC sequences from `buf`, replying with a
    // minimal "dumb client" stance: agree to ECHO/SGA, accept
    // TTYPE (24) and answer "ANSI"
    fn process(&mut self, peer: &mut TcpStream, buf: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(buf.len());
        for &b in buf {
            if self.in_sb {
                if self.in_iac {
                    if b == 240 {
                        // SE: process the captured subnegotiation.
                        self.handle_sb(peer);
                        self.sb_buf.clear();
                        self.in_sb = false;
                    } else if b == 255 {
                        // escaped 0xFF inside SB
                        self.sb_buf.push(0xFF);
                    }
                    self.in_iac = false;
                } else if b == 255 {
                    self.in_iac = true;
                } else {
                    self.sb_buf.push(b);
                }
                continue;
            }
            if self.in_iac {
                if let Some(cmd) = self.cmd {
                    let opt = b;
                    let reply = match cmd {
                        // DO -> WILL only for SGA(3) and TTYPE(24)
                        // WONT otherwise
                        253 => {
                            if opt == 3 || opt == 24 {
                                Some([255, 251, opt])
                            } else {
                                Some([255, 252, opt])
                            }
                        }
                        // DONT -> WONT
                        254 => Some([255, 252, opt]),
                        // WILL -> DO only for SGA(3) and ECHO(1)
                        // DONT otherwise
                        251 => {
                            if opt == 1 || opt == 3 {
                                Some([255, 253, opt])
                            } else {
                                Some([255, 254, opt])
                            }
                        }
                        // WONT -> DONT
                        252 => Some([255, 254, opt]),
                        _ => None,
                    };
                    if let Some(r) = reply {
                        let _ = peer.write_all(&r);
                    }
                    self.cmd = None;
                    self.in_iac = false;
                } else if b == 250 {
                    self.in_sb = true;
                    self.in_iac = false;
                } else if b == 255 {
                    out.push(0xFF);
                    self.in_iac = false;
                } else if matches!(b, 251..=254) {
                    self.cmd = Some(b);
                } else {
                    self.in_iac = false;
                }
            } else if b == 255 {
                self.in_iac = true;
            } else {
                out.push(b);
            }
        }
        Ok(out)
    }

    fn handle_sb(&self, peer: &mut TcpStream) {
        // TTYPE SEND: IAC SB 24 1 IAC SE -> reply IAC SB 24 0 'ANSI' IAC SE
        if self.sb_buf.len() >= 2 && self.sb_buf[0] == 24 && self.sb_buf[1] == 1 {
            let mut reply: Vec<u8> = Vec::with_capacity(12);
            reply.extend_from_slice(&[255, 250, 24, 0]);
            reply.extend_from_slice(b"ANSI");
            reply.extend_from_slice(&[255, 240]);
            let _ = peer.write_all(&reply);
        }
    }
}

fn parse_hostport(raw: &str) -> anyhow::Result<(String, u16)> {
    if let Some((h, p)) = raw.rsplit_once(':') {
        Ok((h.to_string(), p.parse()?))
    } else {
        Ok((raw.to_string(), 23))
    }
}

// DHGR terminal modules, assembled by build.rs to load at $0900 on
// the //c.  The boot terminal's `ESC L <len_lo> <len_hi> <bytes>`
// command receives the blob and auto-jumps to $0900 once the byte
// count is satisfied.
//
// `DHGR_MODULE` is the 80-col text + DHGR color flicker variant.
// `DHGR40_MODULE` is the 40-col DHGR true-color variant.
const DHGR_MODULE: &[u8] = include_bytes!("../../../build/asm/dhgr_term.bin");
const DHGR40_MODULE: &[u8] = include_bytes!("../../../build/asm/dhgr40_term.bin");

fn upload_module(stream: &mut TcpStream, module: &[u8]) -> anyhow::Result<()> {
    let len = module.len();
    if len == 0 || len > u16::MAX as usize {
        anyhow::bail!("dhgr module size out of range: {}", len);
    }
    let mut header = [0u8; 4];
    header[0] = 0x1b; // ESC
    header[1] = b'L';
    header[2] = (len & 0xff) as u8;
    header[3] = ((len >> 8) & 0xff) as u8;
    stream.write_all(&header)?;
    stream.write_all(module)?;
    stream.flush()?;
    log::info!("telnet: uploaded {} bytes of dhgr module to //c", len);
    Ok(())
}

// Pack a (fg, bg) pair into the byte the DHGR module expects:
// hi nibble = fg, lo nibble = bg.  fg/bg of 0xFF means vt100 reported
// `Color::Default` for that channel.
//
// In 80-col flicker mode the //c text page is hardware-monochrome
// (white-on-black) and the DHGR frame can only paint a solid color
// block per cell.  So when the BBS sets only fg (no bg) we mirror
// fg into bg, otherwise chromatic text would be invisible (white
// glyph from text page on black DHGR block).
fn pack_color(ch: u8, fg_raw: u8, bg_raw: u8, cols: usize) -> u8 {
    let fg_explicit = fg_raw != 0xFF;
    let bg_explicit = bg_raw != 0xFF;
    let fg = if fg_explicit { fg_raw } else { 15 };
    let chromatic = matches!(fg, 1..=14);
    let bg = if bg_explicit {
        bg_raw
    } else if cols == 80 && fg_explicit && ch != b' ' && chromatic {
        fg
    } else {
        0
    };
    (fg << 4) | bg
}

// Map a vt100 ANSI color to a 4-bit DHGR palette index for our
// `ESC F` color protocol.  `is_fg` selects the default colour when
// the cell uses `Color::Default` (white for fg, black for bg).
//
// DHGR palette (matches asm `solid_table` order):
//   0=black 1=magenta 2=brown   3=orange  4=dark green 5=grey1
//   6=green 7=yellow  8=dark blue 9=violet 10=grey2    11=pink
//   12=med blue 13=light blue 14=aqua 15=white
fn ansi_to_dhgr(c: vt100::Color, is_fg: bool) -> u8 {
    let idx = match c {
        vt100::Color::Default => return if is_fg { 15 } else { 0 },
        vt100::Color::Idx(i) => i,
        vt100::Color::Rgb(r, g, b) => {
            // crude RGB -> 4-bit ANSI fold
            let bright = (r as u16 + g as u16 + b as u16) / 3 > 128;
            let mut n = 0u8;
            if r > 64 {
                n |= 1;
            }
            if g > 64 {
                n |= 2;
            }
            if b > 64 {
                n |= 4;
            }
            if bright {
                n |= 8;
            }
            n
        }
    };
    // ANSI 0-7 normal, 8-15 bright -> DHGR 16-color
    //   0 black   1 dk blue 2 dk green 3 med blue 4 brown   5 grey1
    //   6 green   7 aqua    8 red      9 pink     A grey2   B lavender
    //   C orange  D purple  E yellow   F white
    match idx {
        0 => 0,   // black
        1 => 8,   // red
        2 => 2,   // green     -> dk green
        3 => 4,   // yellow    -> brown (closest dark-yellow)
        4 => 1,   // blue      -> dk blue
        5 => 13,  // magenta   -> purple
        6 => 3,   // cyan      -> med blue (DHGR 7 reads as green)
        7 => 15,  // white
        8 => 5,   // bright black -> grey
        9 => 9,   // bright red    -> pink
        10 => 6,  // bright green
        11 => 14, // bright yellow -> yellow
        12 => 3,  // bright blue   -> med blue
        13 => 13, // bright magenta -> purple
        14 => 3,  // bright cyan   -> med blue
        15 => 15, // bright white
        _ => {
            if is_fg {
                15
            } else {
                0
            }
        }
    }
}

// if `buf` ends with the prefix of an unfinished UTF-8 sequence,
// return how many trailing bytes belong to it (1..=3).  Otherwise 0.
fn trailing_utf8_partial_len(buf: &[u8]) -> usize {
    // walk back up to 3 bytes looking for a UTF-8 lead.
    for i in 1..=3.min(buf.len()) {
        let b = buf[buf.len() - i];
        if b < 0x80 {
            return 0; // ASCII boundary
        }
        let need = if b & 0xE0 == 0xC0 {
            1
        } else if b & 0xF0 == 0xE0 {
            2
        } else if b & 0xF8 == 0xF0 {
            3
        } else {
            // continuation byte; keep looking back for the lead
            continue;
        };
        // found lead at offset `i` from the end; we have `i` bytes
        // collected (lead + i-1 continuations). less than full
        // sequence is a partial
        if i < 1 + need {
            return i;
        }
        return 0;
    }
    0
}

fn decode_utf8(buf: &[u8]) -> (u32, usize) {
    if buf.is_empty() {
        return (0, 0);
    }
    let b0 = buf[0];
    if b0 < 0x80 {
        return (b0 as u32, 1);
    }
    let (need, mut cp) = if b0 & 0xE0 == 0xC0 {
        (1usize, (b0 & 0x1F) as u32)
    } else if b0 & 0xF0 == 0xE0 {
        (2usize, (b0 & 0x0F) as u32)
    } else if b0 & 0xF8 == 0xF0 {
        (3usize, (b0 & 0x07) as u32)
    } else {
        // invalid leading byte, treat as raw CP437 byte
        return (b0 as u32 | 0x10000, 1);
    };
    if buf.len() < 1 + need {
        return (b0 as u32 | 0x10000, 1);
    }
    for i in 0..need {
        let b = buf[1 + i];
        if b & 0xC0 != 0x80 {
            return (b0 as u32 | 0x10000, 1);
        }
        cp = (cp << 6) | (b & 0x3F) as u32;
    }
    (cp, 1 + need)
}

fn unicode_to_ascii(cp: u32) -> (u8, bool) {
    if cp & 0x10000 != 0 {
        // Raw CP437 byte: original DOS encoding.
        let b = (cp & 0xFF) as u8;
        let blocky = matches!(b, 0xB0 | 0xB1 | 0xB2 | 0xDB | 0xDC | 0xDD | 0xDE | 0xDF);
        let ch = if b >= 0x80 {
            CP437_ASCII[(b - 0x80) as usize]
        } else {
            b
        };
        return (ch, blocky);
    }
    // ASCII pass-through.
    if cp < 0x80 {
        return (cp as u8, false);
    }
    // box-drawing single (U+2500..U+257F)
    let ch = match cp {
        // single-line box drawing
        0x2500 | 0x2501 => b'-',
        0x2502 | 0x2503 => b'|',
        0x250C..=0x254B => b'+',
        // double-line box drawing
        0x2550 => b'=',
        0x2551 => b'|',
        0x2552..=0x256C => b'+',
        // half/full blocks U+2580..U+259F -- "blocky", render as
        // reverse-video for solid color fill.
        0x2580 => return (b'^', true), // upper half
        0x2584 => return (b'_', true), // lower half
        0x2588 => return (b'#', true), // full
        0x258C => return (b'[', true), // left half
        0x2590 => return (b']', true), // right half
        0x2591 => return (b'.', true), // light shade
        0x2592 => return (b':', true), // medium shade
        0x2593 => return (b'#', true), // dark shade
        0x2581..=0x259F => return (b'#', true),
        // bullets / stars
        0x2022 | 0x2023 | 0x25CF | 0x25CB | 0x2219 => b'*',
        0x2190 => b'<',
        0x2191 => b'^',
        0x2192 => b'>',
        0x2193 => b'v',
        0x2194 => b'-',
        // Arrows / triangles.
        0x25B2 | 0x25BC | 0x25C0 | 0x25B6 => b'>',
        // Smart quotes.
        0x2018 | 0x2019 => b'\'',
        0x201C | 0x201D => b'"',
        0x2013 | 0x2014 => b'-',
        0x2026 => b'.', // ellipsis
        _ if (0x80..0x100).contains(&cp) => {
            // Latin-1 supplement, use CP437 table (close enough)
            CP437_ASCII[(cp - 0x80) as usize]
        }
        _ => b'?',
    };
    (ch, false)
}

// CP437 high-half (0x80..=0xFF) -> ASCII fallback.
#[rustfmt::skip]
const CP437_ASCII: [u8; 128] = [
    // 0x80-0x8F: accented letters
    b'C', b'u', b'e', b'a', b'a', b'a', b'a', b'c',
    b'e', b'e', b'e', b'i', b'i', b'i', b'A', b'A',
    // 0x90-0x9F
    b'E', b'a', b'A', b'o', b'o', b'o', b'u', b'u',
    b'y', b'O', b'U', b'c', b'L', b'Y', b'P', b'f',
    // 0xA0-0xAF
    b'a', b'i', b'o', b'u', b'n', b'N', b'a', b'o',
    b'?', b'-', b'-', b'/', b'/', b'!', b'<', b'>',
    // 0xB0-0xBF: shading + box-drawing
    b'.', b':', b'#', b'|', b'+', b'+', b'+', b'+',
    b'+', b'+', b'|', b'+', b'+', b'+', b'+', b'+',
    // 0xC0-0xCF: box-drawing
    b'+', b'+', b'+', b'+', b'-', b'+', b'+', b'+',
    b'+', b'+', b'+', b'+', b'+', b'=', b'+', b'+',
    // 0xD0-0xDF: box + half-blocks
    b'+', b'+', b'+', b'+', b'+', b'+', b'+', b'+',
    b'+', b'+', b'+', b'#', b'=', b'[', b']', b'=',
    // 0xE0-0xEF: greek / math
    b'a', b'B', b'G', b'p', b'S', b's', b'u', b't',
    b'F', b'O', b'O', b'd', b'8', b'f', b'e', b'n',
    // 0xF0-0xFF: math / symbols
    b'=', b'+', b'>', b'<', b'(', b')', b'/', b'~',
    b'o', b'.', b'.', b'V', b'n', b'2', b'#', b' ',
];

fn debug_dump(buf: &[u8]) -> String {
    let mut s = String::with_capacity(buf.len() * 4);
    for &b in buf {
        match b {
            0x1b => s.push_str("<ESC>"),
            0x0c => s.push_str("<FF>"),
            b'\r' => s.push_str("<CR>"),
            b'\n' => s.push_str("<LF>"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("<{:02x}>", b)),
        }
    }
    s
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn write_line(s: &mut TcpStream, line: &str) -> anyhow::Result<()> {
    s.write_all(line.as_bytes())?;
    s.write_all(b"\r")?;
    s.flush()?;
    Ok(())
}

fn write_str(s: &mut TcpStream, text: &str) -> anyhow::Result<()> {
    s.write_all(text.as_bytes())?;
    s.flush()?;
    Ok(())
}

fn read_line(s: &mut TcpStream) -> anyhow::Result<String> {
    // Echo + simple BS handling, //c terminal sends raw bytes
    let mut line = String::new();
    let mut buf = [0u8; 1];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return Ok(line),
            Ok(_) => {}
            Err(ref e) if is_timeout(e) => continue,
            Err(e) => return Err(e.into()),
        }
        let b = buf[0];
        match b {
            b'\r' | b'\n' => {
                let _ = s.write_all(b"\r");
                let _ = s.flush();
                return Ok(line);
            }
            0x08 | 0x7f => {
                if line.pop().is_some() {
                    let _ = s.write_all(b"\x08 \x08");
                    let _ = s.flush();
                }
            }
            0x03 => {
                // Ctrl-C cancels
                return Ok(String::new());
            }
            _ if b.is_ascii_graphic() || b == b' ' => {
                line.push(b as char);
                let _ = s.write_all(&[b]);
                let _ = s.flush();
            }
            _ => {}
        }
    }
}

fn utf8_to_ascii(cleaned: &[u8], scrubbed: &mut Vec<u8>) {
    let mut i = 0;
    while i < cleaned.len() {
        let b = cleaned[i];
        if b < 0x80 {
            scrubbed.push(b);
            i += 1;
            continue;
        }
        let (cp, len) = decode_utf8(&cleaned[i..]);
        i += len;
        let (ch, blocky) = unicode_to_ascii(cp);
        if blocky {
            scrubbed.extend_from_slice(b"\x1b[7m");
            scrubbed.push(ch);
            scrubbed.extend_from_slice(b"\x1b[27m");
        } else {
            scrubbed.push(ch);
        }
    }
}
