// NTSC chroma processing (Apple //c).
//
// One render pass that performs, in YIQ space:
//   1. Horizontal asymmetric chroma blur on I/Q channels (left-heavy 7-tap;
//      simulates the ~0.5 MHz NTSC chroma bandwidth and the well-known
//      Apple II "rainbow tail" smear).
//   2. Vertical 2-line comb filter on I/Q (bleeds chroma from the previous
//      and next scanline; matches a 2-line comb decoder).
//
// Luma (Y) is preserved so text and pixel edges stay sharp. A white-protection
// branch reduces tinting near saturation. Mono mode uses a wide horizontal
// Gaussian as an anti-aliasing fallback.
//
const PI: f32 = 3.14159265358979;

// NTSC RGB <-> YIQ matrices (FCC primaries / standard).
const RGB_TO_YIQ: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.299,  0.5959,  0.2115),
    vec3<f32>(0.587, -0.2746, -0.5227),
    vec3<f32>(0.114, -0.3213,  0.3112),
);
const YIQ_TO_RGB: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.0,    1.0,    1.0   ),
    vec3<f32>(0.9563,-0.2721,-1.1070),
    vec3<f32>(0.6210,-0.6474, 1.7046),
);

fn rgb_to_yiq(rgb: vec3<f32>) -> vec3<f32> { return RGB_TO_YIQ * rgb; }
fn yiq_to_rgb(yiq: vec3<f32>) -> vec3<f32> { return YIQ_TO_RGB * yiq; }

// NOTE on color space: the source texture is `Rgba8UnormSrgb`, so
// `textureSample` returns LINEAR-light values (auto sRGB->linear decode) and
// the render-target write auto-encodes linear->sRGB. All math here therefore
// runs in linear-light, which matches the CPU path's sRGB-LUT pipeline.
// Do NOT manually srgb_to_linear / linear_to_srgb in this shader.

// 7-tap left-heavy chroma kernel. Sum = 1.0.
// Tap k offsets the source by (k - 3) texels horizontally.
const CHROMA_KERNEL: array<f32, 7> = array<f32, 7>(
    0.15, 0.20, 0.25, 0.20, 0.10, 0.07, 0.03,
);
// Per-neighbor blend weight for the 2-line vertical comb. Applied
// once for the previous scanline and once for the next, so the
// effective previous-line contribution after both passes is
// roughly 0.4 * 0.6 ≈ 0.24 of "current line" replaced by neighbor
// chroma. CPU equivalent stacks chroma_blur then comb (two
// RGB↔YIQ round-trips), which compounds the effect; the shader
// performs both in a single pass, so a slightly higher BLEND here
// lands at the same visible strength as the CPU pipeline.
const COMB_BLEND: f32 = 0.40;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct NtscUniforms {
    // x = filter_strength (0-1), y = source_width, z = source_height, w = is_mono
    params: vec4<f32>,
    // left, top, right, bottom in UV
    content_rect: vec4<f32>,
    // x = chroma_blur on/off, y = comb_filter on/off, z = phosphor_spread on/off,
    // w = white_preservation (1.0 = clean Apple-white, 0.0 = full NTSC bleed)
    toggles: vec4<f32>,
    // x = phosphor_spread sigma_x, y = sigma_y, z = intensity, w = reserved
    // (consumed by phosphor_spread.wgsl; ignored here)
    spread: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: NtscUniforms;

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return out;
}

// Sample I/Q on a single row at integer-pixel offsets and return the
// kernel-weighted blur of (I, Q). Texture samples are already linear-light.
fn sample_iq_row_blur(uv: vec2<f32>, texel_x: f32) -> vec2<f32> {
    var sum = vec2<f32>(0.0);
    for (var k: i32 = 0; k < 7; k = k + 1) {
        let off = f32(k - 3) * texel_x;
        let s = textureSample(r_texture, r_sampler, vec2<f32>(uv.x + off, uv.y)).rgb;
        let yiq = rgb_to_yiq(s);
        sum = sum + vec2<f32>(yiq.y, yiq.z) * CHROMA_KERNEL[k];
    }
    return sum;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let strength = uniforms.params.x;
    let src_w = uniforms.params.y;
    let is_mono = uniforms.params.w > 0.5;
    let cr = uniforms.content_rect;
    let chroma_blur_on = uniforms.toggles.x > 0.5;
    let comb_on = uniforms.toggles.y > 0.5;

    let original = textureSample(r_texture, r_sampler, uv);

    // Outside content rect or fully disabled: pass through unchanged.
    if uv.x < cr.x || uv.x > cr.z || uv.y < cr.y || uv.y > cr.w || strength < 0.01 {
        return original;
    }

    let content_span_x = cr.z - cr.x;
    let texel_x = content_span_x / src_w;
    let texel_y = (cr.w - cr.y) / 192.0; // Apple //c always 192 active scanlines

    // Mono mode: a wide horizontal Gaussian to mask scaler aliasing.
    if is_mono {
        let radius = texel_x * 3.0;
        let step = radius / 6.0;
        var sum = vec3<f32>(0.0);
        var weight_sum = 0.0;
        for (var i: i32 = -6; i <= 6; i = i + 1) {
            let t = f32(i) / 6.0;
            let weight = exp(-t * t * 2.0);
            let s = textureSample(r_texture, r_sampler, vec2<f32>(uv.x + f32(i) * step, uv.y)).rgb;
            sum = sum + s * weight;
            weight_sum = weight_sum + weight;
        }
        let blurred = sum / weight_sum;
        let mix_amt = max(strength, 0.5);
        return vec4<f32>(mix(original.rgb, blurred, mix_amt), original.a);
    }

    // Color path. Samples and writes are linear-light (sRGB framebuffer).
    let center_yiq = rgb_to_yiq(original.rgb);
    var i_val = center_yiq.y;
    var q_val = center_yiq.z;

    if chroma_blur_on {
        let blurred = sample_iq_row_blur(uv, texel_x);
        i_val = blurred.x;
        q_val = blurred.y;
    }

    // 2-line comb: blend toward prev/next scanline's I/Q (blurred if enabled,
    // raw otherwise). Skip neighbors falling outside the content rect.
    if comb_on {
        let uv_prev = vec2<f32>(uv.x, uv.y - texel_y);
        let uv_next = vec2<f32>(uv.x, uv.y + texel_y);

        var prev_iq = vec2<f32>(0.0);
        var next_iq = vec2<f32>(0.0);
        if chroma_blur_on {
            prev_iq = sample_iq_row_blur(uv_prev, texel_x);
            next_iq = sample_iq_row_blur(uv_next, texel_x);
        } else {
            let p = rgb_to_yiq(textureSample(r_texture, r_sampler, uv_prev).rgb);
            let n = rgb_to_yiq(textureSample(r_texture, r_sampler, uv_next).rgb);
            prev_iq = vec2<f32>(p.y, p.z);
            next_iq = vec2<f32>(n.y, n.z);
        }

        let has_prev = uv.y - texel_y >= cr.y;
        let has_next = uv.y + texel_y <= cr.w;
        if has_prev {
            i_val = i_val + (prev_iq.x - i_val) * COMB_BLEND;
            q_val = q_val + (prev_iq.y - q_val) * COMB_BLEND;
        }
        if has_next {
            i_val = i_val + (next_iq.x - i_val) * COMB_BLEND;
            q_val = q_val + (next_iq.y - q_val) * COMB_BLEND;
        }
    }

    // White protection: near saturation, blend back toward raw I/Q so bright
    // pixels do not get tinted by chroma blur. `white_preservation` (toggles.w)
    // scales the effect: 1.0 = full protection (clean Apple-white), 0.0 = no
    // protection (real NTSC chroma bleeds into white). Mirrors video.rs at 1.0.
    let y_val = center_yiq.x;
    var y_eff = y_val;
    let white_pres = clamp(uniforms.toggles.w, 0.0, 1.0);
    if y_val > 0.85 && white_pres > 0.0 {
        let proximity = clamp((1.0 - y_val) * (1.0 / 0.15), 0.0, 1.0);
        // At white_pres=1: tint = 0.20 * proximity (current behavior).
        // At white_pres=0: tint = 1.0 (no protection, full blurred chroma).
        let protected_tint = 0.20 * proximity;
        let tint = mix(1.0, protected_tint, white_pres);
        i_val = mix(center_yiq.y, i_val, tint);
        q_val = mix(center_yiq.z, q_val, tint);
        let y_boosted = min(y_val * 1.03, 1.0);
        y_eff = mix(y_val, y_boosted, white_pres);
    }

    let processed = clamp(yiq_to_rgb(vec3<f32>(y_eff, i_val, q_val)),
                          vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(mix(original.rgb, processed, strength), original.a);
}
