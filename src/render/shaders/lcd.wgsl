// Apple IIc Flat Panel LCD Shader
// based on libretro lcd1x.slang by jdgleaver
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
    
    let contrast: f32 = 1.15;
    
    //   bg  : lightest (off pixel, lit background)
    //   mid1: light gray-green (cell shadow / faint pixel)
    //   mid2: dark gray-green  (drop shadow under lit pixel)
    //   fg  : darkest (fully lit pixel)
    let bg_color   = vec3<f32>(0.788, 0.863, 0.502);  // #C9DC80 (201,220,128)
    let mid1_color = vec3<f32>(0.690, 0.776, 0.408);  // slightly darker bg (grid)
    let mid2_color = vec3<f32>(0.443, 0.541, 0.227);  // pixel drop shadow
    let fg_color   = vec3<f32>(0.090, 0.235, 0.075);  // dark green
    
    // Convert screen UV to emulator coordinates [0,1]
    let content_size = vec2<f32>(cr_right - cr_left, cr_bot - cr_top);
    let emu_coord = (uv - vec2<f32>(cr_left, cr_top)) / content_size;
    
    // Calculate pixel coordinates in source resolution
    let pixel_coord = emu_coord * vec2<f32>(src_w, src_h);
    
    // Sample the current pixel (nearest-neighbor)
    let cur_pixel = floor(pixel_coord);
    let cur_uv = vec2<f32>(cr_left, cr_top)
                 + ((cur_pixel + 0.5) / vec2<f32>(src_w, src_h)) * content_size;
    let cur_color = textureSampleLevel(r_texture, r_sampler, cur_uv, 0.0);
    let cur_lum = clamp(
        (dot(cur_color.rgb, vec3<f32>(0.299, 0.587, 0.114)) - 0.5) * contrast + 0.5,
        0.0, 1.0
    );
    
    // Sample upper-left neighbor for drop shadow (real DMG LCDs cast a
    // soft shadow down-and-right from each lit cell because light enters
    // from above-left through the polarizer stack).
    let nbr_pixel = cur_pixel - vec2<f32>(1.0, 1.0);
    let nbr_uv = vec2<f32>(cr_left, cr_top)
                 + ((nbr_pixel + 0.5) / vec2<f32>(src_w, src_h)) * content_size;
    let nbr_color = textureSampleLevel(r_texture, r_sampler, nbr_uv, 0.0);
    let nbr_lum = clamp(
        (dot(nbr_color.rgb, vec3<f32>(0.299, 0.587, 0.114)) - 0.5) * contrast + 0.5,
        0.0, 1.0
    );
    
    // Position within the current LCD cell (0..1)
    let frac = pixel_coord - cur_pixel;
    
    // Pixel body mask: square cell with small gap between cells (the
    // LCD column/row gutter). Slight softness on edges.
    let cell_inset: f32 = 0.08;       // gutter width as fraction of cell
    let edge_soft: f32  = 0.04;       // anti-alias band
    let dist_from_edge = min(min(frac.x, 1.0 - frac.x), min(frac.y, 1.0 - frac.y));
    let body_mask = smoothstep(cell_inset - edge_soft,
                               cell_inset + edge_soft,
                               dist_from_edge);
    
    // Drop-shadow mask: a soft band along the top and left edges of the
    // cell, modulated by the upper-left neighbor's darkness. Only shows
    // where the current cell body isn't already drawn.
    let shadow_band: f32 = 0.18;      // shadow extent into the cell
    let shadow_soft: f32 = 0.06;
    let top_shadow  = 1.0 - smoothstep(shadow_band - shadow_soft,
                                       shadow_band + shadow_soft,
                                       frac.y);
    let left_shadow = 1.0 - smoothstep(shadow_band - shadow_soft,
                                       shadow_band + shadow_soft,
                                       frac.x);
    // Shadow is strongest in the upper-left corner of the cell, where
    // both bands overlap. (1 - body_mask) restricts it to the gutter.
    let shadow_amt = max(top_shadow, left_shadow) * (1.0 - body_mask) * nbr_lum;
    
    // Compose: start with bg, paint cell shadow gutter, then drop shadow,
    // then the lit pixel body on top.
    var lcd_color = bg_color;
    // Slight all-over gutter darkening (cell separation lines)
    lcd_color = mix(mid1_color, lcd_color, body_mask);
    // Drop shadow from neighbor
    lcd_color = mix(lcd_color, mid2_color, shadow_amt);
    // Pixel body fills with fg_color modulated by lum
    let body_color = mix(bg_color, fg_color, cur_lum);
    lcd_color = mix(lcd_color, body_color, body_mask);
    
    return vec4<f32>(lcd_color, 1.0);
}
