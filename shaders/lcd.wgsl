// Apple IIc Flat Panel LCD Shader
// Inspired by libretro lcd1x.slang by jdgleaver (GPL v2+)
// with Apple IIc-specific adaptations for the passive-matrix STN display.
//
// The Apple IIc flat panel display was a 9" diagonal passive-matrix LCD
// with classic green monochrome colors. It featured visible pixel grid
// structure and the characteristic "25 lines squashed into 16 lines" aspect.
//
// Bindings mirror CRT shader for compatibility:
//   0: intermediate texture  1: sampler  2: uniforms  3: blur_texture (unused)  4: ShaderParams (unused)

// ShaderParams kept for bind group compatibility with CRT shader
struct ShaderParams {
    group0: vec4<f32>,
    group1: vec4<f32>,
    group2: vec4<f32>,
    group3: vec4<f32>,
    group4: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct Uniforms {
    content_rect: vec4<f32>,  // left, top, right, bottom in normalized coords
    params: vec4<f32>,        // surface_w, source_h, bar_h, source_w
    extra: vec4<f32>,         // monochrome, reserved, reserved, reserved
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
@group(0) @binding(3) var r_blur: texture_2d<f32>;  // unused, kept for bind group compatibility
@group(0) @binding(4) var<uniform> params: ShaderParams;  // unused, kept for bind group compatibility

const PI: f32 = 3.141592653589;

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

// LCD pixel grid effect based on lcd1x.slang
// Creates darkened borders between pixels using sine waves
fn lcd_grid_factor(pixel_coord: vec2<f32>, grid_intensity: f32, softness: f32) -> f32 {
    // Offset by 0.25 to ensure grid lines fall between pixels
    let angle = 2.0 * PI * (pixel_coord - 0.25);
    
    // Higher grid_intensity = less visible grid (brighter overall)
    // Y factor: horizontal lines between rows
    let y_factor = (grid_intensity + sin(angle.y)) / (grid_intensity + 1.0);
    // X factor: vertical lines between columns
    let x_factor = (grid_intensity + sin(angle.x)) / (grid_intensity + 1.0);
    
    // Apply softness adjustment - controls how sharp the grid edges are
    let combined = y_factor * x_factor;
    return mix(combined, sqrt(combined), softness);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coord;
    
    // Content rect boundaries
    let cr_left  = uniforms.content_rect.x;
    let cr_top   = uniforms.content_rect.y;
    let cr_right = uniforms.content_rect.z;
    let cr_bot   = uniforms.content_rect.w;
    
    // Source dimensions
    let src_h = uniforms.params.y;
    let src_w = uniforms.params.w;
    
    // Pass through outside content rect (status bar, borders)
    if uv.x < cr_left || uv.x > cr_right || uv.y < cr_top || uv.y > cr_bot {
        return textureSampleLevel(r_texture, r_sampler, uv, 0.0);
    }
    
    // Apple IIc LCD parameters (hardcoded for authentic look)
    let grid_intensity: f32 = 12.0;     // Grid visibility (higher = less visible)
    let brightness: f32 = 1.0;          // Overall brightness
    let contrast: f32 = 1.2;            // Contrast boost for crisp text
    let pixel_softness: f32 = 0.2;      // Sharp pixel edges
    
    // Classic green monochrome LCD colors (like Game Boy / Apple IIc flat panel)
    // Background (light/off): bright lime green
    let bg_color = vec3<f32>(0.61, 0.74, 0.06);  // #9CBC0F-ish
    // Foreground (dark/on pixels): very dark green  
    let fg_color = vec3<f32>(0.06, 0.22, 0.06);  // #0F380F-ish
    
    // Convert screen UV to emulator coordinates [0,1]
    let content_size = vec2<f32>(cr_right - cr_left, cr_bot - cr_top);
    let emu_coord = (uv - vec2<f32>(cr_left, cr_top)) / content_size;
    
    // Calculate pixel coordinates in source resolution
    let pixel_coord = emu_coord * vec2<f32>(src_w, src_h);
    
    // Snap to nearest source pixel center for sharp sampling
    let snapped_pixel = floor(pixel_coord) + 0.5;
    let snapped_emu = snapped_pixel / vec2<f32>(src_w, src_h);
    let snapped_uv = vec2<f32>(cr_left, cr_top) + snapped_emu * content_size;
    
    // Sample the source texture at snapped coordinates (nearest-neighbor effect)
    let source_color = textureSampleLevel(r_texture, r_sampler, snapped_uv, 0.0);
    
    // Convert to grayscale luminance (Apple IIc LCD was monochrome)
    let lum = dot(source_color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    
    // Apply contrast adjustment (centered around 0.5)
    let contrasted_lum = clamp((lum - 0.5) * contrast + 0.5, 0.0, 1.0);
    
    // Apply brightness
    let adjusted_lum = clamp(contrasted_lum * brightness, 0.0, 1.0);
    
    // LCD effect: bright source = dark LCD pixel (LCD blocks light)
    let lcd_darkness = adjusted_lum;
    
    // Mix between background (light green) and foreground (dark green) based on pixel value
    var lcd_color = mix(bg_color, fg_color, lcd_darkness);
    
    // Apply LCD pixel grid effect
    let grid_factor = lcd_grid_factor(pixel_coord, grid_intensity, pixel_softness);
    
    // The grid darkens the spaces between pixels
    lcd_color = mix(fg_color * 0.7, lcd_color, grid_factor);
    
    return vec4<f32>(lcd_color, 1.0);
}
