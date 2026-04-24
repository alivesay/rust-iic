use std::cell::Cell;
use std::sync::OnceLock;

use crate::{device::{iwm::Iwm, joystick::Joystick, keyboard::Keyboard, memexp::MemoryExpansion, mockingboard::Mockingboard, mouse::Mouse, scc::Scc, speaker::{AudioProducer, Speaker}, zip::ZipChip}, mmu::{LcRamMode, MemStateMask, LCRAMMODEMASK}, video::VideoModeMask};

fn ultima_lc_trace_enabled() -> bool {
  static ENABLED: OnceLock<bool> = OnceLock::new();
  *ENABLED.get_or_init(|| std::env::var_os("RUSTIIC_ULTIMA_LC_TRACE").is_some())
}

/// Even $C08x access: apply mode (never includes WRITE).
/// Also resets the consecutive-read tracking since any even access
/// breaks the double-read sequence needed for write-enable.
macro_rules! set_lcram_mode {
  ($mem_state:expr, $mode:expr, $last_addr:expr) => {{
      let current = $mem_state.get();
      $mem_state.set((current & !LCRAMMODEMASK) | ($mode & LCRAMMODEMASK));
      $last_addr.set(0);
      0x00
  }};
}

/// Odd $C08x access: WRITE is enabled only on the second consecutive
/// access of the SAME odd address. On first access of a different odd
/// address, the WRITE latch is preserved.
macro_rules! set_lcram_mode_rr {
  ($mem_state:expr, $mode:expr, $addr:expr, $last_addr:expr) => {{
      let current_mode = $mode;
      let last = $last_addr.get();

      if last == $addr {
          // second consecutive access of same address: enable write
          let current = $mem_state.get();
          $mem_state.set((current & !LCRAMMODEMASK) | (current_mode & LCRAMMODEMASK));
      } else {
          // first access of new address: set LCRAM/BANK bits but preserve
          // the current WRITE state (don't clear it on bank switch)
          let current = $mem_state.get();
          let preserve_write = current & MemStateMask::WRITE;
          let new_mode = (current_mode & !MemStateMask::WRITE) | preserve_write;
          $mem_state.set((current & !LCRAMMODEMASK) | (new_mode & LCRAMMODEMASK));
          $last_addr.set($addr);
      }
      0x00
  }};
}

pub struct IOU {
  pub mem_state: Cell<u8>,
  pub last_read_addr: Cell<u16>,
  pub current_pc: Cell<u16>,

  pub is_80store: Cell<bool>,
  pub ioudis: Cell<bool>,

  pub video_mode: Cell<u8>,

  pub keyboard: Keyboard,
  pub joystick: Joystick,
  pub scc: Scc,      // Zilog 8530 SCC — Ch A: Modem, Ch B: Printer
  pub iwm: Iwm,
  pub mouse: Mouse,
  pub speaker: Speaker,
  pub memexp: MemoryExpansion, // Apple IIc Memory Expansion Card (Slot 4)
  pub mockingboard: Mockingboard, // Mockingboard sound card (Slot 5)
  pub mockingboard2: Mockingboard, // Second Mockingboard (Slot 4, conflicts with memexp)
  pub zip: ZipChip,  // ZIP Chip accelerator (optional)
  pub cycles: u64,
  pub scan_cycle: u64,  // Position within NTSC frame (resets every 17030 cycles)
  pub floating_bus: u8,  // Last byte video hardware would read from RAM at current scan position
  pub col80_switch: bool, // Physical 80/40 column slide switch (true = 80 col)
  pub disk35_mode: bool, // $C031 bit 6: false=5.25" drives, true=3.5"/SmartPort
  pub debug: bool,
  pub self_test: bool,
}

impl IOU {
    pub fn new(self_test: bool, audio_producer: AudioProducer, sample_rate: u32) -> Self {
      Self {
          mem_state: Cell::new(MemStateMask::INIT),
          last_read_addr: Cell::new(0x0000),
          current_pc: Cell::new(0x0000),
          is_80store: Cell::new(false),
          ioudis: Cell::new(false), // IOU enabled (mouse accessible)
        
          video_mode: Cell::new(VideoModeMask::TEXT),

          keyboard: Keyboard::new(),
          joystick: Joystick::new(),
          scc: Scc::new(),
          iwm: Iwm::new(),
          mouse: Mouse::new(),
          speaker: Speaker::new(audio_producer, sample_rate),
          memexp: MemoryExpansion::new(),
          mockingboard: Mockingboard::new(),  // Disabled by default, enabled via --mockingboard
          mockingboard2: Mockingboard::new(), // Disabled by default, enabled via --mockingboard2
          zip: ZipChip::new(false),  // Disabled by default, enabled via --zip
          cycles: 0,
          scan_cycle: 0,
          floating_bus: 0,
          col80_switch: true, // Default: 80-column switch ON
          disk35_mode: false, // Start in 5.25" mode
          debug: false,
          self_test,
      }
    }

    /// Enable or disable the ZIP Chip accelerator.
    pub fn set_zip_enabled(&mut self, present: bool) {
        self.zip = ZipChip::new(present);
    }

    /// Enable the Mockingboard sound card in slot 5.
    pub fn set_mockingboard_enabled(&mut self, enabled: bool) {
        self.mockingboard.set_enabled(enabled);
    }

    /// Enable the second Mockingboard in slot 4 (disables memory expansion).
    pub fn set_mockingboard2_enabled(&mut self, enabled: bool) {
        self.mockingboard2.set_enabled(enabled);
        if enabled {
            // Disable memexp - they share slot 4
            self.memexp.set_enabled(false);
        }
    }

    pub fn reset(&mut self) {
        self.mem_state.set(MemStateMask::INIT);
        self.last_read_addr.set(0x0000);
        self.is_80store.set(false);
        self.ioudis.set(false); // IOU enabled (mouse accessible) — matches IIc hardware reset state
        self.video_mode.set(VideoModeMask::TEXT);
        self.disk35_mode = false;
        self.keyboard.reset();

        self.mouse.reset();

        self.scc.reset();
        self.iwm.reset();
        self.zip.reset();
        self.mockingboard.reset();
        self.mockingboard2.reset();
    }

    #[rustfmt::skip]
    pub fn ss_read(&mut self, addr: u16) -> u8 {
      let ioudis = self.ioudis.get();
      let is_80store = self.is_80store.get();

      let result = match addr {
        0xC000 => {
          let value = self.keyboard.read_data(self.cycles);
          if self.debug && value != 0x00 {
            println!("KBD READ: C000 -> {value:02X} at cycle {}", self.cycles);
          }
          value
        },

        0xC001..=0xC00F => 0x00, 
        0xC010 => {
          let value = self.keyboard.read_strobe(self.cycles);
          if self.debug && value != 0x00 {
            println!("KBD READ: C010 -> {value:02X} at cycle {}", self.cycles);
          }
          value
        },
        
        // Status Reads (MMU & Video)
        0xC011 => (check_bits_cell!(self.mem_state, MemStateMask::RDBNK) as u8) << 7,
        0xC012 => (check_bits_cell!(self.mem_state, MemStateMask::LCRAM) as u8) << 7,
        0xC013 => (check_bits_cell!(self.mem_state, MemStateMask::RAMRD) as u8) << 7,
        0xC014 => (check_bits_cell!(self.mem_state, MemStateMask::RAMWRT) as u8) << 7,
        
        0xC015 => { 
            let val = (self.mouse.x_int.get() as u8) << 7;
            self.mouse.x_int.set(false); 
            val 
        }, //  RSTXINT        C   R   Reset Mouse X0 Interrupt
        
        0xC016 => (check_bits_cell!(self.mem_state, MemStateMask::ALTZP) as u8) << 7,

        0xC017 => { 
            let val = (self.mouse.y_int.get() as u8) << 7;
            self.mouse.y_int.set(false); 
            val 
        }, //  RSTYINT        C   R   Reset Mouse Y0 Interrupt
        
        0xC018 => (is_80store as u8) << 7,
        
        0xC019 => { 
            // Live VBL status based on scan_cycle position within NTSC frame
            // 262 scanlines × 65 cycles = 17030 cycles/frame
            // Active display: scanlines 0-191 (cycles 0-12479)
            // VBL: scanlines 192-261 (cycles 12480-17029)
            let in_vbl = self.scan_cycle >= 12480;
            self.mouse.vbl_int.set(false); // Side effect: reset VBL interrupt
            (in_vbl as u8) << 7
        }, //  RSTVBL         C   R   Reset Vertical Blanking Interrupt

        0xC01A => (check_bits_cell!(self.video_mode, VideoModeMask::TEXT) as u8) << 7,
        0xC01B => (check_bits_cell!(self.video_mode, VideoModeMask::MIXED) as u8) << 7,
        0xC01C => (check_bits_cell!(self.video_mode, VideoModeMask::PAGE2) as u8) << 7,
        0xC01D => (check_bits_cell!(self.video_mode, VideoModeMask::HIRES) as u8) << 7,
        0xC01E => (check_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR) as u8) << 7,
        0xC01F => (check_bits_cell!(self.video_mode, VideoModeMask::COL80) as u8) << 7,

        0xC020 => ((self.cycles & 1) as u8) << 7, // TAPEOUT / Cassette input — bit 7 toggles, used as entropy source

        0xC028 => { toggle_bits_cell!(self.mem_state, MemStateMask::ALTROM); 0x00 }, // ROMBANK

        0xC030 => { self.speaker.toggle(self.cycles); 0x00 }, // C030 48200 SPKR         OECG  R   Toggle Speaker

        // C031 - DISKREG: Disk interface control
        // Bit 6: 0=5.25" drives, 1=3.5"/SmartPort mode
        // Bit 7: Read/Write head select (for double-sided 3.5")
        0xC031 => {
            ((self.disk35_mode as u8) << 6) | ((self.iwm.get_head35() as u8) << 7)
        },

        // Zilog 8530 SCC — $C038: ChB Cmd, $C039: ChA Cmd, $C03A: ChB Data, $C03B: ChA Data
        0xC038..=0xC03B => self.scc.read(addr),

        0xC040 => (self.mouse.xy_mask.get() as u8) << 7, // RDXYMSK        C   R7  Read X0/Y0 Interrupt
        0xC041 => (self.mouse.vbl_mask.get() as u8) << 7, // C041 49217 RDVBLMSK       C   R7  Read VBL Interrupt
        0xC042 => (self.mouse.x0_edge.get() as u8) << 7, // C042 49218 RDX0EDGE       C   R7  Read X0 Edge Selector
        0xC043 => (self.mouse.y0_edge.get() as u8) << 7, // C043 49219 RDY0EDGE       C   R7  Read Y0 Edge Selector
        0xC048 => { self.mouse.x_int.set(false); self.mouse.y_int.set(false); 0x00 }, // C048 49224 RSTXY          C  WR   Reset X and Y Interrupts
    
        0xC070..=0xC07F => {
            // Trigger Paddle Timer - starts the RC timing circuit for analog inputs
            // Any access to $C070-$C07F triggers the paddle timers
            self.joystick.trigger(self.cycles);
            
            if addr == 0xC070 {
                self.mouse.vbl_int.set(false); // Reset VBLInt
            }
            
            match addr {
                0xC07E => (ioudis as u8) << 7, // RdIOUDis: 1 = IOUDIS on (IOU disabled)
                0xC07F => (!check_bits_cell!(self.video_mode, VideoModeMask::DHIRES) as u8) << 7, // RdDHIRES: 1 = DHIRES off (AN3 on)
                _ => 0x00,
            }
        },

        // MMU Language Card switches (reads)
        // Even addresses: set mode directly
        // Odd addresses: enable WRITE only on second consecutive read of same address
        0xC080 | 0xC084 => { set_lcram_mode!(self.mem_state, LcRamMode::C080, self.last_read_addr) },
        0xC081 | 0xC085 => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C081, addr, self.last_read_addr) },
        0xC082 | 0xC086 => { set_lcram_mode!(self.mem_state, LcRamMode::C082, self.last_read_addr) },
        0xC083 | 0xC087 => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C083, addr, self.last_read_addr) },
        0xC088 => { set_lcram_mode!(self.mem_state, LcRamMode::C088, self.last_read_addr) },
        0xC089 => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C089, addr, self.last_read_addr) },
        0xC08A => { set_lcram_mode!(self.mem_state, LcRamMode::C08A, self.last_read_addr) },
        0xC08B => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C08B, addr, self.last_read_addr) },
        0xC08C => { set_lcram_mode!(self.mem_state, LcRamMode::C08C, self.last_read_addr) },
        0xC08D => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C08D, addr, self.last_read_addr) },
        0xC08E => { set_lcram_mode!(self.mem_state, LcRamMode::C08E, self.last_read_addr) },
        0xC08F => { set_lcram_mode_rr!(self.mem_state, LcRamMode::C08F, addr, self.last_read_addr) },
      
        0xC050 => clear_bits_cell!(self.video_mode, VideoModeMask::TEXT),
        0xC051 => set_bits_cell!(self.video_mode, VideoModeMask::TEXT),
        0xC052 => clear_bits_cell!(self.video_mode, VideoModeMask::MIXED),
        0xC053 => set_bits_cell!(self.video_mode, VideoModeMask::MIXED),
        0xC054 => clear_bits_cell!(self.video_mode, VideoModeMask::PAGE2),
        0xC055 => set_bits_cell!(self.video_mode, VideoModeMask::PAGE2),

        0xC056 => {
          clear_bits_cell!(self.video_mode, VideoModeMask::HIRES);
          set_bits_cell!(self.video_mode, VideoModeMask::LORES)
        },
        0xC057 => {
          clear_bits_cell!(self.video_mode, VideoModeMask::LORES);
          set_bits_cell!(self.video_mode, VideoModeMask::HIRES)
        },

        0xC058 => if !ioudis {
          self.mouse.xy_mask.set(true); 0x00 // DISXY          C  WR   If IOUDIS off: Mask X0/Y0 Move Interrupts
        } else {
          0x00 // AN0 OFF
        },
        0xC059 => if !ioudis {
          self.mouse.xy_mask.set(false); 0x00 // ENBXY          C  WR   If IOUDIS off: Allow X0/Y0 Move Interrupts
        } else {
          0x00 // AN0 ON
        },
        0xC05A => if !ioudis {
          self.mouse.vbl_mask.set(true); 0x00 // DISVBL         C  WR   If IOUDIS off: Disable VBL Interrupts
        } else {
          0x00 // AN1 OFF
        },
        0xC05B => if !ioudis {
          self.mouse.vbl_mask.set(false); 0x00 // ENVBL          C  WR   If IOUDIS off: Enable VBL Interrupts
        } else {
          0x00 // AN1 ON
        },
        0xC05C => if !ioudis {
          self.mouse.x0_edge.set(false); 0x00 // X0EDGE         C  WR   If IOUDIS off: Interrupt on X0 Rising
        } else {
          0x00 // AN2 OFF
        },
        0xC05D => if !ioudis {
          self.mouse.x0_edge.set(true); 0x00 // X0EDGE         C  WR   If IOUDIS off: Interrupt on X0 Falling
        } else {
          0x00 // AN2 ON
        },
        0xC05E => if ioudis {
          // DHIRESON: Enable double-width graphics
          set_bits_cell!(self.video_mode, VideoModeMask::DHIRES);
          0x00
        } else {
          // IOUDis OFF: Mouse Y0 edge rising
          self.mouse.y0_edge.set(false); 0x00
        },
        0xC05F => if ioudis {
          // DHIRESOFF: Disable double-width graphics
          clear_bits_cell!(self.video_mode, VideoModeMask::DHIRES);
          0x00
        } else {
          // IOUDis OFF: Mouse Y0 edge falling
          self.mouse.y0_edge.set(true); 0x00
        },
          0xC060 => (self.col80_switch as u8) << 7, //   C   R7  Physical 80/40 Column Switch (1=80col, 0=40col)
          0xC061 => {
              // PB0 - Open Apple key / Joystick Button 0
              // Also wired to mouse button for mouse-aware apps
              let pressed = self.mouse.button0.get() || (self.self_test && self.cycles < 2_000_000);
              (pressed as u8) << 7
          }, // C061 49249 RDBTN0        ECG  R7  Switch Input 0 / Solid Apple
          0xC062 => {
              // PB1 - Solid Apple key / Joystick Button 1
              let pressed = self.mouse.button1.get() || (self.self_test && self.cycles < 2_000_000);
              (pressed as u8) << 7
          }, // C062 49250 RDBTN1        ECG  R7  Switch Input 1 / Open Apple
          0xC063 => (!self.mouse.button0.get() as u8) << 7, //                           C   R7  Bit 7 = Mouse Button Not Pressed
          // Paddle/Joystick analog inputs - delegated to Joystick module
          0xC064 => self.joystick.read(0, self.cycles),
          0xC065 => self.joystick.read(1, self.cycles),
          0xC066 => (self.mouse.x_dir.get() as u8) << 7, //           RDMOUX1        C   R7  Mouse X1 Direction (1 = right)
          0xC067 => (self.mouse.y_dir.get() as u8) << 7, //           RDMOUY1        C   R7  Mouse Y1 Direction (1 = down)
          0xC068 => 0x00, // STATEREG (IIGS) - Ignore on IIc

          0xC0E0..=0xC0EF => self.iwm.access(addr, 0, false, self.floating_bus, self.disk35_mode),

          // Slot 1 — SCC Channel A / Modem ($C098–$C09F)
          // Slot 2 — SCC Channel B / Printer ($C0A8–$C0AF)
          0xC098..=0xC09F | 0xC0A8..=0xC0AF => self.scc.slot_read(addr),

          // Slot 4 Mockingboard or Memory Expansion ($C0C0–$C0CF)
          0xC0C0..=0xC0CF => {
            if self.mockingboard2.is_enabled() {
                  self.mockingboard2.read((addr & 0x0F) as u8)
              } else {
                  self.memexp.read((addr & 0x0F) as u8)
              }
          },

          // Slot 5 Mockingboard ($C0D0–$C0DF)
          0xC0D0..=0xC0DF => {
              if self.mockingboard.is_enabled() {
                  self.mockingboard.read((addr & 0x0F) as u8)
              } else {
                  self.floating_bus
              }
          },

          // Other slot I/O (floating bus)
          0xC090..=0xC097 | 0xC0A0..=0xC0A7 | 0xC0B0..=0xC0BF | 0xC0F0..=0xC0FF => self.floating_bus,
          _ => {
            if self.debug { println!("IOU: Unhandled read at address {:04X}", addr); }
            0x00
          },
        };
        
        result
    }

    // Write Annunciator State
    #[rustfmt::skip]
    pub fn ss_write(&mut self, addr: u16, val: u8) -> u8 {    
      let ioudis = self.ioudis.get();

      let result = match addr {
          0xC000 => { 
              if self.debug { println!("IOU: 80STORE OFF"); }
            self.is_80store.set(false);
            0x00 
          },
          0xC001 => { 
              if self.debug { println!("IOU: 80STORE ON"); }
            self.is_80store.set(true);
            0x00 
          },
          0xC00C => {
              if self.debug { println!("IOU: 80COL OFF"); }
            let result = clear_bits_cell!(self.video_mode, VideoModeMask::COL80);
            result
          },
          0xC00D => {
              if self.debug { println!("IOU: 80COL ON"); }
            let result = set_bits_cell!(self.video_mode, VideoModeMask::COL80);
            result
          },
          0xC00E => {
              if self.debug { println!("IOU: ALTCHAR OFF"); }
              clear_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR)
          },
          0xC00F => {
              if self.debug { println!("IOU: ALTCHAR ON"); }
              set_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR)
          },
            0xC010 => {
                // KBDSTRB - writing also clears the strobe
                self.keyboard.write_strobe();
                0x00
            },
            
            0xC011..=0xC01F => 0x00,

          0xC030 => { self.speaker.toggle(self.cycles); 0x00 }, // Speaker toggles on any access

          // C031 - DISKREG: Disk interface control
          // Bit 6: 0=5.25" drives, 1=3.5"/SmartPort mode
          // Bit 7: Read/Write head select (for double-sided 3.5")
          0xC031 => {
              self.disk35_mode = (val & 0x40) != 0;
              self.iwm.set_head35((val >> 7) & 1);
              0x00
          },

          // Zilog 8530 SCC
          0xC038..=0xC03B => { self.scc.write(addr, val); 0x00 },

          0xC048 => { self.mouse.x_int.set(false); self.mouse.y_int.set(false); 0x00 }, // RSTXY

          0xC070..=0xC07F => {
              // Trigger Paddle Timer - starts the RC timing circuit for analog inputs
              self.joystick.trigger(self.cycles);
              
              if addr == 0xC070 {
                  self.mouse.vbl_int.set(false); // Reset VBLInt
              }

              match addr {
                  0xC078 | 0xC07E => { self.ioudis.set(true); 0x00 },  // IOUDis ON (disable IOU/mouse, enable DHIRES)
                  0xC079 | 0xC07F => { self.ioudis.set(false); 0x00 }, // IOUDis OFF (enable IOU/mouse, disable DHIRES)
                  _ => 0x00,
              }
          },
  
          // MMU
          0xC008 => clear_bits_cell!(self.mem_state, MemStateMask::ALTZP),
          0xC009 => set_bits_cell!(self.mem_state, MemStateMask::ALTZP),

          // MMU — Language Card switches (writes)
          // Same behavior as reads, matching f630a3b
          0xC080 | 0xC084 => set_lcram_mode!(self.mem_state, LcRamMode::C080, self.last_read_addr),
          0xC081 | 0xC085 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C081, addr, self.last_read_addr),
          0xC082 | 0xC086 => set_lcram_mode!(self.mem_state, LcRamMode::C082, self.last_read_addr),
          0xC083 | 0xC087 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C083, addr, self.last_read_addr),
          0xC088 => set_lcram_mode!(self.mem_state, LcRamMode::C088, self.last_read_addr),
          0xC089 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C089, addr, self.last_read_addr),
          0xC08A => set_lcram_mode!(self.mem_state, LcRamMode::C08A, self.last_read_addr),
          0xC08B => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08B, addr, self.last_read_addr),
          0xC08C => set_lcram_mode!(self.mem_state, LcRamMode::C08C, self.last_read_addr),
          0xC08D => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08D, addr, self.last_read_addr),
          0xC08E => set_lcram_mode!(self.mem_state, LcRamMode::C08E, self.last_read_addr),
          0xC08F => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08F, addr, self.last_read_addr),

          0xC002 => clear_bits_cell!(self.mem_state, MemStateMask::RAMRD),
          0xC003 => set_bits_cell!(self.mem_state, MemStateMask::RAMRD),
          0xC004 => clear_bits_cell!(self.mem_state, MemStateMask::RAMWRT),
          0xC005 => set_bits_cell!(self.mem_state, MemStateMask::RAMWRT),
          
          0xC028 => toggle_bits_cell!(self.mem_state, MemStateMask::ALTROM),


          0xC006 | 0xC007 | 0xC00A | 0xC00B => 0x00,

          0xC050 => {
              if self.debug { println!("IOU: TEXT OFF"); }
              let result = clear_bits_cell!(self.video_mode, VideoModeMask::TEXT);
              result
          }, 
          0xC051 => {
              if self.debug { println!("IOU: TEXT ON"); }
              let result = set_bits_cell!(self.video_mode, VideoModeMask::TEXT);
              result
          },   
          0xC052 => {
              if self.debug { println!("IOU: MIXED OFF"); }
              let result = clear_bits_cell!(self.video_mode, VideoModeMask::MIXED);
              result
          }, 
          0xC053 => {
              if self.debug { println!("IOU: MIXED ON"); }
              let result = set_bits_cell!(self.video_mode, VideoModeMask::MIXED);
              result
          },   
          0xC054 => {
              if self.debug { println!("IOU: PAGE2 OFF"); }
              let result = clear_bits_cell!(self.video_mode, VideoModeMask::PAGE2);
              result
          }, 
          0xC055 => {
              if self.debug { println!("IOU: PAGE2 ON"); }
              let result = set_bits_cell!(self.video_mode, VideoModeMask::PAGE2);
              result
          },   

          0xC056 => {
            if self.debug { println!("IOU: LORES ON / HIRES OFF"); }
            clear_bits_cell!(self.video_mode, VideoModeMask::HIRES);
            let result = set_bits_cell!(self.video_mode, VideoModeMask::LORES);
            result
          },
          0xC057 => {
            if self.debug { println!("IOU: HIRES ON / LORES OFF"); }
            clear_bits_cell!(self.video_mode, VideoModeMask::LORES);
            let result = set_bits_cell!(self.video_mode, VideoModeMask::HIRES);
            result
          },

          0xC062 => 0x00, // Ignore write to Button 1
          0xC068 => 0x00, // STATEREG (IIGS) - Ignore on IIc

          0xC058 => if !ioudis {
            self.mouse.xy_mask.set(true); 0x00 // DISXY  If IOUDIS off: Mask X0/Y0 Move Interrupts
          } else {
            0x00 // AN0 OFF
          },
          0xC059 => if !ioudis {
            self.mouse.xy_mask.set(false); 0x00 // ENBXY  If IOUDIS off: Allow X0/Y0 Move Interrupts
          } else {
            0x00 // AN0 ON
          },
          0xC05A => if !ioudis {
            self.mouse.vbl_mask.set(true); 0x00 // DISVBL  If IOUDIS off: Disable VBL Interrupts
          } else {
            0x00 // AN1 OFF
          },
          0xC05B => if !ioudis {
            self.mouse.vbl_mask.set(false); 0x00 // ENVBL  If IOUDIS off: Enable VBL Interrupts
          } else {
            0x00 // AN1 ON
          },
          0xC05C => if !ioudis {
            self.mouse.x0_edge.set(false); 0x00 // X0EDGE  If IOUDIS off: Interrupt on X0 Rising
          } else {
            0x00 // AN2 OFF
          },
          0xC05D => if !ioudis {
            self.mouse.x0_edge.set(true); 0x00 // X0EDGE  If IOUDIS off: Interrupt on X0 Falling
          } else {
            0x00 // AN2 ON
          },
          0xC05E => if ioudis {
            // DHIRESON: Enable double-width graphics
            set_bits_cell!(self.video_mode, VideoModeMask::DHIRES);
            0x00
          } else {
            // IOUDis OFF: Mouse Y0 edge rising
            self.mouse.y0_edge.set(false); 0x00
          },
          0xC05F => if ioudis {
            // DHIRESOFF: Disable double-width graphics
            clear_bits_cell!(self.video_mode, VideoModeMask::DHIRES);
            0x00
          } else {
            // IOUDis OFF: Mouse Y0 edge falling
            self.mouse.y0_edge.set(true); 0x00
          },


          0xC0E0..=0xC0EF => self.iwm.access(addr, val, true, self.floating_bus, self.disk35_mode),

          // Slot 1 SCC Channel A / Modem ($C098–$C09F)
          // Slot 2 SCC Channel B / Printer ($C0A8–$C0AF)
          0xC098..=0xC09F | 0xC0A8..=0xC0AF => { self.scc.slot_write(addr, val); 0x00 },

          // Slot 4 Mockingboard or Memory Expansion ($C0C0–$C0CF)
          0xC0C0..=0xC0CF => {
              if self.mockingboard2.is_enabled() {
                  self.mockingboard2.write((addr & 0x0F) as u8, val);
              } else {
                  self.memexp.write((addr & 0x0F) as u8, val);
              }
              0x00
          },

          // Slot 5 Mockingboard ($C0D0–$C0DF)
          0xC0D0..=0xC0DF => {
              if self.mockingboard.is_enabled() {
                  self.mockingboard.write((addr & 0x0F) as u8, val);
              }
              0x00
          },

          // Other slot I/O
          0xC090..=0xC097 | 0xC0A0..=0xC0A7 | 0xC0B0..=0xC0BF | 0xC0F0..=0xC0FF => 0x00,

            _ => {
              println!("IOU: Unhandled write at address {:04X}", addr);
              0x00
            },
        };

        result
    }

    pub fn check_interrupts(&self) -> bool {
        // Mouse Interrupts
        // Interrupts are active if the flag is set AND the mask is NOT set (enabled).
        // Note: xy_mask: true = masked (disabled).
        let mouse_irq = (self.mouse.x_int.get() && !self.mouse.xy_mask.get()) ||
                        (self.mouse.y_int.get() && !self.mouse.xy_mask.get()) ||
                        (self.mouse.vbl_int.get() && !self.mouse.vbl_mask.get());
        
        // SCC interrupts
        let scc_irq = self.scc.irq_pending();

        // Mockingboard interrupts (VIA timers)
        let mockingboard_irq = self.mockingboard.irq_active() || self.mockingboard2.irq_active();

        mouse_irq || scc_irq || mockingboard_irq
    }
}
