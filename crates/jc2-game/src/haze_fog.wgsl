#define_import_path jc2::haze_fog

// The native atmospheric fog of the vertex and pixel shaders (research
// `research/formats/haze-fog-next.md`, JC2-116; `haze.rs` computes the constants, JC2-155).
//
// Vertex shader (the same seven instructions in `general`, `fastterrainshader` and `merged`):
//
//   d = clip.w - clip.z                     (about the eye depth: reverse-Z, w = eye depth)
//   h = world.y * cb0[13].z - cb0[13].x     (y - p3) / p4
//   e = d * cb0[12].z + (sat(d * cb0[13].w) * (1 - min(h, cb0[13].y)) - cb0[12].x)
//
// Pixel shader: out = fog + (lit - fog) * min(2^-e, 1)   (SM4 `exp` is 2^x).
//
//   c12 = cb0[12] = (p0 / (p1 - p0), p1, 1 / (p1 - p0), camera y)
//   c13 = cb0[13] = (p3 / p4, sat((camera y - p3) / p4), 1 / p4, p2)
//   fog = pixel cb0[13].rgb (the fog colour in scene units; .w repeats p2)
//
// The constants arrive in the `SunHorizon` storage buffer that terrain and General already
// bind (`fog_a` = c12, `fog_b` = c13, `fog_colour`); all zero means no fog (e = 0, T = 1).
// Pass the vectors, not the struct (naga_oil mis-types an imported struct as an argument).
//
// `clip` is the fragment's clip-space position `clip_from_view * view_from_world * world`;
// Bevy's projection is reverse-Z with an infinite far plane, so `clip.z` is the near distance
// and `d` is the eye depth minus near. The native camera's near/far are an open gap
// (haze-fog-next.md Gap 2): below half a metre at 0.1 / 40000.

fn haze_exponent(clip: vec4<f32>, world_y: f32, c12: vec4<f32>, c13: vec4<f32>) -> f32 {
    let d = clip.w - clip.z;
    let h = world_y * c13.z - c13.x;
    return d * c12.z + (saturate(d * c13.w) * (1.0 - min(h, c13.y)) - c12.x);
}

// The fraction of the lit colour that survives: min(2^-e, 1), no lower cap.
fn haze_transmittance(e: f32) -> f32 {
    return min(exp2(-e), 1.0);
}

// The sky-card variant (JC2-163, shaders `cirrusclouds` and `clouds`, VS at 104892 / 106016 of
// DX10_Shaders_F.shader_bundle; PS 4814356 / 4814844 are `out = tex * vcol * (1 - F) + fog * F`):
// the same seven-instruction exponent, but the distance is the radial distance `len` from the
// camera to the card vertex scaled by `scale` (0.25 cirrus, 0.7 clouds), the height is
// `y' = (y - camY) * scale + camY`, and the exponent is saturated:
//
//   len = |scale * (p - cam)|
//   e   = sat(len * cb0[12].z + (sat(len * cb0[13].w) * (1 - min(y' * cb0[13].z - cb0[13].x, cb0[13].y)) - cb0[12].x))
//   cirrus: F = 1 - 2^-e        clouds: F = 1 - 2^-max(2 (e - 0.5), 0)
//
// Returns F, the fraction of the fog colour mixed in (alpha is untouched).
fn haze_sky_card_factor(world: vec3<f32>, cam: vec3<f32>, scale: f32, is_cloud: bool, c12: vec4<f32>, c13: vec4<f32>) -> f32 {
    let len = length((world - cam) * scale);
    let y = (world.y - cam.y) * scale + cam.y;
    let h = y * c13.z - c13.x;
    var e = saturate(len * c12.z + (saturate(len * c13.w) * (1.0 - min(h, c13.y)) - c12.x));
    if is_cloud {
        e = max((e - 0.5) * 2.0, 0.0);
    }
    return 1.0 - exp2(-e);
}

fn haze_fog(lit: vec3<f32>, clip: vec4<f32>, world_y: f32, c12: vec4<f32>, c13: vec4<f32>, fog_colour: vec4<f32>) -> vec3<f32> {
    let t = haze_transmittance(haze_exponent(clip, world_y, c12, c13));
    return fog_colour.rgb + (lit - fog_colour.rgb) * t;
}
