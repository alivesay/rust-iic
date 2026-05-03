pub mod server;
pub mod session;
pub mod source;
pub mod sources;
pub mod transport;

pub use server::{start, BbsEvent, BbsHandle};

pub(crate) const TERM_BIN: &[u8] = include_bytes!("../../build/asm/rustiic_term.bin");

const DISK_II_BOOT: u16 = 0xC600;

pub fn jumpstart_term(cpu: &mut crate::cpu::CPU) {
    debug_assert_eq!(TERM_BIN.len(), 512, "rustiic_term.bin must be 512 bytes (two sectors)");
    cpu.hooks.add_oneshot(DISK_II_BOOT, "bbs_jumpstart_term", |_ctx| {
        crate::hooks::HookResult::Continue
    });
}
