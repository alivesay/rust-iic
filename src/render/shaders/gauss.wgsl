// Separable Gaussian blur pass for CRT-Geom-Deluxe halation.
// 17-tap Gaussian with adjustable sigma for smooth, wide blur.
// Direction controlled by uniform: (1,0) = horizontal, (0,1) = vertical.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct GaussUniforms {
    // x = direction_x, y = direction_y, z = blur_width (sigma), w = source_size (in blur axis)
    params: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: GaussUniforms;

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

const GAMMA: f32 = 2.2;

// Sample with threshold - only bright pixels contribute to glow
fn tex2D_glow(uv: vec2<f32>) -> vec3<f32> {
    let raw = textureSampleLevel(r_texture, r_sampler, uv, 0.0).rgb;
    let linear = pow(max(raw, vec3<f32>(0.0)), vec3<f32>(GAMMA));
    // Threshold: only values above 0.1 contribute, with soft falloff
    let lum = dot(linear, vec3<f32>(0.299, 0.587, 0.114));
    let threshold = 0.1;
    let soft = smoothstep(threshold * 0.5, threshold * 2.0, lum);
    return linear * soft;
}

// Compute Gaussian weight: exp(-x^2 / (2*sigma^2))
fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let dir = vec2<f32>(uniforms.params.x, uniforms.params.y);
    let blur_width = uniforms.params.z;
    let src_size = uniforms.params.w;

    // Scale sigma based on screen resolution to maintain consistent visual size
    // Reference: 540 = 1080p height at 1/2 resolution
    let reference_size = 540.0;
    let sigma = blur_width * (src_size / reference_size);
    
    // Texel step in blur direction (1 texel at blur texture resolution)
    let step = dir / vec2<f32>(src_size);

    // 17-tap symmetric Gaussian (-8 to +8)
    // Pre-compute weights
    let w0 = gaussian(0.0, sigma);
    let w1 = gaussian(1.0, sigma);
    let w2 = gaussian(2.0, sigma);
    let w3 = gaussian(3.0, sigma);
    let w4 = gaussian(4.0, sigma);
    let w5 = gaussian(5.0, sigma);
    let w6 = gaussian(6.0, sigma);
    let w7 = gaussian(7.0, sigma);
    let w8 = gaussian(8.0, sigma);
    
    // Normalization factor
    let norm = 1.0 / (w0 + 2.0 * (w1 + w2 + w3 + w4 + w5 + w6 + w7 + w8));

    // Accumulate samples using thresholded glow
    var sum = tex2D_glow(uv) * w0;
    sum += (tex2D_glow(uv - 1.0 * step) + tex2D_glow(uv + 1.0 * step)) * w1;
    sum += (tex2D_glow(uv - 2.0 * step) + tex2D_glow(uv + 2.0 * step)) * w2;
    sum += (tex2D_glow(uv - 3.0 * step) + tex2D_glow(uv + 3.0 * step)) * w3;
    sum += (tex2D_glow(uv - 4.0 * step) + tex2D_glow(uv + 4.0 * step)) * w4;
    sum += (tex2D_glow(uv - 5.0 * step) + tex2D_glow(uv + 5.0 * step)) * w5;
    sum += (tex2D_glow(uv - 6.0 * step) + tex2D_glow(uv + 6.0 * step)) * w6;
    sum += (tex2D_glow(uv - 7.0 * step) + tex2D_glow(uv + 7.0 * step)) * w7;
    sum += (tex2D_glow(uv - 8.0 * step) + tex2D_glow(uv + 8.0 * step)) * w8;

    let result = pow(sum * norm, vec3<f32>(1.0 / GAMMA));
    return vec4<f32>(result, 1.0);
}
