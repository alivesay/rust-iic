// NTSC Decode Shader for Apple II
// Simulates analog NTSC signal processing with YIQ color space
// - Separate luma/chroma bandwidth filtering (luma sharp, chroma blurry)
// - Optional color subcarrier artifacts and crosstalk
// - Video.rs handles artifact coloring, this simulates analog decode

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717958;

// RGB to YIQ conversion matrix
const RGB_TO_YIQ: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.299, 0.596, 0.211),
    vec3<f32>(0.587, -0.274, -0.523),
    vec3<f32>(0.114, -0.322, 0.312)
);

// YIQ to RGB conversion matrix
const YIQ_TO_RGB: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.0, 1.0, 1.0),
    vec3<f32>(0.956, -0.272, -1.106),
    vec3<f32>(0.621, -0.647, 1.703)
);

fn rgb_to_yiq(rgb: vec3<f32>) -> vec3<f32> {
    return RGB_TO_YIQ * rgb;
}

fn yiq_to_rgb(yiq: vec3<f32>) -> vec3<f32> {
    return YIQ_TO_RGB * yiq;
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct NtscUniforms {
    // x = filter_strength (0-1), y = source_width, z = source_height, w = is_mono
    params: vec4<f32>,
    // content_rect: left, top, right, bottom in UV space
    content_rect: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: NtscUniforms;

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

// Simplified FIR filter coefficients
// Luma: sharper filter (~3MHz bandwidth)
const LUMA_KERNEL_SIZE: i32 = 9;
const LUMA_KERNEL: array<f32, 9> = array<f32, 9>(
    0.02, 0.05, 0.12, 0.18, 0.26, 0.18, 0.12, 0.05, 0.02
);

// Chroma: much blurrier filter (~0.5MHz bandwidth)
const CHROMA_KERNEL_SIZE: i32 = 13;
const CHROMA_KERNEL: array<f32, 13> = array<f32, 13>(
    0.03, 0.05, 0.07, 0.09, 0.11, 0.12, 0.14, 0.12, 0.11, 0.09, 0.07, 0.05, 0.03
);

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    let filter_strength = uniforms.params.x;
    let src_w = uniforms.params.y;
    let is_mono = uniforms.params.w;
    
    let original = textureSample(r_texture, r_sampler, uv);
    
    // Content rect bounds
    let cr_left = uniforms.content_rect.x;
    let cr_top = uniforms.content_rect.y;
    let cr_right = uniforms.content_rect.z;
    let cr_bot = uniforms.content_rect.w;
    
    // Pass through outside content rect
    if uv.x < cr_left || uv.x > cr_right || uv.y < cr_top || uv.y > cr_bot {
        return original;
    }
    
    // Pass through if disabled
    if filter_strength < 0.01 {
        return original;
    }
    
    let content_span_x = cr_right - cr_left;
    let content_span_y = cr_bot - cr_top;
    let texel_x = content_span_x / src_w;
    let texel_y = content_span_y / 192.0;  // 192 scanlines
    
    // Mono mode: apply horizontal anti-aliasing to match what color mode gets from NTSC filtering
    // Without this, non-integer window scales cause vertical banding (columns alias differently)
    // KEY: Use a STRONG blur to completely hide scaling artifacts
    if is_mono > 0.5 {
        // Wide 13-tap Gaussian blur covering ~6 source texels
        // This is wide enough to average out any aliasing pattern from non-integer scaling
        let blur_radius = texel_x * 3.0;  // 3 source texels on each side
        let step = blur_radius / 6.0;     // 13 samples across the range
        
        // Gaussian weights (hand-tuned for smooth rolloff)
        var sum = vec3<f32>(0.0);
        var weight_sum = 0.0;
        
        // Sample at 13 points with Gaussian weights
        for (var i = -6; i <= 6; i++) {
            let t = f32(i) / 6.0;  // Normalized position [-1, 1]
            let weight = exp(-t * t * 2.0);  // Gaussian: e^(-2x²)
            let sample_uv = vec2<f32>(uv.x + f32(i) * step, uv.y);
            sum = sum + textureSample(r_texture, r_sampler, sample_uv).rgb * weight;
            weight_sum = weight_sum + weight;
        }
        
        let blurred = sum / weight_sum;
        
        // Mix based on filter strength (but ensure at least some blur for anti-aliasing)
        let aa_strength = max(filter_strength, 0.5);  // Minimum 50% blur for AA
        let result = mix(original.rgb, blurred, aa_strength);
        
        return vec4<f32>(result, original.a);
    }
    
    let texel = content_span_x / src_w;
    
    // Scale blur with filter strength
    let luma_scale = mix(0.3, 1.0, filter_strength);
    let chroma_scale = mix(0.5, 2.0, filter_strength);
    
    // === Luma filtering (sharper) ===
    var luma_sum = 0.0;
    var luma_weight = 0.0;
    let luma_half = LUMA_KERNEL_SIZE / 2;
    
    for (var i = 0; i < LUMA_KERNEL_SIZE; i++) {
        let offset = f32(i - luma_half) * texel * luma_scale;
        let sample_uv = vec2<f32>(uv.x + offset, uv.y);
        let sample_rgb = textureSample(r_texture, r_sampler, sample_uv).rgb;
        let yiq = rgb_to_yiq(sample_rgb);
        luma_sum += yiq.x * LUMA_KERNEL[i];
        luma_weight += LUMA_KERNEL[i];
    }
    let filtered_luma = luma_sum / luma_weight;
    
    // === Chroma filtering (much blurrier) ===
    var chroma_i_sum = 0.0;
    var chroma_q_sum = 0.0;
    var chroma_weight = 0.0;
    let chroma_half = CHROMA_KERNEL_SIZE / 2;
    
    for (var i = 0; i < CHROMA_KERNEL_SIZE; i++) {
        let offset = f32(i - chroma_half) * texel * chroma_scale;
        let sample_uv = vec2<f32>(uv.x + offset, uv.y);
        let sample_rgb = textureSample(r_texture, r_sampler, sample_uv).rgb;
        let yiq = rgb_to_yiq(sample_rgb);
        chroma_i_sum += yiq.y * CHROMA_KERNEL[i];
        chroma_q_sum += yiq.z * CHROMA_KERNEL[i];
        chroma_weight += CHROMA_KERNEL[i];
    }
    let filtered_i = chroma_i_sum / chroma_weight;
    let filtered_q = chroma_q_sum / chroma_weight;
    
    // Reconstruct YIQ and convert back to RGB
    // No crosstalk - the luma/chroma filtering is the main effect
    let final_yiq = vec3<f32>(filtered_luma, filtered_i, filtered_q);
    var result = yiq_to_rgb(final_yiq);
    
    // Blend with original based on filter strength (allow some unfiltered signal through)
    let blend = min(filter_strength * 1.2, 1.0);
    result = mix(original.rgb, result, blend);
    
    result = clamp(result, vec3<f32>(0.0), vec3<f32>(1.0));
    
    return vec4<f32>(result, original.a);
}
