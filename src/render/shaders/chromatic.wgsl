// Chromatic Aberration post-processing shader
// Operates on the CRT output (before glow) for smooth screen-pixel shifts
// Glow is added AFTER chromatic aberration so it's not affected

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct ChromaUniforms {
    // x = chromatic_aberration amount (0-1), y = is_mono (0 or 1)
    // z = glow_amt, w = curvature_on
    params: vec4<f32>,
    // content_rect: left, top, right, bottom (normalized screen coords)
    content_rect: vec4<f32>,
    // x = d (distance), y = R (radius), z = overscan_x/100, w = overscan_y/100
    curvature: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;  // CRT output (no glow)
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: ChromaUniforms;
@group(0) @binding(3) var r_glow: texture_2d<f32>;     // Glow texture

const PI: f32 = 3.141592653589;
const ASPECT: vec2<f32> = vec2<f32>(1.0, 0.75);

fn FIX_f(c: f32) -> f32 { return max(abs(c), 1e-5); }

// Curvature geometry (same as CRT shader)
fn intersect_crt(xy: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> f32 {
    let A = dot(xy, xy) + d * d;
    let B = 2.0 * (R * (dot(xy, sa) - d * ca.x * ca.y) - d * d);
    let C = d * d + 2.0 * R * d * ca.x * ca.y;
    return (-B - sqrt(B * B - 4.0 * A * C)) / (2.0 * A);
}

fn bkwtrans(xy: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec2<f32> {
    let c = intersect_crt(xy, sa, ca, d, R);
    var pt = vec2<f32>(c) * xy;
    pt -= vec2<f32>(-R) * sa;
    pt /= vec2<f32>(R);
    let tang = sa / ca;
    let poc = pt / ca;
    let A = dot(tang, tang) + 1.0;
    let B = -2.0 * dot(poc, tang);
    let C = dot(poc, poc) - 1.0;
    let a = (-B + sqrt(B * B - 4.0 * A * C)) / (2.0 * A);
    let uv = (pt - a * sa) / ca;
    let r = FIX_f(R * acos(a));
    return uv * r / sin(r / R);
}

fn fwtrans(uv: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec2<f32> {
    let r = FIX_f(sqrt(dot(uv, uv)));
    let uv2 = uv * sin(r / R) / r;
    let x = 1.0 - cos(r / R);
    let D = d / R + x * ca.x * ca.y + dot(uv2, sa);
    return d * (uv2 * ca - x * sa) / D;
}

fn maxscale_crt(sa: vec2<f32>, ca: vec2<f32>, d: f32, R: f32) -> vec3<f32> {
    let c = bkwtrans(-R * sa / (1.0 + R / d * ca.x * ca.y), sa, ca, d, R);
    let a = vec2<f32>(0.5, 0.5) * ASPECT;
    let lo = vec2<f32>(
        fwtrans(vec2<f32>(-a.x, c.y), sa, ca, d, R).x,
        fwtrans(vec2<f32>(c.x, -a.y), sa, ca, d, R).y
    ) / ASPECT;
    let hi = vec2<f32>(
        fwtrans(vec2<f32>(a.x, c.y), sa, ca, d, R).x,
        fwtrans(vec2<f32>(c.x, a.y), sa, ca, d, R).y
    ) / ASPECT;
    return vec3<f32>((hi + lo) * ASPECT * 0.5, max(hi.x - lo.x, hi.y - lo.y));
}

fn crt_transform(coord: vec2<f32>, sa: vec2<f32>, ca: vec2<f32>,
                 stretch: vec3<f32>, d: f32, R: f32, ovs: vec2<f32>) -> vec2<f32> {
    let c = (coord - vec2<f32>(0.5)) * ASPECT * stretch.z + stretch.xy;
    return bkwtrans(c, sa, ca, d, R) / ovs / ASPECT + vec2<f32>(0.5);
}

// Corner mask - returns 0 at rounded corners, 1 inside
fn corner_mask(coord: vec2<f32>, ovs: vec2<f32>, csize: f32, csmooth: f32) -> f32 {
    let c = (coord - vec2<f32>(0.5)) * ovs + vec2<f32>(0.5);
    let cc = min(c, vec2<f32>(1.0) - c) * ASPECT;
    let cdist = vec2<f32>(csize);
    let dd = cdist - min(cc, cdist);
    let dist = sqrt(dot(dd, dd));
    return clamp((cdist.x - dist) * csmooth, 0.0, 1.0);
}

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
    let curv_on = uniforms.params.w;
    
    let cr_left = uniforms.content_rect.x;
    let cr_top = uniforms.content_rect.y;
    let cr_right = uniforms.content_rect.z;
    let cr_bot = uniforms.content_rect.w;
    
    let d = uniforms.curvature.x;
    let R = uniforms.curvature.y;
    let ovs_x = uniforms.curvature.z;
    let ovs_y = uniforms.curvature.w;
    
    var result: vec3<f32>;
    var glow_factor: f32;
    
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
        glow_factor = (r_sample.a + g_sample.a + b_sample.a) / 3.0;
    } else {
        let sample = textureSample(r_texture, r_sampler, uv);
        result = sample.rgb;
        glow_factor = sample.a;
    }
    
    // Add glow AFTER chromatic aberration (glow not affected by aberration)
    if glow_amt > 0.0 {
        // Check if we're inside the content rect
        let in_content = uv.x >= cr_left && uv.x <= cr_right && uv.y >= cr_top && uv.y <= cr_bot;
        
        var glow_uv = uv;
        var glow_mask = 1.0;  // Mask to fade glow at screen edges
        
        if in_content {
            // Convert screen UV to emulator-local [0,1] coordinates
            let content_span = vec2<f32>(cr_right - cr_left, cr_bot - cr_top);
            let emu_uv = vec2<f32>(
                (uv.x - cr_left) / content_span.x,
                (uv.y - cr_top) / content_span.y,
            );
            
            // Apply curvature transform (same as CRT shader)
            let ovs = vec2<f32>(ovs_x, ovs_y);
            let sa = vec2<f32>(0.001, 0.001);
            let ca = vec2<f32>(1.001, 1.001);
            let stretch = maxscale_crt(sa, ca, d, R);
            
            var xy: vec2<f32>;
            if curv_on > 0.5 {
                xy = crt_transform(emu_uv, sa, ca, stretch, d, R, ovs);
            } else {
                xy = (emu_uv - vec2<f32>(0.5)) / ovs + vec2<f32>(0.5);
            }
            
            // Apply corner mask to fade glow at rounded corners
            let csize = 0.001;
            let csmooth = 2000.0;
            glow_mask = corner_mask(xy, ovs, csize, csmooth);
            
            // Also fade glow if xy is outside [0,1] (beyond screen edges)
            let edge_fade = smoothstep(0.0, 0.02, xy.x) * smoothstep(0.0, 0.02, 1.0 - xy.x)
                          * smoothstep(0.0, 0.02, xy.y) * smoothstep(0.0, 0.02, 1.0 - xy.y);
            glow_mask *= edge_fade;
            
            // Sample glow at CURVED coordinates (xy) to match the curved CRT image
            // The glow should appear around where the pixels are ON SCREEN, not in flat space
            glow_uv = vec2<f32>(
                cr_left + xy.x * content_span.x,
                cr_top + xy.y * content_span.y,
            );
        } else {
            // Outside content rect - no glow
            glow_mask = 0.0;
        }
        
        let glow_raw = textureSample(r_glow, r_sampler, glow_uv).rgb;
        // Convert from gamma to linear for additive blend
        var glow = pow(max(glow_raw, vec3<f32>(0.0)), vec3<f32>(2.2));
        
        // Apply power curve to concentrate glow around bright sources
        glow = pow(glow, vec3<f32>(1.8));
        
        // Boost glow saturation
        let glow_lum = dot(glow, vec3<f32>(0.2126, 0.7152, 0.0722));
        let glow_sat_boost = 2.0;
        glow = mix(vec3<f32>(glow_lum), glow, glow_sat_boost);
        glow = max(glow, vec3<f32>(0.0));
        
        // Add glow with mask - slider 0-1 maps to visible intensity
        result = result + glow * glow_amt * glow_mask * 40.0;
    }
    
    return vec4<f32>(result, 1.0);
}
