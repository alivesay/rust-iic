struct GbParams {
    output_size:  vec4<f32>,
    source_size:  vec4<f32>,
    content_rect: vec4<f32>,
    pass1_size:   vec4<f32>,
    config_a:     vec4<f32>,
    config_b:     vec4<f32>,
    config_c:     vec4<f32>,
    config_d:     vec4<f32>,  // contrast, screen_light, pixel_opacity, bg_smoothing
    config_e:     vec4<f32>,  // shadow_opacity, shadow_x, shadow_y, shadow_enable
    config_f:     vec4<f32>,  // screen_x, screen_y, response_time, integer_mode
    panel_extras: vec4<f32>,  // overscan_uv_x, overscan_uv_y, corner_radius_px, ghost_decay
    vignette_params: vec4<f32>, // strength, inner_r, outer_r, _
    vignette_tint:   vec4<f32>, // rgb (sRGB), _
    lcd_extras:      vec4<f32>, // threshold, contrast, _, _
    lcd_bg_color:    vec4<f32>, // rgb (sRGB), _
    lcd_fg_color:    vec4<f32>, // rgb (sRGB), _
};

@group(0) @binding(0) var orig_tex:   texture_2d<f32>;  // pre-filled framebuffer (bezel/overscan colour outside content_rect)
@group(0) @binding(1) var samp:       sampler;
@group(0) @binding(2) var<uniform> P: GbParams;
@group(0) @binding(3) var pass1_tex:  texture_2d<f32>;  // foreground (alpha-blended dots)
@group(0) @binding(4) var pass3_tex:  texture_2d<f32>;  // blurred alpha (shadow)

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

// sRGB → linear conversion. The surface is an sRGB texture so the
// hardware applies linear → sRGB encoding on write; we therefore have
// to feed it linear values to display the intended sRGB colour.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let cutoff = vec3<f32>(0.04045);
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= cutoff);
}

// Target sRGB colours (these are what we want to see on screen).
// Both can be overridden at runtime via the LCD shader UI; defaults
// roughly match the //c flat panel green tint.
fn bg_color() -> vec3<f32> {
    return srgb_to_linear(P.lcd_bg_color.rgb);
}

fn fg_color() -> vec3<f32> {
    return srgb_to_linear(P.lcd_fg_color.rgb);
}

fn shadow_color() -> vec3<f32> {
    return srgb_to_linear(vec3<f32>(48.0 / 255.0, 65.0 / 255.0, 50.0 / 255.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cr_l = P.content_rect.x;
    let cr_t = P.content_rect.y;
    let cr_r = P.content_rect.z;
    let cr_b = P.content_rect.w;

    // Panel rect (in uv) = content_rect inflated by overscan ring.
    let ovx = P.panel_extras.x;
    let ovy = P.panel_extras.y;
    let pl = cr_l - ovx;
    let pt = cr_t - ovy;
    let pr = cr_r + ovx;
    let pb = cr_b + ovy;

    // Outside the LCD panel: bezel (black). Inside panel but outside the
    // content_rect → overscan ring. Outer corners of the panel are
    // rounded by `corner_radius_px` (panel_extras.z).
    let invert = P.config_f.w;

    let ring   = fg_color();

    let out_size = P.output_size.xy;
    let radius_px = max(P.panel_extras.z, 0.0);
    // Convert uv → pixel position for distance test.
    let px = in.uv * out_size;
    let pmin = vec2<f32>(pl, pt) * out_size + vec2<f32>(radius_px);
    let pmax = vec2<f32>(pr, pb) * out_size - vec2<f32>(radius_px);

    if in.uv.x < cr_l || in.uv.x > cr_r || in.uv.y < cr_t || in.uv.y > cr_b {
        // Are we within the rounded panel?
        let in_outer = in.uv.x >= pl && in.uv.x <= pr && in.uv.y >= pt && in.uv.y <= pb;
        if !in_outer {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        // Rounded-corner test: compute distance from clamped center.
        let q = clamp(px, pmin, pmax);
        let d = length(px - q);
        if d > radius_px {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0); // outside rounded corner = bezel
        }
        return vec4<f32>(ring, 1.0);
    }

    let texel = vec2<f32>(P.pass1_size.z, P.pass1_size.w);

    // Screen offset (libretro original: a small global pixel shift).
    let screen_off = vec2<f32>(
        (P.config_f.x - 1.0) * texel.x,
        (P.config_f.y - 1.0) * texel.y,
    );

    // Shadow offset.
    let shadow_off = vec2<f32>(
        P.config_e.y * texel.x,
        P.config_e.z * texel.y,
    );

    let foreground = textureSampleLevel(pass1_tex, samp, in.uv - screen_off, 0.0);
    let shadows    = textureSampleLevel(pass3_tex, samp, in.uv - (shadow_off + screen_off), 0.0);

    let bg = bg_color();
    let fg = fg_color();
    let sh = shadow_color();

    // Invert flag (config_f.w): swap fg/bg roles → lit pixels become the
    // background colour and the gutter / off pixels go dark.
    let body_color   = mix(fg, bg, invert);
    let surface_color = mix(bg, fg, invert);
    let shadow_tint  = mix(sh, sh, 0.0);  // shadow stays dark either way

    // Drop shadow blended into background.
    let contrast      = P.config_d.x;
    let shadow_alpha  = clamp(P.config_e.x * P.config_e.w * shadows.a, 0.0, 1.0);
    let bg_with_shadow = mix(surface_color, shadow_tint, shadow_alpha);

    // Dot intensity: foreground alpha is the brightness*coverage product.
    let dot_alpha = clamp(foreground.a * contrast, 0.0, 1.0);
    var out_rgb = mix(bg_with_shadow, body_color, dot_alpha);

    // LCD vignette: real DMG-style panels have a darker, slightly cool
    // (blue-green) tint toward the edges that fades smoothly into the
    // normal panel colour near the centre. Compute an elliptical distance
    // from the content-rect centre in normalised content space (so the
    // falloff follows the panel's actual aspect ratio).
    let cr_size   = vec2<f32>(cr_r - cr_l, cr_b - cr_t);
    let cr_center = vec2<f32>(cr_l + cr_size.x * 0.5, cr_t + cr_size.y * 0.5);
    let n         = (in.uv - cr_center) / (cr_size * 0.5);   // -1..1 across content
    let r2        = dot(n, n);
    let v_strength = P.vignette_params.x;
    let v_inner    = P.vignette_params.y;
    let v_outer    = max(P.vignette_params.z, v_inner + 0.001);
    let v          = clamp((sqrt(r2) - v_inner) / (v_outer - v_inner), 0.0, 1.0);
    let v_shaped   = v * v;
    let vignette_tint = srgb_to_linear(P.vignette_tint.rgb);
    out_rgb = mix(out_rgb, vignette_tint, v_shaped * v_strength);

    // 8-bit dither: the swapchain is sRGB8, so smooth low-amplitude
    // gradients (the vignette here) quantize into visible bands. Add a
    // tiny triangular-PDF noise (±1 LSB in sRGB space) decorrelated by
    // a hashed pixel position. Done in linear with the sRGB step
    // approximated as 1/255 — close enough that the banding disappears
    // without visibly noisy output.
    let pix = in.uv * P.output_size.xy;
    // Two independent uniform[0,1) hashes from the pixel coordinate.
    let h1 = fract(sin(dot(pix, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let h2 = fract(sin(dot(pix, vec2<f32>(93.9898, 67.345))) * 24634.6345);
    // Triangular distribution in [-1, 1].
    let t = (h1 - h2);
    let dither = t * (1.0 / 255.0);
    out_rgb = out_rgb + vec3<f32>(dither);

    return vec4<f32>(out_rgb, 1.0);
}
