/// Emulation of the MOS 6551 ACIA (Async Communications Interface Adapter)
/// with integrated Hayes-compatible virtual modem.
///
/// The Apple IIc has two built-in ACIAs:
///   - Slot 1 (Modem port):  registers at $C098–$C09B
///   - Slot 2 (Printer port): registers at $C0A8–$C0AB
///
/// Register map (offset from base):
///   +0  Data Register    — read: receive data, write: transmit data
///   +1  Status Register  — read: status flags, write: programmed reset
///   +2  Command Register — read/write
///   +3  Control Register — read/write
///
/// Virtual modem AT commands (when modem_enabled):
///   ATDThost:port  — connect to TCP host (telnet)
///   ATH            — hang up
///   ATO            — return to online/data mode
///   ATE0/ATE1      — echo off/on
///   ATV0/ATV1      — numeric/verbose result codes
///   ATZ            — reset modem
///   AT             — just returns OK
///   +++            — escape to command mode (with 1s guard time)

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ModemState {
    Command,    // Accepting AT commands, no connection
    Dialing,    // TCP connect in progress
    Online,     // Connected, data passes through
    Escape,     // Connected but in command mode (after +++)
}

pub struct Acia {
    pub id: &'static str,
    pub debug: bool,

    // Registers
    command: u8,
    control: u8,

    // Receive buffer (data arriving from external → CPU reads)
    rx_buffer: VecDeque<u8>,

    // Status flags
    overrun: bool,
    irq: bool,

    // TCP connection (non-blocking)
    stream: Option<TcpStream>,
    connected: bool,

    // Throttle polling — don't try to read the socket every cycle
    poll_countdown: u64,
    poll_interval: u64,

    // Virtual modem
    pub modem_enabled: bool,
    modem_state: ModemState,
    cmd_buffer: String,       // Accumulates AT command line
    echo: bool,               // ATE — echo typed characters
    verbose: bool,            // ATV — verbose vs numeric result codes
    plus_count: u8,           // +++ escape sequence counter
    last_data_cycle: u64,     // Cycle of last non-'+' data (guard time)
}

impl Acia {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            debug: false,
            command: 0x00,
            control: 0x00,
            rx_buffer: VecDeque::new(),
            overrun: false,
            irq: false,
            stream: None,
            connected: false,
            poll_countdown: 0,
            poll_interval: 1023,
            modem_enabled: false,
            modem_state: ModemState::Command,
            cmd_buffer: String::new(),
            echo: true,
            verbose: true,
            plus_count: 0,
            last_data_cycle: 0,
        }
    }

    /// Connect to a remote host (telnet BBS, etc.)
    pub fn tcp_connect(&mut self, addr: &str) -> Result<(), std::io::Error> {
        println!("ACIA[{}]: connecting to {}...", self.id, addr);
        let stream = TcpStream::connect(addr)?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        println!("ACIA[{}]: connected to {}", self.id, addr);
        self.stream = Some(stream);
        self.connected = true;
        Ok(())
    }

    /// Disconnect the current connection
    fn tcp_disconnect(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.connected = false;
        if self.modem_enabled {
            self.modem_state = ModemState::Command;
        }
        println!("ACIA[{}]: disconnected", self.id);
    }

    /// Programmed reset — writing any value to the status register address
    fn programmed_reset(&mut self) {
        // Per 6551 datasheet: programmed reset clears bits 0-4 of command register,
        // overrun flag, and IRQ. Control register and bits 5-7 of command are unchanged.
        self.command &= 0xE0;
        self.overrun = false;
        self.irq = false;
        if self.debug {
            println!("ACIA[{}]: programmed reset", self.id);
        }
    }

    /// Hardware reset (power-on or system reset)
    pub fn reset(&mut self) {
        self.command = 0x00;
        self.control = 0x00;
        self.rx_buffer.clear();
        self.overrun = false;
        self.irq = false;
        // Note: does NOT disconnect — reset preserves the TCP connection
        if self.debug {
            println!("ACIA[{}]: hardware reset", self.id);
        }
    }

    /// Read a register (offset 0–3 from base address)
    pub fn read(&mut self, offset: u8) -> u8 {
        match offset & 0x03 {
            // Data register — read received byte
            0 => {
                self.irq = false;
                if let Some(byte) = self.rx_buffer.pop_front() {
                    self.overrun = false;
                    if self.debug {
                        println!("ACIA[{}]: read data {:#04X} '{}'", self.id, byte,
                            if byte.is_ascii_graphic() || byte == b' ' { byte as char } else { '.' });
                    }
                    byte
                } else {
                    0x00
                }
            }

            // Status register
            1 => {
                let rdrf = !self.rx_buffer.is_empty();
                let tdre = true; // Transmit always ready (we flush immediately)
                let dcd = !self.connected; // DCD: 0 = carrier present, 1 = no carrier
                let dsr = !self.connected; // DSR: 0 = ready, 1 = not ready

                let status = (if self.irq { 0x80 } else { 0 })
                    | (if dsr { 0x40 } else { 0 })
                    | (if dcd { 0x20 } else { 0 })
                    | (if tdre { 0x10 } else { 0 })
                    | (if rdrf { 0x08 } else { 0 })
                    | (if self.overrun { 0x04 } else { 0 });
                // Reading status clears IRQ
                self.irq = false;
                status
            }

            // Command register
            2 => self.command,

            // Control register
            3 => self.control,

            _ => unreachable!(),
        }
    }

    /// Write a register (offset 0–3 from base address)
    pub fn write(&mut self, offset: u8, value: u8) {
        match offset & 0x03 {
            // Data register — transmit byte
            0 => {
                if self.debug {
                    println!("ACIA[{}]: transmit {:#04X} '{}'", self.id, value,
                        if value.is_ascii_graphic() || value == b' ' { value as char } else { '.' });
                }
                if self.modem_enabled {
                    self.modem_transmit(value);
                } else if let Some(stream) = self.stream.as_mut() {
                    let _ = stream.write_all(&[value]);
                }
            }

            // Status register write = programmed reset
            1 => {
                self.programmed_reset();
            }

            // Command register
            2 => {
                self.command = value;
                if self.debug {
                    let dtr = value & 0x01 != 0;
                    let rx_irq_disable = value & 0x02 != 0;
                    let tx_control = (value >> 2) & 0x03;
                    let echo = value & 0x10 != 0;
                    let parity = (value >> 5) & 0x07;
                    println!("ACIA[{}]: command={:#04X} DTR={} RxIRQ={} TxCtrl={} Echo={} Parity={}",
                        self.id, value, dtr, !rx_irq_disable, tx_control, echo, parity);
                }
            }

            // Control register
            3 => {
                self.control = value;
                if self.debug {
                    let baud_idx = value & 0x0F;
                    let _rx_clock = value & 0x10 != 0;
                    let word_len = 8 - ((value >> 5) & 0x03);
                    let stop_bits = if value & 0x80 != 0 { 2 } else { 1 };
                    let baud = match baud_idx {
                        0x00 => 16, // 16x external clock
                        0x01 => 50,
                        0x02 => 75,
                        0x03 => 110,
                        0x04 => 135,
                        0x05 => 150,
                        0x06 => 300,
                        0x07 => 600,
                        0x08 => 1200,
                        0x09 => 1800,
                        0x0A => 2400,
                        0x0B => 3600,
                        0x0C => 4800,
                        0x0D => 7200,
                        0x0E => 9600,
                        0x0F => 19200,
                        _ => 0,
                    };
                    println!("ACIA[{}]: control={:#04X} baud={} {}{}{}",
                        self.id, value, baud, word_len, 
                        if (self.command >> 5) & 0x01 == 0 { "N" } else { "P" },
                        stop_bits);
                }
            }

            _ => unreachable!(),
        }
    }

    /// Tick the ACIA — polls TCP socket for incoming data, checks modem state
    pub fn tick(&mut self, cycles: u64) {
        // Check +++ guard time: if we got 3 plusses and enough time has passed
        if self.modem_enabled && self.plus_count >= 3 && self.modem_state == ModemState::Online {
            if self.poll_countdown == 0 || cycles > 0 {
                // Check if enough time passed since the last plus
                // We use a simple approach: if tick is called and plus_count >= 3,
                // we already waited (the guard time check happens in modem_transmit)
                self.plus_count = 0;
                self.modem_state = ModemState::Escape;
                self.send_response("OK");
                println!("ACIA[{}]: +++ escape to command mode", self.id);
                return;
            }
        }

        if self.stream.is_none() {
            return;
        }

        // Throttle polling
        if self.poll_countdown > cycles {
            self.poll_countdown -= cycles;
            return;
        }
        self.poll_countdown = self.poll_interval;

        // Read incoming data from socket
        let mut buf = [0u8; 64];
        let disconnect = if let Some(stream) = self.stream.as_mut() {
            match stream.read(&mut buf) {
                Ok(0) => {
                    println!("ACIA[{}]: remote disconnected", self.id);
                    true
                }
                Ok(n) => {
                    for &byte in &buf[..n] {
                        self.receive_byte_internal(byte);
                    }
                    false
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    false
                }
                Err(e) => {
                    println!("ACIA[{}]: read error: {}", self.id, e);
                    true
                }
            }
        } else {
            false
        };

        if disconnect {
            self.tcp_disconnect();
            if self.modem_enabled {
                self.send_response("NO CARRIER");
            }
        }
    }

    /// Push a byte into the receive buffer (from external source → CPU)
    fn receive_byte_internal(&mut self, byte: u8) {
        if self.rx_buffer.len() >= 256 {
            self.overrun = true;
        }
        self.rx_buffer.push_back(byte);

        // Generate IRQ if receive interrupts are enabled
        // Command register bit 1: 0 = IRQ enabled, 1 = IRQ disabled
        // Command register bit 0: 1 = DTR asserted
        if self.command & 0x02 == 0 && self.command & 0x01 != 0 {
            self.irq = true;
        }
    }

    // ─── Virtual Hayes Modem ───────────────────────────────────────────

    /// Handle a transmitted byte through the modem layer
    fn modem_transmit(&mut self, byte: u8) {
        match self.modem_state {
            ModemState::Online => {
                // Check for +++ escape sequence
                if byte == b'+' {
                    self.plus_count += 1;
                    if self.plus_count >= 3 {
                        // Will be handled in tick() after guard time
                        return;
                    }
                } else {
                    // If we had partial plusses, flush them as data first
                    if self.plus_count > 0 {
                        let count = self.plus_count;
                        self.plus_count = 0;
                        for _ in 0..count {
                            if let Some(stream) = self.stream.as_mut() {
                                let _ = stream.write_all(&[b'+']);
                            }
                        }
                    }
                    self.last_data_cycle = 0; // Reset guard time tracking
                }

                // Send data byte to TCP
                if self.plus_count == 0 {
                    if let Some(stream) = self.stream.as_mut() {
                        let _ = stream.write_all(&[byte]);
                    }
                }
            }

            ModemState::Command | ModemState::Escape => {
                // Echo if enabled
                if self.echo {
                    self.receive_byte_internal(byte);
                }

                if byte == b'\r' || byte == b'\n' {
                    let cmd = self.cmd_buffer.trim().to_uppercase();
                    self.cmd_buffer.clear();
                    if !cmd.is_empty() {
                        self.process_at_command(&cmd);
                    }
                } else if byte == 0x08 || byte == 0x7F {
                    // Backspace / DEL
                    self.cmd_buffer.pop();
                } else if byte >= 0x20 {
                    self.cmd_buffer.push(byte as char);
                }
            }

            ModemState::Dialing => {
                // Ignore input while connecting
            }
        }
    }

    /// Process a complete AT command line
    fn process_at_command(&mut self, cmd: &str) {
        // Must start with AT
        if !cmd.starts_with("AT") {
            self.send_response("ERROR");
            return;
        }

        let args = &cmd[2..];

        // Empty AT → just OK
        if args.is_empty() {
            self.send_response("OK");
            return;
        }

        // Parse commands (can be chained, e.g. ATE1V1)
        let mut chars = args.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                // ATDT — dial (TCP connect)
                'D' => {
                    // Consume 'T' or 'P' (tone/pulse, ignored)
                    if let Some(&next) = chars.peek() {
                        if next == 'T' || next == 'P' {
                            chars.next();
                        }
                    }
                    // Rest of string is the address
                    let addr: String = chars.collect();
                    self.modem_dial(&addr);
                    return; // Dial consumes rest of line
                }

                // ATH — hang up
                'H' => {
                    // Consume optional '0'
                    if let Some(&next) = chars.peek() {
                        if next == '0' { chars.next(); }
                    }
                    if self.connected {
                        self.tcp_disconnect();
                        self.send_response("OK");
                    } else {
                        self.send_response("OK");
                    }
                }

                // ATO — return to online mode
                'O' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { chars.next(); }
                    }
                    if self.connected {
                        self.modem_state = ModemState::Online;
                        self.plus_count = 0;
                        self.send_response("CONNECT");
                        return;
                    } else {
                        self.send_response("NO CARRIER");
                        return;
                    }
                }

                // ATE — echo control
                'E' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { self.echo = false; chars.next(); }
                        else if next == '1' { self.echo = true; chars.next(); }
                    } else {
                        self.echo = true; // ATE with no arg = ATE1
                    }
                }

                // ATV — verbose/numeric result codes
                'V' => {
                    if let Some(&next) = chars.peek() {
                        if next == '0' { self.verbose = false; chars.next(); }
                        else if next == '1' { self.verbose = true; chars.next(); }
                    } else {
                        self.verbose = true;
                    }
                }

                // ATZ — reset
                'Z' => {
                    if self.connected {
                        self.tcp_disconnect();
                    }
                    self.echo = true;
                    self.verbose = true;
                    self.cmd_buffer.clear();
                    self.send_response("OK");
                    return;
                }

                // ATS — S-registers (stub: accept and ignore)
                'S' => {
                    // Consume register number, '=', value
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii_digit() || next == '=' || next == '?' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }

                // AT& — extended commands (stub)
                '&' => {
                    // Consume the next char
                    chars.next();
                    // Consume optional digit
                    if let Some(&next) = chars.peek() {
                        if next.is_ascii_digit() { chars.next(); }
                    }
                }

                // Unknown — ignore single char
                _ => {}
            }
        }

        self.send_response("OK");
    }

    /// Dial a TCP address
    fn modem_dial(&mut self, addr: &str) {
        if addr.is_empty() {
            self.send_response("ERROR");
            return;
        }

        // If already connected, disconnect first
        if self.connected {
            self.tcp_disconnect();
        }

        self.modem_state = ModemState::Dialing;

        // Add default port 23 (telnet) if not specified
        let addr = if addr.contains(':') {
            addr.to_string()
        } else {
            format!("{}:23", addr)
        };

        println!("ACIA[{}]: modem dialing {}", self.id, addr);

        match self.tcp_connect(&addr) {
            Ok(()) => {
                self.modem_state = ModemState::Online;
                self.plus_count = 0;
                // Report baud rate from control register
                let baud = self.get_baud_rate();
                self.send_response(&format!("CONNECT {}", baud));
            }
            Err(e) => {
                self.modem_state = ModemState::Command;
                println!("ACIA[{}]: dial failed: {}", self.id, e);
                self.send_response("NO CARRIER");
            }
        }
    }

    /// Send a modem response to the CPU (into rx_buffer)
    fn send_response(&mut self, msg: &str) {
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

        for byte in response.bytes() {
            self.receive_byte_internal(byte);
        }
    }

    /// Get configured baud rate from control register
    fn get_baud_rate(&self) -> u32 {
        match self.control & 0x0F {
            0x06 => 300,
            0x07 => 600,
            0x08 => 1200,
            0x09 => 1800,
            0x0A => 2400,
            0x0C => 4800,
            0x0E => 9600,
            0x0F => 19200,
            _ => 2400, // Default for display purposes
        }
    }

    // ─── Public API ────────────────────────────────────────────────────

    /// Check if an IRQ is pending
    pub fn irq_pending(&self) -> bool {
        self.irq
    }

    /// Check if connected to a remote host
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}
