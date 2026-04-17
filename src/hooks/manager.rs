//! ROM/Execution Hook System
//!
//! Provides a flexible mechanism to hook into specific PC addresses during
//! emulation. Useful for:
//! - Installing drivers at specific boot points (e.g., Mockingboard after mouse init)
//! - Game modding: intercepting draw functions, game state, etc.
//! - Debugging: breakpoints with callbacks
//! - Patching ROM behavior without modifying ROM data
//!
//! Hook Types:
//! - OneShot: Fires once, then automatically removes itself
//! - Persistent: Fires every time the PC hits the address
//! - Replace: Fires and skips the original instruction (for patching)

use std::collections::HashMap;

/// Hook execution mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HookMode {
    /// Fire once, then remove the hook
    OneShot,
    /// Fire every time PC reaches this address  
    Persistent,
    /// Fire and skip the original instruction (advanced patching)
    Replace,
}

/// Result of hook execution
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HookResult {
    /// Continue normal execution after hook
    Continue,
    /// Skip the instruction at this address (only valid for Replace mode)
    Skip,
    /// Remove this hook after execution
    Remove,
}

/// Context passed to hook callbacks with CPU/system state
#[derive(Debug)]
#[allow(dead_code)]
pub struct HookContext {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub cycles: u64,
}

/// Filter conditions for hooks - determines when a hook should be active
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum HookFilter {
    /// Always active (no filtering)
    Always,
    /// Only when ProDOS is detected (checks MLI signature at $BF00)
    ProDOS,
    /// Only when DOS 3.3 is detected
    DOS33,
    /// Only when BASIC.SYSTEM is active (ProDOS BASIC)
    BasicSystem,
    /// Only when Applesoft BASIC is running (ROM BASIC)
    Applesoft,
    /// Custom memory signature check: (address, expected_bytes)
    MemorySignature(u16, Vec<u8>),
    /// Multiple filters - ALL must match
    All(Vec<HookFilter>),
    /// Multiple filters - ANY must match
    Any(Vec<HookFilter>),
    /// Invert another filter
    Not(Box<HookFilter>),
}

impl Default for HookFilter {
    fn default() -> Self {
        HookFilter::Always
    }
}

/// A registered hook
pub struct Hook {
    pub address: u16,
    pub mode: HookMode,
    pub name: String,
    pub callback: Box<dyn FnMut(&HookContext) -> HookResult + Send>,
    pub enabled: bool,
    pub filter: HookFilter,
}

impl Hook {
    pub fn new<F>(address: u16, mode: HookMode, name: impl Into<String>, callback: F) -> Self
    where
        F: FnMut(&HookContext) -> HookResult + Send + 'static,
    {
        Self {
            address,
            mode,
            name: name.into(),
            callback: Box::new(callback),
            enabled: true,
            filter: HookFilter::Always,
        }
    }
    
    /// Add a filter to this hook
    pub fn with_filter(mut self, filter: HookFilter) -> Self {
        self.filter = filter;
        self
    }
}

/// A timer-based hook that fires after a certain number of cycles
pub struct TimedHook {
    pub trigger_cycle: u64,
    pub name: String,
    pub callback: Box<dyn FnMut(u64) + Send>,
}

impl TimedHook {
    pub fn new<F>(trigger_cycle: u64, name: impl Into<String>, callback: F) -> Self
    where
        F: FnMut(u64) + Send + 'static,
    {
        Self {
            trigger_cycle,
            name: name.into(),
            callback: Box::new(callback),
        }
    }
}

/// Manages all registered hooks (address-based and timer-based)
pub struct HookManager {
    /// Hooks indexed by address for fast lookup
    hooks: HashMap<u16, Vec<Hook>>,
    /// Timer-based hooks (sorted by trigger cycle)
    timed_hooks: Vec<TimedHook>,
    /// Count of hooks that have fired (for debugging)
    pub fire_count: u64,
    /// Pending action: activate Mockingboard slot 4 (set by hooks, cleared after processing)
    pub pending_mockingboard_activate: bool,
    /// Pending action: activate Mockingboard slot 5 (set by hooks, cleared after processing)
    pub pending_mockingboard2_activate: bool,
    /// Pending action: check for custom commands (set by GETLN hook)
    pub pending_custom_command_check: bool,
}

impl Default for HookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HookManager {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
            timed_hooks: Vec::new(),
            fire_count: 0,
            pending_mockingboard_activate: false,
            pending_mockingboard2_activate: false,
            pending_custom_command_check: false,
        }
    }

    /// Register a new hook
    pub fn add_hook(&mut self, hook: Hook) {
        let address = hook.address;
        log::debug!("Hook registered: '{}' at ${:04X} ({:?})", hook.name, address, hook.mode);
        self.hooks.entry(address).or_default().push(hook);
    }

    /// Convenience method to add a one-shot hook
    #[allow(dead_code)]
    pub fn add_oneshot<F>(&mut self, address: u16, name: impl Into<String>, callback: F)
    where
        F: FnMut(&HookContext) -> HookResult + Send + 'static,
    {
        self.add_hook(Hook::new(address, HookMode::OneShot, name, callback));
    }

    /// Convenience method to add a persistent hook
    #[allow(dead_code)]
    pub fn add_persistent<F>(&mut self, address: u16, name: impl Into<String>, callback: F)
    where
        F: FnMut(&HookContext) -> HookResult + Send + 'static,
    {
        self.add_hook(Hook::new(address, HookMode::Persistent, name, callback));
    }

    /// Remove all hooks at an address
    #[allow(dead_code)]
    pub fn remove_hooks_at(&mut self, address: u16) {
        self.hooks.remove(&address);
    }

    /// Remove a hook by name
    #[allow(dead_code)]
    pub fn remove_hook_by_name(&mut self, name: &str) {
        for hooks in self.hooks.values_mut() {
            hooks.retain(|h| h.name != name);
        }
        // Clean up empty entries
        self.hooks.retain(|_, v| !v.is_empty());
    }

    /// Add a timer-based hook that fires after N cycles from boot
    pub fn add_timed_hook<F>(&mut self, cycles_from_boot: u64, name: impl Into<String>, callback: F)
    where
        F: FnMut(u64) + Send + 'static,
    {
        let name = name.into();
        log::debug!("Timed hook registered: '{}' at cycle {}", name, cycles_from_boot);
        self.timed_hooks.push(TimedHook::new(cycles_from_boot, name, callback));
        // Keep sorted by trigger cycle for efficient checking
        self.timed_hooks.sort_by_key(|h| h.trigger_cycle);
    }

    /// Check and execute any timed hooks that should fire at current cycle
    /// Returns true if any hooks fired
    pub fn check_timed_hooks(&mut self, current_cycle: u64) -> bool {
        let mut fired = false;
        
        // Process all hooks that should fire (they're sorted, so we can stop early)
        while let Some(hook) = self.timed_hooks.first() {
            if hook.trigger_cycle <= current_cycle {
                let mut hook = self.timed_hooks.remove(0);
                self.fire_count += 1;
                println!("==> Timed hook fired: '{}' at cycle {} (target: {})", 
                    hook.name, current_cycle, hook.trigger_cycle);
                
                // Check for special system hooks
                if hook.name == "mockingboard_activate" {
                    println!("==> Setting pending_mockingboard_activate = true");
                    self.pending_mockingboard_activate = true;
                }
                if hook.name == "mockingboard2_activate" {
                    println!("==> Setting pending_mockingboard2_activate = true");
                    self.pending_mockingboard2_activate = true;
                }
                
                (hook.callback)(current_cycle);
                fired = true;
            } else {
                break; // No more hooks ready to fire
            }
        }
        
        fired
    }

    /// Check if any timed hooks are pending
    #[inline]
    pub fn has_pending_timed_hooks(&self) -> bool {
        !self.timed_hooks.is_empty()
    }

    /// Clear all timed hooks (call on reset)
    pub fn clear_timed_hooks(&mut self) {
        self.timed_hooks.clear();
    }

    /// Register the Mockingboard activation hook using a timer.
    /// This fires after enough cycles for DOS/ProDOS to fully initialize.
    /// ~3 seconds at 1MHz = ~3,000,000 cycles is safe for most boot scenarios.
    /// `current_cycle` allows registering relative to current time (for reset handling).
    /// `slot` is 0 for slot 4, 1 for slot 5 (second Mockingboard).
    pub fn register_mockingboard_hook(&mut self, slot: u8, delay_cycles: u64) {
        let trigger_at = delay_cycles;  // From boot
        let hook_name = if slot == 0 { "mockingboard_activate" } else { "mockingboard2_activate" };
        let slot_num = if slot == 0 { 4 } else { 5 };
        
        println!("Mockingboard slot {} timed activation: will activate at cycle {} (~{:.1}s from boot at 1MHz)",
            slot_num, trigger_at, delay_cycles as f64 / 1_000_000.0);
        self.add_timed_hook(
            trigger_at,
            hook_name,
            move |cycle| {
                log::info!("Timed hook triggered at cycle {}: Mockingboard slot {} activating", cycle, slot_num);
            }
        );
    }

    /// Check if any hooks exist at this address (fast path for execution loop)
    #[inline]
    pub fn has_hooks_at(&self, address: u16) -> bool {
        self.hooks.contains_key(&address)
    }
    
    /// Check if a filter matches the current system state
    fn check_filter<F>(filter: &HookFilter, peek: &mut F) -> bool 
    where
        F: FnMut(u16) -> u8,
    {
        match filter {
            HookFilter::Always => true,
            
            HookFilter::ProDOS => {
                // ProDOS MLI entry point at $BF00 is a JMP instruction
                // Different ProDOS versions jump to different addresses, but they all:
                // 1. Have JMP ($4C) at $BF00
                // 2. Jump somewhere in the $BFxx page (MLI handler)
                // This provides better filtering than just checking for $4C
                let opcode = peek(0xBF00);
                let target_hi = peek(0xBF02);
                opcode == 0x4C && target_hi == 0xBF
            }
            
            HookFilter::DOS33 => {
                // DOS 3.3 has "DOS" at $9D84 (in the RWTS area)
                peek(0x9D84) == 0xC4 && peek(0x9D85) == 0xCF && peek(0x9D86) == 0xD3 // "DOS" in Apple ASCII
            }
            
            HookFilter::BasicSystem => {
                // BASIC.SYSTEM sets up BI entry at $BE00
                // Check for ProDOS AND BASIC.SYSTEM signature
                Self::check_filter(&HookFilter::ProDOS, peek) &&
                peek(0xBE00) != 0x00 // BASIC.SYSTEM modifies this area
            }
            
            HookFilter::Applesoft => {
                // Applesoft BASIC is active when BASIC warm start vector is set
                // Check $67-$68 (MEMSIZ) is reasonable for BASIC
                let memsiz_lo = peek(0x67);
                let memsiz_hi = peek(0x68);
                memsiz_hi >= 0x08 && memsiz_hi <= 0xBF && memsiz_lo == 0x00
            }
            
            HookFilter::MemorySignature(addr, bytes) => {
                for (i, &expected) in bytes.iter().enumerate() {
                    if peek(*addr + i as u16) != expected {
                        return false;
                    }
                }
                true
            }
            
            HookFilter::All(filters) => {
                filters.iter().all(|f| Self::check_filter(f, peek))
            }
            
            HookFilter::Any(filters) => {
                filters.iter().any(|f| Self::check_filter(f, peek))
            }
            
            HookFilter::Not(filter) => {
                !Self::check_filter(filter, peek)
            }
        }
    }

    /// Execute all hooks at the given address, with memory access for filter checking
    /// Returns true if instruction should be skipped (Replace mode)
    /// `peek` is a function that reads a byte from memory without side effects
    pub fn execute_hooks_filtered<F>(&mut self, ctx: &HookContext, peek: &mut F) -> bool 
    where
        F: FnMut(u16) -> u8,
    {
        let Some(hooks) = self.hooks.get_mut(&ctx.pc) else {
            return false;
        };

        let mut skip_instruction = false;
        let mut hooks_to_remove: Vec<usize> = Vec::new();

        for (idx, hook) in hooks.iter_mut().enumerate() {
            if !hook.enabled {
                continue;
            }
            
            // Check filter before firing
            if !Self::check_filter(&hook.filter, peek) {
                log::trace!("Hook '{}' at ${:04X} skipped (filter not matched)", hook.name, ctx.pc);
                continue;
            }

            self.fire_count += 1;
            log::trace!("Hook fired: '{}' at ${:04X}", hook.name, ctx.pc);
            
            // Check for special system hooks that need to set pending actions
            // (Note: address-based mockingboard_activate is deprecated, use timed hooks)
            if hook.name == "mockingboard_activate" {
                self.pending_mockingboard_activate = true;
            }
            // Custom command check: only trigger when CR ($8D) was just entered
            // GETLN fires this hook for every keystroke; we only want to check after Enter
            if hook.name == "custom_command_check" && ctx.a == 0x8D {
                self.pending_custom_command_check = true;
            }

            let result = (hook.callback)(ctx);

            match result {
                HookResult::Skip => {
                    if hook.mode == HookMode::Replace {
                        skip_instruction = true;
                    }
                }
                HookResult::Remove => {
                    hooks_to_remove.push(idx);
                }
                HookResult::Continue => {}
            }

            // OneShot hooks always remove themselves
            if hook.mode == HookMode::OneShot {
                hooks_to_remove.push(idx);
            }
        }

        // Remove hooks in reverse order to preserve indices
        hooks_to_remove.sort_unstable();
        hooks_to_remove.dedup();
        for idx in hooks_to_remove.into_iter().rev() {
            let removed = hooks.remove(idx);
            log::debug!("Hook removed: '{}' at ${:04X}", removed.name, ctx.pc);
        }

        // Clean up empty hook lists
        if hooks.is_empty() {
            self.hooks.remove(&ctx.pc);
        }

        skip_instruction
    }

    /// List all registered hooks (for debugging)
    #[allow(dead_code)]
    pub fn list_hooks(&self) -> Vec<(u16, &str, HookMode)> {
        let mut result = Vec::new();
        for hooks in self.hooks.values() {
            for hook in hooks {
                result.push((hook.address, hook.name.as_str(), hook.mode));
            }
        }
        result.sort_by_key(|(addr, _, _)| *addr);
        result
    }

    /// Clear all hooks
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.hooks.clear();
    }
}

/// Well-known Apple IIc ROM addresses for hooking
#[allow(dead_code)]
pub mod iic_addresses {
    /// RESET entry point
    pub const RESET: u16 = 0xFA62;
    
    /// After SETNORM in reset sequence
    pub const AFTER_SETNORM: u16 = 0xFA66;
    
    /// After INIT in reset sequence
    pub const AFTER_INIT: u16 = 0xFA69;
    
    /// After ZZQUIT (Setvid/Setkbd) in reset sequence
    pub const AFTER_ZZQUIT: u16 = 0xFA6C;
    
    /// Mouse firmware init call (jsr initmouse)
    pub const INITMOUSE_CALL: u16 = 0xFA6C;
    
    /// First instruction AFTER mouse init returns - ideal for Mockingboard activation
    pub const AFTER_INITMOUSE: u16 = 0xFA6F;
    
    /// After CLRPORT in reset sequence
    pub const AFTER_CLRPORT: u16 = 0xFA72;
    
    /// After RESET_X (all device init done)
    pub const AFTER_RESET_X: u16 = 0xFA7B;
    
    /// PWRUP2 - about to display "Apple //c" banner
    pub const PWRUP2: u16 = 0xFB12;
    
    /// About to jump to boot device ($C600 usually)
    pub const BEFORE_BOOT: u16 = 0xFB19;
    
    /// Monitor entry point
    pub const MON: u16 = 0xFF65;
    
    /// COUT - character output
    pub const COUT: u16 = 0xFDED;
    
    /// KEYIN - keyboard input
    pub const KEYIN: u16 = 0xFD1B;
    
    /// HOME - clear screen
    pub const HOME: u16 = 0xFC58;
}