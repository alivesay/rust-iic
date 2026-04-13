use std::cell::Cell;

use crate::{iou::IOU, mmu::MMU, util::apple_iic_font_index};

const CHAR_ROM: &[u8; 1024] = include_bytes!("../assets/font.bin");

pub const TEXT_MODE_BASE_ADDRESSES: [u16; 24] = [
    0x0400, 0x0480, 0x0500, 0x0580, 0x0600, 0x0680, 0x0700, 0x0780, 0x0428, 0x04A8, 0x0528, 0x05A8,
    0x0628, 0x06A8, 0x0728, 0x07A8, 0x0450, 0x04D0, 0x0550, 0x05D0, 0x0650, 0x06D0, 0x0750, 0x07D0,
];

pub struct VideoModeMask;
#[rustfmt::skip]
impl VideoModeMask {
    pub const TEXT: u8     = 0b0000_0001; // Text mode active
    pub const LORES: u8    = 0b0000_0010; // Lo-Res graphics
    pub const HIRES: u8    = 0b0000_0100; // Hi-Res graphics
    pub const DHIRES: u8   = 0b0000_1000; // Double Hi-Res mode (80-Col required)
    pub const MIXED: u8    = 0b0001_0000; // Mixed mode (text+graphics)
    pub const PAGE2: u8    = 0b0010_0000; // Page 2 mode (ALT screen buffer)
    pub const COL80: u8    = 0b0100_0000; // 80-column mode
    pub const ALTCHAR: u8  = 0b1000_0000; // Alternate Character Set
}

pub struct VideoMode;
#[rustfmt::skip]
#[allow(dead_code)]
impl VideoMode {
    pub const TEXT: u8       = VideoModeMask::TEXT;
    pub const LORES: u8      = VideoModeMask::LORES;
    pub const HIRES: u8      = VideoModeMask::HIRES;
    pub const DHIRES: u8     = VideoModeMask::DHIRES | VideoModeMask::COL80; // DHiRes requires 80-Col
    pub const MIXED_TEXT: u8 = VideoModeMask::TEXT | VideoModeMask::MIXED;
    pub const MIXED_HIRES: u8 = VideoModeMask::HIRES | VideoModeMask::MIXED;
    pub const MIXED_DHIRES: u8 = VideoModeMask::DHIRES | VideoModeMask::MIXED;
    pub const LORES_PAGE2: u8 = VideoModeMask::LORES | VideoModeMask::PAGE2;
    pub const HIRES_PAGE2: u8 = VideoModeMask::HIRES | VideoModeMask::PAGE2;
}

// macro_rules! set_video_mode {
//     ($video_state:expr, $mode:expr) => {{
//         let current = $video_state.get();
//         $video_state.set(current | $mode);
//         0x00
//     }};
// }

pub struct Video {
    framebuffer: Vec<u8>, // RGBA
    width: usize,
    height: usize,
    // Active area dimensions (without border)
    active_width: usize,
    active_height: usize,
    //  video_mode: Cell<u8>,
    extra: Cell<u8>,
    frame_count: usize,
    pub monochrome: bool,
    pub crt_enabled: bool,
    pub scanline_intensity: f32, // 0.0 = full black gap, 1.0 = no scanlines
    pub border_size: usize,     // Black border in pixels around active area
}

impl Video {
    pub fn new() -> Self {
        let border = 16;
        let active_width = 560;
        let active_height = 192;
        let width = active_width + border * 2;
        let height = active_height + border * 2;
        Self {
            framebuffer: vec![0; width * height * 4],
            width,
            height,
            active_width,
            active_height,
            //  video_mode: Cell::new(VideoMode::TEXT),
            extra: Cell::new(0),
            frame_count: 0,
            monochrome: false,
            crt_enabled: false,
            scanline_intensity: 0.5,
            border_size: border,
        }
    }

    pub fn set_monochrome(&mut self, enabled: bool) {
        self.monochrome = enabled;
    }

    #[allow(dead_code)]
    fn get_display_address(&self, video_mode: u8, is_80store: bool, addr: u16) -> u16 {
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80col = check_bits_u8!(video_mode, VideoModeMask::COL80);
        let is_dhires = check_bits_u8!(video_mode, VideoModeMask::DHIRES);

        match addr {
            0x0400..=0x07FF => {
                if is_80store || is_page2 {
                    addr.wrapping_add(0x0400) // Page2 offset
                } else {
                    addr
                }
            }
            0x0800..=0x0BFF => addr,
            0x2000..=0x3FFF => {
                if is_80store || is_page2 {
                    addr.wrapping_add(0x2000)
                } else {
                    addr
                }
            }
            _ if is_dhires && addr & 0x200 != 0 => addr.wrapping_add(0x200),
            _ if is_80col && (0x0400..=0x07FF).contains(&addr) => {
                let col = (addr.wrapping_sub(0x0400)) % 40;
                if col % 2 == 0 {
                    addr
                } else {
                    addr.wrapping_add(0x200)
                }
            }
            _ => addr,
        }
    }

    pub fn update(&mut self, iou: &IOU, mmu: &MMU) -> bool {
        self.frame_count = self.frame_count.wrapping_add(1);
        
        // clear
        self.framebuffer.fill(0);

        let video_mode = iou.video_mode.get();
        
        if self.extra.get() != video_mode {
             self.extra.set(video_mode);
        }

        let _is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80col = check_bits_u8!(video_mode, VideoModeMask::COL80);
        let is_dhires = check_bits_u8!(video_mode, VideoModeMask::DHIRES);
        let lo_res_mode = check_bits_u8!(video_mode, VideoModeMask::LORES);
        let is_hires = check_bits_u8!(video_mode, VideoModeMask::HIRES);
        let mixed_mode = check_bits_u8!(video_mode, VideoModeMask::MIXED);
        let text_mode = check_bits_u8!(video_mode, VideoModeMask::TEXT);
        let _is_80store: bool = iou.is_80store.get();

        let new_active_width = 560;
        let new_active_height = 384;
        let new_width = new_active_width + self.border_size * 2;
        let new_height = new_active_height + self.border_size * 2;

        if new_width != self.width || new_height != self.height {
            self.active_width = new_active_width;
            self.active_height = new_active_height;
            self.resize_framebuffer(new_width, new_height);
        }

        if text_mode {
            self.render_text_mode(iou, mmu);
        } else if is_hires {
            if is_dhires && is_80col {
                self.render_double_hires_mode(iou, mmu);
            } else {
                self.render_hires_mode(iou, mmu);
            }
            if mixed_mode {
                self.render_text_mode_overlay(iou, mmu);
            }
        } else if lo_res_mode {
            self.render_lores_mode(iou, mmu);
            if mixed_mode {
                self.render_text_mode_overlay(iou, mmu);
            }
        } else {
            self.render_text_mode(iou, mmu);
        }

        // Apply scanline effect: dim every odd row (the 2nd row of each doubled pair)
        if self.scanline_intensity < 1.0 {
            self.apply_scanlines();
        }

        true
    }

    fn resize_framebuffer(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.framebuffer = vec![0; new_width * new_height * 4];
    }

    /// Convert active-area (x, y) to framebuffer byte index, accounting for border
    #[inline(always)]
    fn fb_index(&self, x: usize, y: usize) -> usize {
        ((y + self.border_size) * self.width + (x + self.border_size)) * 4
    }

    fn apply_scanlines(&mut self) {
        let intensity = self.scanline_intensity.clamp(0.0, 1.0);
        // Dim every odd row (row 1, 3, 5, ...) within the active area
        for y in (1..self.active_height).step_by(2) {
            let abs_y = y + self.border_size;
            let row_start = (abs_y * self.width + self.border_size) * 4;
            let row_end = row_start + self.active_width * 4;
            if row_end <= self.framebuffer.len() {
                for i in (row_start..row_end).step_by(4) {
                    self.framebuffer[i]     = (self.framebuffer[i]     as f32 * intensity) as u8;
                    self.framebuffer[i + 1] = (self.framebuffer[i + 1] as f32 * intensity) as u8;
                    self.framebuffer[i + 2] = (self.framebuffer[i + 2] as f32 * intensity) as u8;
                }
            }
        }
    }



    fn read_hires_memory(&self, iou: &IOU, mmu: &MMU, addr: u16) -> u8 {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        if is_80store {
             let real_addr = addr.wrapping_add(0x2000);
             if is_page2 {
                 mmu.read_aux_byte(real_addr)
             } else {
                 mmu.read_main_byte(real_addr)
             }
        } else {
             if is_page2 {
                 mmu.read_main_byte(addr.wrapping_add(0x4000))
             } else {
                 mmu.read_main_byte(addr.wrapping_add(0x2000))
             }
        }
    }
   
    pub fn render_text_mode(&mut self, iou: &IOU, mmu: &MMU) {
        self.render_text_rows(iou, mmu, 0..24);
    }

    #[allow(dead_code)]
    pub fn render_text_mode_overlay(&mut self, iou: &IOU, mmu: &MMU) {
        self.render_text_rows(iou, mmu, 20..24);
        // In mixed mode, color burst stays active on text lines,
        // so text shows NTSC color fringing on white→black edges.
        if !self.monochrome {
            self.apply_mixed_mode_text_fringing(20);
        }
    }

    fn render_text_rows(&mut self, iou: &IOU, mmu: &MMU, rows: std::ops::Range<u16>) {
        let video_mode = iou.video_mode.get();
        let is_80col = check_bits_u8!(video_mode, VideoModeMask::COL80);
        let is_altchar = check_bits_u8!(video_mode, VideoModeMask::ALTCHAR);
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        // In 80-column mode, we draw single-width characters (7 pixels).
        // In 40-column mode, we draw double-width characters (14 pixels).
        let double_width = !is_80col;

        for row in rows {
            let row_base = TEXT_MODE_BASE_ADDRESSES[row as usize];

            if is_80col {
                for col_pair in 0..40_u16 {
                    let addr = row_base + col_pair;
                    
                    // Even column (0, 2, 4...) -> AUX Memory
                    let char_even = mmu.read_aux_byte(addr);
                    self.draw_char(row, col_pair * 2, char_even, is_altchar, double_width);

                    // Odd column (1, 3, 5...) -> MAIN Memory
                    let char_odd = mmu.read_main_byte(addr);
                    self.draw_char(row, col_pair * 2 + 1, char_odd, is_altchar, double_width);
                }
            } else {
                for col in 0..40_u16 {
                    // Handle Page 2 offset if 80STORE is OFF
                    let (effective_addr, use_aux) = if !is_80store && is_page2 {
                        (row_base + 0x0400 + col, false)
                    } else if is_80store && is_page2 {
                        (row_base + col, true)
                    } else {
                        (row_base + col, false)
                    };

                    let vram_code = if use_aux {
                        mmu.read_aux_byte(effective_addr)
                    } else {
                        mmu.read_main_byte(effective_addr)
                    };
                    self.draw_char(row, col, vram_code, is_altchar, double_width);
                }
            }
        }
    }

    fn draw_char(&mut self, row: u16, col: u16, char_code: u8, is_altchar: bool, double_width: bool) {
        let mut vram_code = char_code;
        // 0x00 as 0xA0 (blank space)
        if vram_code == 0x00 {
            vram_code = 0xA0;
        }

        let (font_offset, mut invert) = apple_iic_font_index(vram_code, is_altchar);

        // Handle Hardware Flashing
        // Flashing characters are mapped to Inverse by apple_iic_font_index.
        // We toggle the 'invert' flag based on frame count to simulate flashing.
        // Flashing range in VRAM: 0x40-0x7F (when not in AltChar/MouseText mode)
        if !is_altchar && (vram_code >= 0x40 && vram_code <= 0x7F) {
            // Flash rate: approx 2Hz. 60fps / 32 = ~1.8Hz
            let flash_on = (self.frame_count / 16) % 2 == 0;
            if !flash_on {
                invert = false; // Render as Normal
            }
        }

        let char_width = if double_width { 14 } else { 7 };

        for char_row in 0..8_u16 {
            let mut font_byte = CHAR_ROM[font_offset + char_row as usize];
            
            if invert {
                font_byte = !font_byte;
            }

            // Scale Y by 2 because we are now using 384 height for everything
            let y = (row * 8 + char_row) * 2;
            let x = col * char_width; 

            let mut rgba_row = [0u8; 14 * 4]; // Max width 14

            for bit in 0..7 {
                let pixel_on = (font_byte >> bit) & 1 != 0;
                let (r, g, b) = if pixel_on {
                    if self.monochrome {
                        (0, 255, 0)
                    } else {
                        (255, 255, 255)
                    }
                } else {
                    (0, 0, 0)
                };
                // if !pixel_on { continue; }
                // let color = 255;

                if double_width {
                    // Draw 2 pixels for each font bit
                    let base_index = bit * 8; // bit * 2 pixels * 4 bytes
                    
                    // Pixel 1
                    rgba_row[base_index] = r;
                    rgba_row[base_index + 1] = g;
                    rgba_row[base_index + 2] = b;
                    rgba_row[base_index + 3] = 255;

                    // Pixel 2
                    rgba_row[base_index + 4] = r;
                    rgba_row[base_index + 5] = g;
                    rgba_row[base_index + 6] = b;
                    rgba_row[base_index + 7] = 255;
                } else {
                    // Draw 1 pixel for each font bit
                    let base_index = bit * 4; // bit * 1 pixel * 4 bytes
                    
                    rgba_row[base_index] = r;
                    rgba_row[base_index + 1] = g;
                    rgba_row[base_index + 2] = b;
                    rgba_row[base_index + 3] = 255;
                }
            }

            // Draw 2 rows for each font row (vertical scaling is always 2x)
            for dy in 0..2 {
                let start_index = self.fb_index(x as usize, y as usize + dy);
                let end_index = start_index + (char_width as usize) * 4;

                if end_index <= self.framebuffer.len() {
                    self.framebuffer[start_index..end_index].copy_from_slice(&rgba_row[0..(char_width as usize * 4)]);
                }
            }
        }
    }

    /// Apply subtle NTSC color fringing to text rows in mixed mode.
    /// In mixed mode the color burst is active, so white pixels show
    /// artifact color at transitions to black, just like hires pixels.
    fn apply_mixed_mode_text_fringing(&mut self, start_text_row: usize) {
        let fringe_strength: u16 = 60; // subtle — about 24% intensity
        // Scan the text area: rows start_text_row..24, each 8 font rows * 2 (doubled)
        let y_start = start_text_row * 8 * 2;
        let y_end = 24 * 8 * 2;

        for y in (y_start..y_end).step_by(2) {
            for x in 1..self.active_width - 1 {
                let idx = self.fb_index(x, y);
                if idx + 4 > self.framebuffer.len() { continue; }

                let r = self.framebuffer[idx] as u16;
                let g = self.framebuffer[idx + 1] as u16;
                let b = self.framebuffer[idx + 2] as u16;
                let is_bright = r + g + b > 400; // white-ish

                if !is_bright { continue; }

                // Check right neighbor — if it's dark, add right fringe
                let ridx = self.fb_index(x + 1, y);
                if ridx + 4 <= self.framebuffer.len() {
                    let rr = self.framebuffer[ridx] as u16;
                    let rg = self.framebuffer[ridx + 1] as u16;
                    let rb = self.framebuffer[ridx + 2] as u16;
                    if rr + rg + rb < 100 {
                        // Fringe color depends on pixel phase (x position mod 4)
                        let fringe = match x % 4 {
                            0 => [0, fringe_strength, 0],           // green tint
                            1 => [fringe_strength, 0, fringe_strength], // violet tint
                            2 => [0, 0, fringe_strength],           // blue tint
                            _ => [fringe_strength, fringe_strength / 2, 0], // orange tint
                        };
                        for dy in 0..2 {
                            let fi = self.fb_index(x + 1, y + dy);
                            if fi + 4 <= self.framebuffer.len() {
                                self.framebuffer[fi] = fringe[0] as u8;
                                self.framebuffer[fi + 1] = fringe[1] as u8;
                                self.framebuffer[fi + 2] = fringe[2] as u8;
                            }
                        }
                    }
                }

                // Check left neighbor — if it's dark, add left fringe
                let lidx = self.fb_index(x - 1, y);
                if lidx + 4 <= self.framebuffer.len() {
                    let lr = self.framebuffer[lidx] as u16;
                    let lg = self.framebuffer[lidx + 1] as u16;
                    let lb = self.framebuffer[lidx + 2] as u16;
                    if lr + lg + lb < 100 {
                        let fringe = match (x - 1) % 4 {
                            0 => [0, fringe_strength, 0],
                            1 => [fringe_strength, 0, fringe_strength],
                            2 => [0, 0, fringe_strength],
                            _ => [fringe_strength, fringe_strength / 2, 0],
                        };
                        for dy in 0..2 {
                            let fi = self.fb_index(x - 1, y + dy);
                            if fi + 4 <= self.framebuffer.len() {
                                self.framebuffer[fi] = fringe[0] as u8;
                                self.framebuffer[fi + 1] = fringe[1] as u8;
                                self.framebuffer[fi + 2] = fringe[2] as u8;
                            }
                        }
                    }
                }
            }
        }
    }

    fn render_lores_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80col = check_bits_u8!(video_mode, VideoModeMask::COL80);
        let is_dhires = check_bits_u8!(video_mode, VideoModeMask::DHIRES);
        let is_double_lores = is_80col && is_dhires;
        let is_80store = iou.is_80store.get();
        let mixed_mode = check_bits_u8!(video_mode, VideoModeMask::MIXED);

        let base_vram: u16 = if !is_80store && is_page2 { 0x0800 } else { 0x0400 };

        // if Mixed Mode is ON, only draw the top 20 blocks (40 half-rows)
        let max_row = if mixed_mode { 40 } else { 48 };

        for row in 0..max_row {
            let base_address = base_vram
                + match row / 2 {
                    0 => 0x000,
                    1 => 0x080,
                    2 => 0x100,
                    3 => 0x180,
                    4 => 0x200,
                    5 => 0x280,
                    6 => 0x300,
                    7 => 0x380,
                    8 => 0x028,
                    9 => 0x0A8,
                    10 => 0x128,
                    11 => 0x1A8,
                    12 => 0x228,
                    13 => 0x2A8,
                    14 => 0x328,
                    15 => 0x3A8,
                    16 => 0x050,
                    17 => 0x0D0,
                    18 => 0x150,
                    19 => 0x1D0,
                    20 => 0x250,
                    21 => 0x2D0,
                    22 => 0x350,
                    23 => 0x3D0,
                    _ => unreachable!(),
                };

            if is_double_lores {
                for col in 0..80_u16 {
                    let mem_addr = base_address + (col / 2);
                    let is_aux = (col % 2) == 0;

                    let color_byte = if is_aux {
                        mmu.read_aux_byte(mem_addr)
                    } else {
                        mmu.read_main_byte(mem_addr)
                    };

                    let nibble = if row % 2 == 0 {
                        color_byte & 0x0F
                    } else {
                        (color_byte >> 4) & 0x0F
                    };

                    let color_code = if is_aux {
                        (nibble << 1 | nibble >> 3) & 0x0F
                    } else {
                        nibble
                    };

                    let color = self.lores_color_lookup(color_code);

                    let x = col * 7;
                    let y = row * 8;
                    
                    for dy in 0..8 {
                        for dx in 0..7 {
                            let index = self.fb_index(x as usize + dx as usize, y as usize + dy as usize);
                            if index + 4 <= self.framebuffer.len() {
                                self.framebuffer[index..index + 4].copy_from_slice(&color);
                            }
                        }
                    }
                }
            } else {
                for col in 0..40_u16 {
                    let addr = base_address + col;
                    
                    let use_aux = is_80store && is_page2;
                    let color_byte = if use_aux {
                        mmu.read_aux_byte(addr)
                    } else {
                        mmu.read_main_byte(addr)
                    };

                    let color_code = if row % 2 == 0 {
                        color_byte & 0x0F
                    } else {
                        (color_byte >> 4) & 0x0F
                    };

                    let color = self.lores_color_lookup(color_code);

                    let x = col * 14;
                    let y = row * 8;
                    
                    for dy in 0..8 {
                        for dx in 0..14 {
                            let index = self.fb_index(x as usize + dx as usize, y as usize + dy as usize);
                            if index + 4 <= self.framebuffer.len() {
                                self.framebuffer[index..index + 4].copy_from_slice(&color);
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    fn render_mixed_mode(&mut self, iou: &IOU, mmu: &MMU) {
        self.render_hires_mode(iou, mmu);
        self.render_text_mode(iou, mmu);
    }

    #[allow(dead_code)]
    fn render_hires_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let base_vram: u16 = 0x0000;

        for group in 0..24_u16 {
            for row in 0..8_u16 {
                // Apple II Hi-Res Interleaving:
                // Row Offset = (row * 1024) + (group % 8) * 128 + (group / 8) * 40
                let row_base = base_vram
                    .wrapping_add(row.wrapping_mul(1024))
                    .wrapping_add((group % 8).wrapping_mul(128))
                    .wrapping_add((group / 8).wrapping_mul(40));

                for col in 0..40_u16 {
                    let addr = row_base.wrapping_add(col);
                    let byte = self.read_hires_memory(iou, mmu, addr);
                    let palette_flag = (byte & 0x80) != 0;

                    // read adjacent bytes for artifacting
                    let prev_byte = if col > 0 {
                        self.read_hires_memory(iou, mmu, addr - 1)
                    } else {
                        0
                    };
                    
                    let next_byte = if col < 39 {
                        self.read_hires_memory(iou, mmu, addr + 1)
                    } else {
                        0
                    };

                    for bit in 0..7_u16 {
                        let pixel_on = (byte >> bit) & 1 != 0;
                        let x_logical = (col as usize) * 7 + (bit as usize);
                        let mut x = x_logical * 2; // scale horizontally to 560 width
                        
                        // shift by half-pixel (1 unit in 560 mode) if palette bit is set
                        if palette_flag {
                            x += 1;
                        }

                        let y = ((group as usize) * 8 + (row as usize)) * 2; // double height

                        // check neighbors for artifacting (White = adjacent pixels on)
                        let prev_on = if bit > 0 {
                            (byte >> (bit - 1)) & 1 != 0
                        } else {
                            (prev_byte >> 6) & 1 != 0
                        };
                        
                        let next_on = if bit < 6 {
                            (byte >> (bit + 1)) & 1 != 0
                        } else {
                            (next_byte >> 0) & 1 != 0
                        };

                        // Apple II Hi-Res Color Logic
                        if pixel_on {
                            if self.monochrome {
                                let color = [0, 255, 0, 255];
                                for dy in 0..2 {
                                    for dx in 0..2_usize {
                                        if x + dx < self.active_width {
                                            let index = self.fb_index(x + dx, y + dy);
                                            if index + 4 <= self.framebuffer.len() {
                                                self.framebuffer[index..index + 4]
                                                    .copy_from_slice(&color);
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Artifact color for this pixel's phase position
                                // On real hardware, an ON pixel has full white luma;
                                // NTSC decoding adds color to that brightness.
                                // These must be bright — the pixel IS on, color is overlaid.
                                let artifact_color = if palette_flag {
                                    if x_logical % 2 == 0 { [100, 140, 255, 255] }   // Blue (Even)
                                    else { [255, 180, 80, 255] }             // Orange (Odd)
                                } else {
                                    if x_logical % 2 == 0 { [180, 100, 255, 255] }   // Violet (Even)
                                    else { [100, 255, 100, 255] }            // Green (Odd)
                                };

                                let is_white = prev_on || next_on;
                                let color = if is_white {
                                    [240, 240, 240, 255]
                                } else {
                                    artifact_color
                                };

                                // Draw pixel at standard 2x2 size
                                for dy in 0..2 {
                                    for dx in 0..2_usize {
                                        if x + dx < self.active_width {
                                            let index = self.fb_index(x + dx, y + dy);
                                            if index + 4 <= self.framebuffer.len() {
                                                self.framebuffer[index..index + 4]
                                                    .copy_from_slice(&color);
                                            }
                                        }
                                    }
                                }

                                // Color fringing at edges of white runs:
                                // Subtle NTSC chroma bleed — blend artifact color with black
                                // at ~40% intensity (real composite signal decay, not full color)
                                if is_white {
                                    let fringe = [
                                        (artifact_color[0] as u16 * 100 / 255) as u8,
                                        (artifact_color[1] as u16 * 100 / 255) as u8,
                                        (artifact_color[2] as u16 * 100 / 255) as u8,
                                        255,
                                    ];
                                    if !prev_on && x > 0 {
                                        let fx = x - 1;
                                        for dy in 0..2 {
                                            if fx < self.active_width {
                                                let index = self.fb_index(fx, y + dy);
                                                if index + 4 <= self.framebuffer.len() {
                                                    self.framebuffer[index..index + 4]
                                                        .copy_from_slice(&fringe);
                                                }
                                            }
                                        }
                                    }
                                    if !next_on {
                                        let fx = x + 2;
                                        for dy in 0..2 {
                                            if fx < self.active_width {
                                                let index = self.fb_index(fx, y + dy);
                                                if index + 4 <= self.framebuffer.len() {
                                                    self.framebuffer[index..index + 4]
                                                        .copy_from_slice(&fringe);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // if pixel is OFF, do nothing
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    fn render_double_hires_mode(&mut self, _iou: &IOU, mmu: &MMU) {
        let base_vram: u16 = 0x2000;

        for group in 0..24_u16 {
            for row in 0..8_u16 {
                let row_base = base_vram
                        .wrapping_add(row.wrapping_mul(1024))
                        .wrapping_add((group % 8).wrapping_mul(128))
                        .wrapping_add((group / 8).wrapping_mul(40));

                let y = (group * 8 + row) * 2; // double height

                if self.monochrome {
                    // Monochrome: 560 pixels (1 bit = 1 pixel)
                    for col in 0..40_u16 {
                        let addr = row_base.wrapping_add(col);
                        let aux_byte = mmu.read_aux_byte(addr);
                        let main_byte = mmu.read_main_byte(addr);

                        for bit in 0..7_u16 {
                            let pixel_on = (aux_byte >> bit) & 1 != 0;
                            let color = if pixel_on { [0, 255, 0, 255] } else { [0, 0, 0, 255] };
                            let x = col as usize * 14 + bit as usize;
                            for dy in 0..2 {
                                let index = self.fb_index(x, y as usize + dy);
                                if index + 4 <= self.framebuffer.len() {
                                    self.framebuffer[index..index + 4].copy_from_slice(&color);
                                }
                            }
                        }
                        for bit in 0..7_u16 {
                            let pixel_on = (main_byte >> bit) & 1 != 0;
                            let color = if pixel_on { [0, 255, 0, 255] } else { [0, 0, 0, 255] };
                            let x = col as usize * 14 + 7 + bit as usize;
                            for dy in 0..2 {
                                let index = self.fb_index(x, y as usize + dy);
                                if index + 4 <= self.framebuffer.len() {
                                    self.framebuffer[index..index + 4].copy_from_slice(&color);
                                }
                            }
                        }
                    }
                } else {
                    // Color: 140 pixels (4-bit nibbles from sliding window across aux/main pairs)
                    // Process 2 columns at a time: 4 bytes = 28 bits = 7 color pixels
                    for col_pair in 0..20_u16 {
                        let col0 = col_pair * 2;
                        let col1 = col0 + 1;

                        let aux0 = mmu.read_aux_byte(row_base.wrapping_add(col0)) as u32;
                        let main0 = mmu.read_main_byte(row_base.wrapping_add(col0)) as u32;
                        let aux1 = mmu.read_aux_byte(row_base.wrapping_add(col1)) as u32;
                        let main1 = mmu.read_main_byte(row_base.wrapping_add(col1)) as u32;

                        // Build 28-bit stream: aux0[0:6], main0[0:6], aux1[0:6], main1[0:6]
                        let bits = (aux0 & 0x7F)
                                 | ((main0 & 0x7F) << 7)
                                 | ((aux1 & 0x7F) << 14)
                                 | ((main1 & 0x7F) << 21);

                        // Extract 7 nibbles, each is a 4-bit color index
                        for nib in 0..7_u32 {
                            let color_idx = ((bits >> (nib * 4)) & 0x0F) as u8;
                            let rgba = self.lores_color_lookup(color_idx);

                            let base_x = (col_pair as usize * 7 + nib as usize) * 4;
                            for px in 0..4 {
                                let x = base_x + px;
                                for dy in 0..2 {
                                    let index = self.fb_index(x, y as usize + dy);
                                    if index + 4 <= self.framebuffer.len() {
                                        self.framebuffer[index..index + 4].copy_from_slice(&rgba);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }



    pub fn get_dimensions(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    pub fn get_pixels(&self) -> &[u8] {
        &self.framebuffer
    }

    fn lores_color_lookup(&self, color: u8) -> [u8; 4] {
        let rgba = match color & 0x0F {
            0x0 => [32, 8, 32, 255],       // Black
            0x1 => [128, 34, 34, 255],     // Deep Red
            0x2 => [34, 34, 128, 255],     // Dark Blue
            0x3 => [73, 0, 128, 255],      // Purple
            0x4 => [39, 84, 18, 255],      // Dark Green
            0x5 => [99, 99, 99, 255],      // Gray 1
            0x6 => [64, 99, 255, 255],     // Medium Blue
            0x7 => [74, 219, 255, 255],    // Light Blue
            0x8 => [123, 69, 19, 255],     // Brown
            0x9 => [255, 140, 0, 255],     // Orange
            0xA => [129, 129, 129, 255],   // Gray 2
            0xB => [248, 126, 252, 255],   // Pink
            0xC => [34, 255, 34, 255],     // Green
            0xD => [255, 255, 34, 255],    // Yellow
            0xE => [173, 255, 241, 255],   // Aqua
            0xF => [240, 240, 240, 255],   // White
            _ => [0, 0, 0, 255],
        };

        if self.monochrome {
            let y = (0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32) as u8;
            [0, y, 0, 255]
        } else {
            rgba
        }
    }
}
