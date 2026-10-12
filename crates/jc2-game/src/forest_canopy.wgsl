// The forest canopy layer (TerrainShaderForest), JC2-160: research/formats/forest-canopy-next.md.
//
// The terrain mesh drawn a second time, 10 m higher (forest_canopy.rs), as an OPAQUE alpha-tested
// cutout: no blending, the pixel is either discarded or overwrites the terrain colour.
//
//   alpha = data.x * forest_dif.a * fade          discard when alpha < 0.5
//   fade  = min(sat((c - 450 / f) / 350), 1 - sat((c - 32000 / f) / 35000))
//   c     = horizontal Chebyshev distance to the camera, f = max(fov^2, 0.38)
//
//   N   = normalize(8.56031 (0.5 - s.x), 0.5, 8.56031 (0.5 - s.y)),  s = globalslopes
//   sun = sat(N.L) * shadow * 0.7 * (0.5 + 0.5 ao)       (the 0.7 is canopy-only)
//   amb = ao * mix(ground, sky, 0.5 + 0.5 N.y)
//   rgb = fog((sun * sunColour + amb) * forest_dif.rgb)
//
// APPROXIMATION: the native evaluates the fade per vertex and interpolates it; this evaluates the
// same function per pixel (identical on the linear stretches of the Chebyshev distance).

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, lights},
}
#import jc2::haze_fog::haze_fog
#import jc2::sun_horizon::{ao_uv, sun_factor, is_sun}

// the per-frame horizon constants (sun_horizon.rs; sun_horizon.wgsl says how to use them)
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    tilt: vec4<f32>,
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var density_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var density_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var forest_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var forest_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var slopes_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var slopes_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var ao_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var<storage, read> horizon: SunHorizon;

const FADE_IN_START: f32 = 450.0;
const FADE_IN_WIDTH: f32 = 350.0;
const FADE_OUT_START: f32 = 32000.0;
const FADE_OUT_WIDTH: f32 = 35000.0;
const MIN_FADE_SCALE: f32 = 0.38;
const ALPHA_REFERENCE: f32 = 0.5;
const SUN_SCALE: f32 = 0.7;
const SLOPE_FACTOR: f32 = 8.56031;
const FOREST_REPEAT: f32 = 400.0;
const WORLD_EXTENT: f32 = 32768.0;
const PI: f32 = 3.14159265;

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let world = in.world_position.xyz;
    let uv = world.xz / WORLD_EXTENT + vec2<f32>(0.5, 0.5);

    // all samples first: the derivative-based ones stay in uniform control flow
    let density = textureSample(density_tex, density_sampler, uv).x;
    let forest = textureSample(forest_tex, forest_sampler, world.xz / FOREST_REPEAT);
    let slope = textureSample(slopes_tex, slopes_sampler, uv).rg;
    let t11 = textureSample(ao_tex, slopes_sampler, ao_uv(world.xz));

    // the fade: f from the realised vertical FOV of the projection (terrain_material::TerrainLod)
    let rel = world.xz - view.world_position.xz;
    let c = max(abs(rel.x), abs(rel.y));
    let fov = 2.0 * atan(1.0 / view.clip_from_view[1][1]);
    let f = max(fov * fov, MIN_FADE_SCALE);
    let fade_in = saturate(c * (1.0 / FADE_IN_WIDTH) - FADE_IN_START / (FADE_IN_WIDTH * f));
    let fade_out = 1.0 - saturate(c * (1.0 / FADE_OUT_WIDTH) - FADE_OUT_START / (FADE_OUT_WIDTH * f));
    let fade = min(fade_in, fade_out);

    // the alpha test (state: alpha test on, reference 0.5; no blending)
    if density * forest.a * fade < ALPHA_REFERENCE {
        discard;
    }

    // lighting normal from the global gradient layer
    let n = normalize(vec3<f32>(
        SLOPE_FACTOR * (0.5 - slope.x), 0.5, SLOPE_FACTOR * (0.5 - slope.y)));

    // the Sun: the project's sun / moon light in native units (lux * exposure / pi), the
    // horizon / AO shadow of sun_horizon.wgsl, and the canopy-only 0.7
    var sun_colour = vec3<f32>(0.0);
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let l = &lights.directional_lights[i];
        if is_sun((*l).direction_to_light, horizon.sun) {
            sun_colour = (*l).color.rgb * view.exposure / PI;
            break;
        }
    }
    let sun = saturate(dot(n, horizon.sun.xyz))
        * sun_factor(t11, horizon.cb0, horizon.sun) * SUN_SCALE;
    // ambient: lerp(ground, sky, 0.5 + 0.5 N.y) = mean + tilt N.y, times the AO
    let ambient = view.exposure * (lights.ambient_color.rgb + horizon.tilt.xyz * n.y) * t11.x;

    var rgb = (sun * sun_colour + ambient) * forest.rgb;
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(world, 1.0));
    rgb = haze_fog(rgb, haze_clip, world.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour);

    var out: FragmentOutput;
    out.color = vec4<f32>(rgb, 1.0);
    return out;
}
