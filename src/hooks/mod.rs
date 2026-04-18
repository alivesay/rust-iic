//! Hook System for Apple IIc Emulator
//!
//! This module provides:
//! - PC-based hooks (fire when execution reaches a specific address)
//! - Timer-based hooks (fire after N CPU cycles)
//! - Hook filters (ProDOS, DOS 3.3, memory signatures)
//! - ProDOS MLI call interception and logging
//! - Custom command support (RUSTIIC, DEBUG, etc.)
//! - SmartPort interception for HDV hard drive support

mod manager;
mod custom_commands;

// Re-export main types from manager
#[allow(unused_imports)]
pub use manager::{
    Hook,
    HookContext,
    HookFilter,
    HookManager,
    HookMode,
    HookResult,
    SmartPortCall,
    TimedHook,
};

// Re-export custom command functions
#[allow(unused_imports)]
pub use custom_commands::{
    check_custom_command_mli,
    check_mli_calls,
    register_hooks,
    register_prodos_hooks,
    MliCallInfo,
};
