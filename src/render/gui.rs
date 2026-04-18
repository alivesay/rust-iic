use std::sync::LazyLock;

/// Drive status info passed from the IWM to the GUI renderer.
pub struct DriveStatusInfo {
    pub has_disk: bool,
    pub is_active: bool,
    pub is_write_protected: bool,
}

/// Apple II character ROM
const CHAR_ROM: &[u8; 1024] = include_bytes!("../../assets/font.bin");

/// Decode a PNG (white-on-black) into a 32×32 luminance mask at first use.
fn decode_icon_mask(png_bytes: &[u8]) -> [u8; 1024] {
    let img = image::load_from_memory(png_bytes).expect("bad icon PNG").to_luma8();
    let mut mask = [0u8; 1024];
    for (i, p) in img.pixels().enumerate().take(1024) {
        mask[i] = p.0[0];
    }
    mask
}

static DISK1_MASK: LazyLock<[u8; 1024]> =
    LazyLock::new(|| decode_icon_mask(include_bytes!("../../assets/disk1.png")));
static DISK2_MASK: LazyLock<[u8; 1024]> =
    LazyLock::new(|| decode_icon_mask(include_bytes!("../../assets/disk2.png")));
static DISK35_1_MASK: LazyLock<[u8; 1024]> =
    LazyLock::new(|| decode_icon_mask(include_bytes!("../../assets/disk35_1.png")));
static DISK35_2_MASK: LazyLock<[u8; 1024]> =
    LazyLock::new(|| decode_icon_mask(include_bytes!("../../assets/disk35_2.png")));

/// Height of the native-resolution status bar at the bottom of the window.
pub const STATUS_BAR_HEIGHT: u32 = 96;

/// Nearest-neighbor blit from src into frame at (dst_x, dst_y) scaled to (dst_w × dst_h).
pub fn blit_scaled(
    frame: &mut [u8], frame_w: u32,
    src: &[u8], src_w: u32, src_h: u32,
    dst_x: u32, dst_y: u32, dst_w: u32, dst_h: u32,
) {
    for y in 0..dst_h {
        let sy = (y as u64 * src_h as u64 / dst_h as u64) as usize;
        let src_row = sy * src_w as usize * 4;
        let dst_row = (dst_y + y) as usize * frame_w as usize * 4;
        for x in 0..dst_w {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as usize;
            let si = src_row + sx * 4;
            let di = dst_row + (dst_x + x) as usize * 4;
            if si + 4 <= src.len() && di + 4 <= frame.len() {
                frame[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

/// Draw the drive status bar at native resolution into the bottom bar_h rows of frame.
pub fn render_status_bar(
    frame: &mut [u8], surf_w: u32, surf_h: u32, bar_h: u32,
    drives: &[DriveStatusInfo; 4], col80: bool,
) {
    let bar_y = surf_h.saturating_sub(bar_h);

    // Dark gray background
    let bg = [32u8, 32, 32, 255];
    for y in bar_y..surf_h {
        for x in 0..surf_w {
            let idx = (y * surf_w + x) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&bg);
            }
        }
    }

    // Separator line
    let sep = [64u8, 64, 64, 255];
    for x in 0..surf_w {
        let idx = (bar_y * surf_w + x) as usize * 4;
        if idx + 4 <= frame.len() {
            frame[idx..idx + 4].copy_from_slice(&sep);
        }
    }

    // Reset buttons on the left side
    let (rx, ry, rw, rh) = reset_button_rect(bar_y, bar_h);
    draw_button(frame, surf_w, rx, ry, rw, rh, b"RST", &[200, 200, 200, 255]);
    let (px2, py2, pw2, ph2) = power_button_rect(bar_y, bar_h);
    draw_button(frame, surf_w, px2, py2, pw2, ph2, b"PWR", &[255, 100, 100, 255]);
    // 80/40 column switch toggle
    let (cx, cy, cw, ch) = col_button_rect(bar_y, bar_h);
    let col_label = if col80 { b"80" as &[u8] } else { b"40" as &[u8] };
    let col_color = if col80 { [100, 200, 100, 255] } else { [200, 200, 100, 255] };
    draw_button(frame, surf_w, cx, cy, cw, ch, col_label, &col_color);

    // Drive slots in the bottom-right — 32×32 icons scaled 2× = 64×64
    // 4 drives: 2x 5.25" + 2x 3.5"
    let icon_scale = 2u32;
    let icon_dim = 32 * icon_scale; // 64px
    let toggle_gap = 8u32;
    let toggle_w = 12u32;
    let toggle_h = 40u32;
    let slot_width = icon_dim + toggle_gap + toggle_w;
    let slot_gap = 16u32;  // Reduced gap for 4 drives
    let total_slots_width = slot_width * 4 + slot_gap * 3;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);

    let masks: [&[u8; 1024]; 4] = [&*DISK1_MASK, &*DISK2_MASK, &*DISK35_1_MASK, &*DISK35_2_MASK];

    for drive in 0..4usize {
        let slot_x = start_x + drive as u32 * (slot_width + slot_gap);
        let icon_y = bar_y + (bar_h.saturating_sub(icon_dim)) / 2;

        // Dark fill behind disk icon
        let fill = [32u8, 32, 32, 255];
        for dy in 0..icon_dim {
            for dx in 0..icon_dim {
                let idx = ((icon_y + dy) * surf_w + (slot_x + dx)) as usize * 4;
                if idx + 4 <= frame.len() {
                    frame[idx..idx + 4].copy_from_slice(&fill);
                }
            }
        }

        // Disk icon (32×32 mask scaled 2×)
        let icon_color: [u8; 4] = if drives[drive].is_active {
            [120, 255, 120, 255]
        } else if drives[drive].has_disk {
            [180, 180, 180, 255]
        } else {
            [80, 80, 80, 255]
        };
        blit_mask(frame, surf_w, masks[drive], 32, 32, slot_x, icon_y, icon_scale, &icon_color);

        // Write-protect toggle switch (only when disk loaded)
        if drives[drive].has_disk {
            let tx = slot_x + icon_dim + toggle_gap;
            let ty = icon_y + (icon_dim.saturating_sub(toggle_h)) / 2;
            draw_toggle_switch(frame, surf_w, tx, ty, toggle_w, toggle_h, !drives[drive].is_write_protected);
        }
    }
}

/// Blit a single-channel luminance mask at (x, y) with integer scale, tinting non-zero
/// pixels with the given color (mask value modulates brightness).
fn blit_mask(
    frame: &mut [u8], stride: u32,
    mask: &[u8], src_w: u32, src_h: u32,
    x: u32, y: u32, scale: u32, color: &[u8; 4],
) {
    for sy in 0..src_h {
        for sx in 0..src_w {
            let v = mask[(sy * src_w + sx) as usize] as u32;
            if v == 0 { continue; }
            let r = (color[0] as u32 * v / 255) as u8;
            let g = (color[1] as u32 * v / 255) as u8;
            let b = (color[2] as u32 * v / 255) as u8;
            let px = [r, g, b, 255u8];
            for dy in 0..scale {
                for dx in 0..scale {
                    let idx = ((y + sy * scale + dy) * stride + (x + sx * scale + dx)) as usize * 4;
                    if idx + 4 <= frame.len() {
                        frame[idx..idx + 4].copy_from_slice(&px);
                    }
                }
            }
        }
    }
}

/// Draw a vertical toggle switch at (x, y) with size (w, h).
/// `on` = true means write-enabled, false = write-protected.
fn draw_toggle_switch(frame: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, on: bool) {
    let border = [80u8, 80, 80, 255];
    let track = [44u8, 44, 44, 255];
    let knob_h = h / 3;

    // Track background
    for dy in 0..h {
        for dx in 0..w {
            let on_border = dy == 0 || dy == h - 1 || dx == 0 || dx == w - 1;
            let c = if on_border { &border } else { &track };
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(c);
            }
        }
    }

    // Knob position: bottom when on (write-enabled), top when off (write-protected)
    let knob_y = if on { y + h - knob_h - 1 } else { y + 1 };
    let knob_color = if on { [220u8, 60, 60, 255] } else { [120u8, 120, 120, 255] };
    let knob_highlight = if on { [255u8, 100, 100, 255] } else { [160u8, 160, 160, 255] };

    for dy in 0..knob_h {
        for dx in 1..w - 1 {
            // Slight highlight on top row of knob
            let c = if dy == 0 { &knob_highlight } else { &knob_color };
            let idx = ((knob_y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(c);
            }
        }
    }
}

/// Returns (x, y, w, h) for the reset button.
fn reset_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let margin = 32u32;
    let btn_w = 128u32;
    let btn_h = 56u32;
    let bx = margin;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Returns (x, y, w, h) for the power/reboot button (right of RST).
fn power_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let (rx, _, rw, _) = reset_button_rect(bar_y, bar_h);
    let gap = 16u32;
    let btn_w = 128u32;
    let btn_h = 56u32;
    let bx = rx + rw + gap;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Returns (x, y, w, h) for the 80/40 column switch button (right of PWR).
fn col_button_rect(bar_y: u32, bar_h: u32) -> (u32, u32, u32, u32) {
    let (px, _, pw, _) = power_button_rect(bar_y, bar_h);
    let gap = 16u32;
    let btn_w = 96u32;
    let btn_h = 56u32;
    let bx = px + pw + gap;
    let by = bar_y + (bar_h.saturating_sub(btn_h)) / 2;
    (bx, by, btn_w, btn_h)
}

/// Draw a button at (x, y) with size (w, h) and a label using the Apple II character ROM.
fn draw_button(frame: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, label: &[u8], text_color: &[u8; 4]) {
    // Button outline
    let border = [100u8, 100, 100, 255];
    let fill = [56u8, 56, 56, 255];
    // Fill
    for dy in 1..h - 1 {
        for dx in 1..w - 1 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&fill);
            }
        }
    }
    // Top/bottom border
    for dx in 0..w {
        let t = ((y) * stride + (x + dx)) as usize * 4;
        let b = ((y + h - 1) * stride + (x + dx)) as usize * 4;
        if t + 4 <= frame.len() { frame[t..t + 4].copy_from_slice(&border); }
        if b + 4 <= frame.len() { frame[b..b + 4].copy_from_slice(&border); }
    }
    // Left/right border
    for dy in 0..h {
        let l = ((y + dy) * stride + x) as usize * 4;
        let r = ((y + dy) * stride + (x + w - 1)) as usize * 4;
        if l + 4 <= frame.len() { frame[l..l + 4].copy_from_slice(&border); }
        if r + 4 <= frame.len() { frame[r..r + 4].copy_from_slice(&border); }
    }
    // Draw label text centered using the Apple II character ROM (7×8 at 4× scale = 28×32 per char)
    let scale = 4u32;
    let char_w = 7 * scale; // 28px per char
    let char_h = 8 * scale; // 32px per char
    let gap = 4u32;          // 4px gap between chars
    let num_chars = label.len() as u32;
    let text_total_w = num_chars * char_w + num_chars.saturating_sub(1) * gap;
    let text_x = x + (w.saturating_sub(text_total_w)) / 2;
    let text_y = y + (h.saturating_sub(char_h)) / 2;
    for (i, &ch) in label.iter().enumerate() {
        draw_font_char(frame, stride, text_x + i as u32 * (char_w + gap), text_y, ch, scale, text_color);
    }
}

/// Draw a character from the Apple II character ROM at (x, y) with the given scale factor.
/// ASCII uppercase letters and digits are in the ROM at offsets 0x40-0x5F (uppercase) and
/// 0x30-0x39 (digits within 0x20-0x3F symbols range). Each char is 7 pixels wide × 8 rows.
fn draw_font_char(frame: &mut [u8], stride: u32, x: u32, y: u32, ch: u8, scale: u32, color: &[u8; 4]) {
    // Map ASCII to font ROM index (font layout: 0x00-0x1F mousetext, 0x20-0x3F symbols/digits, 0x40-0x5F uppercase)
    let font_idx = if ch.is_ascii_uppercase() {
        ch as usize  // A=0x41 etc — directly indexes uppercase block
    } else if ch.is_ascii_digit() {
        (ch - b'0' + 0x30) as usize // '0'=0x30 etc — symbols/digits block
    } else {
        0x20 // space
    };
    let rom_offset = font_idx * 8;

    for row in 0..8u32 {
        let font_byte = CHAR_ROM[rom_offset + row as usize];
        for bit in 0..7u32 {
            if (font_byte >> bit) & 1 != 0 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = x + bit * scale + sx;
                        let py = y + row * scale + sy;
                        let pi = (py * stride + px) as usize * 4;
                        if pi + 4 <= frame.len() {
                            frame[pi..pi + 4].copy_from_slice(color);
                        }
                    }
                }
            }
        }
    }
}

/// Hit-test the reset button. Returns true if (px, py) is inside it.
pub fn hit_test_reset_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (rx, ry, rw, rh) = reset_button_rect(bar_y, bar_h);
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

/// Hit-test the power/reboot button. Returns true if (px, py) is inside it.
pub fn hit_test_power_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (bx, by, bw, bh) = power_button_rect(bar_y, bar_h);
    px >= bx && px < bx + bw && py >= by && py < by + bh
}

/// Hit-test the 80/40 column switch button.
pub fn hit_test_col_button(px: u32, py: u32, surf_h: u32, bar_h: u32) -> bool {
    let bar_y = surf_h.saturating_sub(bar_h);
    let (bx, by, bw, bh) = col_button_rect(bar_y, bar_h);
    px >= bx && px < bx + bw && py >= by && py < by + bh
}

/// Hit-test drive icons in the status bar using native window coordinates.
pub fn hit_test_drive_icon(px: u32, py: u32, surf_w: u32, surf_h: u32, bar_h: u32) -> Option<usize> {
    let bar_y = surf_h.saturating_sub(bar_h);
    if py < bar_y || py >= surf_h {
        return None;
    }

    let (icon_dim, slot_width, slot_gap, start_x) = drive_slot_layout(surf_w);

    for drive in 0..4u32 {
        let slot_x = start_x + drive * (slot_width + slot_gap);
        // Hit-test the icon area only (not the toggle)
        if px >= slot_x && px < slot_x + icon_dim {
            return Some(drive as usize);
        }
    }
    None
}

/// Hit-test the write-protect toggle switch for a drive.
pub fn hit_test_write_toggle(px: u32, py: u32, surf_w: u32, surf_h: u32, bar_h: u32) -> Option<usize> {
    let bar_y = surf_h.saturating_sub(bar_h);
    if py < bar_y || py >= surf_h {
        return None;
    }

    let (icon_dim, slot_width, slot_gap, start_x) = drive_slot_layout(surf_w);
    let toggle_gap = 8u32;
    let toggle_w = 12u32;
    let toggle_h = 40u32;

    for drive in 0..4u32 {
        let slot_x = start_x + drive * (slot_width + slot_gap);
        let tx = slot_x + icon_dim + toggle_gap;
        let ty = bar_y + (bar_h.saturating_sub(icon_dim)) / 2 + (icon_dim.saturating_sub(toggle_h)) / 2;
        if px >= tx && px < tx + toggle_w && py >= ty && py < ty + toggle_h {
            return Some(drive as usize);
        }
    }
    None
}

/// Shared drive slot layout: returns (icon_dim, slot_width, slot_gap, start_x).
fn drive_slot_layout(surf_w: u32) -> (u32, u32, u32, u32) {
    let icon_dim = 32 * 2u32;
    let toggle_gap = 8u32;
    let toggle_w = 12u32;
    let slot_width = icon_dim + toggle_gap + toggle_w;
    let slot_gap = 16u32;  // Reduced for 4 drives
    let total_slots_width = slot_width * 4 + slot_gap * 3;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);
    (icon_dim, slot_width, slot_gap, start_x)
}
