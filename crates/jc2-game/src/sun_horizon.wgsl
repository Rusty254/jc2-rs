#define_import_path jc2::sun_horizon

// The game's terrain horizon / ambient-occlusion shadow of the Sun
// (research/formats/cloud-shadow-consumers-next.md, FastTer pixel shader, `t11`):
//
//   t      = AmbientOcclusionMap(world.xz / 32768 + 0.5)    x = AO, y / w = horizon heights
//   shadow = max(saturate(cb0[0].z - dot(t.wy, cb0[0].xy)), cb0[10].w)
//   sun    = sun colour * N.L * shadow * (t.x / 2 + 1/2)    only the Sun's direct term
//   ambient= ambient * t.x
//
// `cb0` and the floor are computed on the CPU every frame (sun_horizon.rs) and come in a
// `SunHorizon` storage buffer that every consuming material binds (the `HorizonShadow`
// resource). Consumers: bind `HorizonShadow::ao` (clamp, linear) and `params`, sample with
// `ao_uv`, multiply the material's ambient by `t.x`, and scale the Sun's direct light by
// `sun_factor`.

// The constants arrive as this struct in a read-only storage buffer. Declare it in the
// consuming shader (naga_oil cannot use an imported struct as a global's type):
//
//   struct SunHorizon {
//       cb0: vec4<f32>,   // cb0[0] = (a, b, c, 1)
//       sun: vec4<f32>,   // xyz: unit vector towards the Sun (0: none), w: floor cb0[10].w
//       tilt: vec4<f32>,  // xyz: (sky - ground) / 2 ambient (sky_light.rs); 0 = uniform
//   }
//   @group(MATERIAL_BIND_GROUP) @binding(N) var<storage, read> horizon: SunHorizon;
//
// and call the functions below with `horizon.cb0` and `horizon.sun` (vectors, not the
// struct: naga_oil mis-types an imported struct passed as a call argument).

const WORLD_SIZE: f32 = 32768.0;

fn ao_uv(world_xz: vec2<f32>) -> vec2<f32> {
    return world_xz / WORLD_SIZE + vec2<f32>(0.5, 0.5);
}

// `cb0` and `sun` are the two vec4 of `SunHorizon` (passed as vectors, not the struct:
// naga_oil does not unify the imported struct with the module's own when it is a call
// argument).
fn horizon_shadow(t11: vec4<f32>, cb0: vec4<f32>, sun: vec4<f32>) -> f32 {
    return max(saturate(cb0.z - dot(t11.wy, cb0.xy)), sun.w);
}

// What the Sun's direct term is multiplied by.
fn sun_factor(t11: vec4<f32>, cb0: vec4<f32>, sun: vec4<f32>) -> f32 {
    return horizon_shadow(t11, cb0, sun) * (t11.x * 0.5 + 0.5);
}

// The hemisphere part of the ambient that Bevy's uniform AmbientLight lacks. Native:
// ambient = lerp(cb0[11], cb0[10], N.y / 2 + 1/2) = (sky + ground) / 2 + (sky - ground) / 2 N.y.
// The uniform AmbientLight carries the first term; this adds `tilt * N.y` on the diffuse
// colour and the occlusion, like Bevy's own ambient (APPROXIMATION: Bevy scales the diffuse
// colour by its split-sum term, which is left out here).
fn ambient_tilt(diffuse: vec3<f32>, n: vec3<f32>, tilt: vec4<f32>, occlusion: vec3<f32>, exposure: f32) -> vec3<f32> {
    return exposure * diffuse * tilt.xyz * n.y * occlusion;
}

// True for the directional light that is the Sun (direction compared with the CPU's).
fn is_sun(direction_to_light: vec3<f32>, sun: vec4<f32>) -> bool {
    return dot(direction_to_light, sun.xyz) > 0.9999;
}
