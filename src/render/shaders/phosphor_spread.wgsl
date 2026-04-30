// Phosphor beam-spot spread (Apple //c).
//
// 2D Gaussian-ish 9-tap kernel applied per fragment. Models the CRT
// electron beam spot size — each phosphor dot is slightly excited by
// all eight of its neighbors, which softens the four corners of each
// rendered pixel into a rounded "dot" rather than a sharp square.
// Mirrors the software path in `src/video.rs::apply_phosphor_spread`,
// extended to 2D for pixel-corner rounding.
//
// Reuses `NtscUniforms` so it can share the NTSC pipeline's bind group layout
// and uniform buffer; only `params.y/z` (source width/height) and
// `content_rect` are consulted here. `toggles.z` is the on/off switch driven
// by ShaderParams.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct NtscUniforms {
    params: vec4<f32>,
    content_rect: vec4<f32>,
    toggles: vec4<f32>,
    // x = sigma_x (src px), y = sigma_y (src px), z = intensity (0..1), w = reserved
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let cr = uniforms.content_rect;
    let src_w = uniforms.params.y;
    let src_h = uniforms.params.z;
    let on = uniforms.toggles.z > 0.5;

    let center = textureSample(r_texture, r_sampler, uv);

    if !on || uv.x < cr.x || uv.x > cr.z || uv.y < cr.y || uv.y > cr.w {
        return center;
    }

    // Separable Gaussian sampled at HALF-source-pixel steps.
    //
    // The intermediate texture is nearest-neighbor upscaled from
    // source pixels, so all output texels inside one source-pixel
    // cell hold the same value. Sampling at integer source-pixel
    // offsets (uv ± k·texel) lands the bilinear sampler in the same
    // cell pair regardless of where the fragment is within the cell
    // → output is constant per source pixel (visible staircase).
    // Sampling at HALF-pixel offsets puts every tap on a cell
    // boundary so the bilinear sampler interpolates between two
    // cells, and the interpolation weight varies continuously with
    // sub-cell fragment position. Result: smooth, screen-pixel-
    // resolution falloff with no grid/staircase.
    //
    // sigma_x / sigma_y / intensity come from ShaderParams (F7 panel).
    let sigma_x = max(uniforms.spread.x, 0.001);
    let sigma_y = max(uniforms.spread.y, 0.0);
    let intensity = clamp(uniforms.spread.z, 0.0, 1.0);
    if intensity <= 0.0 {
        return center;
    }

    let texel_x = (cr.z - cr.x) / src_w;
    let texel_y = (cr.w - cr.y) / src_h;
    let inv_2sx2 = 1.0 / (2.0 * sigma_x * sigma_x);
    let has_y = sigma_y > 0.0001;
    let inv_2sy2 = select(0.0, 1.0 / (2.0 * sigma_y * sigma_y), has_y);

    // ±9 half-steps = ±4.5 source pixels. At sigma_x = 1.4 the
    // weight at 4.5 is ≈ 0.0006 — well below visible threshold.
    var sum = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var ix: i32 = -9; ix <= 9; ix = ix + 1) {
        let fx = f32(ix) * 0.5;
        let wx = exp(-fx * fx * inv_2sx2);
        let sx = clamp(uv.x + fx * texel_x, cr.x, cr.z);
        if !has_y {
            let s = textureSample(r_texture, r_sampler, vec2<f32>(sx, uv.y)).rgb;
            sum = sum + s * wx;
            weight_sum = weight_sum + wx;
        } else {
            for (var iy: i32 = -3; iy <= 3; iy = iy + 1) {
                let fy = f32(iy) * 0.5;
                let wy = exp(-fy * fy * inv_2sy2);
                let sy = clamp(uv.y + fy * texel_y, cr.y, cr.w);
                let s = textureSample(r_texture, r_sampler, vec2<f32>(sx, sy)).rgb;
                let w = wx * wy;
                sum = sum + s * w;
                weight_sum = weight_sum + w;
            }
        }
    }
    let blurred = sum / max(weight_sum, 0.0001);
    let rgb = mix(center.rgb, blurred, intensity);
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), center.a);
}
