use std::cell::Cell;

use crate::{mmu::{LcRamMode, MemStateMask, LCRAMMODEMASK}, video::{VideoMode, VideoModeMask}};

macro_rules! set_lcram_mode {
  ($mem_state:expr, $mode:expr) => {{
      let current = $mem_state.get();
      $mem_state.set((current & !LCRAMMODEMASK) | ($mode & LCRAMMODEMASK));
      0x00
  }};
}

macro_rules! set_lcram_mode_rr {
  ($mem_state:expr, $mode:expr, $addr:expr, $counter:expr) => {{
      let (last_addr, count) = $counter.get();
      let new_count = if last_addr == $addr { count + 1 } else { 1 };
      $counter.set(($addr, new_count));

      if new_count >= 2 {
          let current = $mem_state.get();
          $mem_state.set((current & !LCRAMMODEMASK) | ($mode & LCRAMMODEMASK));
      }

      0x00
  }};
}

pub struct IOU {
  pub mem_state: Cell<u8>,
  c081_rr: Cell<(u16, u8)>, // (last read address, counter)
  c083_rr: Cell<(u16, u8)>,
  c089_rr: Cell<(u16, u8)>,
  c08b_rr: Cell<(u16, u8)>,
  c08d_rr: Cell<(u16, u8)>,
  c08f_rr: Cell<(u16, u8)>,

  pub is_80store: Cell<bool>,
  pub ioudis: Cell<bool>,

  pub video_mode: Cell<u8>,
  // extra_flags: Cell<u8>,

  pub last_key: Cell<u8>,
  pub key_ready: Cell<bool>, 
}

impl IOU {
    pub fn new() -> Self {
      Self {
          mem_state: Cell::new(MemStateMask::INIT),
          c081_rr: Cell::new((0x0000, 0)),
          c083_rr: Cell::new((0x0000, 0)),
          c089_rr: Cell::new((0x0000, 0)),
          c08b_rr: Cell::new((0x0000, 0)),
          c08d_rr: Cell::new((0x0000, 0)),
          c08f_rr: Cell::new((0x0000, 0)),

          is_80store: Cell::new(false),
          ioudis: Cell::new(false),
        
          video_mode: Cell::new(VideoMode::TEXT),
          // extra_flags: Cell::new(0),

          last_key: Cell::new(0),
          key_ready: Cell::new(false),
      }
    }

    #[rustfmt::skip]
    pub fn ss_read(&self, addr: u16) -> u8 {
      let ioudis = self.ioudis.get();
      let is_80store = self.is_80store.get();

        match addr {
            0xC000 => 0x00, // C000 49152 KBD          OECG  R   Last Key Pressed + 128
            0xC015 => 0x00, //  RSTXINT        C   R   Reset Mouse X0 Interrupt
            0xC017 => 0x00, //  RSTYINT        C   R   Reset Mouse Y0 Interrupt
            0xC018 => (is_80store as u8) << 7,
            0xC019 => 0x00, //  RSTVBL         C   R   Reset Vertical Blanking Interrupt
            0xC030 => 0x00, // C030 48200 SPKR         OECG  R   Toggle Speaker
            0xC040 => 0x00, // RDXYMSK        C   R7  Read X0/Y0 Interrupt
            0xC041 => 0x00, // C041 49217 RDVBLMSK       C   R7  Read VBL Interrupt
            0xC042 => 0x00, // C042 49218 RDX0EDGE       C   R7  Read X0 Edge Selector
            0xC043 => 0x00, // C043 49219 RDY0EDGE       C   R7  Read Y0 Edge Selector
            0xC048 => 0x00, // C048 49224 RSTXY          C  WR   Reset X and Y Interrupts
        
            0xC07E => (ioudis as u8) << 7,
            0xC07F => (check_bits_cell!(self.video_mode, VideoModeMask::DHIRES) as u8) << 7,
    
            // MMU
            0xC080 => set_lcram_mode!(self.mem_state, LcRamMode::C080),
            0xC081 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C081, addr, self.c081_rr),
            0xC082 => set_lcram_mode!(self.mem_state, LcRamMode::C082),
            0xC083 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C083, addr, self.c083_rr),
            0xC088 => set_lcram_mode!(self.mem_state, LcRamMode::C088),
            0xC089 => set_lcram_mode_rr!(self.mem_state, LcRamMode::C089, addr, self.c089_rr),
            0xC08A => set_lcram_mode!(self.mem_state, LcRamMode::C08A),
            0xC08B => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08B, addr, self.c08b_rr),
            0xC08C => set_lcram_mode!(self.mem_state, LcRamMode::C08C),
            0xC08D => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08D, addr, self.c08d_rr),
            0xC08E => set_lcram_mode!(self.mem_state, LcRamMode::C08E),
            0xC08F => set_lcram_mode_rr!(self.mem_state, LcRamMode::C08F, addr, self.c08f_rr),
            
            0xC011 => (check_bits_cell!(self.mem_state, MemStateMask::RDBNK) as u8) << 7,
            0xC012 => (check_bits_cell!(self.mem_state, MemStateMask::LCRAM) as u8) << 7,
            0xC013 => (check_bits_cell!(self.mem_state, MemStateMask::RAMRD) as u8) << 7,
            0xC014 => (check_bits_cell!(self.mem_state, MemStateMask::RAMWRT) as u8) << 7,
            0xC016 => (check_bits_cell!(self.mem_state, MemStateMask::ALTZP) as u8) << 7,

            // Display
            0xC01A => (check_bits_cell!(self.video_mode, VideoModeMask::TEXT) as u8) << 7, // RdTEXT
            0xC01B => (check_bits_cell!(self.video_mode, VideoModeMask::MIXED) as u8) << 7, // RdMIXED
            0xC01C => (check_bits_cell!(self.video_mode, VideoModeMask::PAGE2) as u8) << 7, // RdPage2
            0xC01D => (check_bits_cell!(self.video_mode, VideoModeMask::HIRES) as u8) << 7, // RdHiRes
            0xC01E => (check_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR) as u8) << 7, // RdAltChar
            0xC01F => (check_bits_cell!(self.video_mode, VideoModeMask::COL80) as u8) << 7, // Rd80Col

            0xC050 => clear_bits_cell!(self.video_mode, VideoModeMask::TEXT), // TEXT OFF
            0xC051 => set_bits_cell!(self.video_mode, VideoModeMask::TEXT),   // TEXT ON
            0xC052 => clear_bits_cell!(self.video_mode, VideoModeMask::MIXED), // MIXED OFF
            0xC053 => set_bits_cell!(self.video_mode, VideoModeMask::MIXED),   // MIXED ON
            0xC054 => clear_bits_cell!(self.video_mode, VideoModeMask::PAGE2), // Page2 OFF
            0xC055 => set_bits_cell!(self.video_mode, VideoModeMask::PAGE2),   // Page2 ON

            0xC056 => {
              clear_bits_cell!(self.video_mode, VideoModeMask::HIRES | VideoModeMask::DHIRES);
              set_bits_cell!(self.video_mode, VideoModeMask::LORES)
            },
            0xC057 => {
              clear_bits_cell!(self.video_mode, VideoModeMask::LORES);
              set_bits_cell!(self.video_mode, VideoModeMask::HIRES)
            },

            0xC058 => 0x00, // DISXY          C  WR   If IOUDIS on: Mask X0/Y0 Move Interrupts
            0xC059 => 0x00, // ENBXY          C  WR   If IOUDIS on: Allow X0/Y0 Move Interrupts
            0xC05A => 0x00, // DISVBL         C  WR   If IOUDIS on: Disable VBL Interrupts
            0xC05B => 0x00, // ENVBL          C  WR   If IOUDIS on: Enable VBL Interrupts
            0xC05C => 0x00, // X0EDGE         C  WR   If IOUDIS on: Interrupt on X0 Rising
            0xC05D => 0x00, // X0EDGE         C  WR   If IOUDIS on: Interrupt on X0 Falling
            0xC05E => if ioudis {
              0x00 // If IOUDIS on: Interrupt on Y0 Rising
            } else {
              set_bits_cell!(self.video_mode, VideoModeMask::DHIRES)
            },
            0xC05F => if ioudis {
              0x00 // If IOUDIS on: Interrupt on Y0 Falling
            } else {
              clear_bits_cell!(self.video_mode, VideoModeMask::DHIRES)
            },
            
            0xC060 => (check_bits_cell!(self.video_mode, VideoModeMask::COL80) as u8) << 7, //   C   R7  Status of 80/40 Column Switch
            0xC061 => 0x00, // C061 49249 RDBTN0        ECG  R7  Switch Input 0 / Open Apple
            0xC063 => 0x00, //                           C   R7  Bit 7 = Mouse Button Not Pressed
            0xC064 => 0x00, // C064 49252 PADDL0       OECG  R7  Analog Input 0
            0xC065 => 0x00, // C065 49253 PADDL1       OECG  R7  Analog Input 1
            0xC066 => 0x00, //           RDMOUX1        C   R7  Mouse Horiz Position
            0xC067 => 0x00, //           RDMOUY1        C   R7  Mouse Vert Position
            0xC070 => 0x00, //                           C  WR   Analog Input Reset + Reset VBLINT Flag

            0xC0E0 => 0x00, // C0E0 DRV_P0_OFF
            0xC0E1 => 0x00, // C0E1 DRV_P0_ON
            0xC0E2 => 0x00, // C0E2 DRV_P1_OFF
            0xC0E3 => 0x00, // C0E3 DRV_P1_ON
            0xC0E4 => 0x00, // C0E4 DRV_P2_OFF
            0xC0E5 => 0x00, // C0E5 DRV_P2_ON
            0xC0E6 => 0x00, // C0E6 DRV_P3_OFF
            0xC0E7 => 0x00, // C0E7 DRV_P3_ON
            0xC0E8 => 0x00, // C0E8 DRV_OFF
            0xC0E9 => 0x00, // C0E9 DRV_ON
            0xC0EA => 0x00, // C0EA DRV_SEL1
            0xC0EB => 0x00, // C0EB DRV_SEL2
            0xC0EC => 0x00, // C0EC DRV_SHIFT
            0xC0ED => 0x00, // C0ED DRV_LOAD
            0xC0EE => 0x00, // C0EE DRV_READ
            //0xC0EF => 0x00, // C0EF DRV_WRITE

            _ => {
              println!("IOU: Unhandled read at address {:04X}", addr);
              0x00
            },
        }
    }

    /// **Write Annunciator State**
    #[rustfmt::skip]
    pub fn ss_write(&self, addr: u16) -> u8 {    
      let ioudis = self.ioudis.get();

      match addr {
          0xC000 => { self.is_80store.set(false); 0x00 },
          0xC001 => { self.is_80store.set(true); 0x00 },
          0xC00C => clear_bits_cell!(self.video_mode, VideoModeMask::COL80),
          0xC00D => set_bits_cell!(self.video_mode, VideoModeMask::COL80),
          0xC00E => clear_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR),
          0xC00F => set_bits_cell!(self.video_mode, VideoModeMask::ALTCHAR),
          0xC010 => 0x00, // C010 49168 KBDSTRB      OECG WR   Keyboard Strobe

          0xC080 => set_lcram_mode!(self.mem_state, LcRamMode::C080),
          0xC081 => set_lcram_mode!(self.mem_state, LcRamMode::C081),
          0xC082 => set_lcram_mode!(self.mem_state, LcRamMode::C082),
          0xC083 => set_lcram_mode!(self.mem_state, LcRamMode::C083),
          0xC088 => set_lcram_mode!(self.mem_state, LcRamMode::C088),
          0xC089 => set_lcram_mode!(self.mem_state, LcRamMode::C089),
          0xC08A => set_lcram_mode!(self.mem_state, LcRamMode::C08A),
          0xC08B => set_lcram_mode!(self.mem_state, LcRamMode::C08B),
          0xC08C => set_lcram_mode!(self.mem_state, LcRamMode::C08C),
          0xC08D => set_lcram_mode!(self.mem_state, LcRamMode::C08D),
          0xC08E => set_lcram_mode!(self.mem_state, LcRamMode::C08E),
          0xC08F => set_lcram_mode!(self.mem_state, LcRamMode::C08F),

          0xC07E => { self.ioudis.set(false); 0x00 },
          0xC07F => { self.ioudis.set(true); 0x00 },
  
          // MMU
          0xC008 => clear_bits_cell!(self.mem_state, MemStateMask::ALTZP),
          0xC009 => set_bits_cell!(self.mem_state, MemStateMask::ALTZP),


          0xC002 => clear_bits_cell!(self.mem_state, MemStateMask::RAMRD),
          0xC003 => set_bits_cell!(self.mem_state, MemStateMask::RAMRD),
          0xC004 => clear_bits_cell!(self.mem_state, MemStateMask::RAMWRT),
          0xC005 => set_bits_cell!(self.mem_state, MemStateMask::RAMWRT),
          
          0xC028 => toggle_bits_cell!(self.mem_state, MemStateMask::ALTROM),

          0xC048 => 0x00, // C048 49224 RSTXY          C  WR   Reset X and Y Interrupts

          0xC050 => clear_bits_cell!(self.video_mode, VideoModeMask::TEXT), // TEXT OFF
          0xC051 => set_bits_cell!(self.video_mode, VideoModeMask::TEXT),   // TEXT ON
          0xC052 => clear_bits_cell!(self.video_mode, VideoModeMask::MIXED), // MIXED OFF
          0xC053 => set_bits_cell!(self.video_mode, VideoModeMask::MIXED),   // MIXED ON
          0xC054 => clear_bits_cell!(self.video_mode, VideoModeMask::PAGE2), // Page2 OFF
          0xC055 => set_bits_cell!(self.video_mode, VideoModeMask::PAGE2),   // Page2 ON

          0xC056 => {
            clear_bits_cell!(self.video_mode, VideoModeMask::HIRES | VideoModeMask::DHIRES);
            set_bits_cell!(self.video_mode, VideoModeMask::LORES)
          },
          0xC057 => {
            clear_bits_cell!(self.video_mode, VideoModeMask::LORES);
            set_bits_cell!(self.video_mode, VideoModeMask::HIRES)
          },


          0xC073 => 0x00, // C073 49267 BANKSEL       ECG W    Memory Bank Select for > 128K
          0xC078 => { self.ioudis.set(true); 0x00 },
          0xC079 => { self.ioudis.set(false); 0x00 },
    
          0xC058 => 0x00, // DISXY          C  WR   If IOUDIS on: Mask X0/Y0 Move Interrupts
          0xC059 => 0x00, // ENBXY          C  WR   If IOUDIS on: Allow X0/Y0 Move Interrupts
          0xC05A => 0x00, // DISVBL         C  WR   If IOUDIS on: Disable VBL Interrupts
          0xC05B => 0x00, // ENVBL          C  WR   If IOUDIS on: Enable VBL Interrupts
          0xC05C => 0x00, // X0EDGE         C  WR   If IOUDIS on: Interrupt on X0 Rising
          0xC05D => 0x00, // X0EDGE         C  WR   If IOUDIS on: Interrupt on X0 Falling
          0xC05E => if ioudis {
            0x00 // If IOUDIS on: Interrupt on Y0 Rising
          } else {
            set_bits_cell!(self.video_mode, VideoModeMask::DHIRES)
          },
          0xC05F => if ioudis {
            0x00 // If IOUDIS on: Interrupt on Y0 Falling
          } else {
            clear_bits_cell!(self.video_mode, VideoModeMask::DHIRES)
          },


          0xC0EF => 0x00, // C0EF DRV_WRITE

            // // **Annunciator 3 Controls DHiRes Mode**
            // 0xC05E => {
            //     clear_bits_cell!(self.annunciators, 0b1000); // Annunciator 3 OFF
            //     if self.is_ioudis() {
            //         clear_bits_cell!(self.video_mode, VideoModeMask::DHIRES); // Disable DHiRes
            //     }
            // }
            // 0xC05F => {
            //     set_bits_cell!(self.annunciators, 0b1000); // Annunciator 3 ON
            //     if self.is_ioudis() {
            //         set_bits_cell!(self.video_mode, VideoModeMask::DHIRES); // Enable DHiRes
            //     }
            // }

            _ => {
              println!("IOU: Unhandled write at address {:04X}", addr);
              0x00
            },
        }
    }
}
