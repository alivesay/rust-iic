/// Drive status info passed from the IWM to the GUI renderer.
pub struct DriveStatusInfo {
    pub has_disk: bool,
    pub is_active: bool,
    pub is_write_protected: bool,
}

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
    drives: &[DriveStatusInfo; 2], col80: bool,
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

    // Drive slots in the bottom-right (4x base scale for Retina)
    let slot_width: u32 = 136;
    let total_slots_width = slot_width * 2 + 32;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);

    for drive in 0..2usize {
        let slot_x = start_x + drive as u32 * (slot_width + 32);
        let slot_y = bar_y + (bar_h.saturating_sub(56)) / 2;

        // LED indicator (24×24)
        let led_color = if drives[drive].is_active {
            [0u8, 255, 0, 255]
        } else if drives[drive].has_disk {
            [0u8, 64, 0, 255]
        } else {
            [48u8, 48, 48, 255]
        };
        let led_x = slot_x;
        let led_y = slot_y + 16;
        for dy in 0..24u32 {
            for dx in 0..24u32 {
                let idx = ((led_y + dy) * surf_w + (led_x + dx)) as usize * 4;
                if idx + 4 <= frame.len() {
                    frame[idx..idx + 4].copy_from_slice(&led_color);
                }
            }
        }

        // Disk icon (64×56)
        let icon_x = slot_x + 40;
        let icon_y = slot_y;
        let disk_color: [u8; 4] = if drives[drive].has_disk {
            [180, 180, 180, 255]
        } else {
            [80, 80, 80, 255]
        };
        draw_disk_icon(frame, surf_w, icon_x, icon_y, &disk_color);

        // Write-protect indicator
        if drives[drive].has_disk && drives[drive].is_write_protected {
            let lock = [255u8, 80, 80, 255];
            let lx = icon_x + 4;
            let ly = icon_y + 4;
            for dy in 0..20u32 {
                for dx in 0..4u32 {
                    let idx = ((ly + dy) * surf_w + (lx + dx)) as usize * 4;
                    if idx + 4 <= frame.len() {
                        frame[idx..idx + 4].copy_from_slice(&lock);
                    }
                }
            }
            for dx in 4..12u32 {
                for dy2 in 0..4u32 {
                    let idx = ((ly + 16 + dy2) * surf_w + (lx + dx)) as usize * 4;
                    if idx + 4 <= frame.len() {
                        frame[idx..idx + 4].copy_from_slice(&lock);
                    }
                }
            }
        }

        // Drive number label
        let label_x = icon_x + 72;
        let label_y = slot_y + 18;
        let label_color = [128u8, 128, 128, 255];
        draw_tiny_digit(frame, surf_w, label_x, label_y, (drive + 1) as u8, &label_color);
    }
}

fn draw_disk_icon(frame: &mut [u8], stride: u32, x: u32, y: u32, color: &[u8; 4]) {
    let dark = [color[0] / 2, color[1] / 2, color[2] / 2, 255];
    let slot_color = [color[0] / 3, color[1] / 3, color[2] / 3, 255];

    // Body (64×56)
    for dy in 0..56u32 {
        for dx in 0..64u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(color);
            }
        }
    }
    // Top label area
    for dy in 4..20u32 {
        for dx in 12..52u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&dark);
            }
        }
    }
    // Bottom slot
    for dy in 36..52u32 {
        for dx in 16..48u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&slot_color);
            }
        }
    }
    // Metal shutter
    let shutter = [color[0].saturating_add(40), color[1].saturating_add(40), color[2].saturating_add(40), 255];
    for dy in 36..52u32 {
        for dx in 28..32u32 {
            let idx = ((y + dy) * stride + (x + dx)) as usize * 4;
            if idx + 4 <= frame.len() {
                frame[idx..idx + 4].copy_from_slice(&shutter);
            }
        }
    }
}

fn draw_tiny_digit(frame: &mut [u8], stride: u32, x: u32, y: u32, digit: u8, color: &[u8; 4]) {
    #[rustfmt::skip]
    let patterns: &[&[u8; 15]] = &[
        &[1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1], // 0
        &[0,1,0, 1,1,0, 0,1,0, 0,1,0, 1,1,1], // 1
        &[1,1,1, 0,0,1, 1,1,1, 1,0,0, 1,1,1], // 2
    ];
    let idx = match digit { 1 => 1, 2 => 2, _ => 0 };
    let pattern = patterns[idx];
    // Each logical pixel is 4×4 physical pixels for Retina
    for dy in 0..5u32 {
        for dx in 0..3u32 {
            if pattern[(dy * 3 + dx) as usize] == 1 {
                for sy in 0..4u32 {
                    for sx in 0..4u32 {
                        let pi = ((y + dy * 4 + sy) * stride + (x + dx * 4 + sx)) as usize * 4;
                        if pi + 4 <= frame.len() {
                            frame[pi..pi + 4].copy_from_slice(color);
                        }
                    }
                }
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

/// Draw a button at (x, y) with size (w, h) and a label.
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
    // Draw label text centered (3 chars × 12px wide + 4px gap = 44px, centered in 128px)
    let char_w = 16u32;
    let num_chars = label.len() as u32;
    let text_total_w = num_chars * char_w - 4; // subtract trailing gap
    let text_x = x + (w.saturating_sub(text_total_w)) / 2;
    let text_y = y + (h - 20) / 2;
    for (i, &ch) in label.iter().enumerate() {
        draw_tiny_letter(frame, stride, text_x + i as u32 * char_w, text_y, ch, text_color);
    }
}

/// Draw a 4x-scaled 3×5 letter at (x, y). Supports R, S, T, P, W, O, 0, 4, 8.
fn draw_tiny_letter(frame: &mut [u8], stride: u32, x: u32, y: u32, ch: u8, color: &[u8; 4]) {
    #[rustfmt::skip]
    let pattern: &[u8; 15] = match ch {
        b'R' => &[1,1,0, 1,0,1, 1,1,0, 1,0,1, 1,0,1],
        b'S' => &[0,1,1, 1,0,0, 0,1,0, 0,0,1, 1,1,0],
        b'T' => &[1,1,1, 0,1,0, 0,1,0, 0,1,0, 0,1,0],
        b'P' => &[1,1,0, 1,0,1, 1,1,0, 1,0,0, 1,0,0],
        b'W' => &[1,0,1, 1,0,1, 1,0,1, 1,1,1, 1,0,1],
        b'0' | b'O' => &[1,1,1, 1,0,1, 1,0,1, 1,0,1, 1,1,1],
        b'4' => &[1,0,1, 1,0,1, 1,1,1, 0,0,1, 0,0,1],
        b'8' => &[1,1,1, 1,0,1, 1,1,1, 1,0,1, 1,1,1],
        _    => &[0,0,0, 0,0,0, 0,0,0, 0,0,0, 0,0,0],
    };
    for dy in 0..5u32 {
        for dx in 0..3u32 {
            if pattern[(dy * 3 + dx) as usize] == 1 {
                for sy in 0..4u32 {
                    for sx in 0..4u32 {
                        let pi = ((y + dy * 4 + sy) * stride + (x + dx * 4 + sx)) as usize * 4;
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

    let slot_width: u32 = 136;
    let total_slots_width = slot_width * 2 + 32;
    let start_x = surf_w.saturating_sub(total_slots_width + 32);

    for drive in 0..2u32 {
        let slot_x = start_x + drive * (slot_width + 32);
        if px >= slot_x && px < slot_x + slot_width {
            return Some(drive as usize);
        }
    }
    None
}
