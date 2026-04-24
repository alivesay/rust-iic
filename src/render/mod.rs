//! Host-side rendering module.
//!
//! This module contains all display/rendering code that is NOT part of
//! Apple IIc emulation - it's the host frontend that can be swapped out
//! for different backends (wgpu, SDL, libretro, embedded, etc.)
//!
//! The core emulation (bus, cpu, video, etc.) generates a pixel buffer,
//! and this module handles displaying it with optional post-processing.

mod crt;
mod gui;
mod lcd;
mod screen;

pub use crt::CrtRenderer;
pub use gui::{
    blit_direct, blit_nearest, DriveStatusInfo, ToolbarAction, render_toolbar_ui,
};
pub use lcd::LcdRenderer;
pub use screen::PostProcessor;
