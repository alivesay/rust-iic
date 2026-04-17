// Phosphor persistence shader
// Blends current frame with decayed history for CRT phosphor afterglow effect

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct PhosphorUniforms {
    // x = decay factor (0.0-0.95), y/z/w = reserved
    params: vec4<f32>,
};

@group(0) @binding(0) var r_current: texture_2d<f32>;  // Current frame (intermediate)
@group(0) @binding(1) var r_history: texture_2d<f32>;  // Previous frame history
@group(0) @binding(2) var r_sampler: sampler;
@group(0) @binding(3) var<uniform> uniforms: PhosphorUniforms;

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
    let decay = uniforms.params.x;
    
    // Sample current frame and history
    let current = textureSampleLevel(r_current, r_sampler, in.tex_coord, 0.0);
    let history = textureSampleLevel(r_history, r_sampler, in.tex_coord, 0.0);
    
    // Phosphor persistence: max of current vs decayed history
    // This gives bright pixels a "trail" as they fade
    var decayed_history = history * decay;
    
    // Threshold tiny values to zero to prevent ghost artifacts
    // Without this, values asymptotically approach zero but never reach it,
    // causing faint ghost images visible at extreme gamma settings
    let threshold = 0.004;  // ~1/255, below visible in 8-bit color
    decayed_history = select(decayed_history, vec4<f32>(0.0), decayed_history.r < threshold && decayed_history.g < threshold && decayed_history.b < threshold);
    
    let result = max(current, decayed_history);
    
    return vec4<f32>(result.rgb, 1.0);
}
