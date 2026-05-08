struct GbParams {
    output_size:  vec4<f32>,
    source_size:  vec4<f32>,
    content_rect: vec4<f32>,
    pass1_size:   vec4<f32>,
    config_a:     vec4<f32>,
    config_b:     vec4<f32>,
    config_c:     vec4<f32>,  // grey_balance, brightness_mode, blending_mode, adjacent_blend
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

fn blend_modifier(a: f32, mode: f32) -> f32 {
    let bb = select(0.0, 1.0, a == 0.0);
    return clamp(bb + mode, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = vec2<f32>(P.output_size.z, P.output_size.w);

    let cur = textureSampleLevel(src_tex, src_samp, in.uv, 0.0);

    let upper = texel * (P.output_size.xy - vec2<f32>(2.0));
    let lower = vec2<f32>(0.0);

    let up    = clamp(in.uv + vec2<f32>(0.0, -texel.y), lower, upper);
    let down  = clamp(in.uv + vec2<f32>(0.0,  texel.y), lower, upper);
    let right = clamp(in.uv + vec2<f32>( texel.x, 0.0), lower, upper);
    let left  = clamp(in.uv + vec2<f32>(-texel.x, 0.0), lower, upper);

    let a1 = textureSampleLevel(src_tex, src_samp, up,    0.0).a;
    let a2 = textureSampleLevel(src_tex, src_samp, down,  0.0).a;
    let a3 = textureSampleLevel(src_tex, src_samp, right, 0.0).a;
    let a4 = textureSampleLevel(src_tex, src_samp, left,  0.0).a;

    let blend_amount = P.config_c.w;
    let blending_mode = P.config_c.z;
    let m = blend_modifier(cur.a, blending_mode);

    let new_a = cur.a - ((cur.a - a1) + (cur.a - a2) + (cur.a - a3) + (cur.a - a4))
                       * blend_amount * m;

    return vec4<f32>(cur.rgb, new_a);
}
