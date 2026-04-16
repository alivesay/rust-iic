// Separable Gaussian blur pass for CRT-Geom-Deluxe halation.
// Port of gaussx.slang / gaussy.slang by cgwg.
// 9-tap (radius 4) Gaussian with width-dependent kernel.
// Direction controlled by uniform: (1,0) = horizontal, (0,1) = vertical.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct GaussUniforms {
    // x = direction_x, y = direction_y, z = blur_width, w = source_size (in blur axis)
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

fn tex2D_linear(uv: vec2<f32>) -> vec3<f32> {
    return pow(max(textureSampleLevel(r_texture, r_sampler, uv, 0.0).rgb, vec3<f32>(0.0)), vec3<f32>(GAMMA));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let dir = vec2<f32>(uniforms.params.x, uniforms.params.y);
    let blur_width = uniforms.params.z;
    let src_size = uniforms.params.w;  // texture size along blur axis

    // Compute Gaussian kernel weights based on blur_width
    // wid = width * src_size / (320 * aspect), aspect component is 1.0 or 0.75
    let aspect_comp = select(0.75, 1.0, dir.x > 0.5);
    let wid = blur_width * src_size / (320.0 * aspect_comp);
    let inv_wid2 = -1.0 / (wid * wid);

    // Gaussian weights for offsets 1..4: exp(-n^2 / wid^2)
    let c1 = exp(1.0 * inv_wid2);
    let c2 = exp(4.0 * inv_wid2);
    let c3 = exp(9.0 * inv_wid2);
    let c4 = exp(16.0 * inv_wid2);

    let norm = 1.0 / (1.0 + 2.0 * (c1 + c2 + c3 + c4));

    // Texel step in blur direction — use source_size (output/blur texture dimensions)
    // not input texture dimensions, so kernel covers proper area at reduced resolution
    let step = dir / vec2<f32>(src_size);

    // 9-tap symmetric Gaussian
    var sum = tex2D_linear(uv);
    sum += tex2D_linear(uv - 1.0 * step) * c1;
    sum += tex2D_linear(uv + 1.0 * step) * c1;
    sum += tex2D_linear(uv - 2.0 * step) * c2;
    sum += tex2D_linear(uv + 2.0 * step) * c2;
    sum += tex2D_linear(uv - 3.0 * step) * c3;
    sum += tex2D_linear(uv + 3.0 * step) * c3;
    sum += tex2D_linear(uv - 4.0 * step) * c4;
    sum += tex2D_linear(uv + 4.0 * step) * c4;

    let result = pow(sum * norm, vec3<f32>(1.0 / GAMMA));
    return vec4<f32>(result, 1.0);
}
