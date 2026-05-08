struct GbParams {
    output_size:  vec4<f32>,
    source_size:  vec4<f32>,
    content_rect: vec4<f32>,
    pass1_size:   vec4<f32>,
    config_a:     vec4<f32>,
    config_b:     vec4<f32>,
    config_c:     vec4<f32>,
    config_d:     vec4<f32>,
    config_e:     vec4<f32>,
    config_f:     vec4<f32>,
    panel_extras: vec4<f32>,
    vignette_params: vec4<f32>,
    vignette_tint:   vec4<f32>,
    lcd_extras:      vec4<f32>,
    lcd_bg_color:    vec4<f32>,
    lcd_fg_color:    vec4<f32>,
};

@group(0) @binding(0) var src_tex:    texture_2d<f32>;
@group(0) @binding(1) var src_samp:   sampler;
@group(0) @binding(2) var<uniform> P: GbParams;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = vec2<f32>(P.output_size.z, P.output_size.w);
    let upper = texel * (P.output_size.xy - vec2<f32>(1.0));
    let lower = vec2<f32>(0.0);

    let w0 = 0.13465834124289953;
    let w1 = 0.13051534237555914;
    let w2 = 0.11883557904592230;
    let w3 = 0.10164546793794160;
    let w4 = 0.08167444001912719;

    var col = textureSampleLevel(src_tex, src_samp, clamp(in.uv, lower, upper), 0.0) * w0;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv + vec2<f32>(0.0, texel.y), lower, upper), 0.0).a * w1;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv - vec2<f32>(0.0, texel.y), lower, upper), 0.0).a * w1;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv + vec2<f32>(0.0, 2.0 * texel.y), lower, upper), 0.0).a * w2;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv - vec2<f32>(0.0, 2.0 * texel.y), lower, upper), 0.0).a * w2;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv + vec2<f32>(0.0, 3.0 * texel.y), lower, upper), 0.0).a * w3;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv - vec2<f32>(0.0, 3.0 * texel.y), lower, upper), 0.0).a * w3;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv + vec2<f32>(0.0, 4.0 * texel.y), lower, upper), 0.0).a * w4;
    col.a += textureSampleLevel(src_tex, src_samp, clamp(in.uv - vec2<f32>(0.0, 4.0 * texel.y), lower, upper), 0.0).a * w4;

    return col;
}
