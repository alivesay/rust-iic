// CRT post-processing shader for Apple IIc emulator.
//
// Pipeline: scaling_renderer → intermediate (mipmapped) → bloom (1/4 res) → this shader → screen
//
// Effects applied (in order):
//   1. Barrel distortion with soft edge fade
//   2. Per-scanline horizontal jitter
//   3. Chromatic aberration (RGB channel offset)
//   4. Phosphor bloom (additive, from pre-blurred texture)
//   5. Phosphor mask (RGB triads)
//   6. Brightness / contrast / saturation
//   7. Temporal flicker
//   8. Analog noise
//   9. Vignette
//
// Scanlines are applied CPU-side (video.rs). The status bar and any
// letterbox/pillarbox regions are passed through unmodified.

// --- Tunable constants ---
const CURVATURE: f32        = 0.08;    // Barrel distortion strength
const CHROMA_SHIFT: f32     = 0.3;     // Chromatic aberration (texels)
const MASK_STRENGTH: f32    = 0.08;    // Phosphor mask intensity
const BRIGHTNESS: f32       = 1.08;
const CONTRAST: f32         = 1.1;
const SATURATION: f32       = 1.05;
const FLICKER_AMOUNT: f32   = 0.02;    // Global brightness wobble
const VIGNETTE_STRENGTH: f32 = 0.4;
const HJITTER_AMOUNT: f32   = 0.00008; // Per-scanline horizontal jitter
const NOISE_AMOUNT: f32     = 0.03;    // Analog noise intensity
const GLOW_STRENGTH: f32    = 0.35;    // Bloom mix strength
const EDGE_WIDTH: f32       = 0.005;   // Soft-clip fade width at CRT edge

// --- Types ---

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct Uniforms {
    content_rect: vec4<f32>,  // UV bounds of scaled content (left, top, right, bottom)
    params: vec4<f32>,        // (bar_uv_y, source_height, time, source_width)
};

// --- Bindings ---

@group(0) @binding(0) var r_texture: texture_2d<f32>;  // Intermediate (mipmapped)
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
@group(0) @binding(3) var r_bloom: texture_2d<f32>;     // Bloom (1/4 res, blurred)

// --- Vertex shader (fullscreen triangle) ---

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

// --- Helper functions ---

// Barrel distortion in normalised emulator space, aspect-corrected.
fn barrel_distort(uv: vec2<f32>, amount: f32, aspect: f32) -> vec2<f32> {
    var c = uv - 0.5;
    c.y /= aspect;
    c *= 1.0 + amount * dot(c, c);
    c.y *= aspect;
    return c + 0.5;
}

// Map emulator-local [0,1]² UV back to full-texture UV.
fn emu_to_tex(emu: vec2<f32>, cr_left: f32, cr_top: f32, cr_right: f32, bar_y: f32) -> vec2<f32> {
    return vec2<f32>(
        cr_left + emu.x * (cr_right - cr_left),
        cr_top  + emu.y * (bar_y   - cr_top),
    );
}

fn adjust_bcs(color: vec3<f32>) -> vec3<f32> {
    var c = color * BRIGHTNESS;
    c = (c - 0.5) * CONTRAST + 0.5;
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(luma), c, SATURATION);
}

fn vignette(uv: vec2<f32>) -> f32 {
    let d = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
    return 1.0 - VIGNETTE_STRENGTH * d * d;
}

fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn scanline_jitter(line: f32, t: f32) -> f32 {
    return hash12(vec2<f32>(line * 31.17, t * 7.23)) * 2.0 - 1.0;
}

// Clamp a texture UV to stay half a texel inside the content rect.
fn clamp_uv(uv: vec2<f32>, lo: vec2<f32>, hi: vec2<f32>, half_px: vec2<f32>) -> vec2<f32> {
    return clamp(uv, lo + half_px, hi - half_px);
}

// --- Fragment shader ---

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;

    // Unpack uniforms
    let cr_left  = uniforms.content_rect.x;
    let cr_top   = uniforms.content_rect.y;
    let cr_right = uniforms.content_rect.z;
    let cr_bot   = uniforms.content_rect.w;
    let bar_y    = uniforms.params.x;
    let src_h    = uniforms.params.y;
    let time     = uniforms.params.z;
    let src_w    = uniforms.params.w;

    // Pass through letterbox / pillarbox
    if uv.x < cr_left || uv.x > cr_right || uv.y < cr_top || uv.y > cr_bot {
        return textureSample(r_texture, r_sampler, uv);
    }

    // Pass through status bar
    if uv.y > bar_y {
        return textureSample(r_texture, r_sampler, uv);
    }

    // Remap to emulator-local [0,1]²
    let emu_uv = vec2<f32>(
        (uv.x - cr_left) / (cr_right - cr_left),
        (uv.y - cr_top)  / (bar_y    - cr_top),
    );

    // 1. Barrel distortion
    let curved = barrel_distort(emu_uv, CURVATURE, src_h / src_w);

    // Soft edge fade (smoothstep to black over EDGE_WIDTH)
    let edge = min(
        min(curved.x, 1.0 - curved.x),
        min(curved.y, 1.0 - curved.y),
    );
    if edge < -EDGE_WIDTH {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let fade = smoothstep(0.0, EDGE_WIDTH, edge);
    let emu = clamp(curved, vec2<f32>(0.0), vec2<f32>(1.0));

    // 2. Horizontal jitter
    let scan = floor(emu.y * src_h);
    let jit  = scanline_jitter(scan, floor(time * 60.0)) * HJITTER_AMOUNT;
    let emu_j = vec2<f32>(emu.x + jit, emu.y);

    // Map to texture UV and compute clamping bounds
    let tex_uv = emu_to_tex(emu_j, cr_left, cr_top, cr_right, bar_y);
    let tex_sz = vec2<f32>(textureDimensions(r_texture));
    let half_px = 0.5 / tex_sz;
    let uv_lo = vec2<f32>(cr_left, cr_top);
    let uv_hi = vec2<f32>(cr_right, bar_y);
    let cuv = clamp_uv(tex_uv, uv_lo, uv_hi, half_px);

    // 3. Base sample + chromatic aberration
    var color = textureSample(r_texture, r_sampler, cuv).rgb;
    let ca = CHROMA_SHIFT / tex_sz.x;
    color.r = mix(color.r, textureSample(r_texture, r_sampler, clamp_uv(cuv + vec2(-ca, 0.0), uv_lo, uv_hi, half_px)).r, 0.3);
    color.b = mix(color.b, textureSample(r_texture, r_sampler, clamp_uv(cuv + vec2( ca, 0.0), uv_lo, uv_hi, half_px)).b, 0.3);

    // 4. Phosphor bloom
    color += textureSample(r_bloom, r_sampler, tex_uv).rgb * GLOW_STRENGTH;

    // 5. Phosphor mask (RGB triads)
    let col = i32(in.position.x) % 3;
    var mask = vec3<f32>(1.0);
    if col == 0      { mask = vec3(1.0, 1.0 - MASK_STRENGTH, 1.0 - MASK_STRENGTH); }
    else if col == 1 { mask = vec3(1.0 - MASK_STRENGTH, 1.0, 1.0 - MASK_STRENGTH); }
    else             { mask = vec3(1.0 - MASK_STRENGTH, 1.0 - MASK_STRENGTH, 1.0); }
    color *= mask;

    // 6. Brightness / contrast / saturation
    color = adjust_bcs(color);

    // 7. Flicker
    color *= 1.0 - FLICKER_AMOUNT * sin(time * 3.7)
                 - FLICKER_AMOUNT * 0.6 * sin(time * 1.13 + 2.0);

    // 8. Analog noise
    let seed = vec2<f32>(in.position.x * 1.5 + time * 131.1,
                         in.position.y * 1.7 + time * 97.3);
    color += (hash12(seed) - 0.5) * NOISE_AMOUNT;

    // 9. Vignette + edge fade
    color *= vignette(emu) * fade;

    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
