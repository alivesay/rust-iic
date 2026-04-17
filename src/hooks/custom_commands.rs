//! ProDOS Hook Demo & Game State Observer
//!
//! Demonstrates two key patterns for Apple II emulator hooks:
//!
//! 1. **MLI Hooking** - Hook the ProDOS Machine Language Interface at $BF00
//!    Every ProDOS call (CAT, OPEN, READ, PREFIX, etc.) goes through here.
//!    This is the reliable way to intercept "commands" - you see the actual
//!    system calls, not just text input.
//!
//! 2. **Memory Watch** - Observe game state by watching specific addresses.
//!    For game modding (3D overlay, trainers, etc.), you watch health, position,
//!    inventory, etc. and react to changes.

use crate::bus::Bus;
use super::{HookFilter, HookManager, HookMode, HookResult, Hook};
use std::sync::atomic::{AtomicU8, AtomicU16, Ordering};

// =============================================================================
// ProDOS MLI Hook System
// =============================================================================

/// ProDOS Machine Language Interface entry point
/// All ProDOS calls: JSR $BF00 / .byte call_num / .word param_ptr
const PRODOS_MLI: u16 = 0xBF00;

/// ProDOS call numbers (selected - see ProDOS Technical Reference)
#[allow(dead_code)]
mod mli_calls {
    pub const CREATE: u8 = 0xC0;
    pub const DESTROY: u8 = 0xC1;
    pub const RENAME: u8 = 0xC2;
    pub const SET_FILE_INFO: u8 = 0xC3;
    pub const GET_FILE_INFO: u8 = 0xC4;
    pub const ONLINE: u8 = 0xC5;
    pub const SET_PREFIX: u8 = 0xC6;
    pub const GET_PREFIX: u8 = 0xC7;
    pub const OPEN: u8 = 0xC8;
    pub const NEWLINE: u8 = 0xC9;
    pub const READ: u8 = 0xCA;
    pub const WRITE: u8 = 0xCB;
    pub const CLOSE: u8 = 0xCC;
    pub const FLUSH: u8 = 0xCD;
    pub const SET_MARK: u8 = 0xCE;
    pub const GET_MARK: u8 = 0xCF;
    pub const SET_EOF: u8 = 0xD0;
    pub const GET_EOF: u8 = 0xD1;
    pub const SET_BUF: u8 = 0xD2;
    pub const GET_BUF: u8 = 0xD3;
    pub const QUIT: u8 = 0x65;
}

/// Decode MLI call number to human-readable name
fn mli_call_name(call: u8) -> &'static str {
    match call {
        0xC0 => "CREATE",
        0xC1 => "DESTROY",
        0xC2 => "RENAME",
        0xC3 => "SET_FILE_INFO",
        0xC4 => "GET_FILE_INFO",
        0xC5 => "ONLINE",
        0xC6 => "SET_PREFIX",
        0xC7 => "GET_PREFIX",
        0xC8 => "OPEN",
        0xC9 => "NEWLINE",
        0xCA => "READ",
        0xCB => "WRITE",
        0xCC => "CLOSE",
        0xCD => "FLUSH",
        0xCE => "SET_MARK",
        0xCF => "GET_MARK",
        0xD0 => "SET_EOF",
        0xD1 => "GET_EOF",
        0xD2 => "SET_BUF",
        0xD3 => "GET_BUF",
        0x40 => "ALLOC_INTERRUPT",
        0x41 => "DEALLOC_INTERRUPT",
        0x65 => "QUIT",
        0x80 => "READ_BLOCK",
        0x81 => "WRITE_BLOCK",
        0x82 => "GET_TIME",
        _ => "UNKNOWN",
    }
}

// Atomics to store info for callback -> main thread communication
static LAST_MLI_CALL: AtomicU8 = AtomicU8::new(0);
static LAST_MLI_PARAM_LO: AtomicU8 = AtomicU8::new(0);
static LAST_MLI_PARAM_HI: AtomicU8 = AtomicU8::new(0);

// =============================================================================
// Game State Observer System  
// =============================================================================

/// Example: Watch for player health changes in a game
/// In a real game, you'd know the actual memory address for HP
#[allow(dead_code)]
const EXAMPLE_PLAYER_HP_ADDR: u16 = 0x1234;  // Placeholder - varies per game
#[allow(dead_code)]
const EXAMPLE_PLAYER_X_ADDR: u16 = 0x1235;
#[allow(dead_code)]
const EXAMPLE_PLAYER_Y_ADDR: u16 = 0x1236;

/// Tracks previous state for change detection
static WATCHED_VALUE: AtomicU8 = AtomicU8::new(0);
static WATCH_ADDR: AtomicU16 = AtomicU16::new(0);

// =============================================================================
// Public API
// =============================================================================

/// Register ProDOS MLI hook and optional memory watches
pub fn register_prodos_hooks(hooks: &mut HookManager) {
    println!("ProDOS MLI hook system enabled");
    
    // Hook the MLI entry point - fires every time ProDOS is called
    let hook = Hook::new(
        PRODOS_MLI,
        HookMode::Persistent,
        "prodos_mli_hook",
        |ctx| {
            // When JSR $BF00 is executed, the return address on stack points to
            // the call_number byte. We can't read memory from here, but we can
            // signal that an MLI call happened.
            log::trace!("MLI called at cycle {}", ctx.cycles);
            HookResult::Continue
        }
    ).with_filter(HookFilter::ProDOS);
    
    hooks.add_hook(hook);
}

/// Called from main loop to process MLI hooks with Bus access
/// This is where we can actually read the call number and parameters
pub fn check_mli_calls(bus: &mut Bus, pc: u16, sp: u8) -> Option<MliCallInfo> {
    // Only process if we're at the MLI entry
    if pc != PRODOS_MLI {
        return None;
    }
    
    // The return address is on the stack. JSR pushes (PC+2), which is the last byte
    // of the JSR instruction. RTS will add 1 to continue after the inline data.
    // Stack is at $0100 + SP, grows downward. After JSR, SP points below return addr.
    let ret_lo = bus.peek_byte(0x0100 + (sp.wrapping_add(1)) as u16);
    let ret_hi = bus.peek_byte(0x0100 + (sp.wrapping_add(2)) as u16);
    let ret_addr = u16::from_le_bytes([ret_lo, ret_hi]);
    
    // The call number is at ret_addr + 1 (the byte AFTER the JSR BF00 instruction)
    // ProDOS calling convention: JSR $BF00 / .byte call_num / .word param_ptr
    let call_num = bus.peek_byte(ret_addr.wrapping_add(1));
    
    // The parameter block pointer is the next two bytes
    let param_lo = bus.peek_byte(ret_addr.wrapping_add(2));
    let param_hi = bus.peek_byte(ret_addr.wrapping_add(3));
    let param_ptr = u16::from_le_bytes([param_lo, param_hi]);
    
    // Store for any callbacks that need it
    LAST_MLI_CALL.store(call_num, Ordering::Relaxed);
    LAST_MLI_PARAM_LO.store(param_lo, Ordering::Relaxed);
    LAST_MLI_PARAM_HI.store(param_hi, Ordering::Relaxed);
    
    Some(MliCallInfo {
        call_num,
        call_name: mli_call_name(call_num),
        param_ptr,
    })
}

/// Info about an MLI call
#[derive(Debug, Clone)]
pub struct MliCallInfo {
    pub call_num: u8,
    pub call_name: &'static str,
    pub param_ptr: u16,
}

impl MliCallInfo {
    /// Read additional details from the parameter block
    pub fn read_details(&self, bus: &mut Bus) -> String {
        match self.call_num {
            mli_calls::OPEN => {
                // OPEN param block: +0 = param_count, +1-2 = pathname ptr, +3 = io_buffer, +5 = ref_num
                let pathname_ptr = u16::from_le_bytes([
                    bus.peek_byte(self.param_ptr + 1),
                    bus.peek_byte(self.param_ptr + 2),
                ]);
                let pathname = read_prodos_string(bus, pathname_ptr);
                format!("OPEN '{}'", pathname)
            }
            mli_calls::GET_FILE_INFO | mli_calls::SET_FILE_INFO | 
            mli_calls::CREATE | mli_calls::DESTROY => {
                let pathname_ptr = u16::from_le_bytes([
                    bus.peek_byte(self.param_ptr + 1),
                    bus.peek_byte(self.param_ptr + 2),
                ]);
                let pathname = read_prodos_string(bus, pathname_ptr);
                format!("{} '{}'", self.call_name, pathname)
            }
            mli_calls::READ | mli_calls::WRITE => {
                let ref_num = bus.peek_byte(self.param_ptr + 1);
                let req_count = u16::from_le_bytes([
                    bus.peek_byte(self.param_ptr + 4),
                    bus.peek_byte(self.param_ptr + 5),
                ]);
                format!("{} ref={} bytes={}", self.call_name, ref_num, req_count)
            }
            mli_calls::SET_PREFIX | mli_calls::GET_PREFIX => {
                let pathname_ptr = u16::from_le_bytes([
                    bus.peek_byte(self.param_ptr + 1),
                    bus.peek_byte(self.param_ptr + 2),
                ]);
                let pathname = read_prodos_string(bus, pathname_ptr);
                format!("{} '{}'", self.call_name, pathname)
            }
            _ => format!("{} params=${:04X}", self.call_name, self.param_ptr),
        }
    }
}

/// Read a ProDOS-style string (length-prefixed)
fn read_prodos_string(bus: &mut Bus, addr: u16) -> String {
    let len = bus.peek_byte(addr) as usize;
    let mut s = String::with_capacity(len);
    for i in 0..len.min(64) {  // Safety limit
        let ch = bus.peek_byte(addr + 1 + i as u16);
        s.push((ch & 0x7F) as char);
    }
    s
}

// =============================================================================
// Custom Command System (via MLI interception)
// =============================================================================

/// Apple II text screen base addresses for each line (40-column mode)
/// The screen memory has an interleaved layout
const TEXT_LINE_ADDRS: [u16; 24] = [
    0x0400, 0x0480, 0x0500, 0x0580, 0x0600, 0x0680, 0x0700, 0x0780,
    0x0428, 0x04A8, 0x0528, 0x05A8, 0x0628, 0x06A8, 0x0728, 0x07A8,
    0x0450, 0x04D0, 0x0550, 0x05D0, 0x0650, 0x06D0, 0x0750, 0x07D0,
];

/// Write a string to the Apple II text screen at specified line
fn write_screen_line(bus: &mut Bus, line: usize, text: &str) {
    if line >= 24 {
        return;
    }
    let base = TEXT_LINE_ADDRS[line];
    for (i, ch) in text.chars().take(40).enumerate() {
        // Convert to Apple II screen code (normal text = char | 0x80, inverse = char & 0x3F)
        let screen_byte = if ch.is_ascii() {
            (ch as u8) | 0x80  // Normal text
        } else {
            0xA0  // Space for non-ASCII
        };
        bus.poke_byte(base + i as u16, screen_byte);
    }
    // Clear rest of line
    for i in text.len()..40 {
        bus.poke_byte(base + i as u16, 0xA0);  // Space
    }
}

/// Check if this MLI call is for a custom command and handle it
/// Returns true if command was handled (caller should suppress normal MLI processing)
pub fn check_custom_command_mli(_bus: &mut Bus, _mli_info: &MliCallInfo) -> bool {
    // Custom command examples - commented out
    // Uncomment to enable RUSTIIC and DEBUG commands at BASIC.SYSTEM prompt
    
    /*
    // Only intercept GET_FILE_INFO - that's what BASIC.SYSTEM uses to check for external commands
    if mli_info.call_num != mli_calls::GET_FILE_INFO {
        return false;
    }
    
    // Get the pathname being looked up
    let pathname_ptr = u16::from_le_bytes([
        bus.peek_byte(mli_info.param_ptr + 1),
        bus.peek_byte(mli_info.param_ptr + 2),
    ]);
    let pathname = read_prodos_string(bus, pathname_ptr).to_uppercase();
    
    match pathname.as_str() {
        "RUSTIIC" => {
            println!("==> Custom command: RUSTIIC");
            println!("    *** RUST-IIC Apple IIc Emulator ***");
            println!("    Written in Rust");
            println!("    github.com/your-repo/rust-iic");
            
            // Also write to Apple II screen (lines 21-23, near bottom)
            write_screen_line(bus, 21, "");
            write_screen_line(bus, 22, "*** RUST-IIC APPLE IIC EMULATOR ***");
            write_screen_line(bus, 23, "WRITTEN IN RUST                     ");
            
            true
        }
        "DEBUG" => {
            // Show some debug info
            let prodos_version = bus.peek_byte(0xBFFF);
            let mli_addr = u16::from_le_bytes([
                bus.peek_byte(0xBF01),
                bus.peek_byte(0xBF02),
            ]);
            println!("==> Custom command: DEBUG");
            println!("    ProDOS version: ${:02X}", prodos_version);
            println!("    MLI entry: ${:04X}", mli_addr);
            
            let debug_line = format!("PRODOS ${:02X}  MLI ${:04X}           ", prodos_version, mli_addr);
            write_screen_line(bus, 22, &debug_line);
            
            true
        }
        _ => false,
    }
    */
    
    false
}

// =============================================================================
// Memory Watch System (for game modding/3D overlay)
// =============================================================================

/// Set up a memory watch at a specific address
/// When the value changes, on_change callback is called with (old, new) values
pub fn watch_memory(addr: u16) {
    WATCH_ADDR.store(addr, Ordering::Relaxed);
    println!("Memory watch set at ${:04X}", addr);
}

/// Check watched memory for changes - call this periodically
/// Returns Some((old, new)) if the value changed
pub fn check_memory_watch(bus: &mut Bus) -> Option<(u8, u8)> {
    let addr = WATCH_ADDR.load(Ordering::Relaxed);
    if addr == 0 {
        return None;  // No watch set
    }
    
    let current = bus.peek_byte(addr);
    let previous = WATCHED_VALUE.swap(current, Ordering::Relaxed);
    
    if current != previous {
        Some((previous, current))
    } else {
        None
    }
}

/// Example: Read player state from known game addresses
/// Replace addresses with actual game-specific locations
#[allow(dead_code)]
pub struct GameState {
    pub player_hp: u8,
    pub player_x: u8,
    pub player_y: u8,
    pub player_gold: u16,
}

#[allow(dead_code)]
impl GameState {
    /// Read current game state - you'd customize addresses per game
    pub fn read(bus: &mut Bus, hp_addr: u16, x_addr: u16, y_addr: u16, gold_addr: u16) -> Self {
        Self {
            player_hp: bus.peek_byte(hp_addr),
            player_x: bus.peek_byte(x_addr),
            player_y: bus.peek_byte(y_addr),
            player_gold: u16::from_le_bytes([
                bus.peek_byte(gold_addr),
                bus.peek_byte(gold_addr + 1),
            ]),
        }
    }
}

// =============================================================================
// Legacy Custom Command Support (kept for reference)
// =============================================================================

/// Action returned by check_custom_command
#[allow(dead_code)]
pub enum CustomCommandAction {
    PrintInfo,
    DebugDump,
}

/// Check and handle custom commands - called from main loop with Bus access
#[allow(dead_code)]
pub fn check_custom_command(_bus: &mut Bus) -> Option<CustomCommandAction> {
    // Legacy - see check_mli_calls for the better approach
    None
}
/// Register all hooks (called from main)
pub fn register_hooks(hooks: &mut HookManager) {
    register_prodos_hooks(hooks);
}

/// Convenience alias for backward compatibility
#[allow(dead_code)]
pub fn register_custom_commands(hooks: &mut HookManager) {
    register_hooks(hooks);
}
