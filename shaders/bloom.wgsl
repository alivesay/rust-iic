// Bloom downsample + blur shader
// Reads the full-resolution intermediate texture and produces a soft blurred version.
// The output texture is 1/4 resolution, and the bilinear downsample + blur taps
// create a wide, smooth glow suitable for additive bloom compositing.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;

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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let tex_size = vec2<f32>(textureDimensions(r_texture));
    let px = 1.0 / tex_size.x;
    let py = 1.0 / tex_size.y;

    // 13-tap Gaussian-weighted cross blur
    // At 1/4 res, each pixel covers 4x4 source pixels, so even small offsets
    // in output-pixel space span many source pixels in input space.
    // We use offsets of 2, 4, 6 input pixels to get a wide ~24px radius blur.
    var acc = textureSample(r_texture, r_sampler, uv).rgb * 0.20;

    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(2.0 * px, 0.0)).rgb * 0.15;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(2.0 * px, 0.0)).rgb * 0.15;
    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(4.0 * px, 0.0)).rgb * 0.08;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(4.0 * px, 0.0)).rgb * 0.08;
    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(6.0 * px, 0.0)).rgb * 0.04;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(6.0 * px, 0.0)).rgb * 0.04;

    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(0.0, 2.0 * py)).rgb * 0.07;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(0.0, 2.0 * py)).rgb * 0.07;
    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(0.0, 4.0 * py)).rgb * 0.04;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(0.0, 4.0 * py)).rgb * 0.04;
    acc += textureSample(r_texture, r_sampler, uv + vec2<f32>(0.0, 6.0 * py)).rgb * 0.02;
    acc += textureSample(r_texture, r_sampler, uv - vec2<f32>(0.0, 6.0 * py)).rgb * 0.02;

    return vec4<f32>(acc, 1.0);
}
