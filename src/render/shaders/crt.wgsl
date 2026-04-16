// CRT-Geom-Deluxe shader ported to WGSL for Apple IIc emulator.
// Based on CRT-Geom-Deluxe by cgwg, Themaister and DOLLS (GPL v2+).
// Features: curvature, Lanczos2 horizontal, 3x oversampled beam profile,
//           halation via blur texture, raster bloom, energy-conserving shadow mask.
//
// Bindings:
//   0: intermediate texture  1: sampler  2: uniforms  3: blur_texture  4: ShaderParams  5: glow_texture

struct ShaderParams {
    group0: vec4<f32>,  // CRTgamma, monitorgamma, d, R
    group1: vec4<f32>,  // cornersize, cornersmooth, overscan_x, overscan_y
    group2: vec4<f32>,  // aperture_strength, aperture_brightboost, scanline_weight, lum
    group3: vec4<f32>,  // curvature_on, saturation, halation, rasterbloom
    group4: vec4<f32>,  // blur_width, mask_type, vignette, phosphor
    group5: vec4<f32>,  // glow, _pad1, _pad2, _pad3
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct Uniforms {
    content_rect: vec4<f32>,
    params: vec4<f32>,
    extra: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
@group(0) @binding(3) var r_blur: texture_2d<f32>;
@group(0) @binding(4) var<uniform> params: ShaderParams;
@group(0) @binding(5) var r_glow: texture_2d<f32>;

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = vec2<f32>(
        (position.x + 1.0) * 0.5,
        (1.0 - position.y) * 0.5,
    );
    return out;
}

const PI: f32 = 3.141592653589;
const ASPECT: vec2<f32> = vec2<f32>(1.0, 0.75);

fn FIX_f(c: f32) -> f32 { return max(abs(c), 1e-5); }

// --- Curvature geometry (same as crt-geom) ---

fn intersect_crt(xy: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> f32 {
    let A = dot(xy, xy) + d * d;
    let B = 2.0 * (R * (dot(xy, sa) - d * ca.x * ca.y) - d * d);
    let C = d * d + 2.0 * R * d * ca.x * ca.y;
    return (-B - sqrt(B * B - 4.0 * A * C)) / (2.0 * A);
}

fn bkwtrans(xy: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec2<f32> {
    let c = intersect_crt(xy, sa, ca, d, R);
    var pt = vec2<f32>(c) * xy;
    pt -= vec2<f32>(-R) * sa;
    pt /= vec2<f32>(R);
    let tang = sa / ca;
    let poc = pt / ca;
    let A = dot(tang, tang) + 1.0;
    let B = -2.0 * dot(poc, tang);
    let C = dot(poc, poc) - 1.0;
    let a = (-B + sqrt(B * B - 4.0 * A * C)) / (2.0 * A);
    let uv = (pt - a * sa) / ca;
    let r = FIX_f(R * acos(a));
    return uv * r / sin(r / R);
}

fn fwtrans(uv: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec2<f32> {
    let r = FIX_f(sqrt(dot(uv, uv)));
    let uv2 = uv * sin(r / R) / r;
    let x = 1.0 - cos(r / R);
    let D = d / R + x * ca.x * ca.y + dot(uv2, sa);
    return d * (uv2 * ca - x * sa) / D;
}

fn maxscale_crt(sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec3<f32> {
    let c = bkwtrans(-R * sa / (1.0 + R / d * ca.x * ca.y), sa, ca, d, R);
    let a = vec2<f32>(0.5, 0.5) * ASPECT;
    let lo = vec2<f32>(
        fwtrans(vec2<f32>(-a.x, c.y), sa, ca, d, R).x,
        fwtrans(vec2<f32>(c.x, -a.y), sa, ca, d, R).y
    ) / ASPECT;
    let hi = vec2<f32>(
        fwtrans(vec2<f32>(a.x, c.y), sa, ca, d, R).x,
        fwtrans(vec2<f32>(c.x, a.y), sa, ca, d, R).y
    ) / ASPECT;
    return vec3<f32>((hi + lo) * ASPECT * 0.5, max(hi.x - lo.x, hi.y - lo.y));
}

fn crt_transform(coord: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>,
                 stretch: vec3<f32>, d: f32, R: f32, ovs: vec2<f32>) -> vec2<f32> {
    let c = (coord - vec2<f32>(0.5)) * ASPECT * stretch.z + stretch.xy;
    return bkwtrans(c, sa, ca, d, R) / ovs / ASPECT + vec2<f32>(0.5);
}

fn corner_mask(coord: vec2<f32>, ovs: vec2<f32>, csize: f32, csmooth: f32) -> f32 {
    let c = (coord - vec2<f32>(0.5)) * ovs + vec2<f32>(0.5);
    let cc = min(c, vec2<f32>(1.0) - c) * ASPECT;
    let cdist = vec2<f32>(csize);
    let dd = cdist - min(cc, cdist);
    let dist = sqrt(dot(dd, dd));
    return clamp((cdist.x - dist) * csmooth, 0.0, 1.0);
}

// --- Scanline beam profile (non-gaussian, 3x oversampled) ---

fn scanline_weights(dist: f32, color: vec4<f32>, sw: f32, lum: f32) -> vec4<f32> {
    let wid = 2.0 + 2.0 * pow(color, vec4<f32>(4.0));
    let w = vec4<f32>(dist / sw);
    return (lum + 1.4) * exp(-pow(w * inverseSqrt(0.5 * wid), wid)) / (0.6 + 0.2 * wid);
}

// --- Sample with CRT gamma ---

fn tex2D_crt(uv: vec2<f32>, g: f32) -> vec4<f32> {
    let underscan = step(vec2<f32>(0.0), uv) * step(vec2<f32>(0.0), vec2<f32>(1.0) - uv);
    let raw = textureSampleLevel(r_texture, r_sampler, uv, 0.0) * vec4<f32>(underscan.x * underscan.y);
    return pow(max(raw, vec4<f32>(0.0)), vec4<f32>(g));
}

// Convert emulator-space [0,1] coords to texture UV
fn emu_to_tex(emu_xy: vec2<f32>, cr_l: f32, cr_t: f32, cs: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(cr_l + emu_xy.x * cs.x, cr_t + emu_xy.y * cs.y);
}

// --- Procedural shadow masks ---

// Aperture grille (vertical RGB stripes, period = 3 pixels)
fn mask_aperture(frag_pos: vec2<f32>) -> vec3<f32> {
    let phase = i32(frag_pos.x) % 3;
    if phase == 0 { return vec3<f32>(1.0, 0.0, 0.0); }
    if phase == 1 { return vec3<f32>(0.0, 1.0, 0.0); }
    return vec3<f32>(0.0, 0.0, 1.0);
}

// Slot mask (3-wide RGB with vertical 2-row offset pattern)
fn mask_slot(frag_pos: vec2<f32>) -> vec3<f32> {
    let row = i32(frag_pos.y) % 4;
    var x_off = 0;
    if row >= 2 { x_off = 1; }
    let phase = (i32(frag_pos.x) + x_off) % 3;
    if phase == 0 { return vec3<f32>(1.0, 0.0, 0.0); }
    if phase == 1 { return vec3<f32>(0.0, 1.0, 0.0); }
    return vec3<f32>(0.0, 0.0, 1.0);
}

// Delta / shadow mask (triangular arrangement)
fn mask_delta(frag_pos: vec2<f32>) -> vec3<f32> {
    let row = i32(frag_pos.y) % 3;
    let col = (i32(frag_pos.x) + row) % 3;
    if col == 0 { return vec3<f32>(1.0, 0.0, 0.0); }
    if col == 1 { return vec3<f32>(0.0, 1.0, 0.0); }
    return vec3<f32>(0.0, 0.0, 1.0);
}

fn get_mask(frag_pos: vec2<f32>, mask_type: f32) -> vec3<f32> {
    let mt = i32(mask_type);
    if mt == 2 { return mask_slot(frag_pos); }
    if mt == 3 { return mask_delta(frag_pos); }
    return mask_aperture(frag_pos);
}

// --- Main fragment shader ---

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let cr_left  = uniforms.content_rect.x;
    let cr_top   = uniforms.content_rect.y;
    let cr_right = uniforms.content_rect.z;
    let cr_bot   = uniforms.content_rect.w;
    let src_h    = uniforms.params.y;
    let src_w    = uniforms.params.w;

    // Pass through outside content rect
    if uv.x < cr_left || uv.x > cr_right || uv.y < cr_top || uv.y > cr_bot {
        return textureSampleLevel(r_texture, r_sampler, uv, 0.0);
    }

    // Unpack params
    let CRTgamma  = params.group0.x;
    let mgamma    = params.group0.y;
    let d         = params.group0.z;
    let R         = params.group0.w;
    let csize     = params.group1.x;
    let csmooth   = params.group1.y;
    let ovs_x     = params.group1.z / 100.0;
    let ovs_y     = params.group1.w / 100.0;
    let ap_str_raw = params.group2.x;  // aperture_strength
    let ap_boost  = params.group2.y;  // aperture_brightboost
    // Disable shadow mask in monochrome mode (no phosphor triad on mono CRTs)
    let is_mono   = uniforms.extra.x;
    let ap_str    = select(ap_str_raw, 0.0, is_mono > 0.5);
    let sw        = params.group2.z;  // scanline_weight
    let lum       = params.group2.w;  // luminance / geom_lum
    let curv_on   = params.group3.x;
    let SATURATION = params.group3.y;
    let halation_amt = params.group3.z;
    let rbloom_amt = params.group3.w / 10.0;  // deluxe divides by 10
    let blur_w    = params.group4.x;
    let mask_type = params.group4.y;
    let vignette_amt = params.group4.z;
    let glow_amt  = params.group5.x;

    let ovs = vec2<f32>(ovs_x, ovs_y);
    let input_size = vec2<f32>(src_w, src_h);
    let content_span = vec2<f32>(cr_right - cr_left, cr_bot - cr_top);

    // Emulator-local [0,1]
    let emu_uv = vec2<f32>(
        (uv.x - cr_left) / content_span.x,
        (uv.y - cr_top) / content_span.y,
    );

    // Curvature
    let sa = vec2<f32>(0.001, 0.001);
    let ca = vec2<f32>(1.001, 1.001);
    let stretch = maxscale_crt(sa, ca, d, R);

    var xy: vec2<f32>;
    if curv_on > 0.5 {
        xy = crt_transform(emu_uv, sa, ca, stretch, d, R, ovs);
    } else {
        xy = (emu_uv - vec2<f32>(0.5)) / ovs + vec2<f32>(0.5);
    }

    let cval = corner_mask(xy, ovs, csize, csmooth);

    // --- Raster bloom: expand/contract based on average brightness ---
    // Sample from intermediate texture (has full mip chain) not blur (no mipmaps)
    let avgbright = dot(textureSampleLevel(r_texture, r_sampler, vec2<f32>(0.5, 0.5), 9.0).rgb, vec3<f32>(1.0)) / 3.0;
    let rbloom = 1.0 - rbloom_amt * (avgbright - 0.5);
    let xy_bloomed = (xy - vec2<f32>(0.5)) * rbloom + vec2<f32>(0.5);
    let xy0 = xy_bloomed;  // save for halation sampling

    // Apple II: non-interlaced
    let ilfac = vec2<f32>(1.0, 1.0);
    let one = ilfac / input_size;

    let ratio_scale = (xy_bloomed * input_size - vec2<f32>(0.5)) / ilfac;

    // Oversample filter width for 3x beam oversampling
    // Approximate fwidth via content_span and output resolution
    let output_size = vec2<f32>(textureDimensions(r_texture));
    let dxy = 1.0 / (content_span.y * output_size.y);
    let oversample_filter = dxy * input_size.y;

    let uv_ratio = fract(ratio_scale);

    // Snap to texel center
    let snapped = (floor(ratio_scale) * ilfac + vec2<f32>(0.5)) / input_size;

    // Convert snapped emu-space coords to texture UV for sampling

    // --- Lanczos2 horizontal filtering (4-tap) ---
    let coeffs_raw = vec4<f32>(
        1.0 + uv_ratio.x,
        uv_ratio.x,
        1.0 - uv_ratio.x,
        2.0 - uv_ratio.x,
    ) * PI;
    let coeffs_fix = max(abs(coeffs_raw), vec4<f32>(1e-5));
    let lanczos = 2.0 * sin(coeffs_fix) * sin(coeffs_fix / 2.0) / (coeffs_fix * coeffs_fix);
    let coeffs = lanczos / dot(lanczos, vec4<f32>(1.0));

    // Sample 4 horizontal texels for current and next scanline
    let s0 = emu_to_tex(snapped + vec2<f32>(-one.x, 0.0), cr_left, cr_top, content_span);
    let s1 = emu_to_tex(snapped, cr_left, cr_top, content_span);
    let s2 = emu_to_tex(snapped + vec2<f32>(one.x, 0.0), cr_left, cr_top, content_span);
    let s3 = emu_to_tex(snapped + vec2<f32>(2.0 * one.x, 0.0), cr_left, cr_top, content_span);

    let col = clamp(
        tex2D_crt(s0, CRTgamma) * coeffs.x +
        tex2D_crt(s1, CRTgamma) * coeffs.y +
        tex2D_crt(s2, CRTgamma) * coeffs.z +
        tex2D_crt(s3, CRTgamma) * coeffs.w,
        vec4<f32>(0.0), vec4<f32>(1.0)
    );

    let s0b = emu_to_tex(snapped + vec2<f32>(-one.x, one.y), cr_left, cr_top, content_span);
    let s1b = emu_to_tex(snapped + vec2<f32>(0.0, one.y), cr_left, cr_top, content_span);
    let s2b = emu_to_tex(snapped + vec2<f32>(one.x, one.y), cr_left, cr_top, content_span);
    let s3b = emu_to_tex(snapped + vec2<f32>(2.0 * one.x, one.y), cr_left, cr_top, content_span);

    let col2 = clamp(
        tex2D_crt(s0b, CRTgamma) * coeffs.x +
        tex2D_crt(s1b, CRTgamma) * coeffs.y +
        tex2D_crt(s2b, CRTgamma) * coeffs.z +
        tex2D_crt(s3b, CRTgamma) * coeffs.w,
        vec4<f32>(0.0), vec4<f32>(1.0)
    );

    // --- 3x oversampled beam profile ---
    var uv_y = uv_ratio.y;
    var w1 = scanline_weights(uv_y, col, sw, lum);
    var w2 = scanline_weights(1.0 - uv_y, col2, sw, lum);

    uv_y = uv_ratio.y + 1.0 / 3.0 * oversample_filter;
    w1 = (w1 + scanline_weights(uv_y, col, sw, lum)) / 3.0;
    w2 = (w2 + scanline_weights(abs(1.0 - uv_y), col2, sw, lum)) / 3.0;

    uv_y = uv_ratio.y - 2.0 / 3.0 * oversample_filter;
    w1 = w1 + scanline_weights(abs(uv_y), col, sw, lum) / 3.0;
    w2 = w2 + scanline_weights(abs(1.0 - uv_y), col2, sw, lum) / 3.0;

    var mul_res = (col * w1 + col2 * w2).rgb;

    // --- Halation: ADDITIVE blend of pre-blurred glow ---
    let blur_uv = emu_to_tex(xy0, cr_left, cr_top, content_span);
    let blur_raw = textureSampleLevel(r_blur, r_sampler, blur_uv, 0.0).rgb;
    let blur = pow(max(blur_raw, vec3<f32>(0.0)), vec3<f32>(CRTgamma));

    // --- Fullscreen CRT glow: larger, softer bloom ---
    // Glow is already gamma-correct from gauss.wgsl, convert to linear for additive blend
    let glow_raw = textureSampleLevel(r_glow, r_sampler, blur_uv, 0.0).rgb;
    var glow = pow(max(glow_raw, vec3<f32>(0.0)), vec3<f32>(2.2));
    
    // Boost glow saturation to preserve color when averaging blurs out hue
    // This makes the glow tinted by the dominant colors rather than grey/white
    let glow_lum = dot(glow, vec3<f32>(0.2126, 0.7152, 0.0722));
    let glow_sat_boost = 2.0;  // Boost saturation 2x to counteract blur desaturation
    glow = mix(vec3<f32>(glow_lum), glow, glow_sat_boost);
    glow = max(glow, vec3<f32>(0.0));  // Clamp negative values from over-saturation

    // Add halation to base image (halation goes through shadow mask)
    // Glow is added AFTER shadow mask to avoid mask pattern in the soft bloom
    mul_res = (mul_res + blur * halation_amt) * vec3<f32>(cval * rbloom);

    // --- Energy-conserving shadow mask (from deluxe) ---
    // Halve position for HiDPI/Retina (physical pixels are 2x logical)
    let mask_pos = in.position.xy * 0.5;
    let mask_rgb = get_mask(mask_pos, mask_type);

    // Fraction of bright subpixels (1/3 for all our masks)
    let fbright = 1.0 / 3.0;
    // Average darkening factor
    let aperture_average = mix(1.0 - ap_str * (1.0 - ap_boost), 1.0, fbright);
    // Dark mask pixel color
    let clow = (1.0 - ap_str) * mul_res + ap_str * ap_boost * mul_res * mul_res;
    let ifbright = 1.0 / fbright;
    // Bright mask pixel color (energy-conserving)
    let chi = ifbright * aperture_average * mul_res - (ifbright - 1.0) * clow;
    var cout_masked = mix(clow, chi, mask_rgb);

    // Add fullscreen glow AFTER shadow mask (glow bypasses mask for smooth bloom)
    cout_masked = cout_masked + glow * glow_amt * 0.1 * cval * rbloom;

    // Output gamma
    var result = pow(max(cout_masked, vec3<f32>(0.0)), vec3<f32>(1.0 / mgamma));

    // Saturation
    let l = dot(result, vec3<f32>(0.2126, 0.7152, 0.0722));
    result = mix(vec3<f32>(l), result, SATURATION);

    // Vignette effect (soft darkening towards edges, never full black)
    if vignette_amt > 0.0 {
        // Distance from center in emulator UV space
        let vig_uv = emu_uv - vec2<f32>(0.5);
        // Squared distance (0 at center, 0.5 at corners)
        let vig_dist = dot(vig_uv, vig_uv);
        // Smooth falloff - starts early (0.05) for larger coverage
        // vignette_amt controls both intensity and spread
        let falloff = smoothstep(0.0, 0.3, vig_dist);
        // Allow up to 85% darkening at max slider, min brightness 15%
        let max_darken = min(vignette_amt * 0.425, 0.85);
        let vig = 1.0 - max_darken * falloff;
        result = result * vig;
    }

    return vec4<f32>(clamp(result, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
