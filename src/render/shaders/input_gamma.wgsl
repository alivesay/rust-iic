// Input gamma decode pass: voltage -> CRT-emitted linear luminance.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

struct InputGammaUniforms {
    // x = alpha1   (pure-power gamma in mode 0; high-side power in mode 1)
    // y = alpha2   (low-side power in mode 1)
    // z = b        (black-lift in mode 1)
    // w = mode     (0/1/2)
    params: vec4<f32>,
};

@group(0) @binding(0) var r_texture: texture_2d<f32>;
@group(0) @binding(1) var r_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: InputGammaUniforms;

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.tex_coord = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return out;
}

// Mode 0: pure power, L = V^gamma.
fn curve_pure_power(v: vec3<f32>, gamma: f32) -> vec3<f32> {
    let g = max(gamma, 0.5);
    return pow(max(v, vec3<f32>(0.0)), vec3<f32>(g));
}

// Mode 1: customizable measured CRT curve with WHITE and BRIGHTNESS.
//
// Forward CRT EOTF (gun voltage V -> emitted linear luminance L), derived
// by inverting the reference C++ `crtProperToLinear`.  Note the C++
// function is named in the "FooToLinear" convention -- its input is
// treated as already encoded with the CRT curve, so it actually goes
// luminance -> voltage.  Our framebuffer holds raw gun voltages, so we
// need the forward direction:
//
//   k = lw / (1 + b)^alpha1
//   if V < vc: L = k * (vc + b)^(alpha1 - alpha2) * (V + b)^alpha2
//   else:      L = k * (V + b)^alpha1
//
// Endpoints: V=0 -> ~0 (b^alpha2 * factor, ~5e-7 with defaults),
// V=1 -> lw=1.  Continuous at V=vc.
fn curve_measured_custom(v_in: vec3<f32>, alpha1_raw: f32, alpha2_raw: f32, b_raw: f32) -> vec3<f32> {
    let alpha1 = max(alpha1_raw, 0.5);
    let alpha2 = max(alpha2_raw, 0.5);
    let b      = max(b_raw,      0.0);
    let vc     = 0.35;
    let lw     = 1.0;
    let v      = max(v_in, vec3<f32>(0.0));
    let k      = lw / pow(1.0 + b, alpha1);

    let low_const = k * pow(vc + b, alpha1 - alpha2);
    let low  = low_const * pow(v + vec3<f32>(b), vec3<f32>(alpha2));
    let high = k         * pow(v + vec3<f32>(b), vec3<f32>(alpha1));
    return max(select(high, low, v < vec3<f32>(vc)), vec3<f32>(0.0));
}

// Mode 2: locked measured curve (single specific tube, no parameters).
// Equivalent to the reference C++ `crtProper2ToLinear`.
fn curve_measured_locked_scalar(v_in: f32) -> f32 {
    let alpha = 0.11157219592173126;
    let beta  = 1.11157219592173129;
    let cut   = 0.09128634211778011;
    let v     = max(v_in, 0.0);
    let high  = pow(v, 2.31);
    if v >= 0.36 {
        return high;
    }
    let toe   = select(pow((v + alpha) / beta, 1.0 / 0.45), v / 4.0, v <= cut);
    let frac  = v / 0.36;
    return toe * (1.0 - frac) + frac * high;
}

fn curve_measured_locked(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        curve_measured_locked_scalar(v.x),
        curve_measured_locked_scalar(v.y),
        curve_measured_locked_scalar(v.z),
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let voltage = textureSample(r_texture, r_sampler, in.tex_coord);
    let mode    = u32(uniforms.params.w + 0.5);

    var luminance = voltage.rgb;
    if mode == 0u {
        luminance = curve_pure_power(voltage.rgb, uniforms.params.x);
    } else if mode == 1u {
        luminance = curve_measured_custom(
            voltage.rgb,
            uniforms.params.x,
            uniforms.params.y,
            uniforms.params.z,
        );
    } else {
        luminance = curve_measured_locked(voltage.rgb);
    }

    return vec4<f32>(luminance, voltage.a);
}
