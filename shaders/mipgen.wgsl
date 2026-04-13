// Simple blit shader for mipmap generation.
// Samples the previous mip level with bilinear filtering to produce the next.

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

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
    return textureSample(r_texture, r_sampler, in.tex_coord);
}
