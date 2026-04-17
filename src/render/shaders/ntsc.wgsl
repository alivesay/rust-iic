// NTSC Color Bleeding for Apple II
// Creates soft color fringing at luminance edges
// No phase-based banding - just asymmetric color shift at edges

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
    
    // Pass through if disabled or mono
    if filter_strength < 0.01 || is_mono > 0.5 {
        return original;
    }
    
    let texel = 1.0 / src_w;
    
    // Sample spread: 1-3 source pixels based on strength
    let spread = 1.0 + filter_strength * 2.0;
    
    // Get luminance of neighbors to detect edges
    let left_rgb = textureSample(r_texture, r_sampler, uv - vec2<f32>(texel * spread, 0.0)).rgb;
    let right_rgb = textureSample(r_texture, r_sampler, uv + vec2<f32>(texel * spread, 0.0)).rgb;
    
    let y_left = dot(left_rgb, vec3<f32>(0.299, 0.587, 0.114));
    let y_center = dot(original.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let y_right = dot(right_rgb, vec3<f32>(0.299, 0.587, 0.114));
    
    // Asymmetric color fringing based on luminance gradient
    // Rising edge (dark to bright): red/orange fringe on the bright side
    // Falling edge (bright to dark): blue/cyan fringe on the dark side
    
    // Left edge contribution (if left is different)
    let left_diff = y_center - y_left;
    // Right edge contribution (if right is different)
    let right_diff = y_center - y_right;
    
    // Create color fringes
    // Red shifts toward bright edges, blue shifts toward dark edges
    let fringe_strength = filter_strength * 0.3;
    
    var fringe = vec3<f32>(0.0);
    
    // If we're brighter than left neighbor, add warm fringe
    if left_diff > 0.05 {
        fringe.r += left_diff * fringe_strength;
        fringe.b -= left_diff * fringe_strength * 0.5;
    }
    // If we're darker than left neighbor, add cool fringe  
    if left_diff < -0.05 {
        fringe.b -= left_diff * fringe_strength;
        fringe.r += left_diff * fringe_strength * 0.5;
    }
    
    // Similar for right neighbor but opposite effect
    if right_diff > 0.05 {
        fringe.b += right_diff * fringe_strength * 0.5;
    }
    if right_diff < -0.05 {
        fringe.r -= right_diff * fringe_strength * 0.5;
    }
    
    var result = original.rgb + fringe;
    result = clamp(result, vec3<f32>(0.0), vec3<f32>(1.0));
    
    return vec4<f32>(result, original.a);
}
