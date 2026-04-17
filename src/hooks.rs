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

/// A registered hook
pub struct Hook {
    pub address: u16,
    pub mode: HookMode,
    pub name: String,
    pub callback: Box<dyn FnMut(&HookContext) -> HookResult + Send>,
    pub enabled: bool,
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
        }
    }
}

/// Manages all registered hooks
pub struct HookManager {
    /// Hooks indexed by address for fast lookup
    hooks: HashMap<u16, Vec<Hook>>,
    /// Count of hooks that have fired (for debugging)
    pub fire_count: u64,
    /// Pending action: activate Mockingboard (set by hooks, cleared after processing)
    pub pending_mockingboard_activate: bool,
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
            fire_count: 0,
            pending_mockingboard_activate: false,
        }
    }

    /// Register a new hook
    pub fn add_hook(&mut self, hook: Hook) {
        let address = hook.address;
        log::debug!("Hook registered: '{}' at ${:04X} ({:?})", hook.name, address, hook.mode);
        self.hooks.entry(address).or_default().push(hook);
    }

    /// Convenience method to add a one-shot hook
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

    /// Register the Mockingboard activation hook at the ideal ROM location
    /// (after mouse firmware init completes at $FA6F).
    /// This is safer than timer-based activation as it hooks at the exact right moment.
    pub fn register_mockingboard_hook(&mut self) {
        self.add_oneshot(
            iic_addresses::AFTER_INITMOUSE, 
            "mockingboard_activate",
            |_ctx| {
                log::info!("Hook triggered: Mouse firmware init complete, Mockingboard ready to activate");
                HookResult::Continue
            }
        );
    }

    /// Check if any hooks exist at this address (fast path for execution loop)
    #[inline]
    pub fn has_hooks_at(&self, address: u16) -> bool {
        self.hooks.contains_key(&address)
    }

    /// Execute all hooks at the given address
    /// Returns true if instruction should be skipped (Replace mode)
    pub fn execute_hooks(&mut self, ctx: &HookContext) -> bool {
        let Some(hooks) = self.hooks.get_mut(&ctx.pc) else {
            return false;
        };

        let mut skip_instruction = false;
        let mut hooks_to_remove: Vec<usize> = Vec::new();

        for (idx, hook) in hooks.iter_mut().enumerate() {
            if !hook.enabled {
                continue;
            }

            self.fire_count += 1;
            log::debug!("Hook fired: '{}' at ${:04X}", hook.name, ctx.pc);
            
            // Check for special system hooks that need to set pending actions
            if hook.name == "mockingboard_activate" {
                self.pending_mockingboard_activate = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_oneshot_hook() {
        let mut manager = HookManager::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        manager.add_oneshot(0x1234, "test", move |_ctx| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        });

        let ctx = HookContext {
            pc: 0x1234,
            a: 0, x: 0, y: 0, sp: 0xFF, p: 0, cycles: 0,
        };

        // First execution should fire
        assert!(!manager.execute_hooks(&ctx));
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second execution should not fire (hook removed)
        assert!(!manager.execute_hooks(&ctx));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_persistent_hook() {
        let mut manager = HookManager::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        manager.add_persistent(0x1234, "test", move |_ctx| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            HookResult::Continue
        });

        let ctx = HookContext {
            pc: 0x1234,
            a: 0, x: 0, y: 0, sp: 0xFF, p: 0, cycles: 0,
        };

        // Should fire multiple times
        manager.execute_hooks(&ctx);
        manager.execute_hooks(&ctx);
        manager.execute_hooks(&ctx);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
