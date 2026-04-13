// Multi-LOD bloom shader
// Samples multiple mip levels of the intermediate texture to build a rich,
// progressive-blur phosphor glow. Each LOD contributes a wider blur radius
// with increasing intensity to simulate CRT phosphor bloom.

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

// sRGB to linear approximation for proper bloom accumulation
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;

    // Sample mid-range mip levels for a tight phosphor halo.
    var bloom = to_linear(textureSampleLevel(r_texture, r_sampler, uv, 2.0).rgb) * 0.5;
    bloom += to_linear(textureSampleLevel(r_texture, r_sampler, uv, 3.0).rgb) * 0.8;
    bloom += to_linear(textureSampleLevel(r_texture, r_sampler, uv, 4.0).rgb) * 1.0;
    bloom += to_linear(textureSampleLevel(r_texture, r_sampler, uv, 5.0).rgb) * 0.6;

    // Normalize and convert back to sRGB
    bloom = bloom / 2.9;

    return vec4<f32>(to_srgb(bloom), 1.0);
}
