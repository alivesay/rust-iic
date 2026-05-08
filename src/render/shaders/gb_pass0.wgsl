struct GbParams {
    output_size:  vec4<f32>,
    source_size:  vec4<f32>,
    content_rect: vec4<f32>,
    pass1_size:   vec4<f32>,
    config_a:     vec4<f32>,  // pixel_size, pixel_softness, sharpening, pixel_shape
    config_b:     vec4<f32>,  // sharp_mode, color_toggle, palette, baseline_alpha
    config_c:     vec4<f32>,  // grey_balance, brightness_mode, blending_mode, adjacent_blend
    config_d:     vec4<f32>,
    config_e:     vec4<f32>,
    config_f:     vec4<f32>,
    panel_extras: vec4<f32>,
    vignette_params: vec4<f32>,
    vignette_tint:   vec4<f32>,
    lcd_extras:      vec4<f32>,
    lcd_bg_color:    vec4<f32>,
    lcd_fg_color:    vec4<f32>,
};

@group(0) @binding(0) var src_tex:    texture_2d<f32>;
@group(0) @binding(1) var src_samp:   sampler;
@group(0) @binding(2) var<uniform> P: GbParams;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(position, 0.0, 1.0);
    o.uv  = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return o;
}

// 1D segment overlap.
fn seg(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    return max(min(a1, b1) - max(a0, b0), 0.0);
}

// 2D rectangle area intersection.
fn rect_area(px: vec4<f32>, dt: vec4<f32>) -> f32 {
    let bl = max(px.xy, dt.xy);
    let tr = min(px.zw, dt.zw);
    let c  = max(tr - bl, vec2<f32>(0.0));
    return c.x * c.y;
}

// Coverage of a single output pixel by the dot at the source-pixel center.
// `frac` = output-pixel position within source-pixel cell (0..1).
fn dot_coverage(frac: vec2<f32>) -> f32 {
    let pixel_size       = P.config_a.x;
    let pixel_softness   = P.config_a.y;
    let sharpening_amount = P.config_a.z;
    let pixel_shape      = P.config_a.w;
    let sharp_mode       = P.config_b.x;

    // Output pixel rect in cell-space (1×1 cell, dot centered at 0.5).
    let pc = frac - 0.5;
    let px = vec4<f32>(pc - 0.5, pc + 0.5);
    let dh = vec2<f32>(pixel_size * 0.5);
    let dt = vec4<f32>(-dh, dh);

    // Rectangular separable.
    let xc = seg(px.x, px.z, dt.x, dt.z);
    let yc = seg(px.y, px.w, dt.y, dt.w);
    let rect_lin = xc * yc;

    var rect_sharp: f32;
    if sharp_mode < 0.5 {
        let s = 1.0 / max(pixel_softness, 0.001);
        rect_sharp = pow(xc, s) * pow(yc, s);
    } else {
        let k = 10.0 / max(pixel_softness, 0.001);
        let xs = 1.0 / (1.0 + exp(-k * (xc - 0.5)));
        let ys = 1.0 / (1.0 + exp(-k * (yc - 0.5)));
        rect_sharp = xs * ys;
    }
    let rect = mix(rect_lin, rect_sharp, sharpening_amount);

    // Circular (2D area).
    let circ_lin = rect_area(px, dt);
    var circ_sharp: f32;
    if sharp_mode < 0.5 {
        circ_sharp = pow(circ_lin, 1.0 / max(pixel_softness, 0.001));
    } else {
        let k = 10.0 / max(pixel_softness, 0.001);
        circ_sharp = 1.0 / (1.0 + exp(-k * (circ_lin - 0.5)));
    }
    let circ = mix(circ_lin, circ_sharp, sharpening_amount);

    return mix(circ, rect, pixel_shape);
}

// Foreground palette colour for dot pixels.
fn foreground_color() -> vec3<f32> {
    let p = P.config_b.z;
    if      p < 0.5 { return vec3<f32>(0.067, 0.098, 0.133); } // 0: source-tex passthrough handled by color_toggle
    else if p < 1.5 { return vec3<f32>(0.067, 0.098, 0.133); } // 1: #111922
    else if p < 2.5 { return vec3<f32>(0.125, 0.125, 0.125); } // 2: #202020
    else if p < 3.5 { return vec3<f32>(0.000, 0.000, 0.000); } // 3: #000000
    else if p < 4.5 { return vec3<f32>(0.114, 0.416, 0.420); } // 4: #1D6A6B (DMG dark teal-ish)
    else if p < 5.5 { return vec3<f32>(0.000, 0.325, 0.200); } // 5: #005333
    else            { return vec3<f32>(0.000, 0.325, 0.314); } // 6: #005350
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cr_l = P.content_rect.x;
    let cr_t = P.content_rect.y;
    let cr_r = P.content_rect.z;
    let cr_b = P.content_rect.w;

    // Outside the LCD panel: emit zero alpha so the blur passes (pass1–3)
    // don't see the bezel/overscan as a fully-lit dot row and bleed it
    // into the panel edge. The bezel/overscan colour is composited
    // directly from orig_tex in pass4.
    if in.uv.x < cr_l || in.uv.x > cr_r || in.uv.y < cr_t || in.uv.y > cr_b {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let cs = vec2<f32>(cr_r - cr_l, cr_b - cr_t);
    let emu = (in.uv - vec2<f32>(cr_l, cr_t)) / cs;

    let src_w = P.source_size.x;
    let src_h = P.source_size.y;

    // Source-pixel coordinate (continuous), and integer/fractional split.
    let tx_coord = emu * vec2<f32>(src_w, src_h);
    let cell     = floor(tx_coord);
    let frac     = tx_coord - cell;

    // Sample the dot's source pixel (nearest). The framebuffer is
    // 384 rows but our LCD dot grid is 192 rows (each dot covers a
    // doubled row pair). Apple II rendering may write a sprite to only
    // one of the two doubled rows, so we sample both halves of the
    // source row pair and OR them — otherwise pixels rendered into
    // odd-only or even-only rows would be invisible.
    let upper = vec2<f32>(cr_l, cr_t) + ((cell + vec2<f32>(0.5, 0.25)) / vec2<f32>(src_w, src_h)) * cs;
    let lower = vec2<f32>(cr_l, cr_t) + ((cell + vec2<f32>(0.5, 0.75)) / vec2<f32>(src_w, src_h)) * cs;
    let su = textureSampleLevel(src_tex, src_samp, upper, 0.0);
    let sl = textureSampleLevel(src_tex, src_samp, lower, 0.0);
    let s  = max(su, sl);

    // The Apple //c flat panel was driven from the digital TTL video
    // expansion port (1-bit, no NTSC), so video.rs forces monochrome
    // when the LCD shader is active. Luma at this point is therefore
    // ~binary; threshold + contrast instead grade the *dot coverage*
    // (which IS continuous, courtesy of pixel_size/softness/shape).
    let luma_raw = 0.2126 * s.r + 0.7152 * s.g + 0.0722 * s.b;
    let lit = select(0.0, 1.0, luma_raw > 0.5);

    // Coverage of this output pixel by the dot (0..1, graded).
    let cov_raw = dot_coverage(frac);

    // Apply contrast around 0.5 then test the threshold. With
    // contrast = 1 and threshold = 0.5 the behaviour matches the
    // unmodified dot-matrix coverage (graded sub-pixel anti-aliasing).
    // contrast > 1 hardens edges; threshold shifts how much coverage
    // counts as "lit".
    let contrast  = P.lcd_extras.y;
    let threshold = P.lcd_extras.x;
    let cov = clamp(0.5 + (cov_raw - 0.5) * contrast, 0.0, 1.0);
    let alpha = lit * smoothstep(threshold - 0.01, threshold + 0.01, cov);

    // Foreground colour (palette or source). For 1-bit mode the colour
    // is unused — pass4 picks fg/bg from constants — but we keep the
    // call so the bind group layout stays stable.
    var rgb: vec3<f32>;
    if P.config_b.y < 0.5 {
        rgb = foreground_color();
    } else {
        rgb = s.rgb;
    }

    return vec4<f32>(rgb, alpha * cov);
}
