// Emulation of the Zilog 8530 SCC (Serial Communications Controller)
//
// The Apple IIc uses a single Z8530 SCC for both serial ports:
//   - Channel A = Modem port (Slot 1 equivalent)
//   - Channel B = Printer port (Slot 2 equivalent)
//
// Apple IIc address mapping:
//   $C038 = Channel B Command/Status (Printer)
//   $C039 = Channel A Command/Status (Modem)
//   $C03A = Channel B Data (Printer)
//   $C03B = Channel A Data (Modem)
//
// Slot-based mirrors (for software using slot I/O):
//   $C098–$C09F = Channel A (Modem, Slot 1)
//   $C0A8–$C0AF = Channel B (Printer, Slot 2)
//
// The SCC uses a register pointer mechanism:
//   1. Write register number to command port (selects WR/RR n)
//   2. Next read/write on command port accesses that register
//   3. Pointer auto-resets to 0 after access

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpStream;

use crate::timing;
use super::modem::{HayesModem, ModemAction, ModemState};

pub struct SccChannel {
    pub id: &'static str,
    pub debug: bool,

    // SCC registers (WR0–WR15)
    wr: [u8; 16],
    // Register pointer (set by writing to WR0 bits 0-3)
    reg_ptr: u8,

    // Receive FIFO (3-deep on real hardware, we use a VecDeque)
    rx_buffer: VecDeque<u8>,
    rx_overrun: bool,

    // Transmit
    tx_empty: bool,

    // External status
    dcd: bool,    // Data Carrier Detect (active low on real hardware)
    cts: bool,    // Clear To Send

    // Interrupt pending flags
    rx_ip: bool,  // Receive interrupt pending
    tx_ip: bool,  // Transmit interrupt pending
    ext_ip: bool, // External/status interrupt pending

    // TCP connection (non-blocking)
    stream: Option<TcpStream>,
    connected: bool,

    // Loopback mode (for diagnostic testing)
    pub loopback: bool,

    // ACIA emulation registers (for slot I/O compatibility)
    acia_command: u8,  // ACIA command register (offset 2)
    acia_control: u8,  // ACIA control register (offset 3)

    // Throttle polling
    poll_countdown: u64,
    poll_interval: u64,

    // Virtual Hayes modem
    pub modem: HayesModem,
}

impl SccChannel {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            debug: false,
            wr: [0u8; 16],
            reg_ptr: 0,
            rx_buffer: VecDeque::new(),
            rx_overrun: false,
            tx_empty: true,
            dcd: false,
            cts: true,
            rx_ip: false,
            tx_ip: false,
            ext_ip: false,
            stream: None,
            connected: false,
            loopback: false,
            acia_command: 0,
            acia_control: 0,
            poll_countdown: 0,
            poll_interval: (timing::CYCLES_PER_SECOND / 1000.0) as u64, // ~1ms
            modem: HayesModem::new(),
        }
    }

    fn reset(&mut self) {
        self.wr = [0u8; 16];
        self.reg_ptr = 0;
        self.rx_buffer.clear();
        self.rx_overrun = false;
        self.tx_empty = true;
        self.rx_ip = false;
        self.tx_ip = false;
        self.ext_ip = false;
        self.acia_command = 0;
        self.acia_control = 0;
    }

    // Return ACIA-compatible status register (6551 format)
    // Apple IIc firmware presents ACIA interface at slot I/O addresses
    // even though actual hardware is SCC
    fn acia_status(&self) -> u8 {
        let mut status = 0u8;
        
        // Bit 3: RDRF (Receive Data Register Full) - 1 = data available
        if !self.rx_buffer.is_empty() {
            status |= 0x08;
        }
        
        // Bit 4: TDRE (Transmit Data Register Empty) - 1 = ready to transmit
        if self.tx_empty {
            status |= 0x10;
        }
        
        // Bit 5: DCD (Data Carrier Detect) - ACTIVE LOW (0 = carrier present)
        // In loopback mode, report carrier present
        if !self.dcd && !self.loopback {
            status |= 0x20;  // No carrier = bit set
        }
        
        // Bit 6: DSR (Data Set Ready) - ACTIVE LOW (0 = ready)
        // Report ready when connected, modem enabled, or loopback mode
        if !self.connected && !self.modem.enabled && !self.loopback {
            status |= 0x40;  // Not ready
        }
        
        // Bit 7: IRQ - set if interrupt pending
        if self.irq_pending() {
            status |= 0x80;
        }
        
        status
    }

    // Read the command/status port
    fn read_command(&mut self) -> u8 {
        let reg = self.reg_ptr;
        self.reg_ptr = 0; // Auto-reset pointer

        match reg {
            // RR0: Transmit/Receive Buffer Status and External Status
            0 => {
                let mut rr0 = 0u8;
                if !self.rx_buffer.is_empty() { rr0 |= 0x01; } // Rx Char Available
                                                                  // bit 1: Zero Count (unused)
                if self.tx_empty                { rr0 |= 0x04; } // Tx Buffer Empty
                if self.dcd || self.loopback    { rr0 |= 0x08; } // DCD (or loopback fakes it)
                                                                  // bit 4: Sync/Hunt (unused)
                if self.cts || self.loopback    { rr0 |= 0x20; } // CTS (or loopback fakes it)
                                                                  // bit 6: Tx Underrun (unused)
                // bit 7: Break/Abort - leave 0
                rr0
            }

            // RR1: Special Receive Condition
            1 => {
                let mut rr1 = 0x01; // All Sent
                if self.rx_overrun { rr1 |= 0x20; } // Rx Overrun
                rr1
            }

            // RR2: Interrupt vector (Channel B returns modified vector)
            2 => self.wr[2], // Return base vector (modification handled at SCC level)

            // RR3: Interrupt Pending (Channel A only on real hardware)
            3 => {
                let mut rr3 = 0u8;
                if self.ext_ip { rr3 |= 0x01; } // Ch B Ext/Status IP
                if self.tx_ip  { rr3 |= 0x02; } // Ch B Tx IP
                if self.rx_ip  { rr3 |= 0x04; } // Ch B Rx IP
                // Bits 3-5 are Channel A (handled at SCC level)
                rr3
            }

            // RR8: Receive Data (same as data port)
            8 => self.read_data(),

            // RR10: Misc status (stub)
            10 => 0x00,

            // RR12: BRG time constant low
            12 => self.wr[12],

            // RR13: BRG time constant high
            13 => self.wr[13],

            // RR15: External/Status IE bits (mirrors WR15)
            15 => self.wr[15],

            _ => 0x00,
        }
    }

    // Write the command/status port
    fn write_command(&mut self, value: u8) {
        let reg = self.reg_ptr;
        self.reg_ptr = 0; // Auto-reset pointer

        match reg {
            // WR0: Command Register & Register Pointer
            0 => {
                // Bits 0-2: Register pointer for next access
                self.reg_ptr = value & 0x07;

                // Bits 3-5: Command codes
                let cmd = (value >> 3) & 0x07;
                match cmd {
                    0 => {} // Null command
                    1 => {  // Point High (select WR8-WR15)
                        self.reg_ptr |= 0x08;
                    }
                    2 => {  // Reset External/Status Interrupts
                        self.ext_ip = false;
                    }
                    3 => {} // Send Abort (SDLC, ignore)
                    4 => {  // Enable Int on Next Rx Char
                        // We always generate interrupts based on WR1
                    }
                    5 => {  // Reset Tx Int Pending
                        self.tx_ip = false;
                    }
                    6 => {  // Error Reset
                        self.rx_overrun = false;
                    }
                    7 => {} // Reset Highest IUS (not needed for our level)
                    _ => {}
                }

                // Bits 6-7: CRC Reset (ignored)
                if self.debug && value != 0 {
                    println!("SCC[{}]: WR0={:#04X} ptr={} cmd={}", self.id, value, value & 0x07, cmd);
                }
            }

            // WR1: Tx/Rx Interrupt and Data Transfer Mode
            1 => {
                self.wr[1] = value;
                if self.debug {
                    println!("SCC[{}]: WR1={:#04X} ExtIE={} TxIE={} RxIE={}",
                        self.id, value,
                        value & 0x01 != 0,
                        value & 0x02 != 0,
                        (value >> 3) & 0x03);
                }
            }

            // WR2: Interrupt vector
            2 => { self.wr[2] = value; }

            // WR3: Receive Parameters
            3 => {
                self.wr[3] = value;
                if self.debug {
                    let rx_enable = value & 0x01 != 0;
                    let bits = match (value >> 6) & 0x03 {
                        0 => 5, 1 => 7, 2 => 6, 3 => 8, _ => 8,
                    };
                    println!("SCC[{}]: WR3={:#04X} RxEnable={} {}bits", self.id, value, rx_enable, bits);
                }
            }

            // WR4: Tx/Rx Misc Parameters
            4 => {
                self.wr[4] = value;
                if self.debug {
                    let parity_en = value & 0x01 != 0;
                    let parity_even = value & 0x02 != 0;
                    let stop = match (value >> 2) & 0x03 {
                        0 => "sync", 1 => "1", 2 => "1.5", 3 => "2", _ => "?",
                    };
                    let clock_mode = match (value >> 6) & 0x03 {
                        0 => "x1", 1 => "x16", 2 => "x32", 3 => "x64", _ => "?",
                    };
                    println!("SCC[{}]: WR4={:#04X} parity={}{} stop={} clock={}",
                        self.id, value,
                        if parity_en { if parity_even { "even" } else { "odd" } } else { "none" },
                        "", stop, clock_mode);
                }
            }

            // WR5: Transmit Parameters
            5 => {
                self.wr[5] = value;
                if self.debug {
                    let tx_enable = value & 0x08 != 0;
                    let rts = value & 0x02 != 0;
                    let dtr = value & 0x80 != 0;
                    let bits = match (value >> 5) & 0x03 {
                        0 => 5, 1 => 7, 2 => 6, 3 => 8, _ => 8,
                    };
                    println!("SCC[{}]: WR5={:#04X} TxEnable={} {}bits RTS={} DTR={}",
                        self.id, value, tx_enable, bits, rts, dtr);
                }
            }

            // WR6: Sync character / SDLC address (ignore)
            6 => { self.wr[6] = value; }

            // WR7: Sync character / SDLC flag (ignore)
            7 => { self.wr[7] = value; }

            // WR8: Transmit Data (same as data port)
            8 => { let _ = self.write_data(value); }

            // WR9: Master Interrupt Control (handled at SCC level)
            9 => {
                self.wr[9] = value;
                if value & 0xC0 != 0 {
                    // Hardware reset commands
                    if value & 0x80 != 0 {
                        if self.debug { println!("SCC[{}]: Channel Reset via WR9", self.id); }
                        self.reset();
                    }
                }
            }

            // WR10: Misc Tx/Rx Control (NRZ/NRZI/FM encoding, ignore)
            10 => { self.wr[10] = value; }

            // WR11: Clock Mode Control
            11 => {
                self.wr[11] = value;
                if self.debug {
                    println!("SCC[{}]: WR11={:#04X} (clock mode)", self.id, value);
                }
            }

            // WR12: BRG Time Constant Low
            12 => { self.wr[12] = value; }

            // WR13: BRG Time Constant High
            13 => { self.wr[13] = value; }

            // WR14: Misc Control (BRG enable, etc.)
            14 => {
                self.wr[14] = value;
                if self.debug {
                    let brg_enable = value & 0x01 != 0;
                    let brg_source = if value & 0x02 != 0 { "PCLK" } else { "XTAL" };
                    println!("SCC[{}]: WR14={:#04X} BRG={} src={}", self.id, value, brg_enable, brg_source);
                }
            }

            // WR15: External/Status Interrupt Control
            15 => {
                self.wr[15] = value;
                if self.debug {
                    println!("SCC[{}]: WR15={:#04X} DCD_IE={} CTS_IE={}", self.id, value,
                        value & 0x08 != 0, value & 0x20 != 0);
                }
            }

            _ => {
                if self.debug {
                    println!("SCC[{}]: WR{}={:#04X} (unhandled)", self.id, reg, value);
                }
            }
        }
    }

    // Read the data port
    fn read_data(&mut self) -> u8 {
        if let Some(byte) = self.rx_buffer.pop_front() {
            // Clear Rx interrupt if buffer now empty
            if self.rx_buffer.is_empty() {
                self.rx_ip = false;
            }
            if self.debug {
                println!("SCC[{}]: read data {:#04X} '{}'", self.id, byte,
                    if byte.is_ascii_graphic() || byte == b' ' { byte as char } else { '.' });
            }
            byte
        } else {
            0x00
        }
    }

    // Write the data port (transmit)
    // Returns the byte if it should be forwarded for cross-channel loopback
    fn write_data(&mut self, value: u8) -> Option<u8> {
        if self.debug {
            println!("SCC[{}]: write data {:#04X} '{}'", self.id, value,
                if value.is_ascii_graphic() || value == b' ' { value as char } else { '.' });
        }

        let forward_for_crossloop = if self.loopback {
            // Internal loopback: echo to own receive buffer (simulates loopback plug)
            self.receive_byte(value);
            false
        } else if self.modem.enabled {
            let action = self.modem.transmit(value, self.stream.as_mut());
            self.drain_modem_rx();
            self.handle_modem_action(action);
            false
        } else if let Some(stream) = self.stream.as_mut() {
            let _ = stream.write_all(&[value]);
            false
        } else {
            // No active destination - data available for cross-loopback
            true
        };

        // Tx buffer is always immediately empty (instant transmit)
        self.tx_empty = true;
        // Generate Tx interrupt if enabled
        if self.wr[1] & 0x02 != 0 {
            self.tx_ip = true;
        }
        
        if forward_for_crossloop { Some(value) } else { None }
    }

    pub fn tcp_connect(&mut self, addr: &str) -> Result<(), std::io::Error> {
        let stream = TcpStream::connect(addr)?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        self.stream = Some(stream);
        self.connected = true;
        self.dcd = true;
        Ok(())
    }

    fn tcp_disconnect(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.connected = false;
        self.dcd = false;
        if self.modem.enabled {
            self.modem.state = ModemState::Command;
        }
        // Trigger external/status interrupt if DCD IE enabled
        if self.wr[15] & 0x08 != 0 {
            self.ext_ip = true;
        }
        println!("SCC[{}]: disconnected", self.id);
    }

    // Tick — polls TCP socket for incoming data
    fn tick(&mut self, cycles: u64) {
        // Check +++ escape sequence
        if self.modem.check_escape() {
            self.drain_modem_rx();
            println!("SCC[{}]: +++ escape to command mode", self.id);
            return;
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
                    println!("SCC[{}]: remote disconnected", self.id);
                    true
                }
                Ok(n) => {
                    for &byte in &buf[..n] {
                        self.receive_byte(byte);
                    }
                    false
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    false
                }
                Err(e) => {
                    println!("SCC[{}]: read error: {}", self.id, e);
                    true
                }
            }
        } else {
            false
        };

        if disconnect {
            self.tcp_disconnect();
            if self.modem.enabled {
                self.modem.on_disconnected();
                self.drain_modem_rx();
            }
        }
    }

    // Push a byte into the receive buffer
    pub fn receive_byte(&mut self, byte: u8) {
        if self.rx_buffer.len() >= 3 {
            // Real SCC has 3-byte FIFO
            self.rx_overrun = true;
        }
        self.rx_buffer.push_back(byte);

        // Generate Rx interrupt if enabled (WR1 bits 3-4)
        let rx_int_mode = (self.wr[1] >> 3) & 0x03;
        if rx_int_mode != 0 {
            self.rx_ip = true;
        }
    }

    // Check if any interrupt is pending for this channel
    fn irq_pending(&self) -> bool {
        self.rx_ip || self.tx_ip || self.ext_ip
    }

    // Drain any bytes the modem queued for the receive buffer.
    fn drain_modem_rx(&mut self) {
        let bytes: Vec<u8> = self.modem.rx_out.drain(..).collect();
        for byte in bytes {
            self.receive_byte(byte);
        }
    }

    // Act on a ModemAction returned by the modem.
    fn handle_modem_action(&mut self, action: ModemAction) {
        match action {
            ModemAction::None => {}
            ModemAction::Dial(addr) => {
                if self.connected { self.tcp_disconnect(); }
                let addr = if addr.contains(':') {
                    addr.to_lowercase()
                } else {
                    format!("{}:23", addr.to_lowercase())
                };
                match self.tcp_connect(&addr) {
                    Ok(()) => self.modem.on_connected(),
                    Err(_) => self.modem.on_connect_failed(),
                }
                self.drain_modem_rx();
            }
            ModemAction::Hangup => {
                if self.connected { self.tcp_disconnect(); }
                self.modem.on_hangup();
                self.drain_modem_rx();
            }
            ModemAction::GoOnline => {
                if self.connected {
                    self.modem.state = ModemState::Online;
                    self.modem.plus_count = 0;
                    self.modem.send_response("CONNECT");
                } else {
                    self.modem.send_response("NO CARRIER");
                }
                self.drain_modem_rx();
            }
        }
    }
}

pub struct Scc {
    pub ch_a: SccChannel,  // Channel A = Modem port
    pub ch_b: SccChannel,  // Channel B = Printer port
    pub crossloop: bool,   // Cross-channel loopback (slot1 TX -> slot2 RX, vice versa)
}

impl Scc {
    pub fn new() -> Self {
        Self {
            ch_a: SccChannel::new("ChA/Modem"),
            ch_b: SccChannel::new("ChB/Printer"),
            crossloop: false,
        }
    }

    pub fn reset(&mut self) {
        self.ch_a.reset();
        self.ch_b.reset();
    }

    // Access SCC via Apple IIc motherboard addresses ($C038–$C03B)
    // addr is the raw address; returns read value for reads
    pub fn read(&mut self, addr: u16) -> u8 {
        match addr & 0x03 {
            0x00 => self.ch_b.read_command(), // $C038: Ch B Command/Status
            0x01 => {                          // $C039: Ch A Command/Status
                // RR3 (Interrupt Pending) is only available on Channel A
                // and includes both channels' status
                if self.ch_a.reg_ptr == 3 {
                    self.ch_a.reg_ptr = 0;
                    let mut rr3 = 0u8;
                    if self.ch_b.ext_ip { rr3 |= 0x01; }
                    if self.ch_b.tx_ip  { rr3 |= 0x02; }
                    if self.ch_b.rx_ip  { rr3 |= 0x04; }
                    if self.ch_a.ext_ip { rr3 |= 0x08; }
                    if self.ch_a.tx_ip  { rr3 |= 0x10; }
                    if self.ch_a.rx_ip  { rr3 |= 0x20; }
                    rr3
                } else {
                    self.ch_a.read_command()
                }
            }
            0x02 => self.ch_b.read_data(),    // $C03A: Ch B Data
            0x03 => self.ch_a.read_data(),    // $C03B: Ch A Data
            _ => unreachable!(),
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 0x03 {
            0x00 => self.ch_b.write_command(value), // $C038
            0x01 => {                                // $C039
                // WR9 is shared (Master Interrupt Control)
                // Channel reset bits in WR9 can reset either channel
                if self.ch_a.reg_ptr == 9 {
                    let reset_cmd = value & 0xC0;
                    match reset_cmd {
                        0x40 => { // Reset Channel B
                            self.ch_b.reset();
                            self.ch_a.wr[9] = value;
                        }
                        0x80 => { // Reset Channel A
                            self.ch_a.reset();
                            self.ch_a.wr[9] = value;
                        }
                        0xC0 => { // Force Hardware Reset (both channels)
                            self.ch_a.reset();
                            self.ch_b.reset();
                        }
                        _ => {
                            self.ch_a.wr[9] = value;
                        }
                    }
                    self.ch_a.reg_ptr = 0;
                } else {
                    self.ch_a.write_command(value);
                }
            }
            0x02 => {  // $C03A - Channel B Data
                if let Some(byte) = self.ch_b.write_data(value) {
                    if self.crossloop { self.ch_a.receive_byte(byte); }
                }
            }
            0x03 => {  // $C03B - Channel A Data
                if let Some(byte) = self.ch_a.write_data(value) {
                    if self.crossloop { self.ch_b.receive_byte(byte); }
                }
            }
            _ => unreachable!(),
        }
    }

    // Access via slot I/O addresses
    // Slot 1 ($C098–$C09F) = Channel B (printer port)
    // Slot 2 ($C0A8–$C0AF) = Channel A (modem port)
    // Apple IIc presents ACIA-compatible interface at these addresses:
    //   offset 0 ($C098/$C0A8) = Data register (read/write)
    //   offset 1 ($C099/$C0A9) = Status register (read-only)
    //   offset 2 ($C09A/$C0AA) = Command register (read/write)
    //   offset 3 ($C09B/$C0AB) = Control register (read/write)
    pub fn slot_read(&mut self, addr: u16) -> u8 {
        match addr {
            // Slot 1 = Channel B (printer port)
            0xC098..=0xC09F => {
                let offset = addr & 0x07;
                match offset {
                    0 => self.ch_b.read_data(),         // Data register
                    1 => self.ch_b.acia_status(),       // Status register
                    2 => self.ch_b.acia_command,        // Command register
                    3 => self.ch_b.acia_control,        // Control register
                    _ => self.ch_b.acia_status(),       // Mirror status for other offsets
                }
            }
            // Slot 2 = Channel A (modem port)
            0xC0A8..=0xC0AF => {
                let offset = addr & 0x07;
                match offset {
                    0 => self.ch_a.read_data(),         // Data register
                    1 => self.ch_a.acia_status(),       // Status register
                    2 => self.ch_a.acia_command,        // Command register
                    3 => self.ch_a.acia_control,        // Control register
                    _ => self.ch_a.acia_status(),       // Mirror status for other offsets
                }
            }
            _ => 0x00,
        }
    }

    pub fn slot_write(&mut self, addr: u16, value: u8) {
        match addr {
            // Slot 1 = Channel B (printer port)
            0xC098..=0xC09F => {
                let offset = addr & 0x07;
                match offset {
                    0 => {  // Data register
                        if let Some(byte) = self.ch_b.write_data(value) {
                            if self.crossloop { self.ch_a.receive_byte(byte); }
                        }
                    }
                    1 => { }  // Status register - writes ignored (or programmed reset)
                    2 => { self.ch_b.acia_command = value; }  // Command register
                    3 => { self.ch_b.acia_control = value; }  // Control register
                    _ => { }
                }
            }
            // Slot 2 = Channel A (modem port)
            0xC0A8..=0xC0AF => {
                let offset = addr & 0x07;
                match offset {
                    0 => {  // Data register
                        if let Some(byte) = self.ch_a.write_data(value) {
                            if self.crossloop { self.ch_b.receive_byte(byte); }
                        }
                    }
                    1 => { }  // Status register - writes ignored (or programmed reset)
                    2 => { self.ch_a.acia_command = value; }  // Command register
                    3 => { self.ch_a.acia_control = value; }  // Control register
                    _ => { }
                }
            }
            _ => {}
        }
    }

    // Tick both channels
    pub fn tick(&mut self, cycles: u64) {
        self.ch_a.tick(cycles);
        self.ch_b.tick(cycles);
    }

    // Check if either channel has a pending interrupt
    pub fn irq_pending(&self) -> bool {
        // Only generate IRQ if Master Interrupt Enable is set (WR9 bit 3)
        let mie = self.ch_a.wr[9] & 0x08 != 0;
        mie && (self.ch_a.irq_pending() || self.ch_b.irq_pending())
    }
}
