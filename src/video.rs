use crate::{iou::Iou, mmu::Mmu, util::apple_iic_font_index};
use rayon::prelude::*;
use wide::{CmpGt, CmpLt};

const CHAR_ROM: &[u8; 1024] = include_bytes!("../assets/font.bin");

pub const DEFAULT_MONO_FG: [u8; 4] = [118, 255, 211, 255];
pub const DEFAULT_MONO_BG: [u8; 4] = [15, 23, 23, 255];

// NTSC 16-color palette (standard LoRes/HiRes ordering)
// Derived from Apple IIc video ROM (video.bin 0x800-0xFFF) via NTSC demodulation
// of the hardware dot patterns at 49° colorburst phase.
// Re-ordered from DHIRES numbering: entries 2<-8, 3<-9, 6<-C, 7<-D swapped.
#[rustfmt::skip]
const NTSC_PALETTE: [[u8; 4]; 16] = [
    [  0,   0,   0, 255], // 0x0: Black
    [208,   0, 100, 255], // 0x1: Magenta        (DHIRES 0x1)
    [ 44,  24, 255, 255], // 0x2: Dark Blue      (DHIRES 0x8)
    [251,   8, 255, 255], // 0x3: Purple/Violet  (DHIRES 0x9) HiRes: Violet
    [  0, 144,  28, 255], // 0x4: Dark Green     (DHIRES 0x4)
    [127, 127, 128, 255], // 0x5: Gray 1
    [  0, 168, 255, 255], // 0x6: Medium Blue    (DHIRES 0xC) HiRes: Blue
    [171, 152, 255, 255], // 0x7: Light Blue     (DHIRES 0xD)
    [ 84, 103,   0, 255], // 0x8: Brown          (DHIRES 0x2)
    [255,  87,   0, 255], // 0x9: Orange         (DHIRES 0x3) HiRes: Orange
    [128, 127, 128, 255], // 0xA: Gray 2
    [255, 111, 227, 255], // 0xB: Pink           (DHIRES 0xB)
    [  4, 247,   0, 255], // 0xC: Light Green    (DHIRES 0x6) HiRes: Green
    [211, 231,   0, 255], // 0xD: Yellow         (DHIRES 0x7)
    [ 47, 255, 155, 255], // 0xE: Aqua           (DHIRES 0xE)
    [255, 255, 255, 255], // 0xF: White
];

#[rustfmt::skip]
const DHIRES_PALETTE: [[u8; 4]; 16] = [
    [  0,   0,   0, 255], // 0x0: Black
    [208,   0, 100, 255], // 0x1: Magenta
    [ 84, 103,   0, 255], // 0x2: Brown
    [255,  87,   0, 255], // 0x3: Orange
    [  0, 144,  28, 255], // 0x4: Dark Green
    [127, 127, 128, 255], // 0x5: Grey 1
    [  4, 247,   0, 255], // 0x6: Green
    [211, 231,   0, 255], // 0x7: Yellow
    [ 44,  24, 255, 255], // 0x8: Dark Blue
    [251,   8, 255, 255], // 0x9: Purple
    [128, 127, 128, 255], // 0xA: Grey 2
    [255, 111, 227, 255], // 0xB: Pink
    [  0, 168, 255, 255], // 0xC: Blue
    [171, 152, 255, 255], // 0xD: Light Blue
    [ 47, 255, 155, 255], // 0xE: Aqua
    [255, 255, 255, 255], // 0xF: White
];

pub const TEXT_MODE_BASE_ADDRESSES: [u16; 24] = [
    0x0400, 0x0480, 0x0500, 0x0580, 0x0600, 0x0680, 0x0700, 0x0780, 0x0428, 0x04A8, 0x0528, 0x05A8,
    0x0628, 0x06A8, 0x0728, 0x07A8, 0x0450, 0x04D0, 0x0550, 0x05D0, 0x0650, 0x06D0, 0x0750, 0x07D0,
];

#[derive(Clone, Copy, Debug)]
pub struct VideoEffects {
    pub chroma_blur: bool,
    pub comb_filter: bool,
    pub phosphor_spread: bool,
    pub scanlines: bool,
    pub white_preservation: f32,
    pub chroma_saturation: f32,
    pub chroma_luma_scale: f32,
}

impl Default for VideoEffects {
    fn default() -> Self {
        Self {
            chroma_blur: true,
            comb_filter: true,
            phosphor_spread: true,
            scanlines: true,
            white_preservation: 0.0,
            chroma_saturation: 2.2,
            chroma_luma_scale: 1.0,
        }
    }
}

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
    pub mono_fg: [u8; 4],
    pub mono_bg: [u8; 4],
    pub force_neutral_mono: bool,
    pub shader_enabled: bool,
    pub scanline_intensity: f32,
    pub border_size: usize,

    pub effects: VideoEffects,

    pub stable_page: bool,
    live_page2: bool,
    live_80store: bool,

    scanline_modes: [u8; 192],
    scanline_80store: [bool; 192],
    scanline_count: usize,

    monitor_partial_fb: Vec<u8>,

    raw_framebuffer: Vec<u8>,

    // sRGB u8 to linear f32
    srgb_to_linear_lut: [f32; 256],
    // linear f32 in [0, 1] to sRGB u8, sampled at 4096 steps
    linear_to_srgb_lut_u8: Box<[u8; 4096]>,
    // YIQ scratch in SoA layout, sized `3 * active_width * 192`
    // (Y region, then I, then Q), hared between the comb filter's two passes
    yiq_buf: Vec<f32>,
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

        let mut srgb_to_linear_lut = [0.0f32; 256];
        for (i, slot) in srgb_to_linear_lut.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *slot = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
        }
        let mut linear_to_srgb_lut_u8 = Box::new([0u8; 4096]);
        for (i, slot) in linear_to_srgb_lut_u8.iter_mut().enumerate() {
            let c = i as f32 / 4095.0;
            let s = if c <= 0.0031308 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            *slot = (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }

        Self {
            framebuffer,
            width,
            height,
            active_width,
            active_height,
            frame_count: 0,
            monochrome: false,
            mono_fg: DEFAULT_MONO_FG,
            mono_bg: DEFAULT_MONO_BG,
            force_neutral_mono: false,
            shader_enabled: false,
            scanline_intensity: 0.15,
            border_size: border,
            effects: VideoEffects::default(),
            stable_page: false,
            live_page2: false,
            live_80store: false,
            scanline_modes: [0; 192],
            scanline_80store: [false; 192],
            scanline_count: 0,
            monitor_partial_fb: Vec::new(),
            raw_framebuffer: Vec::new(),
            srgb_to_linear_lut,
            linear_to_srgb_lut_u8,
            yiq_buf: Vec::with_capacity(active_width * 192 * 3),
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

    pub fn set_mono_colors(&mut self, fg: [u8; 3], bg: [u8; 3]) {
        self.mono_fg = [fg[0], fg[1], fg[2], 255];
        self.mono_bg = [bg[0], bg[1], bg[2], 255];
    }

    pub fn update(&mut self, iou: &Iou, mmu: &Mmu) -> bool {
        self.frame_count = self.frame_count.wrapping_add(1);
        
        self.framebuffer.fill(0);

        self.live_page2   = (iou.video_mode.get() & VideoModeMask::PAGE2) != 0;
        self.live_80store = iou.is_80store.get();

        let new_active_width = 560;
        let new_active_height = 384;
        let new_width = new_active_width + self.border_size * 2;
        let new_height = new_active_height + self.border_size * 2;

        if new_width != self.width || new_height != self.height {
            self.active_width = new_active_width;
            self.active_height = new_active_height;
            self.resize_framebuffer(new_width, new_height);
        }

        let has_snapshots = self.scanline_count >= 192;
        let mut any_graphics = false;

        let mut any_non_hires_color = false;

        for scanline in 0..192_usize {
            let mode = if has_snapshots {
                self.scanline_modes[scanline]
            } else {
                iou.video_mode.get()
            };
            let is_80store = if has_snapshots {
                self.scanline_80store[scanline]
            } else {
                iou.is_80store.get()
            };

            let text_mode = (mode & VideoModeMask::TEXT) != 0;
            let is_hires = (mode & VideoModeMask::HIRES) != 0;
            let lo_res_mode = (mode & VideoModeMask::LORES) != 0;
            let is_dhires = (mode & VideoModeMask::DHIRES) != 0;
            let is_80col = (mode & VideoModeMask::COL80) != 0;
            let mixed_mode = (mode & VideoModeMask::MIXED) != 0;

            // in mixed mode, scanlines 160..192 (text rows 20-23) are
            // always rendered as text regardless of the active graphics
            // mode for that scanline.
            let force_text = mixed_mode && scanline >= 160;

            if text_mode || force_text {
                self.render_text_scanline(iou, mmu, scanline, mode, is_80store);
            } else if is_hires {
                if is_dhires && is_80col {
                    self.render_double_hires_scanline(iou, mmu, scanline);
                    any_non_hires_color = true;
                } else {
                    self.render_hires_scanline(iou, mmu, scanline, mode, is_80store);
                }
                any_graphics = true;
            } else if lo_res_mode {
                self.render_lores_scanline(iou, mmu, scanline, mode, is_80store);
                any_graphics = true;
                any_non_hires_color = true;
            } else {
                self.render_text_scanline(iou, mmu, scanline, mode, is_80store);
            }
        }

        let final_mode = if has_snapshots {
            self.scanline_modes[160]
        } else {
            iou.video_mode.get()
        };
        let final_mixed = (final_mode & VideoModeMask::MIXED) != 0;
        let final_text_only = (final_mode & VideoModeMask::TEXT) != 0;

        if self.raw_framebuffer.len() != self.framebuffer.len() {
            self.raw_framebuffer.resize(self.framebuffer.len(), 0);
        }
        self.raw_framebuffer.copy_from_slice(&self.framebuffer);

        if final_mixed && !final_text_only && any_graphics && !self.monochrome {
            self.apply_mixed_mode_text_fringing(20);
        }


        let gpu_owns_chroma = self.shader_enabled;
        let gpu_owns_scanlines = self.shader_enabled;
        let gpu_owns_phosphor_spread = self.shader_enabled;

        if any_graphics && !self.monochrome && !gpu_owns_chroma && any_non_hires_color {
            if self.effects.chroma_blur {
                self.apply_chroma_blur(0, 192 * 2);
            }
            if self.effects.comb_filter {
                self.apply_comb_filter();
            }
        }

        if self.effects.phosphor_spread && !gpu_owns_phosphor_spread {
            self.apply_phosphor_spread();
        }

        if !gpu_owns_scanlines && self.effects.scanlines && self.scanline_intensity < 1.0 {
            self.apply_scanlines();
        }

        true
    }

    // Dispatch a single Apple scanline (0..192) to the appropriate
    // per-mode renderer using the captured `scanline_modes` /
    // `scanline_80store` snapshot.
    fn dispatch_scanline(&mut self, iou: &Iou, mmu: &Mmu, scanline: usize) {
        let mode = self.scanline_modes[scanline];
        let is_80store = self.scanline_80store[scanline];

        let text_mode = (mode & VideoModeMask::TEXT) != 0;
        let is_hires = (mode & VideoModeMask::HIRES) != 0;
        let lo_res_mode = (mode & VideoModeMask::LORES) != 0;
        let is_dhires = (mode & VideoModeMask::DHIRES) != 0;
        let is_80col = (mode & VideoModeMask::COL80) != 0;
        let mixed_mode = (mode & VideoModeMask::MIXED) != 0;
        let force_text = mixed_mode && scanline >= 160;

        if text_mode || force_text {
            self.render_text_scanline(iou, mmu, scanline, mode, is_80store);
        } else if is_hires {
            if is_dhires && is_80col {
                self.render_double_hires_scanline(iou, mmu, scanline);
            } else {
                self.render_hires_scanline(iou, mmu, scanline, mode, is_80store);
            }
        } else if lo_res_mode {
            self.render_lores_scanline(iou, mmu, scanline, mode, is_80store);
        } else {
            self.render_text_scanline(iou, mmu, scanline, mode, is_80store);
        }
    }

    pub fn compose_monitor_partial(
        &mut self,
        iou: &Iou,
        mmu: &Mmu,
        up_to_scanline: usize,
    ) -> &[u8] {
        let len = self.framebuffer.len();
        if self.monitor_partial_fb.len() != len {
            self.monitor_partial_fb.resize(len, 0);
        }

        // Seed the partial buffer with the last completed framebuffer so
        // unrendered scanlines (and the border) carry over from the
        // previous frame.
        self.monitor_partial_fb.copy_from_slice(&self.framebuffer);

        // Clamp + dim the un-executed region so the beam frontier is
        // visually obvious. Each Apple scanline is 2 framebuffer rows
        // (vertical doubling). Active video starts after the top border.
        let stride = self.width * 4;
        let up_to = up_to_scanline.min(192);
        let stale_y_start = self.border_size + up_to * 2;
        let stale_y_end = (self.border_size + 192 * 2).min(self.height);
        for y in stale_y_start..stale_y_end {
            let row = y * stride;
            // Halve RGB; leave alpha alone (every 4th byte).
            for px in (row..row + stride).step_by(4) {
                self.monitor_partial_fb[px]     >>= 1;
                self.monitor_partial_fb[px + 1] >>= 1;
                self.monitor_partial_fb[px + 2] >>= 1;
            }
        }

        std::mem::swap(&mut self.framebuffer, &mut self.monitor_partial_fb);
        for scanline in 0..up_to {
            self.dispatch_scanline(iou, mmu, scanline);
        }
        std::mem::swap(&mut self.framebuffer, &mut self.monitor_partial_fb);

        &self.monitor_partial_fb
    }

    fn resize_framebuffer(&mut self, new_width: usize, new_height: usize) {
        self.width = new_width;
        self.height = new_height;
        self.framebuffer = vec![0; new_width * new_height * 4];
        for i in (3..self.framebuffer.len()).step_by(4) {
            self.framebuffer[i] = 255;
        }

        self.monitor_partial_fb.clear();
        self.raw_framebuffer.clear();
    }

    #[inline(always)]
    fn fb_index(&self, x: usize, y: usize) -> usize {
        ((y + self.border_size) * self.width + (x + self.border_size)) * 4
    }

    // 2-line comb filter in YIQ space
    fn apply_comb_filter(&mut self) {
        use wide::f32x8;
        let aw = self.active_width;
        const BLEND: f32 = 0.25;
        let stride = self.width * 4;
        let border = self.border_size;

        let total = 192 * aw;
        self.yiq_buf.clear();
        self.yiq_buf.resize(total * 3, 0.0);
        let (y_buf, iq_buf) = self.yiq_buf.split_at_mut(total);
        let (i_buf, q_buf) = iq_buf.split_at_mut(total);

        // Pass 1: gather YIQ from row 0 of each doubled scanline pair.
        let fb = &self.framebuffer[..];
        y_buf.par_chunks_mut(aw)
            .zip(i_buf.par_chunks_mut(aw))
            .zip(q_buf.par_chunks_mut(aw))
            .enumerate()
            .for_each(|(src_line, ((yr, ir), qr))| {
                let y_cur = src_line * 2;
                let fb_row = (y_cur + border) * stride + border * 4;
                for x in 0..aw {
                    let p = fb_row + x * 4;
                    let r = fb[p] as f32 * (1.0 / 255.0);
                    let g = fb[p + 1] as f32 * (1.0 / 255.0);
                    let b = fb[p + 2] as f32 * (1.0 / 255.0);
                    yr[x] = 0.299 * r + 0.587 * g + 0.114 * b;
                    ir[x] = 0.5959 * r - 0.2746 * g - 0.3213 * b;
                    qr[x] = 0.2115 * r - 0.5227 * g + 0.3112 * b;
                }
            });

        // Pass 2: blend with prev/next row and write back to the framebuffer.
        let y_buf_ref: &[f32] = y_buf;
        let i_buf_ref: &[f32] = i_buf;
        let q_buf_ref: &[f32] = q_buf;
        let fb_active_start = border * stride + border * 4;
        let active_bytes = 192 * 2 * stride;

        let blend_v = f32x8::splat(BLEND);
        let zero_v = f32x8::splat(0.0);
        let one_v = f32x8::splat(1.0);
        let m255 = f32x8::splat(255.0);
        let cy_r = f32x8::splat(0.9563);
        let cy_g = f32x8::splat(-0.2721);
        let cy_b = f32x8::splat(-1.1070);
        let cq_r = f32x8::splat(0.6210);
        let cq_g = f32x8::splat(-0.6474);
        let cq_b = f32x8::splat(1.7046);

        let simd_chunks = aw / 8;
        let simd_end = simd_chunks * 8;

        self.framebuffer[fb_active_start..fb_active_start + active_bytes]
            .par_chunks_mut(2 * stride)
            .enumerate()
            .for_each(|(src_line, chunk)| {
                let row = src_line * aw;
                let prev_row = src_line.saturating_sub(1) * aw;
                let next_row = (src_line + 1).min(191) * aw;
                let has_prev = src_line > 0;
                let has_next = src_line < 191;

                for blk in 0..simd_chunks {
                    let x = blk * 8;
                    let y_arr: [f32; 8] = y_buf_ref[row + x..row + x + 8].try_into().unwrap();
                    let i_arr: [f32; 8] = i_buf_ref[row + x..row + x + 8].try_into().unwrap();
                    let q_arr: [f32; 8] = q_buf_ref[row + x..row + x + 8].try_into().unwrap();
                    let y_v = f32x8::from(y_arr);
                    let mut i_v = f32x8::from(i_arr);
                    let mut q_v = f32x8::from(q_arr);

                    if has_prev {
                        let pi: [f32; 8] = i_buf_ref[prev_row + x..prev_row + x + 8].try_into().unwrap();
                        let pq: [f32; 8] = q_buf_ref[prev_row + x..prev_row + x + 8].try_into().unwrap();
                        i_v = i_v + (f32x8::from(pi) - i_v) * blend_v;
                        q_v = q_v + (f32x8::from(pq) - q_v) * blend_v;
                    }
                    if has_next {
                        let ni: [f32; 8] = i_buf_ref[next_row + x..next_row + x + 8].try_into().unwrap();
                        let nq: [f32; 8] = q_buf_ref[next_row + x..next_row + x + 8].try_into().unwrap();
                        i_v = i_v + (f32x8::from(ni) - i_v) * blend_v;
                        q_v = q_v + (f32x8::from(nq) - q_v) * blend_v;
                    }

                    let r_v = (y_v + i_v * cy_r + q_v * cq_r).fast_max(zero_v).fast_min(one_v);
                    let g_v = (y_v + i_v * cy_g + q_v * cq_g).fast_max(zero_v).fast_min(one_v);
                    let b_v = (y_v + i_v * cy_b + q_v * cq_b).fast_max(zero_v).fast_min(one_v);

                    let r_arr = (r_v * m255).to_array();
                    let g_arr = (g_v * m255).to_array();
                    let b_arr = (b_v * m255).to_array();
                    for j in 0..8 {
                        let rb = r_arr[j] as u8;
                        let gb = g_arr[j] as u8;
                        let bb = b_arr[j] as u8;
                        let off_top = (x + j) * 4;
                        let off_bot = stride + (x + j) * 4;
                        chunk[off_top]     = rb;
                        chunk[off_top + 1] = gb;
                        chunk[off_top + 2] = bb;
                        chunk[off_bot]     = rb;
                        chunk[off_bot + 1] = gb;
                        chunk[off_bot + 2] = bb;
                    }
                }

                for x in simd_end..aw {
                    let y_val = y_buf_ref[row + x];
                    let mut i_val = i_buf_ref[row + x];
                    let mut q_val = q_buf_ref[row + x];
                    if has_prev {
                        i_val += (i_buf_ref[prev_row + x] - i_val) * BLEND;
                        q_val += (q_buf_ref[prev_row + x] - q_val) * BLEND;
                    }
                    if has_next {
                        i_val += (i_buf_ref[next_row + x] - i_val) * BLEND;
                        q_val += (q_buf_ref[next_row + x] - q_val) * BLEND;
                    }

                    let r = (y_val + 0.9563 * i_val + 0.6210 * q_val).clamp(0.0, 1.0);
                    let g = (y_val - 0.2721 * i_val - 0.6474 * q_val).clamp(0.0, 1.0);
                    let b = (y_val - 1.1070 * i_val + 1.7046 * q_val).clamp(0.0, 1.0);

                    let rb = (r * 255.0) as u8;
                    let gb = (g * 255.0) as u8;
                    let bb = (b * 255.0) as u8;

                    let off_top = x * 4;
                    let off_bot = stride + x * 4;
                    chunk[off_top]     = rb;
                    chunk[off_top + 1] = gb;
                    chunk[off_top + 2] = bb;
                    chunk[off_bot]     = rb;
                    chunk[off_bot + 1] = gb;
                    chunk[off_bot + 2] = bb;
                }
            });
    }

    // CRT electron beam spot simulation. Applies a 3-tap horizontal kernel
    // `[0.05, 0.90, 0.05]` to soften adjacent pixel edges. Each doubled
    // scanline pair is fully independent, so this runs rayon-parallel across
    // pairs with f32x8 SIMD across the scanline.
    fn apply_phosphor_spread(&mut self) {
        use wide::f32x8;
        let aw = self.active_width;
        let stride = self.width * 4;
        let border = self.border_size;
        let n_lines = self.active_height / 2;

        let fb_active_start = border * stride + border * 4;
        let active_bytes = n_lines * 2 * stride;

        let center = f32x8::splat(0.90);
        let side = f32x8::splat(0.05);
        let zero_v = f32x8::splat(0.0);
        let m255 = f32x8::splat(255.0);

        // SIMD inner range: x in [1, aw - 1) so x-1 and x+1 reads are in bounds.
        let inner_count = (aw - 2) / 8;
        let inner_end = 1 + inner_count * 8;

        self.framebuffer[fb_active_start..fb_active_start + active_bytes]
            .par_chunks_mut(2 * stride)
            .for_each_init(
                || (vec![0.0f32; aw], vec![0.0f32; aw], vec![0.0f32; aw]),
                |(rch, gch, bch), chunk| {
                    for x in 0..aw {
                        let p = x * 4;
                        rch[x] = chunk[p] as f32;
                        gch[x] = chunk[p + 1] as f32;
                        bch[x] = chunk[p + 2] as f32;
                    }

                    // Left edge: clamp missing left neighbor to center.
                    {
                        let cr = rch[0]; let cg = gch[0]; let cb = bch[0];
                        let rr_ = if aw > 1 { rch[1] } else { cr };
                        let rg_ = if aw > 1 { gch[1] } else { cg };
                        let rb_ = if aw > 1 { bch[1] } else { cb };
                        let nr = (cr * 0.05 + cr * 0.90 + rr_ * 0.05) as u8;
                        let ng = (cg * 0.05 + cg * 0.90 + rg_ * 0.05) as u8;
                        let nb = (cb * 0.05 + cb * 0.90 + rb_ * 0.05) as u8;
                        chunk[0] = nr; chunk[1] = ng; chunk[2] = nb;
                        chunk[stride] = nr; chunk[stride + 1] = ng; chunk[stride + 2] = nb;
                    }

                    for blk in 0..inner_count {
                        let x = 1 + blk * 8;
                        let rl: [f32; 8] = rch[x - 1..x - 1 + 8].try_into().unwrap();
                        let rc: [f32; 8] = rch[x..x + 8].try_into().unwrap();
                        let rr: [f32; 8] = rch[x + 1..x + 1 + 8].try_into().unwrap();
                        let gl: [f32; 8] = gch[x - 1..x - 1 + 8].try_into().unwrap();
                        let gc: [f32; 8] = gch[x..x + 8].try_into().unwrap();
                        let gr: [f32; 8] = gch[x + 1..x + 1 + 8].try_into().unwrap();
                        let bl: [f32; 8] = bch[x - 1..x - 1 + 8].try_into().unwrap();
                        let bc: [f32; 8] = bch[x..x + 8].try_into().unwrap();
                        let br: [f32; 8] = bch[x + 1..x + 1 + 8].try_into().unwrap();

                        let r_v = (f32x8::from(rl) + f32x8::from(rr)) * side
                            + f32x8::from(rc) * center;
                        let g_v = (f32x8::from(gl) + f32x8::from(gr)) * side
                            + f32x8::from(gc) * center;
                        let b_v = (f32x8::from(bl) + f32x8::from(br)) * side
                            + f32x8::from(bc) * center;

                        let r_arr = r_v.fast_max(zero_v).fast_min(m255).to_array();
                        let g_arr = g_v.fast_max(zero_v).fast_min(m255).to_array();
                        let b_arr = b_v.fast_max(zero_v).fast_min(m255).to_array();
                        for j in 0..8 {
                            let nr = r_arr[j] as u8;
                            let ng = g_arr[j] as u8;
                            let nb = b_arr[j] as u8;
                            let p_top = (x + j) * 4;
                            let p_bot = stride + (x + j) * 4;
                            chunk[p_top]     = nr;
                            chunk[p_top + 1] = ng;
                            chunk[p_top + 2] = nb;
                            chunk[p_bot]     = nr;
                            chunk[p_bot + 1] = ng;
                            chunk[p_bot + 2] = nb;
                        }
                    }

                    // Scalar tail and right edge (clamps missing right neighbor to center).
                    for x in inner_end..aw {
                        let cr = rch[x]; let cg = gch[x]; let cb = bch[x];
                        let (lr, lg, lb) = (rch[x - 1], gch[x - 1], bch[x - 1]);
                        let (rr_, rg_, rb_) = if x + 1 < aw {
                            (rch[x + 1], gch[x + 1], bch[x + 1])
                        } else {
                            (cr, cg, cb)
                        };
                        let nr = (lr * 0.05 + cr * 0.90 + rr_ * 0.05) as u8;
                        let ng = (lg * 0.05 + cg * 0.90 + rg_ * 0.05) as u8;
                        let nb = (lb * 0.05 + cb * 0.90 + rb_ * 0.05) as u8;
                        let p_top = x * 4;
                        let p_bot = stride + x * 4;
                        chunk[p_top]     = nr;
                        chunk[p_top + 1] = ng;
                        chunk[p_top + 2] = nb;
                        chunk[p_bot]     = nr;
                        chunk[p_bot + 1] = ng;
                        chunk[p_bot + 2] = nb;
                    }
                },
            );
    }

    fn apply_scanlines(&mut self) {
        use wide::f32x8;
        let intensity = self.scanline_intensity.clamp(0.0, 1.0);
        let aw = self.active_width;
        let width = self.width;
        let border = self.border_size;
        let active_height = self.active_height;
        let stride = width * 4;

        let fb_active_start = border * stride + border * 4;
        let intensity_v = f32x8::splat(intensity);
        let m255 = f32x8::splat(255.0);
        let zero_v = f32x8::splat(0.0);

        self.framebuffer[fb_active_start..fb_active_start + active_height * stride]
            .par_chunks_mut(2 * stride)
            .for_each(|chunk| {
                // top row kept, dim only the bottom row of the pair
                let bot = &mut chunk[stride..stride + aw * 4];
                let simd_chunks = aw / 8;
                let simd_end = simd_chunks * 8;
                for blk in 0..simd_chunks {
                    let x0 = blk * 8;
                    let mut r_arr = [0.0f32; 8];
                    let mut g_arr = [0.0f32; 8];
                    let mut b_arr = [0.0f32; 8];
                    for j in 0..8 {
                        let p = (x0 + j) * 4;
                        r_arr[j] = bot[p] as f32;
                        g_arr[j] = bot[p + 1] as f32;
                        b_arr[j] = bot[p + 2] as f32;
                    }
                    let r_v = (f32x8::from(r_arr) * intensity_v).fast_max(zero_v).fast_min(m255).to_array();
                    let g_v = (f32x8::from(g_arr) * intensity_v).fast_max(zero_v).fast_min(m255).to_array();
                    let b_v = (f32x8::from(b_arr) * intensity_v).fast_max(zero_v).fast_min(m255).to_array();
                    for j in 0..8 {
                        let p = (x0 + j) * 4;
                        bot[p]     = r_v[j] as u8;
                        bot[p + 1] = g_v[j] as u8;
                        bot[p + 2] = b_v[j] as u8;
                    }
                }
                for x in simd_end..aw {
                    let p = x * 4;
                    bot[p]     = (bot[p]     as f32 * intensity) as u8;
                    bot[p + 1] = (bot[p + 1] as f32 * intensity) as u8;
                    bot[p + 2] = (bot[p + 2] as f32 * intensity) as u8;
                }
            });
    }


    fn read_hires_memory(
        &self,
        mmu: &Mmu,
        addr: u16,
        mode: u8,
        is_80store: bool,
    ) -> u8 {
        let (is_page2, is_80store) = if self.stable_page {
            (self.live_page2, self.live_80store)
        } else {
            (check_bits_u8!(mode, VideoModeMask::PAGE2), is_80store)
        };

        if is_80store {
            let real_addr = addr.wrapping_add(0x2000);
            if is_page2 {
                mmu.read_aux_byte(real_addr)
            } else {
                mmu.read_main_byte(real_addr)
            }
        } else if is_page2 {
            mmu.read_main_byte(addr.wrapping_add(0x4000))
        } else {
            mmu.read_main_byte(addr.wrapping_add(0x2000))
        }
    }
   

    fn render_text_scanline(
        &mut self,
        _iou: &Iou,
        mmu: &Mmu,
        scanline: usize,
        mode: u8,
        is_80store: bool,
    ) {
        let is_80col = check_bits_u8!(mode, VideoModeMask::COL80);
        let is_altchar = check_bits_u8!(mode, VideoModeMask::ALTCHAR);
        let is_page2 = check_bits_u8!(mode, VideoModeMask::PAGE2);
        let double_width = !is_80col;

        let text_row = (scanline / 8) as u16;
        let char_row = (scanline % 8) as u16;
        let row_base = TEXT_MODE_BASE_ADDRESSES[text_row as usize];

        if is_80col {
            for col_pair in 0..40_u16 {
                let addr = row_base + col_pair;

                // Even column (0, 2, 4...) -> AUX Memory
                let char_even = mmu.read_aux_byte(addr);
                self.draw_char_scanline(text_row, col_pair * 2, char_even, char_row, is_altchar, double_width);

                // Odd column (1, 3, 5...) -> MAIN Memory
                let char_odd = mmu.read_main_byte(addr);
                self.draw_char_scanline(text_row, col_pair * 2 + 1, char_odd, char_row, is_altchar, double_width);
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
                self.draw_char_scanline(text_row, col, vram_code, char_row, is_altchar, double_width);
            }
        }
    }

    fn draw_char_scanline(
        &mut self,
        row: u16,
        col: u16,
        char_code: u8,
        char_row: u16,
        is_altchar: bool,
        double_width: bool,
    ) {
        let (font_offset, mut invert) = apple_iic_font_index(char_code, is_altchar);

        // Flashing range in VRAM: 0x40-0x7F (when not in AltChar/MouseText mode).
        // Flash rate ~2 Hz: 60 fps / 32 ≈ 1.8 Hz.
        if !is_altchar && (0x40..=0x7F).contains(&char_code) {
            let flash_on = (self.frame_count / 16).is_multiple_of(2);
            if !flash_on {
                invert = false;
            }
        }

        let char_width = if double_width { 14 } else { 7 };

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
                    (self.mono_fg[0], self.mono_fg[1], self.mono_fg[2])
                } else {
                    (255, 255, 255)
                }
            } else {
                if self.monochrome {
                    (self.mono_bg[0], self.mono_bg[1], self.mono_bg[2])
                } else {
                    (0, 0, 0)
                }
            };

            if double_width {
                // 2 fb pixels per font bit
                let base_index = bit * 8;
                rgba_row[base_index]     = r;
                rgba_row[base_index + 1] = g;
                rgba_row[base_index + 2] = b;
                rgba_row[base_index + 3] = 255;
                rgba_row[base_index + 4] = r;
                rgba_row[base_index + 5] = g;
                rgba_row[base_index + 6] = b;
                rgba_row[base_index + 7] = 255;
            } else {
                let base_index = bit * 4;
                rgba_row[base_index]     = r;
                rgba_row[base_index + 1] = g;
                rgba_row[base_index + 2] = b;
                rgba_row[base_index + 3] = 255;
            }
        }

        for dy in 0..2 {
            let start_index = self.fb_index(x as usize, y as usize + dy);
            let end_index = start_index + (char_width as usize) * 4;

            if end_index <= self.framebuffer.len() {
                self.framebuffer[start_index..end_index]
                    .copy_from_slice(&rgba_row[0..(char_width as usize * 4)]);
            }
        }
    }

    fn apply_mixed_mode_text_fringing(&mut self, start_text_row: usize) {
        let seam_y = start_text_row * 8 * 2;

        for x in 0..self.active_width {
            let mut top_bright_y: Option<usize> = None;
            for dy in 0..16 {
                let y = seam_y + dy;
                let idx = self.fb_index(x, y);
                if idx + 4 > self.framebuffer.len() { continue; }
                let lum = self.framebuffer[idx] as u16
                        + self.framebuffer[idx + 1] as u16
                        + self.framebuffer[idx + 2] as u16;
                if lum > 400 {
                    top_bright_y = Some(y);
                    break;
                }
            }
            let Some(top_y) = top_bright_y else { continue };

            let fringe = Self::ntsc_fringe_color(x % 4);

            for k in 0..2 {
                let y = top_y + k;
                let fi = self.fb_index(x, y);
                if fi + 4 <= self.framebuffer.len() {
                    self.framebuffer[fi]     = fringe[0];
                    self.framebuffer[fi + 1] = fringe[1];
                    self.framebuffer[fi + 2] = fringe[2];
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


    fn render_lores_scanline(
        &mut self,
        _iou: &Iou,
        mmu: &Mmu,
        scanline: usize,
        mode: u8,
        is_80store: bool,
    ) {
        let is_page2 = (mode & VideoModeMask::PAGE2) != 0;
        let is_80col = (mode & VideoModeMask::COL80) != 0;
        let is_dhires = (mode & VideoModeMask::DHIRES) != 0;
        let is_double_lores = is_80col && is_dhires;
        let mixed_mode = (mode & VideoModeMask::MIXED) != 0;

        let base_vram: u16 = if !is_80store && is_page2 { 0x0800 } else { 0x0400 };

        // The original renderer carved the screen into "half-rows" — each
        // 4 Apple scanlines tall, corresponding to either the low or
        // high nibble of a VRAM byte. Map the input scanline back to its
        // half-row index so the lookup table stays unchanged.
        let half_row = scanline / 4;
        if mixed_mode && half_row >= 40 { return; }
        if half_row >= 48 { return; }

        let base_address = base_vram
            + match half_row / 2 {
                0  => 0x000, 1  => 0x080, 2  => 0x100, 3  => 0x180,
                4  => 0x200, 5  => 0x280, 6  => 0x300, 7  => 0x380,
                8  => 0x028, 9  => 0x0A8, 10 => 0x128, 11 => 0x1A8,
                12 => 0x228, 13 => 0x2A8, 14 => 0x328, 15 => 0x3A8,
                16 => 0x050, 17 => 0x0D0, 18 => 0x150, 19 => 0x1D0,
                20 => 0x250, 21 => 0x2D0, 22 => 0x350, 23 => 0x3D0,
                _  => unreachable!(),
            };

        // 2 fb rows per Apple scanline.
        let y_fb = scanline * 2;

        if is_double_lores {
            for col in 0..80_u16 {
                let mem_addr = base_address + (col / 2);
                let is_aux = (col % 2) == 0;

                let color_byte = if is_aux {
                    mmu.read_aux_byte(mem_addr)
                } else {
                    mmu.read_main_byte(mem_addr)
                };

                let nibble = if half_row.is_multiple_of(2) {
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

                let x = (col * 7) as usize;
                for dy in 0..2 {
                    for dx in 0..7 {
                        let index = self.fb_index(x + dx, y_fb + dy);
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

                let color_code = if half_row.is_multiple_of(2) {
                    color_byte & 0x0F
                } else {
                    (color_byte >> 4) & 0x0F
                };

                let color = self.lores_color_lookup(color_code);

                let x = (col * 14) as usize;
                for dy in 0..2 {
                    for dx in 0..14 {
                        let index = self.fb_index(x + dx, y_fb + dy);
                        if index + 4 <= self.framebuffer.len() {
                            self.framebuffer[index..index + 4].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }

    // Render HiRes mode using direct NTSC artifact color palette lookup.
    // HiRes only has 4 possible artifact colors per palette: violet/green
    // (palette 0) and blue/orange (palette 1)
    fn render_hires_scanline(
        &mut self,
        _iou: &Iou,
        mmu: &Mmu,
        scanline: usize,
        mode: u8,
        is_80store: bool,
    ) {
        let base_vram: u16 = 0x0000;

        let group = scanline / 8;
        let row_in_group = (scanline % 8) as u16;
        let group16 = group as u16;

        let row_base = base_vram
            .wrapping_add(row_in_group.wrapping_mul(1024))
            .wrapping_add((group16 % 8).wrapping_mul(128))
            .wrapping_add((group16 / 8).wrapping_mul(40));

        let y = scanline * 2;

        if self.monochrome {
            for col in 0..40_u16 {
                let addr = row_base.wrapping_add(col);
                let byte = self.read_hires_memory(mmu, addr, mode, is_80store);
                for bit in 0..7_usize {
                    let pixel_on = (byte >> bit) & 1 != 0;
                    let color = if pixel_on { self.mono_fg } else { self.mono_bg };
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
            return;
        }

        let mut comp = [0.0f32; 560];
        for col in 0..40_usize {
            let addr = row_base.wrapping_add(col as u16);
            let byte = self.read_hires_memory(mmu, addr, mode, is_80store);
            let palette = (byte & 0x80) != 0;
            let base_sample = col * 14 + if palette { 1 } else { 0 };
            for bit in 0..7_usize {
                let on = (byte >> bit) & 1 != 0;
                if !on { continue; }
                let s0 = base_sample + bit * 2;
                let s1 = s0 + 1;
                if s0 < 560 { comp[s0] = 1.0; }
                if s1 < 560 { comp[s1] = 1.0; }
            }
        }
        self.ntsc_decode_hires_row(&comp, y);
    }

    fn ntsc_decode_hires_row(&mut self, comp: &[f32; 560], fb_y: usize) {
        const PHI: f32 = std::f32::consts::FRAC_PI_4;

        // Chroma lowpass
        const LP_KERN: [f32; 11] = [
            1.0/32.0, 2.0/32.0, 3.0/32.0, 4.0/32.0, 4.0/32.0,
            4.0/32.0,
            4.0/32.0, 4.0/32.0, 3.0/32.0, 2.0/32.0, 1.0/32.0,
        ];
        const HALF: i32 = 5;

        // Luminance lowpass
        const Y_LP: [f32; 5] = [0.125, 0.25, 0.25, 0.25, 0.125];
        const Y_HALF: i32 = 2;

        // Saturation
        let sat = self.effects.chroma_saturation;
        let luma_scale = self.effects.chroma_luma_scale;

        // Demodulate to baseband I, Q
        let mut i_demod = [0.0f32; 560];
        let mut q_demod = [0.0f32; 560];
        for k in 0..560 {
            let theta = (k as f32) * std::f32::consts::FRAC_PI_2 + PHI;
            let (s, c) = theta.sin_cos();
            i_demod[k] = comp[k] * c;
            q_demod[k] = comp[k] * s;
        }

        // Lowpass I, Q
        let mut i_lp = [0.0f32; 560];
        let mut q_lp = [0.0f32; 560];
        let mut y_lp = [0.0f32; 560];
        for k in 0..560 {
            let mut acc_i = 0.0f32;
            let mut acc_q = 0.0f32;
            for (t, &w) in LP_KERN.iter().enumerate() {
                let src = (k as i32 + t as i32 - HALF).clamp(0, 559) as usize;
                acc_i += i_demod[src] * w;
                acc_q += q_demod[src] * w;
            }
            i_lp[k] = acc_i;
            q_lp[k] = acc_q;

            let mut acc_y = 0.0f32;
            for (t, &w) in Y_LP.iter().enumerate() {
                let src = (k as i32 + t as i32 - Y_HALF).clamp(0, 559) as usize;
                acc_y += comp[src] * w;
            }
            y_lp[k] = acc_y;
        }

        for k in 0..560_usize {
            let i_val = i_lp[k] * sat;
            let q_val = q_lp[k] * sat;

            let chroma_mag = (i_val * i_val + q_val * q_val).sqrt().min(1.0);
            let y_factor = 1.0 + (luma_scale - 1.0) * chroma_mag;
            let y_val = y_lp[k] * y_factor;

            let r = (y_val + 0.9563 * i_val + 0.6210 * q_val).clamp(0.0, 1.0);
            let g = (y_val - 0.2721 * i_val - 0.6474 * q_val).clamp(0.0, 1.0);
            let b = (y_val - 1.1070 * i_val + 1.7046 * q_val).clamp(0.0, 1.0);

            let rb = (r * 255.0 + 0.5) as u8;
            let gb = (g * 255.0 + 0.5) as u8;
            let bb = (b * 255.0 + 0.5) as u8;
            let color = [rb, gb, bb, 255];

            if k < self.active_width {
                let i_top = self.fb_index(k, fb_y);
                let i_bot = self.fb_index(k, fb_y + 1);
                if i_top + 4 <= self.framebuffer.len() {
                    self.framebuffer[i_top..i_top + 4].copy_from_slice(&color);
                }
                if i_bot + 4 <= self.framebuffer.len() {
                    self.framebuffer[i_bot..i_bot + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn apply_chroma_blur(&mut self, y_start: usize, y_end: usize) {
        const I_KERNEL: [f32; 7] = [0.15, 0.2, 0.25, 0.2, 0.1, 0.07, 0.03];
        const Q_KERNEL: [f32; 7] = [0.15, 0.2, 0.25, 0.2, 0.1, 0.07, 0.03];
        const HALF: usize = 3;
        let aw = self.active_width;
        let stride = self.width * 4;
        let border = self.border_size;

        // per-x kernel sums for edge pixels where the kernel is truncated
        // interior pixels share `i_full` / `q_full`.
        let mut i_wsum_left = [0.0f32; HALF];
        let mut q_wsum_left = [0.0f32; HALF];
        let i_full: f32 = I_KERNEL.iter().sum();
        let q_full: f32 = Q_KERNEL.iter().sum();
        for x in 0..HALF {
            let mut si = 0.0f32;
            let mut sq = 0.0f32;
            for k in 0..7 {
                let sx = x as i32 - HALF as i32 + k as i32;
                if sx >= 0 {
                    si += I_KERNEL[k];
                    sq += Q_KERNEL[k];
                }
            }
            i_wsum_left[x] = si;
            q_wsum_left[x] = sq;
        }
        let mut i_wsum_right = [0.0f32; HALF];
        let mut q_wsum_right = [0.0f32; HALF];
        for j in 0..HALF {
            let x = aw - HALF + j;
            let mut si = 0.0f32;
            let mut sq = 0.0f32;
            for k in 0..7 {
                let sx = x as i32 - HALF as i32 + k as i32;
                if sx < aw as i32 {
                    si += I_KERNEL[k];
                    sq += Q_KERNEL[k];
                }
            }
            i_wsum_right[j] = si;
            q_wsum_right[j] = sq;
        }

        let s2l = self.srgb_to_linear_lut;
        let l2s = *self.linear_to_srgb_lut_u8;

        let n_lines = (y_end - y_start) / 2;
        let fb_active_start = (y_start + border) * stride + border * 4;
        let active_bytes = n_lines * 2 * stride;

        use wide::f32x8;
        let inv_i_full = f32x8::splat(1.0 / i_full);
        let inv_q_full = f32x8::splat(1.0 / q_full);
        let i_kern_v: [f32x8; 7] = [
            f32x8::splat(I_KERNEL[0]), f32x8::splat(I_KERNEL[1]), f32x8::splat(I_KERNEL[2]),
            f32x8::splat(I_KERNEL[3]), f32x8::splat(I_KERNEL[4]), f32x8::splat(I_KERNEL[5]),
            f32x8::splat(I_KERNEL[6]),
        ];
        let q_kern_v: [f32x8; 7] = [
            f32x8::splat(Q_KERNEL[0]), f32x8::splat(Q_KERNEL[1]), f32x8::splat(Q_KERNEL[2]),
            f32x8::splat(Q_KERNEL[3]), f32x8::splat(Q_KERNEL[4]), f32x8::splat(Q_KERNEL[5]),
            f32x8::splat(Q_KERNEL[6]),
        ];
        let zero_v = f32x8::splat(0.0);
        let one_v = f32x8::splat(1.0);
        let f4095 = f32x8::splat(4095.0);
        let thresh_white = f32x8::splat(0.85);
        let inv_015 = f32x8::splat(1.0 / 0.15);
        let tint_max = f32x8::splat(0.20);
        let y_boost = f32x8::splat(1.03);
        let wp_v = f32x8::splat(self.effects.white_preservation.clamp(0.0, 1.0));
        let wp_scalar = self.effects.white_preservation.clamp(0.0, 1.0);
        let cy_r = f32x8::splat(0.9563);
        let cy_g = f32x8::splat(-0.2721);
        let cy_b = f32x8::splat(-1.1070);
        let cq_r = f32x8::splat(0.6210);
        let cq_g = f32x8::splat(-0.6474);
        let cq_b = f32x8::splat(1.7046);
        let f0003 = f32x8::splat(0.0003);

        // SIMD interior [HALF, mid_end) uses the full 7-tap kernel.
        // Edges and any tail fall back to the scalar pixel macro.
        let mid_count = (aw - 2 * HALF) / 8;
        let mid_end = HALF + mid_count * 8;

        self.framebuffer[fb_active_start..fb_active_start + active_bytes]
            .par_chunks_mut(2 * stride)
            .for_each_init(
                || (vec![0.0f32; aw], vec![0.0f32; aw], vec![0.0f32; aw]),
                |scratch, chunk| {
                    let y_row = &mut scratch.0;
                    let i_row = &mut scratch.1;
                    let q_row = &mut scratch.2;
                    // sRGB row -> linear YIQ via LUT, in SoA layout.
                    for x in 0..aw {
                        let p = x * 4;
                        let r = s2l[chunk[p] as usize];
                        let g = s2l[chunk[p + 1] as usize];
                        let b = s2l[chunk[p + 2] as usize];
                        y_row[x] = 0.299 * r + 0.587 * g + 0.114 * b;
                        i_row[x] = 0.5959 * r - 0.2746 * g - 0.3213 * b;
                        q_row[x] = 0.2115 * r - 0.5227 * g + 0.3112 * b;
                    }

                    // Scalar fallback for one pixel; used at edges and the SIMD tail.
                    macro_rules! scalar_pixel { ($x:expr) => {{
                        let x = $x;
                        let y_val = y_row[x];
                        let (wsum_i, wsum_q) = if x < HALF {
                            (i_wsum_left[x], q_wsum_left[x])
                        } else if x >= aw - HALF {
                            let j = x - (aw - HALF);
                            (i_wsum_right[j], q_wsum_right[j])
                        } else {
                            (i_full, q_full)
                        };

                        let mut bi = 0.0f32;
                        let mut bq = 0.0f32;
                        let kx_lo = (x as i32 - HALF as i32).max(0) as usize;
                        let kx_hi = (x as i32 - HALF as i32 + 7).min(aw as i32) as usize;
                        let kshift = (kx_lo as i32 - (x as i32 - HALF as i32)) as usize;
                        for (k, sx) in (kx_lo..kx_hi).enumerate() {
                            bi += i_row[sx] * I_KERNEL[k + kshift];
                            bq += q_row[sx] * Q_KERNEL[k + kshift];
                        }

                        let (i_val, q_val);
                        let mut boosted_y = y_val;
                        if y_val > 0.85 {
                            let proximity = ((1.0 - y_val) * (1.0 / 0.15)).clamp(0.0, 1.0);
                            // protected_tint=0.20*prox; with wp=0 use full blur (tint=1).
                            let protected_tint = 0.20f32 * proximity;
                            let tint = wp_scalar * protected_tint + (1.0 - wp_scalar);
                            i_val = i_row[x] * (1.0 - tint) + (bi / wsum_i) * tint;
                            q_val = q_row[x] * (1.0 - tint) + (bq / wsum_q) * tint;
                            let y_boosted = (y_val * 1.03).min(1.0);
                            boosted_y = y_val + (y_boosted - y_val) * wp_scalar;
                        } else {
                            i_val = bi / wsum_i;
                            q_val = bq / wsum_q;
                        }

                        let r_lin = (boosted_y + 0.9563 * i_val + 0.6210 * q_val).clamp(0.0, 1.0);
                        let g_lin = (boosted_y - 0.2721 * i_val - 0.6474 * q_val).clamp(0.0, 1.0);
                        let b_lin = (boosted_y - 1.1070 * i_val + 1.7046 * q_val).clamp(0.0, 1.0);

                        let encode = |c: f32| -> u8 {
                            if c < 0.0003 { 0 } else { l2s[(c * 4095.0) as usize] }
                        };
                        let rb = encode(r_lin);
                        let gb = encode(g_lin);
                        let bb = encode(b_lin);

                        let p_top = x * 4;
                        let p_bot = stride + x * 4;
                        chunk[p_top]     = rb;
                        chunk[p_top + 1] = gb;
                        chunk[p_top + 2] = bb;
                        chunk[p_bot]     = rb;
                        chunk[p_bot + 1] = gb;
                        chunk[p_bot + 2] = bb;
                    }}; }

                    for x in 0..HALF { scalar_pixel!(x); }

                    for blk in 0..mid_count {
                        let x = HALF + blk * 8;

                        let raw_i_arr: [f32; 8] = i_row[x..x + 8].try_into().unwrap();
                        let raw_q_arr: [f32; 8] = q_row[x..x + 8].try_into().unwrap();
                        let y_arr: [f32; 8] = y_row[x..x + 8].try_into().unwrap();
                        let raw_i = f32x8::from(raw_i_arr);
                        let raw_q = f32x8::from(raw_q_arr);
                        let y_v = f32x8::from(y_arr);

                        // Convolve 7 taps; tap k reads from x - 3 + k.
                        let mut bi_v = zero_v;
                        let mut bq_v = zero_v;
                        for k in 0..7 {
                            let off = x + k - HALF;
                            let ii: [f32; 8] = i_row[off..off + 8].try_into().unwrap();
                            let qq: [f32; 8] = q_row[off..off + 8].try_into().unwrap();
                            bi_v = f32x8::from(ii).mul_add(i_kern_v[k], bi_v);
                            bq_v = f32x8::from(qq).mul_add(q_kern_v[k], bq_v);
                        }

                        let i_blur = bi_v * inv_i_full;
                        let q_blur = bq_v * inv_q_full;

                        // White-protection: where Y is near white, blend back toward
                        // raw I/Q so bright pixels don't get tinted by the chroma blur.
                        // Scaled by white_preservation: 1.0 = full protection,
                        // 0.0 = no protection (real NTSC bleed).
                        let white_mask = y_v.cmp_gt(thresh_white);
                        let proximity = ((one_v - y_v) * inv_015).fast_max(zero_v).fast_min(one_v);
                        let protected_tint = tint_max * proximity;
                        // tint = wp*protected_tint + (1-wp); at wp=0, tint=1 -> alpha=0 -> full blur.
                        let tint = protected_tint * wp_v + (one_v - wp_v);
                        let alpha = white_mask.blend(one_v - tint, zero_v);
                        let i_val = i_blur + (raw_i - i_blur) * alpha;
                        let q_val = q_blur + (raw_q - q_blur) * alpha;
                        let y_boosted = (y_v * y_boost).fast_min(one_v);
                        let y_lerped = y_v + (y_boosted - y_v) * wp_v;
                        let y_eff = white_mask.blend(y_lerped, y_v);

                        let r_lin = (y_eff + i_val * cy_r + q_val * cq_r).fast_max(zero_v).fast_min(one_v);
                        let g_lin = (y_eff + i_val * cy_g + q_val * cq_g).fast_max(zero_v).fast_min(one_v);
                        let b_lin = (y_eff + i_val * cy_b + q_val * cq_b).fast_max(zero_v).fast_min(one_v);

                        // Per-lane bitmask for values below the 0.0003 floor;
                        // remaining lanes gather their u8 from the LUT.
                        let r_idx_arr = (r_lin * f4095).fast_trunc_int().to_array();
                        let g_idx_arr = (g_lin * f4095).fast_trunc_int().to_array();
                        let b_idx_arr = (b_lin * f4095).fast_trunc_int().to_array();
                        let r_tiny = r_lin.cmp_lt(f0003).move_mask() as u32;
                        let g_tiny = g_lin.cmp_lt(f0003).move_mask() as u32;
                        let b_tiny = b_lin.cmp_lt(f0003).move_mask() as u32;

                        for j in 0..8 {
                            let bit = 1u32 << j;
                            let rb = if (r_tiny & bit) != 0 { 0 } else { l2s[r_idx_arr[j] as usize] };
                            let gb = if (g_tiny & bit) != 0 { 0 } else { l2s[g_idx_arr[j] as usize] };
                            let bb = if (b_tiny & bit) != 0 { 0 } else { l2s[b_idx_arr[j] as usize] };
                            let p_top = (x + j) * 4;
                            let p_bot = stride + (x + j) * 4;
                            chunk[p_top]     = rb;
                            chunk[p_top + 1] = gb;
                            chunk[p_top + 2] = bb;
                            chunk[p_bot]     = rb;
                            chunk[p_bot + 1] = gb;
                            chunk[p_bot + 2] = bb;
                        }
                    }

                    for x in mid_end..aw { scalar_pixel!(x); }
                },
            );
    }

    fn render_double_hires_scanline(&mut self, _iou: &Iou, mmu: &Mmu, scanline: usize) {
        let base_vram: u16 = 0x2000;

        let group = scanline / 8;
        let row_in_group = (scanline % 8) as u16;
        let group16 = group as u16;

        let row_base = base_vram
            .wrapping_add(row_in_group.wrapping_mul(1024))
            .wrapping_add((group16 % 8).wrapping_mul(128))
            .wrapping_add((group16 / 8).wrapping_mul(40));

        let y = scanline * 2; // double height

        if self.monochrome {
            // monochrome: 560 pixels (1 bit = 1 pixel)
            for col in 0..40_u16 {
                let addr = row_base.wrapping_add(col);
                let aux_byte = mmu.read_aux_byte(addr);
                let main_byte = mmu.read_main_byte(addr);

                for bit in 0..7_u16 {
                    let pixel_on = (aux_byte >> bit) & 1 != 0;
                    let color = if pixel_on { self.mono_fg } else { self.mono_bg };
                    let x = col as usize * 14 + bit as usize;
                    for dy in 0..2 {
                        let index = self.fb_index(x, y + dy);
                        if index + 4 <= self.framebuffer.len() {
                            self.framebuffer[index..index + 4].copy_from_slice(&color);
                        }
                    }
                }
                for bit in 0..7_u16 {
                    let pixel_on = (main_byte >> bit) & 1 != 0;
                    let color = if pixel_on { self.mono_fg } else { self.mono_bg };
                    let x = col as usize * 14 + 7 + bit as usize;
                    for dy in 0..2 {
                        let index = self.fb_index(x, y + dy);
                        if index + 4 <= self.framebuffer.len() {
                            self.framebuffer[index..index + 4].copy_from_slice(&color);
                        }
                    }
                }
            }
            return;
        }


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

        // Extract 4-bit color using a sliding window with phase rotation.
        // Each pixel gets its own nibble from a 4-bit window centered on
        // its position. The phase term rotates which bit maps to which
        // nibble position, so a repeating 4-bit pattern (e.g. 0,0,1,1
        // for blue) maps to the same palette index at every pixel.
        for i in 0..560_usize {
            let phase = i % 4;
            let mut nibble: u8 = 0;
            for j in 0..4_usize {
                if scanline_bits[i + j] {
                    nibble |= 1 << (3 - ((phase + j) % 4));
                }
            }

            let rgba = DHIRES_PALETTE[nibble as usize];

            for dy in 0..2 {
                let index = self.fb_index(i, y + dy);
                if index + 4 <= self.framebuffer.len() {
                    self.framebuffer[index..index + 4].copy_from_slice(&rgba);
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

    // Pre-effects framebuffer snapshot. Captured immediately after
    // per-scanline rendering and before NTSC chroma blur / comb
    // filter / phosphor spread / CPU scanlines are applied. Empty
    // until the first frame has been composed.
    pub fn get_raw_pixels(&self) -> &[u8] {
        if self.raw_framebuffer.is_empty() {
            &self.framebuffer
        } else {
            &self.raw_framebuffer
        }
    }

    fn lores_color_lookup(&self, color: u8) -> [u8; 4] {
        let rgba = NTSC_PALETTE[(color & 0x0F) as usize];

        if self.monochrome {
            let y = (0.299 * rgba[0] as f32 + 0.587 * rgba[1] as f32 + 0.114 * rgba[2] as f32) as u8;
            // When a downstream shader provides its own coloring (LCD),
            // emit a neutral grayscale image so the shader sees true
            // source luminance instead of the user's tint.
            if self.force_neutral_mono {
                return [y, y, y, 255];
            }
            if y < 24 {
                self.mono_bg
            } else {

                let fg = self.mono_fg;
                let r = ((y as u32 * fg[0] as u32) / 255) as u8;
                let g = ((y as u32 * fg[1] as u32) / 255) as u8;
                let b = ((y as u32 * fg[2] as u32) / 255) as u8;
                [r, g, b, 255]
            }
        } else {
            rgba
        }
    }
}
