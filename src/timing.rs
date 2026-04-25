// Apple IIc NTSC timing constants.
//
// Derived from Steve Chamberlin's analysis of Apple II clock timing:
// <https://www.bigmessowires.com/2020/11/12/how-many-bits-in-a-track-revisiting-basic-assumptions/>
//
// The Apple II's CPU clock is derived from the NTSC color-burst frequency:
//   Master clock = 4 × colorburst = 4 × 3.579545 MHz = 14.318181 MHz
//   CPU clock = master / 14 = 1.022727 MHz (nominal)
//
// However, every 65th clock cycle (at each scanline boundary) is stretched
// to 8/7ths of a normal cycle. This "long cycle" aligns the video timing
// with the NTSC color subcarrier. The effective average clock rate is:
//   1/(65×7) slower → 1.020484 MHz effective
//
// Timing hierarchy:
//   65 CPU cycles per scanline (64 normal + 1 long)
//   262 scanlines per frame
//   17030 CPU cycles per frame
//   ~59.9227 frames per second (NTSC field rate: 60/1.001)

// NTSC color-burst frequency in Hz (exact: 315/88 MHz).
pub const COLORBURST_HZ: f64 = 3_579_545.0;

// Master oscillator: 4× color-burst.
pub const MASTER_CLOCK_HZ: f64 = COLORBURST_HZ * 4.0; // 14,318,180 Hz

// Nominal CPU clock: master / 14.
// This is the "fast" cycle rate — 64 of every 65 cycles run at this speed.
pub const CPU_CLOCK_NOMINAL_HZ: f64 = MASTER_CLOCK_HZ / 14.0; // 1,022,727.14 Hz

// Effective CPU clock accounting for the long cycle (every 65th is 8/7ths).
// Average cycle period = (64 + 8/7) / 65 normal periods
// = (64×7 + 8) / (65×7) = 456/455 normal periods
// Effective clock = nominal × 455/456 = 1,020,484.32 Hz
pub const CPU_CLOCK_EFFECTIVE_HZ: f64 = CPU_CLOCK_NOMINAL_HZ * 455.0 / 456.0; // 1,020,484.32 Hz

// CPU cycles per scanline (includes the long cycle).
pub const CYCLES_PER_SCANLINE: u64 = 65;

// NTSC scanlines per frame (262 for non-interlaced).
pub const SCANLINES_PER_FRAME: u64 = 262;

// Total CPU cycles per NTSC frame.
pub const CYCLES_PER_FRAME: u64 = CYCLES_PER_SCANLINE * SCANLINES_PER_FRAME; // 17030

// First cycle of VBL region (scanline 192 × 65 cycles/scanline).
pub const VBL_START_CYCLE: u64 = 192 * CYCLES_PER_SCANLINE; // 12480

// NTSC frame rate in Hz (exact: 30/1.001 × 2 fields ≈ 59.9401).
// Actually for Apple II: master_clock / (CYCLES_PER_FRAME × 14 × 65/64... )
// Simplified: effective_clock / cycles_per_frame.
pub const FRAME_RATE_HZ: f64 = CPU_CLOCK_EFFECTIVE_HZ / CYCLES_PER_FRAME as f64; // ~59.922 Hz

// Duration of one frame in microseconds.
pub const FRAME_DURATION_MICROS: u64 = (1_000_000.0 / FRAME_RATE_HZ) as u64; // ~16688 µs

// Cycles per second for real-time conversion (audio, timers, etc.).
pub const CYCLES_PER_SECOND: f64 = CPU_CLOCK_EFFECTIVE_HZ;
