use crate::{iou::IOU, mmu::MMU, util::apple_iic_font_index};

const CHAR_ROM: &[u8; 1024] = include_bytes!("../assets/font.bin");

const MONO_GREEN: (u8, u8, u8) = (118, 247, 211);
const MONO_GREEN_RGBA: [u8; 4] = [MONO_GREEN.0, MONO_GREEN.1, MONO_GREEN.2, 255];
const MONO_BLACK_RGBA: [u8; 4] = [15, 23, 23, 255];

/// NTSC 16-color palette
/// Source: R. Munafo (mrob.com/pub/xapple2/colors.html)
#[rustfmt::skip]
const NTSC_PALETTE: [[u8; 4]; 16] = [
    [  0,   0,   0, 255], // 0x0: Black
    [227,  30,  96, 255], // 0x1: Deep Red       (phase  90°, chroma 60)
    [ 96,  78, 189, 255], // 0x2: Dark Blue      (phase   0°, chroma 60)
    [255,  68, 253, 255], // 0x3: Purple/Violet  (phase  45°, chroma 100) HiRes: Violet
    [  0, 163,  96, 255], // 0x4: Dark Green     (phase 270°, chroma 60)
    [156, 156, 156, 255], // 0x5: Gray 1
    [  0, 180, 255, 255], // 0x6: Medium Blue    (phase 315°, chroma 100) HiRes: Blue
    [208, 195, 255, 255], // 0x7: Light Blue     (phase   0°, chroma 60)
    [ 96, 114,   3, 255], // 0x8: Brown          (phase 180°, chroma 60)
    [255, 106,  60, 255], // 0x9: Orange         (phase 135°, chroma 100) HiRes: Orange
    [156, 156, 156, 255], // 0xA: Gray 2
    [255, 160, 208, 255], // 0xB: Pink           (phase  90°, chroma 60)
    [ 20, 245,  60, 255], // 0xC: Light Green    (phase 225°, chroma 100) HiRes: Green
    [210, 212,  26, 255], // 0xD: Yellow         (phase 180°, chroma 60)
    [114, 255, 208, 255], // 0xE: Aqua           (phase 270°, chroma 60)
    [255, 255, 255, 255], // 0xF: White
];

/// DHIRES extracts 4-bit color from a sliding window across the 560-bit scanline.
/// The bit-to-nibble phase rotation means nibble values map to different colors
/// than the LoRes palette: entries 2<-8, 3<-9, 6<-C, 7<-D are swapped.
/// Source: mii_emu (buserror/mii_emu) DHIRES CLUT
#[rustfmt::skip]
const DHIRES_PALETTE: [[u8; 4]; 16] = [
    [  0,   0,   0, 255], // 0x0: Black
    [227,  30,  96, 255], // 0x1: Magenta/Deep Red
    [ 96, 114,   3, 255], // 0x2: Brown          (LoRes 0x8)
    [255, 106,  60, 255], // 0x3: Orange         (LoRes 0x9)
    [  0, 163,  96, 255], // 0x4: Dark Green
    [156, 156, 156, 255], // 0x5: Gray 1
    [ 20, 245,  60, 255], // 0x6: Green          (LoRes 0xC)
    [210, 212,  26, 255], // 0x7: Yellow         (LoRes 0xD)
    [ 96,  78, 189, 255], // 0x8: Dark Blue      (LoRes 0x2)
    [255,  68, 253, 255], // 0x9: Purple         (LoRes 0x3)
    [156, 156, 156, 255], // 0xA: Gray 2
    [255, 160, 208, 255], // 0xB: Pink
    [  0, 180, 255, 255], // 0xC: Blue           (LoRes 0x6)
    [208, 195, 255, 255], // 0xD: Light Blue     (LoRes 0x7)
    [114, 255, 208, 255], // 0xE: Aqua
    [255, 255, 255, 255], // 0xF: White
];

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


pub struct Video {
    framebuffer: Vec<u8>,
    width: usize,
    height: usize,
    active_width: usize,
    active_height: usize,
    frame_count: usize,
    pub monochrome: bool,
    pub shader_enabled: bool,
    pub scanline_intensity: f32,
    pub border_size: usize,

    scanline_modes: [u8; 192],
    scanline_80store: [bool; 192],
    scanline_count: usize,
}

impl Video {
    pub fn new() -> Self {
        let border = 16;
        let active_width = 560;
        let active_height = 384;
        let width = active_width + border * 2;
        let height = active_height + border * 2;
        let mut framebuffer = vec![0u8; width * height * 4];
        for i in (3..framebuffer.len()).step_by(4) {
            framebuffer[i] = 255;
        }
        Self {
            framebuffer,
            width,
            height,
            active_width,
            active_height,
            frame_count: 0,
            monochrome: false,
            shader_enabled: false,
            scanline_intensity: 0.15,
            border_size: border,
            scanline_modes: [0; 192],
            scanline_80store: [false; 192],
            scanline_count: 0,
        }
    }

    pub fn snapshot_scanline(&mut self, scanline: usize, video_mode: u8, is_80store: bool) {
        if scanline < 192 {
            self.scanline_modes[scanline] = video_mode;
            self.scanline_80store[scanline] = is_80store;
            if scanline >= self.scanline_count {
                self.scanline_count = scanline + 1;
            }
        }
    }

    pub fn begin_frame(&mut self) {
        self.scanline_count = 0;
    }

    pub fn set_monochrome(&mut self, enabled: bool) {
        self.monochrome = enabled;
    }

    pub fn update(&mut self, iou: &IOU, mmu: &MMU) -> bool {
        self.frame_count = self.frame_count.wrapping_add(1);
        
        // clear
        self.framebuffer.fill(0);

        let video_mode = iou.video_mode.get();

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

        let is_graphics = !text_mode && (is_hires || lo_res_mode);

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

        if is_graphics && !self.monochrome {
            self.apply_chroma_blur(0, 192 * 2);
            self.apply_comb_filter();
        }

        if !self.shader_enabled && self.scanline_intensity < 1.0 {
            self.apply_scanlines();
        }

        true
    }

    fn resize_framebuffer(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.framebuffer = vec![0; new_width * new_height * 4];
        for i in (3..self.framebuffer.len()).step_by(4) {
            self.framebuffer[i] = 255;
        }
    }

    #[inline(always)]
    fn fb_index(&self, x: usize, y: usize) -> usize {
        ((y + self.border_size) * self.width + (x + self.border_size)) * 4
    }

    fn apply_comb_filter(&mut self) {
        let original = self.framebuffer.clone();
        const BLEND: f32 = 0.1;

        for src_line in 0..192_usize {
            let y_cur = src_line * 2;

            for x in 0..self.active_width {
                let idx_cur = self.fb_index(x, y_cur);
                if idx_cur + 4 > original.len() { continue; }

                let cr = original[idx_cur] as f32;
                let cg = original[idx_cur + 1] as f32;
                let cb = original[idx_cur + 2] as f32;

                let mut blend_r = cr;
                let mut blend_g = cg;
                let mut blend_b = cb;

                if src_line > 0 {
                    let idx = self.fb_index(x, (src_line - 1) * 2);
                    blend_r += (original[idx] as f32 - cr) * BLEND;
                    blend_g += (original[idx + 1] as f32 - cg) * BLEND;
                    blend_b += (original[idx + 2] as f32 - cb) * BLEND;
                }

                if src_line < 191 {
                    let idx = self.fb_index(x, (src_line + 1) * 2);
                    blend_r += (original[idx] as f32 - cr) * BLEND;
                    blend_g += (original[idx + 1] as f32 - cg) * BLEND;
                    blend_b += (original[idx + 2] as f32 - cb) * BLEND;
                }

                for dy in 0..2_usize {
                    let idx = self.fb_index(x, y_cur + dy);
                    if idx + 4 <= self.framebuffer.len() {
                        self.framebuffer[idx] = blend_r as u8;
                        self.framebuffer[idx + 1] = blend_g as u8;
                        self.framebuffer[idx + 2] = blend_b as u8;
                    }
                }
            }
        }
    }

    fn apply_scanlines(&mut self) {
        let intensity = self.scanline_intensity.clamp(0.0, 1.0);

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

    pub fn render_text_mode_overlay(&mut self, iou: &IOU, mmu: &MMU) {
        self.render_text_rows(iou, mmu, 20..24);

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
        let (font_offset, mut invert) = apple_iic_font_index(char_code, is_altchar);

       // Flashing range in VRAM: 0x40-0x7F (when not in AltChar/MouseText mode)
        if !is_altchar && (char_code >= 0x40 && char_code <= 0x7F) {
            // Flash rate: approx 2Hz. 60fps / 32 = ~1.8Hz
            let flash_on = (self.frame_count / 16) % 2 == 0;
            if !flash_on {
                invert = false;
            }
        }

        let char_width = if double_width { 14 } else { 7 };

        for char_row in 0..8_u16 {
            let mut font_byte = CHAR_ROM[font_offset + char_row as usize];
            
            if invert {
                font_byte = !font_byte;
            }

            let y = (row * 8 + char_row) * 2;
            let x = col * char_width; 

            let mut rgba_row = [0u8; 14 * 4];

            for bit in 0..7 {
                let pixel_on = (font_byte >> bit) & 1 != 0;
                let (r, g, b) = if pixel_on {
                    if self.monochrome {
                        (MONO_GREEN.0, MONO_GREEN.1, MONO_GREEN.2)
                    } else {
                        (255, 255, 255)
                    }
                } else {
                    (0, 0, 0)
                };

                if double_width {
                    // draw 2 pixels for each font bit
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
                    // draw 1 pixel for each font bit
                    let base_index = bit * 4; // bit * 1 pixel * 4 bytes
                    
                    rgba_row[base_index] = r;
                    rgba_row[base_index + 1] = g;
                    rgba_row[base_index + 2] = b;
                    rgba_row[base_index + 3] = 255;
                }
            }

            for dy in 0..2 {
                let start_index = self.fb_index(x as usize, y as usize + dy);
                let end_index = start_index + (char_width as usize) * 4;

                if end_index <= self.framebuffer.len() {
                    self.framebuffer[start_index..end_index].copy_from_slice(&rgba_row[0..(char_width as usize * 4)]);
                }
            }
        }
    }

    fn apply_mixed_mode_text_fringing(&mut self, start_text_row: usize) {
        let fringe_alpha: f32 = 0.25; // blend strength (~25%)
        let y_start = start_text_row * 8 * 2;
        let y_end = 24 * 8 * 2;

        for y in (y_start..y_end).step_by(2) {
            for x in 1..self.active_width - 1 {
                let idx = self.fb_index(x, y);
                if idx + 4 > self.framebuffer.len() { continue; }

                let r = self.framebuffer[idx] as u16;
                let g = self.framebuffer[idx + 1] as u16;
                let b = self.framebuffer[idx + 2] as u16;
                let is_bright = r + g + b > 400;

                if !is_bright { continue; }

                // right neighbor
                let ridx = self.fb_index(x + 1, y);
                if ridx + 4 <= self.framebuffer.len() {
                    let rr = self.framebuffer[ridx] as u16;
                    let rg = self.framebuffer[ridx + 1] as u16;
                    let rb = self.framebuffer[ridx + 2] as u16;
                    if rr + rg + rb < 100 {
                        // phase-based fringe
                        let fringe = Self::ntsc_fringe_color((x + 1) % 4);
                        for dy in 0..2 {
                            let fi = self.fb_index(x + 1, y + dy);
                            if fi + 4 <= self.framebuffer.len() {
                                self.framebuffer[fi]     = (fringe[0] as f32 * fringe_alpha) as u8;
                                self.framebuffer[fi + 1] = (fringe[1] as f32 * fringe_alpha) as u8;
                                self.framebuffer[fi + 2] = (fringe[2] as f32 * fringe_alpha) as u8;
                            }
                        }
                    }
                }

                // left neighbor
                let lidx = self.fb_index(x - 1, y);
                if lidx + 4 <= self.framebuffer.len() {
                    let lr = self.framebuffer[lidx] as u16;
                    let lg = self.framebuffer[lidx + 1] as u16;
                    let lb = self.framebuffer[lidx + 2] as u16;
                    if lr + lg + lb < 100 {
                        let fringe = Self::ntsc_fringe_color((x - 1) % 4);
                        for dy in 0..2 {
                            let fi = self.fb_index(x - 1, y + dy);
                            if fi + 4 <= self.framebuffer.len() {
                                self.framebuffer[fi]     = (fringe[0] as f32 * fringe_alpha) as u8;
                                self.framebuffer[fi + 1] = (fringe[1] as f32 * fringe_alpha) as u8;
                                self.framebuffer[fi + 2] = (fringe[2] as f32 * fringe_alpha) as u8;
                            }
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn ntsc_fringe_color(phase: usize) -> [u8; 4] {
        match phase % 4 {
            0 => NTSC_PALETTE[2],  // Dark Blue   (phase   0°)
            1 => NTSC_PALETTE[1],  // Red         (phase  90°)
            2 => NTSC_PALETTE[8],  // Brown       (phase 180°)
            3 => NTSC_PALETTE[4],  // Dark Green  (phase 270°)
            _ => unreachable!()
        }
    }

    /// Palette 0 (bit 7=0): even->Violet (3), odd->Green (12)
    /// Palette 1 (bit 7=1): even->Blue (6), odd->Orange (9)
    #[inline]
    fn ntsc_hires_artifact_color(
        cur: bool, prev: bool, next: bool,
        phase_column: usize, palette: bool,
    ) -> [u8; 4] {
        if cur {
            if prev || next {
                // adjacent ON pixels cancel chroma
                NTSC_PALETTE[15]
            } else {
                if palette {
                    if phase_column % 2 == 0 { NTSC_PALETTE[6] } else { NTSC_PALETTE[9] }
                } else {
                    if phase_column % 2 == 0 { NTSC_PALETTE[3] } else { NTSC_PALETTE[12] }
                }
            }
        } else if prev && next {
            // between two ON pixels
            if palette {
                if phase_column % 2 == 0 { NTSC_PALETTE[9] } else { NTSC_PALETTE[6] }
            } else {
                if phase_column % 2 == 0 { NTSC_PALETTE[12] } else { NTSC_PALETTE[3] }
            }
        } else if prev || next {
            // Edge of a colored region, uminance-weighted bleed...
            // bright colors (orange, green) get visible spread (~0.4-0.5),
            // dark colors (blue) get minimal spread (~0.15).
            let base = if palette {
                if phase_column % 2 == 0 { NTSC_PALETTE[9] } else { NTSC_PALETTE[6] }
            } else {
                if phase_column % 2 == 0 { NTSC_PALETTE[12] } else { NTSC_PALETTE[3] }
            };
            let luma = (0.299 * base[0] as f32 + 0.587 * base[1] as f32
                + 0.114 * base[2] as f32) / 255.0;
            let factor = luma; // linear
            [
                (base[0] as f32 * factor) as u8,
                (base[1] as f32 * factor) as u8,
                (base[2] as f32 * factor) as u8,
                255,
            ]
        } else {
            NTSC_PALETTE[0] // black
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

    /// Render HiRes mode using direct NTSC artifact color palette lookup.
    /// HiRes only has 4 possible artifact colors per palette — violet/green
    /// (palette 0) and blue/orange (palette 1) — so a lookup from Munafo's
    /// calibrated palette is more accurate than a full composite decode.
    fn render_hires_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let base_vram: u16 = 0x0000;

        for group in 0..24_u16 {
            for row in 0..8_u16 {
                let row_base = base_vram
                    .wrapping_add(row.wrapping_mul(1024))
                    .wrapping_add((group % 8).wrapping_mul(128))
                    .wrapping_add((group / 8).wrapping_mul(40));

                let y = ((group as usize) * 8 + (row as usize)) * 2;

                if self.monochrome {
                    for col in 0..40_u16 {
                        let addr = row_base.wrapping_add(col);
                        let byte = self.read_hires_memory(iou, mmu, addr);
                        for bit in 0..7_usize {
                            let pixel_on = (byte >> bit) & 1 != 0;
                            let color = if pixel_on { MONO_GREEN_RGBA } else { MONO_BLACK_RGBA };
                            let x = col as usize * 14 + bit * 2;
                            for dy in 0..2_usize {
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
                        }
                    }
                    continue;
                }

                // Color HiRes: direct palette lookup from pixel context
                let mut prev_byte: u8 = 0;
                for col in 0..40_usize {
                    let addr = row_base.wrapping_add(col as u16);
                    let byte = self.read_hires_memory(iou, mmu, addr);
                    let next_byte = if col < 39 {
                        self.read_hires_memory(iou, mmu, row_base.wrapping_add(col as u16 + 1))
                    } else {
                        0
                    };

                    let palette = (byte & 0x80) != 0;

                    for bit in 0..7_usize {
                        let cur = (byte >> bit) & 1 != 0;
                        let prev = if bit == 0 {
                            (prev_byte >> 6) & 1 != 0
                        } else {
                            (byte >> (bit - 1)) & 1 != 0
                        };
                        let next = if bit == 6 {
                            (next_byte >> 0) & 1 != 0
                        } else {
                            (byte >> (bit + 1)) & 1 != 0
                        };

                        let phase_column = col * 7 + bit;
                        let color = Self::ntsc_hires_artifact_color(
                            cur, prev, next, phase_column, palette,
                        );

                        let x = col * 14 + bit * 2;
                        for dy in 0..2_usize {
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
                    }
                    prev_byte = byte;
                }
            }
        }

    }

    /// Blur I and Q channels independently in YIQ space.
    /// Simulates analog chroma bandwidth limiting: I (~1.3 MHz, 7-tap)
    /// and Q (~0.5 MHz, 11-tap) blur at different widths. Luma (Y)
    /// is left sharp. At color-to-black edges, chroma fades gradually
    /// producing desaturated pixels at the correct hue.
    fn apply_chroma_blur(&mut self, y_start: usize, y_end: usize) {
        const I_KERNEL: [f32; 3] = [0.25, 0.5, 0.25];
        const Q_KERNEL: [f32; 3] = [0.25, 0.5, 0.25];

        #[inline]
        fn rgb_to_yiq(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
            let r = r as f32 / 255.0;
            let g = g as f32 / 255.0;
            let b = b as f32 / 255.0;
            (
                0.299 * r + 0.587 * g + 0.114 * b,
                0.5959 * r - 0.2746 * g - 0.3213 * b,
                0.2115 * r - 0.5227 * g + 0.3112 * b,
            )
        }

        #[inline]
        fn yiq_to_rgb(y: f32, i: f32, q: f32) -> (u8, u8, u8) {
            (
                ((y + 0.9563 * i + 0.6210 * q).clamp(0.0, 1.0) * 255.0) as u8,
                ((y - 0.2721 * i - 0.6474 * q).clamp(0.0, 1.0) * 255.0) as u8,
                ((y - 1.1070 * i + 1.7046 * q).clamp(0.0, 1.0) * 255.0) as u8,
            )
        }

        let aw = self.active_width;
        let i_half = I_KERNEL.len() / 2;
        let q_half = Q_KERNEL.len() / 2;

        for y in (y_start..y_end).step_by(2) {
            let row_yiq: Vec<(f32, f32, f32)> = (0..aw)
                .map(|x| {
                    let idx = self.fb_index(x, y);
                    rgb_to_yiq(
                        self.framebuffer[idx],
                        self.framebuffer[idx + 1],
                        self.framebuffer[idx + 2],
                    )
                })
                .collect();

            for x in 0..aw {
                let y_val = row_yiq[x].0;

                let mut bi = 0.0f32;
                let mut bw_i = 0.0f32;
                for (k, &w) in I_KERNEL.iter().enumerate() {
                    let sx = x as i32 - i_half as i32 + k as i32;
                    if sx >= 0 && sx < aw as i32 {
                        bi += row_yiq[sx as usize].1 * w;
                        bw_i += w;
                    }
                }
                let i_val = bi / bw_i;

                let mut bq = 0.0f32;
                let mut bw_q = 0.0f32;
                for (k, &w) in Q_KERNEL.iter().enumerate() {
                    let sx = x as i32 - q_half as i32 + k as i32;
                    if sx >= 0 && sx < aw as i32 {
                        bq += row_yiq[sx as usize].2 * w;
                        bw_q += w;
                    }
                }
                let q_val = bq / bw_q;

                let (r, g, b) = yiq_to_rgb(y_val, i_val, q_val);

                for dy in 0..2_usize {
                    let idx = self.fb_index(x, y + dy);
                    if idx + 4 <= self.framebuffer.len() {
                        self.framebuffer[idx] = r;
                        self.framebuffer[idx + 1] = g;
                        self.framebuffer[idx + 2] = b;
                    }
                }
            }
        }
    }

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
                            let color = if pixel_on { MONO_GREEN_RGBA } else { MONO_BLACK_RGBA };
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
                            let color = if pixel_on { MONO_GREEN_RGBA } else { MONO_BLACK_RGBA };
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
                    // Color DHIRES: sliding-window 4-bit color extraction.
                    // Each of 560 output pixels gets its own 4-bit color nibble
                    // from a phase-rotated window of 4 bits in the scanline.
                    // This matches how an NTSC decoder extracts color from the
                    // composite signal: the 4 bits map to different nibble
                    // positions depending on their phase in the color clock.

                    // Build 560-bit scanline from interleaved aux/main bytes
                    let mut scanline_bits = [false; 564]; // +4 for sliding window
                    for col in 0..40_usize {
                        let addr = row_base.wrapping_add(col as u16);
                        let aux_byte = mmu.read_aux_byte(addr);
                        let main_byte = mmu.read_main_byte(addr);
                        for bit in 0..7_usize {
                            scanline_bits[col * 14 + bit] = (aux_byte >> bit) & 1 != 0;
                            scanline_bits[col * 14 + 7 + bit] = (main_byte >> bit) & 1 != 0;
                        }
                    }

                    // Extract 4-bit color using aligned 4-pixel groups.
                    // Each group of 4 pixels shares one color.
                    for i in 0..560_usize {
                        let group_start = i - (i % 4);
                        let mut nibble: u8 = 0;
                        for j in 0..4_usize {
                            if scanline_bits[group_start + j] {
                                nibble |= 1 << j; // LSB-first
                            }
                        }

                        let rgba = DHIRES_PALETTE[nibble as usize];

                        for dy in 0..2 {
                            let index = self.fb_index(i, y as usize + dy);
                            if index + 4 <= self.framebuffer.len() {
                                self.framebuffer[index..index + 4].copy_from_slice(&rgba);
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

    pub fn get_active_dimensions(&self) -> (u32, u32) {
        (self.active_width as u32, self.active_height as u32)
    }

    pub fn get_border_size(&self) -> u32 {
        self.border_size as u32
    }

    pub fn get_pixels(&self) -> &[u8] {
        &self.framebuffer
    }

    fn lores_color_lookup(&self, color: u8) -> [u8; 4] {
        let rgba = NTSC_PALETTE[(color & 0x0F) as usize];

        if self.monochrome {
            let y = (0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32) as u8;
            if y < 24 {
                MONO_BLACK_RGBA
            } else {
                [MONO_GREEN.0.min(20), y, MONO_GREEN.2.min(20), 255]
            }
        } else {
            rgba
        }
    }
}
