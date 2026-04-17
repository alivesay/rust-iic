// Chromatic Aberration post-processing shader
// Operates on the CRT output (before glow) for smooth screen-pixel shifts
// Glow is added AFTER chromatic aberration so it's not affected

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct ChromaUniforms {
    // x = chromatic_aberration amount (0-1), y = is_mono (0 or 1)
    // z = glow_amt, w = reserved
    params: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;  // CRT output (no glow)
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: ChromaUniforms;
@group(0) @binding(3) var r_glow: texture_2d<f32>;     // Glow texture

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = position * 0.5 + 0.5;
    out.tex_coord.y = 1.0 - out.tex_coord.y;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let chroma_amt = uniforms.params.x;
    let is_mono = uniforms.params.y;
    let glow_amt = uniforms.params.z;
    
    var result: vec3<f32>;
    var glow_factor: f32;  // cval * rbloom from CRT shader
    
    // Apply chromatic aberration if enabled and not mono
    if chroma_amt > 0.0 && is_mono < 0.5 {
        let tex_size = vec2<f32>(textureDimensions(r_texture));
        let pixel_size = 1.0 / tex_size;
        
        // Slider 0-1 maps to 0-3 screen pixels of displacement
        let displacement = chroma_amt * 3.0 * pixel_size.x;
        
        // R shifts right, B shifts left, G stays centered
        let r_uv = uv + vec2<f32>(displacement, 0.0);
        let b_uv = uv - vec2<f32>(displacement, 0.0);
        
        let r_sample = textureSample(r_texture, r_sampler, r_uv);
        let g_sample = textureSample(r_texture, r_sampler, uv);
        let b_sample = textureSample(r_texture, r_sampler, b_uv);
        
        result = vec3<f32>(r_sample.r, g_sample.g, b_sample.b);
        // Average the glow factors from the three samples
        glow_factor = (r_sample.a + g_sample.a + b_sample.a) / 3.0;
    } else {
        let sample = textureSample(r_texture, r_sampler, uv);
        result = sample.rgb;
        glow_factor = sample.a;
    }
    
    // Add glow AFTER chromatic aberration (glow not affected by aberration)
    if glow_amt > 0.0 {
        let glow_raw = textureSample(r_glow, r_sampler, uv).rgb;
        // Convert from gamma to linear for additive blend
        var glow = pow(max(glow_raw, vec3<f32>(0.0)), vec3<f32>(2.2));
        
        // Apply power curve to concentrate glow around bright sources
        glow = pow(glow, vec3<f32>(1.8));
        
        // Boost glow saturation
        let glow_lum = dot(glow, vec3<f32>(0.2126, 0.7152, 0.0722));
        let glow_sat_boost = 2.0;
        glow = mix(vec3<f32>(glow_lum), glow, glow_sat_boost);
        glow = max(glow, vec3<f32>(0.0));
        
        // Add glow with cval*rbloom factor from CRT shader
        result = result + glow * glow_amt * 0.25 * glow_factor;
    }
    
    return vec4<f32>(result, 1.0);
}
