use std::cell::Cell;

use crate::{iou::IOU, mmu::MMU, util::apple_iic_font_index};

const CHAR_ROM: &[u8; 4096] = include_bytes!("../video.bin");

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
    //  video_mode: Cell<u8>,
    extra: Cell<u8>,
}

impl Video {
    pub fn new() -> Self {
        let width = 560;
        let height = 192;
        Self {
            framebuffer: vec![0; width * height * 4],
            width,
            height,
            //  video_mode: Cell::new(VideoMode::TEXT),
            extra: Cell::new(0),
        }
    }

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
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80col = check_bits_u8!(video_mode, VideoModeMask::COL80);
        let is_dhires = check_bits_u8!(video_mode, VideoModeMask::DHIRES);
        let lo_res_mode = check_bits_u8!(video_mode, VideoModeMask::LORES);
        let is_hires = check_bits_u8!(video_mode, VideoModeMask::HIRES);
        let mixed_mode = check_bits_u8!(video_mode, VideoModeMask::MIXED);
        let text_mode = check_bits_u8!(video_mode, VideoModeMask::TEXT);
        let is_80store: bool = iou.is_80store.get();

        let new_width = if text_mode {
            if is_80col {
                560
            } else {
                280
            }
        } else if lo_res_mode {
            280
        } else if is_hires {
            if mixed_mode {
                560
            } else {
                560
            }
        } else if mixed_mode {
            280
        } else {
            560
        };

        let new_height = if text_mode {
            192
        } else if lo_res_mode {
            48
        } else if is_hires {
            if mixed_mode {
                384
            } else {
                192
            }
        } else if mixed_mode {
            192
        } else {
            192
        };

        if new_width != self.width || new_height != self.height {
            println!(
                "Resizing framebuffer from {}x{} to {}x{}",
                self.width, self.height, new_width, new_height
            );
            self.resize_framebuffer(new_width, new_height);
        }

        if text_mode {
            //self.dump_text_vram(iou, mmu);
            self.render_text_mode(iou, mmu);
        }

        true
    }

    fn resize_framebuffer(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.framebuffer = vec![0; new_width * new_height * 4];
    }

    fn read_text_memory(&self, iou: &IOU, mmu: &MMU, addr: u16) -> u8 {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        let real_addr = if is_page2 {
            if !is_80store {
                addr.wrapping_add(0x800)
            } else {
                addr.wrapping_add(0x400)
            }
        } else {
            addr.wrapping_add(0x400)
        };

        mmu.read_aux_byte(real_addr)
    }

    fn read_aux_text_memory(&self, iou: &IOU, mmu: &MMU, addr: u16) -> u8 {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        let real_addr = if is_page2 {
            if !is_80store {
                addr.wrapping_add(0x800)
            } else {
                addr.wrapping_add(0x400)
            }
        } else {
            addr.wrapping_add(0x400)
        };

        mmu.read_aux_byte(real_addr)
    }

    fn read_hires_memory(&self, iou: &IOU, mmu: &MMU, addr: u16) -> u8 {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        let real_addr = if is_page2 {
            if !is_80store {
                addr.wrapping_add(0x4000)
            } else {
                addr.wrapping_add(0x2000)
            }
        } else {
            addr.wrapping_add(0x2000)
        };

        mmu.read_byte(iou, real_addr)
    }

    fn read_aux_hires_memory(&self, iou: &IOU, mmu: &MMU, addr: u16) -> u8 {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);
        let is_80store = iou.is_80store.get();

        let real_addr = if is_page2 {
            if !is_80store {
                addr.wrapping_add(0x4000)
            } else {
                addr.wrapping_add(0x2000)
            }
        } else {
            addr.wrapping_add(0x2000)
        };

        mmu.read_aux_byte(real_addr)
    }

    fn read_text_memory_mock(&self, addr: u16) -> u8 {
        let base_char = (addr & 0xFF) as u8;
        let inverse = if (addr & 0x100) != 0 { 0x80 } else { 0x00 };

        base_char | inverse
    }

    fn get_font_offset(&self, char_code: u8) -> usize {
        match char_code {
            0x00..=0x1F => ((char_code + 0x40) as usize) * 8, // Symbols (inverse)
            0x20..=0x3F => ((char_code - 0x20) as usize + 2) * 8, // Normal Symbols
            0x40..=0x5F => ((char_code - 0x40) as usize + 0) * 8, // Uppercase Letters
            0x60..=0x7F => ((char_code - 0x60) as usize + 6) * 8, // Lowercase
            0x80..=0x9F => ((char_code - 0x80) as usize + 8) * 8, // Inverse Uppercase
            0xA0..=0xBF => ((char_code - 0xA0) as usize + 10) * 8, // Inverse Symbols
            0xC0..=0xDF => ((char_code - 0xC0) as usize + 12) * 8, // Flash Uppercase
            0xE0..=0xFF => ((char_code - 0xE0) as usize + 14) * 8, // Flash Lowercase
            _ => 0,
        }
    }

    pub fn dump_text_vram(&self, iou: &IOU, mmu: &MMU) {
        // println!("--- Apple IIc Text VRAM Hex Dump ---");
        // for (row, &base_addr) in TEXT_MODE_BASE_ADDRESSES.iter().enumerate() {
        //     print!("{:02}: ", row);
        //     for col in 0..40 {
        //         let addr = base_addr + col;
        //         let vram_byte = mmu.read_byte(iou, addr);
        //         print!("{:02X} ", vram_byte);
        //     }
        //     println!();
        // }
        // println!("-----------------------------------");
    }

    pub fn render_text_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let video_mode = iou.video_mode.get();
        let is_altchar = check_bits_u8!(video_mode, VideoModeMask::ALTCHAR);

        for row in 0..24_u16 {
            let row_base = TEXT_MODE_BASE_ADDRESSES[row as usize];

            for col in 0..40_u16 {
                let addr = row_base + col;
                let mut vram_code = mmu.read_byte(iou, addr);

                // 0x00 as 0xA0 (blank space)
                if vram_code == 0x00 {
                    vram_code = 0xA0;
                }

                let font_offset = apple_iic_font_index(vram_code, is_altchar);

                for char_row in 0..8_u16 {
                    let font_byte = CHAR_ROM[font_offset + char_row as usize].reverse_bits();
                    let y = row * 8 + char_row;
                    let x = col * 7;

                    let mut rgba_row = [0u8; 7 * 4];

                    for bit in 0..7 {
                        let pixel_on = (font_byte >> (6 - bit)) & 1 != 0;
                        let color = if pixel_on { 255 } else { 0 };

                        let base_index = bit * 4;
                        rgba_row[base_index] = color; // R
                        rgba_row[base_index + 1] = color; // G
                        rgba_row[base_index + 2] = color; // B
                        rgba_row[base_index + 3] = 255; // A
                    }

                    let start_index = (y as usize * self.width + x as usize) * 4;
                    let end_index = start_index + 7 * 4;

                    if end_index <= self.framebuffer.len() {
                        self.framebuffer[start_index..end_index].copy_from_slice(&rgba_row);
                    }
                }
            }
        }
    }

    pub fn read_aux_byte_mock(&self, addr: u16) -> u8 {
        match addr & 0x3FF {
            0x000..=0x07F => 0b00001111, // Cyan/Magenta alternating
            0x080..=0x0FF => 0b11110000, // Yellow/Green alternating
            0x100..=0x17F => 0b10101010, // Striped
            0x180..=0x1FF => 0b01010101, // Striped inverse
            _ => 0b11001100,             // Checkerboard pattern
        }
    }

    fn render_lores_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let video_mode = iou.video_mode.get();
        let is_page2 = check_bits_u8!(video_mode, VideoModeMask::PAGE2);

        let base_vram: u16 = if is_page2 { 0x0800 } else { 0x0400 };

        for row in 0..48_u16 {
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

            for col in 0..40_u16 {
                let addr = base_address + col;
                let color_byte = self.read_aux_byte_mock(addr);

                let color_code = if row % 2 == 0 {
                    color_byte & 0x0F // Lower nibble
                } else {
                    (color_byte >> 4) & 0x0F // Upper nibble
                };

                let color = self.lores_color_lookup(color_code);

                let x = col * 14;
                let y = row * 4;

                for dy in 0..4 {
                    let y_idx = (y as usize + dy as usize) * self.width;
                    for dx in 0..7 {
                        let index = (y_idx + (x as usize + dx as usize)) * 4;
                        if index + 4 <= self.framebuffer.len() {
                            self.framebuffer[index..index + 4].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    fn render_mixed_mode(&mut self, iou: &IOU, mmu: &MMU) {
        self.render_hires_mode(iou, mmu);
        self.render_text_mode(iou, mmu);
    }

    fn render_hires_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let base_vram: u16 = 0x2000;

        for group in 0..24_u16 {
            for row in 0..8_u16 {
                let row_base = base_vram
                    .wrapping_add(row.wrapping_mul(1024))
                    .wrapping_add(group.wrapping_mul(40));

                for col in 0..40_u16 {
                    let addr = row_base.wrapping_add(col);
                    let byte = self.read_hires_memory(iou, mmu, addr);

                    let mut left_pixel = false;
                    let mut right_pixel = false;

                    for bit in 0..7_u16 {
                        let pixel_on = (byte >> (6 - bit)) & 1 != 0;

                        let color = if pixel_on {
                            if left_pixel {
                                [255, 255, 255, 255] // White (Artifact)
                            } else {
                                [255, 0, 0, 255] // Red (Artifact)
                            }
                        } else {
                            if right_pixel {
                                [0, 255, 255, 255] // Cyan (Artifact)
                            } else {
                                [0, 0, 0, 255] // Black
                            }
                        };

                        left_pixel = pixel_on;
                        right_pixel = !pixel_on;

                        let y = (group as usize).wrapping_mul(8) + (row as usize);
                        let x = (col as usize).wrapping_mul(7) + (bit as usize);
                        let index = (y * self.width + x) * 4;

                        if (index as usize) + 4 <= self.framebuffer.len() {
                            self.framebuffer[(index as usize)..(index as usize + 4)]
                                .copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    fn render_double_hires_mode(&mut self, iou: &IOU, mmu: &MMU) {
        let base_vram: u16 = 0x2000;
        let video_mode = iou.video_mode.get();
        let is_80store = iou.is_80store.get();

        for group in 0..24_u16 {
            for row in 0..8_u16 {
                let row_base = self.get_display_address(
                    video_mode,
                    is_80store,
                    base_vram
                        .wrapping_add(row.wrapping_mul(1024))
                        .wrapping_add(group.wrapping_mul(40)),
                );

                for col in 0..40_u16 {
                    let addr = row_base.wrapping_add(col);
                    let main_byte = self.read_hires_memory(iou, mmu, addr);
                    let aux_byte = self.read_aux_hires_memory(iou, mmu, addr);

                    for bit in 0..7_u16 {
                        let even_pixel = (main_byte >> (6 - bit)) & 1 != 0;
                        let odd_pixel = (aux_byte >> (6 - bit)) & 1 != 0;

                        let color = match (even_pixel, odd_pixel) {
                            (true, true) => [255, 255, 255, 255], // White
                            (true, false) => [255, 0, 255, 255],  // Magenta
                            (false, true) => [0, 255, 255, 255],  // Cyan
                            (false, false) => [0, 0, 0, 255],     // Black
                        };

                        let x = col.wrapping_mul(7).wrapping_add(bit);
                        let y = group.wrapping_mul(8).wrapping_add(row);
                        let index = ((y * (self.width as u16)) + x) * 4;

                        if (index as usize) + 4 <= self.framebuffer.len() {
                            self.framebuffer[(index as usize)..(index as usize + 4)]
                                .copy_from_slice(&color);
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
        match color & 0x0F {
            0x0 => [0, 0, 0, 255],
            0x1 => [227, 30, 96, 255],
            0x2 => [96, 78, 189, 255],
            0x3 => [255, 68, 253, 255],
            0x4 => [0, 129, 64, 255],
            _ => [255, 255, 255, 255],
        }
    }
}
