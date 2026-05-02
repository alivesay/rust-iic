# rust-iic

A cycle-accurate Apple //c emulator.

## Specifications

| | |
|---|---|
| Processor | 65C02 @ 1.023 MHz (8 MHz via optional ZIP CHIP II-8) |
| Memory | 128 KB RAM, 32 KB ROM, 1 MB expansion (slot 4) |
| Text | 40 / 80 column, 24 lines |
| Graphics | Lo-Res 40x48 (16 colors), Hi-Res 280x192 (6 colors), Double Hi-Res 560x192 (16 colors), Mixed mode, NTSC |
| Display | CRT (monochrome/color), LCD |
| Sound | Speaker, up to 2x Mockingboard (4x AY-3-8910) |
| Storage | 2x 5.25" floppy (WOZ / DOS / ProDOS / nibble / 2mg), 2x 3.5" SmartPort floppy, 2x SmartPort hard disk (HDV) |
| Input | Keyboard, mouse, paddles |
| Serial | Zilog 8530 SCC, Hayes modem emulation, TCP networking |

## Build & Run

```
rustup update stable
cargo run --release -- [disk] [options]
```

Requires a Rust toolchain and a working GPU (Metal/Vulkan/DX12).

## Common Options

| Option | Description |
|--------|-------------|
| `[disk]` | Disk for drive 1 |
| `--disk2`, `--disk35`, `--disk35-2`, `--hdv`, `--hdv2` | More drives |
| `--shader none\|crt\|lcd` | Display shader (default `crt`) |
| `--monochrome` | Green phosphor |
| `--fast-disk` | Skip rotational latency |
| `--speed <n>` | CPU speed multiplier |
| `--fullscreen` | Start fullscreen |
| `--mockingboard`, `--mockingboard2` | Mockingboards in slot 5 / 4 |
| `--zip` | ZIP CHIP II-8 |
| `--mouse`, `--paddle` | Input devices |
| `--serial host:port`, `--modem` | Serial / modem |

Run with `--help` for the full list.

## Disk Formats

`.woz` (1 & 2), `.dsk`, `.do`, `.po`, `.d13`, `.nib`, `.nb2`, `.2mg`, `.2img`, `.hdv`.

Note: Non-WOZ 5.25" images are converted to WOZ and saved to working sidecar `<file>.woz`.

## Keys

| Key | Action |
|-----|--------|
| F1 | Load disk into drive 1 (5.25") |
| F2 | Load disk into drive 2 (5.25") |
| F3 | Load disk into drive 3 (3.5") |
| F4 | Load disk into drive 4 (3.5") |
| F5 | Mono / color |
| F6 | Toolbar |
| F8 | Settings |
| Ctrl+F7 | Shader settings |
| Ctrl+F8 | Drive audio |
| Ctrl+F10 | Debug logging |
| F12 | CPU monitor |
| Cmd+Enter | Fullscreen |
| Ctrl+Backspace | Soft reset |
| Ctrl+Cmd+Backspace | Hard reset |
| Ctrl+Z | Toggle ZIP CHIP |
| Left/Right Cmd | Open / Solid Apple |
| Cmd+V | Paste text to keyboard input |

Note: When //c mouse is enabled, click the window to grab the mouse then Cmd+Tab to release.
