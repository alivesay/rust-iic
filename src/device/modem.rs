// Virtual Hayes-compatible modem emulation.
//
// Implements the AT command set used by terminal software like ProTerm
// to dial out to BBSes and remote hosts via TCP. The modem sits between
// the SCC channel and the TCP stream, handling command/online mode
// switching, echo, +++ escape sequence, and AT command parsing.

use std::io::Write;
use std::net::TcpStream;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModemState {
    Command,
    Online,
    Escape,
}

// Actions the modem wants the SCC channel to perform.
pub enum ModemAction {
    // No action needed.
    None,
    // Modem wants to dial a TCP address.
    Dial(String),
    // Modem wants to hang up (disconnect TCP).
    Hangup,
    // Modem entered online mode (after ATO).
    GoOnline,
}

pub struct HayesModem {
    pub enabled: bool,
    pub state: ModemState,
    cmd_buffer: String,
    pub echo: bool,
    verbose: bool,
    pub plus_count: u8,
    pub last_data_cycle: u64,
    // Bytes to be pushed into the SCC receive buffer.
    // The SCC channel drains this after each modem operation.
    pub rx_out: Vec<u8>,
}

impl HayesModem {
    pub fn new() -> Self {
        Self {
            enabled: false,
            state: ModemState::Command,
            cmd_buffer: String::new(),
            echo: true,
            verbose: true,
            plus_count: 0,
            last_data_cycle: 0,
            rx_out: Vec::new(),
        }
    }

    // Check for +++ escape sequence timeout. Called from SCC tick().
    // Returns true if escape was triggered (caller should send "OK").
    pub fn check_escape(&mut self) -> bool {
        if self.enabled && self.plus_count >= 3 && self.state == ModemState::Online {
            self.plus_count = 0;
            self.state = ModemState::Escape;
            self.send_response("OK");
            return true;
        }
        false
    }

    // Process a transmitted byte. Returns an action for the SCC channel.
    pub fn transmit(&mut self, byte: u8, mut stream: Option<&mut TcpStream>) -> ModemAction {
        match self.state {
            ModemState::Online => {
                if byte == b'+' {
                    self.plus_count += 1;
                    if self.plus_count >= 3 {
                        return ModemAction::None;
                    }
                } else {
                    if self.plus_count > 0 {
                        let count = self.plus_count;
                        self.plus_count = 0;
                        for _ in 0..count {
                            if let Some(ref mut s) = stream {
                                let _ = s.write_all(&[b'+']);
                            }
                        }
                    }
                    self.last_data_cycle = 0;
                }

                if self.plus_count == 0 {
                    if let Some(s) = stream {
                        let _ = s.write_all(&[byte]);
                    }
                }
                ModemAction::None
            }

            ModemState::Command | ModemState::Escape => {
                if self.echo {
                    self.rx_out.push(byte);
                }

                if byte == b'\r' || byte == b'\n' {
                    let cmd = self.cmd_buffer.trim().to_uppercase();
                    self.cmd_buffer.clear();
                    if !cmd.is_empty() {
                        return self.process_at_command(&cmd);
                    }
                } else if byte == 0x08 || byte == 0x7F {
                    self.cmd_buffer.pop();
                } else if byte >= 0x20 {
                    self.cmd_buffer.push(byte as char);
                }
                ModemAction::None
            }
        }
    }

    fn process_at_command(&mut self, cmd: &str) -> ModemAction {
        if !cmd.starts_with("AT") {
            self.send_response("ERROR");
            return ModemAction::None;
        }

        let args = &cmd[2..];
        if args.is_empty() {
            self.send_response("OK");
            return ModemAction::None;
        }

        let mut chars = args.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                'I' => {
                    if let Some(&next) = chars.peek() {
                        if next.is_ascii_digit() { chars.next(); }
                    }
                    self.send_response("rust-iic Virtual Hayes Modem v1.0");
                    return ModemAction::None;
                }
                'D' => {
                    if let Some(&next) = chars.peek() {
                        if next == 'T' || next == 'P' { chars.next(); }
                    }
                    let addr: String = chars.collect();
                    let addr = addr.trim().to_string();
                    if addr.is_empty() {
                        self.send_response("ERROR");
                        return ModemAction::None;
                    }
                    return ModemAction::Dial(addr);
                }
                'H' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { chars.next(); }
                    }
                    return ModemAction::Hangup;
                }
                'O' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { chars.next(); }
                    }
                    return ModemAction::GoOnline;
                }
                'E' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { self.echo = false; chars.next(); }
                        else if next == '1' { self.echo = true; chars.next(); }
                    } else {
                        self.echo = true;
                    }
                }
                'V' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { self.verbose = false; chars.next(); }
                        else if next == '1' { self.verbose = true; chars.next(); }
                    } else {
                        self.verbose = true;
                    }
                }
                'Z' => {
                    self.echo = true;
                    self.verbose = true;
                    self.cmd_buffer.clear();
                    self.send_response("OK");
                    return ModemAction::Hangup;
                }
                'S' => {
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii_digit() || next == '=' || next == '?' {
                            chars.next();
                        } else { break; }
                    }
                }
                '&' => {
                    chars.next();
                    if let Some(&next) = chars.peek() {
                        if next.is_ascii_digit() { chars.next(); }
                    }
                }
                _ => {}
            }
        }
        self.send_response("OK");
        ModemAction::None
    }

    // Called by SCC after a successful TCP connect (ATDT).
    pub fn on_connected(&mut self) {
        self.state = ModemState::Online;
        self.plus_count = 0;
        self.send_response("CONNECT 2400");
    }

    // Called by SCC when a dial attempt fails.
    pub fn on_connect_failed(&mut self) {
        self.state = ModemState::Command;
        self.send_response("NO CARRIER");
    }

    // Called by SCC when the remote disconnects.
    pub fn on_disconnected(&mut self) {
        self.state = ModemState::Command;
        self.send_response("NO CARRIER");
    }

    // Called by SCC after a successful hangup (ATH).
    pub fn on_hangup(&mut self) {
        self.send_response("OK");
    }

    pub fn send_response(&mut self, msg: &str) {
        let response = if self.verbose {
            format!("\r\n{}\r\n", msg)
        } else {
            let code = match msg {
                "OK" => "0",
                s if s.starts_with("CONNECT") => "1",
                "RING" => "2",
                "NO CARRIER" => "3",
                "ERROR" => "4",
                "NO DIALTONE" => "6",
                "BUSY" => "7",
                "NO ANSWER" => "8",
                _ => "4",
            };
            format!("{}\r", code)
        };
        self.rx_out.extend(response.bytes());
    }
}
