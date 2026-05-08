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

@group(0) @binding(0) var pass1_tex: texture_2d<f32>;
@group(0) @binding(1) var samp:      sampler;
@group(0) @binding(2) var<uniform> P: GbParams;
@group(0) @binding(3) var prev_tex:  texture_2d<f32>;

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
    let cur  = textureSampleLevel(pass1_tex, samp, in.uv, 0.0);
    let prev = textureSampleLevel(prev_tex,  samp, in.uv, 0.0);
    let decay = P.panel_extras.w;
    return max(cur, prev * decay);
}
