// Terrain splat: StandardMaterial lighting with the base colour and normal the game's
// FastTer_Lod0 / Lod01 / Lod1 / Lod12 / Lod2 pixel shaders produce (see
// crates/jc2-game/src/terrain_material.rs and research/formats/terrain.md, "LOD blend
// distances, mips and lighting terms").
//
// Per fragment: find the tile cell under the fragment, read its four slots (point
// sampled), weights and gradient (bilinear), run the weights through the game's contrast
// curve, keep the two heaviest slots, split each between the slope band of the gradient
// and the next band by the band fraction, keep the two heaviest of those four candidates,
// bend the gradient by the displacement atlas and choose again, look each one's atlas
// texture up in the zone's slot x band table, sample it in world space and mix. With
// distance the game's LOD weights t0 / t1 (Chebyshev distance, radii scaled by the
// camera FOV) mix in the 21.3 m repeat and then the colour table. The normal atlas bends
// the slope normal in the game's frame: N' = N (1 + n.z) + 4 n.x T + 4 n.y B with
// T = (Ny, -Nx, 0) and B = (0, Nz, -Ny) in engine coordinates.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_view_bindings::view,
}
#import jc2::haze_fog::haze_fog
#import jc2::sun_horizon::{ao_uv, sun_factor, is_sun, ambient_tilt}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{
        apply_pbr_lighting, main_pass_post_lighting_processing, calculate_diffuse_color,
        calculate_F0_dielectric,
    },
    pbr_types::PbrInput,
    mesh_view_bindings as view_bindings,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types,
    lighting,
    lighting::LAYER_BASE,
    shadows,
}
#endif

// the per-frame horizon constants (sun_horizon.rs; sun_horizon.wgsl says how to use them)
struct SunHorizon {
    cb0: vec4<f32>,
    sun: vec4<f32>,
    // xyz: (sky - ground) / 2 ambient, see ambient_tilt in sun_horizon.wgsl
    tilt: vec4<f32>,
    // haze fog (haze.rs, haze_fog.wgsl): cb0[12], cb0[13], fog colour; all zero = no fog
    fog_a: vec4<f32>,
    fog_b: vec4<f32>,
    fog_colour: vec4<f32>,
}

struct SplatParams {
    // x0, z0 of the tile in engine coordinates; the near and the far repeat in metres
    origin: vec4<f32>,
    // the gradient that counts as 1 in the slope measure, the number of slope bands,
    // the normal-map strength (0: no normal atlas), the displacement (0: none, 1: on)
    slope: vec4<f32>,
    // table[slot * 16 + band] = atlas texture index | displacement tile << 8 |
    // undefined << 16 | f2 << 17 | f3 << 25 (see entry_tile)
    table: array<vec4<u32>, 64>,
    // the game's colour table for the same entries (sRGB RGBA8, MetaMaterialColors)
    colours: array<vec4<u32>, 64>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> splat: SplatParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var atlas: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var atlas_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var slot_tex: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var weight_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var weight_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var normal_atlas: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var disp_atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var disp_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var gradient_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var gradient_sampler: sampler;
// the AO / horizon map (sampled with weight_sampler: clamp, linear) and the per-frame
// horizon constants (sun_horizon.rs)
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var ao_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var<storage, read> horizon: SunHorizon;

const CELL: f32 = 4.0;
const CELLS: f32 = 128.0;
// the game's LOD blend (terrain_material::LOD0_RADIUS ..., FUN_00a60d30 / FUN_00a61220):
// radii before the FOV scale, blend widths, the scale's clamp
const LOD0_RADIUS: f32 = 25.0;
const LOD0_WIDTH: f32 = 25.0;
const LOD1_RADIUS: f32 = 250.0;
const LOD1_WIDTH: f32 = 50.0;
const LOD_SCALE_MAX: f32 = 999.99994;
// the displacement atlas repeats every 256 m
const DISP_REPEAT: f32 = 256.0;

fn pick_word(v: vec4<u32>, i: u32) -> u32 {
    switch (i % 4u) {
        case 0u: { return v.x; }
        case 1u: { return v.y; }
        case 2u: { return v.z; }
        default: { return v.w; }
    }
}

fn table_entry(slot: u32, band: u32) -> u32 {
    let i = slot * 16u + min(band, 15u);
    return pick_word(splat.table[i / 4u], i);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// FastTer_Lod2's colour: the colour table entry of (slot, band), linear.
fn table_colour(slot: u32, band: u32) -> vec3<f32> {
    let i = slot * 16u + min(band, 15u);
    return srgb_to_linear(unpack4x8unorm(pick_word(splat.colours[i / 4u], i)).rgb);
}

// The game's weight curve: below one half a weight is divided by 8, above it is
// stretched to reach 1, so a cell shows its dominant material and blends only near an
// even split.
fn weight_curve(w: f32) -> f32 {
    return select(w * 1.875 - 0.875, w * 0.125, w < 0.5);
}

struct Pick {
    e1: u32,
    e2: u32,
    w1: f32,
    w2: f32,
    c1: vec3<f32>,
    c2: vec3<f32>,
}

// Four candidates: both slots at this slope band and at the next, split by the fraction;
// the two heaviest are kept and renormalised (FastTer_Lod0's selection).
fn pick_layers(sa: u32, sb: u32, wa: f32, wb: f32, s: f32) -> Pick {
    let band = u32(floor(s));
    let f = s - floor(s);
    let cand_w = vec4<f32>(wa * (1.0 - f), wb * (1.0 - f), wa * f, wb * f);
    let cand_s = vec4<u32>(sa, sb, sa, sb);
    let cand_b = vec4<u32>(band, band, band + 1u, band + 1u);
    var i1 = 0u;
    var w1 = cand_w.x;
    if cand_w.y > w1 { i1 = 1u; w1 = cand_w.y; }
    if cand_w.z > w1 { i1 = 2u; w1 = cand_w.z; }
    if cand_w.w > w1 { i1 = 3u; w1 = cand_w.w; }
    var i2 = 0u;
    var w2 = -1.0;
    if i1 != 0u && cand_w.x > w2 { i2 = 0u; w2 = cand_w.x; }
    if i1 != 1u && cand_w.y > w2 { i2 = 1u; w2 = cand_w.y; }
    if i1 != 2u && cand_w.z > w2 { i2 = 2u; w2 = cand_w.z; }
    if i1 != 3u && cand_w.w > w2 { i2 = 3u; w2 = cand_w.w; }
    let total2 = max(w1 + w2, 0.000001);
    var p: Pick;
    p.w1 = w1 / total2;
    p.w2 = w2 / total2;
    p.e1 = table_entry(cand_s[i1], cand_b[i1]);
    p.e2 = table_entry(cand_s[i2], cand_b[i2]);
    p.c1 = table_colour(cand_s[i1], cand_b[i1]);
    p.c2 = table_colour(cand_s[i2], cand_b[i2]);
    return p;
}

// An entry of the slot x band table: bits 0..7 the atlas texture, 8..9 the displacement
// tile, 16 "slot undefined", 17..24 f2 (x 255), 25..31 f3 (x 200).
fn entry_tile(e: u32) -> u32 {
    return (e >> 8u) & 3u;
}
fn entry_f2(e: u32) -> f32 {
    return f32((e >> 17u) & 0xffu) / 255.0;
}
fn entry_f3(e: u32) -> f32 {
    return f32((e >> 25u) & 0x7fu) / 200.0;
}

// One tile of the displacement atlas (4 x 1 tiles of 1024 px, each repeating every
// 256 m) at the game's explicit mip, inset by one texel of that mip
// (terrain_material::displace_uv), decoded: x, y the bend, z the "height" of the hollows.
fn disp_tile(tile: u32, world_xz: vec2<f32>, mip: f32) -> vec3<f32> {
    let m = exp2(mip);
    let local = fract(world_xz / DISP_REPEAT) * (1.0 - m / 512.0);
    let at = vec2<f32>(f32(tile) * 0.25 + m / 4096.0 + local.x * 0.25, m / 1024.0 + local.y);
    let c = textureSampleLevel(disp_atlas, disp_sampler, at, mip);
    return vec3<f32>(c.r, c.g, c.a) * 2.0 - 1.0;
}

fn displacement(world_xz: vec2<f32>, mip: f32, ta: u32, tb: u32, wa: f32, wb: f32) -> vec3<f32> {
    let a = disp_tile(ta, world_xz, mip);
    if ta == tb {
        return a;
    }
    return a * wa + disp_tile(tb, world_xz, mip) * wb;
}

// Colour of one atlas texture at this fragment, tiled in world space (engine x, z) at
// `repeat` metres and the game's explicit mip.
fn layer_colour(entry: u32, world_xz: vec2<f32>, repeat: f32, mip: f32) -> vec3<f32> {
    let texture = i32(entry & 0xffu);
    var c = textureSampleLevel(atlas, atlas_sampler, world_xz / repeat, texture, mip).rgb;
    if (entry & 0x10000u) != 0u {
        c = vec3<f32>(1.0, 0.0, 1.0);
    }
    return c;
}

// Tangent-space xy of one atlas texture's normal map at this fragment (near repeat).
fn layer_normal(entry: u32, world_xz: vec2<f32>, mip: f32) -> vec2<f32> {
    let texture = i32(entry & 0xffu);
    let n = textureSampleLevel(normal_atlas, normal_sampler, world_xz / splat.origin.z, texture, mip).xy;
    return n * 2.0 - 1.0;
}

// The game's normal bend, in engine coordinates (x east, y up, z south; the same as
// Bevy's world axes, `coords.rs`). `strength` is 4 t0 (FastTer_Lod01).
fn bend_normal(n_engine: vec3<f32>, nt: vec2<f32>, strength: f32) -> vec3<f32> {
    let xy = nt * strength;
    // the game takes the absolute value, not a clamp (sqrt |1 - |xy|^2|)
    let z = sqrt(abs(1.0 - dot(xy, xy)));
    let t = vec3<f32>(n_engine.y, -n_engine.x, 0.0);
    let b = vec3<f32>(0.0, n_engine.z, -n_engine.y);
    return normalize(n_engine * (1.0 + z) + xy.x * t + xy.y * b);
}

#ifndef PREPASS_PIPELINE
// The Sun's direct term is scaled by the horizon / AO factor (sun_horizon.wgsl): Bevy sums
// all its lights inside apply_pbr_lighting, so the Sun's own contribution (light x cascade
// shadow, as there) is evaluated again and the difference to the scaled one added. Fill,
// Moon, point lights, ambient and fog are untouched.
fn sun_correction(p: PbrInput, factor: f32) -> vec3<f32> {
    let n_lights = view_bindings::lights.n_directional_lights;
    for (var i: u32 = 0u; i < n_lights; i = i + 1u) {
        let l = &view_bindings::lights.directional_lights[i];
        if !is_sun((*l).direction_to_light, horizon.sun) {
            continue;
        }
        let NdotV = max(dot(p.N, p.V), 0.0001);
        let roughness = lighting::perceptualRoughnessToRoughness(p.material.perceptual_roughness);
        let base = p.material.base_color.rgb;
        var li: lighting::LightingInput;
        li.layers[LAYER_BASE].NdotV = NdotV;
        li.layers[LAYER_BASE].N = p.N;
        li.layers[LAYER_BASE].R = reflect(-p.V, p.N);
        li.layers[LAYER_BASE].perceptual_roughness = p.material.perceptual_roughness;
        li.layers[LAYER_BASE].roughness = roughness;
        li.P = p.world_position.xyz;
        li.V = p.V;
        li.diffuse_color = calculate_diffuse_color(
            base, p.material.metallic, p.material.specular_transmission,
            p.material.diffuse_transmission);
        li.metallic = p.material.metallic;
        li.F0_dielectric = calculate_F0_dielectric(p.material.reflectance);
        li.F0_metallic = base;
        li.F_ab = lighting::F_AB(p.material.perceptual_roughness, NdotV);
        var cascade = 1.0;
        if (p.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
            && ((*l).flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            let view_z = dot(vec4<f32>(
                view.view_from_world[0].z, view.view_from_world[1].z,
                view.view_from_world[2].z, view.view_from_world[3].z), p.world_position);
            cascade = shadows::fetch_directional_shadow(
                i, p.world_position, p.world_normal, view_z, p.frag_coord.xy);
        }
        let sun = lighting::directional_light(i, &li, true) * cascade;
        return view.exposure * sun * (factor - 1.0);
    }
    return vec3<f32>(0.0);
}
#endif

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // engine coordinates: x east, z south, the same as Bevy's (`coords.rs`)
    let world_xz = in.world_position.xz;
    let local = world_xz - splat.origin.xy;
    // texel (r, c) sits on height sample (r + 2, c + 2) = local (4c, 4r)
    let texel = local / CELL + vec2<f32>(0.5, 0.5);
    let tc = clamp(vec2<i32>(floor(texel)), vec2<i32>(0), vec2<i32>(127));
    let slots = textureLoad(slot_tex, tc, 0);
    let w = textureSample(weight_tex, weight_sampler, texel / CELLS);
    // the game's gradient layer (A8L8: L = dh/dx, A = dh/dz in units of the slope unit)
    let g = textureSample(gradient_tex, gradient_sampler, texel / CELLS).rg * 2.0 - 1.0;
    // explicit mips from the screen derivatives of world x, z, taken where the control
    // flow is uniform: D = 0.5 log2(|d/dX|^2 + |d/dY|^2)
    let dx = dpdx(world_xz);
    let dy = dpdy(world_xz);
    let d_mip = 0.5 * log2(max(dot(dx, dx) + dot(dy, dy), 1e-20));
    let disp_mip = clamp(d_mip + 0.5, 0.0, 6.0);
    let near_mip = clamp(d_mip + 5.5, 0.0, 6.0);
    let far_mip = clamp(d_mip + 2.8, 0.0, 6.0);

    let unit = splat.slope.x;
    let s = min(length(g), 1.0) * splat.slope.y;

    // the two heaviest slots after the curve
    let cw = vec4<f32>(weight_curve(w.r), weight_curve(w.g), weight_curve(w.b), weight_curve(w.a));
    var ia = 0u;
    var wa = cw.x;
    if cw.y > wa { ia = 1u; wa = cw.y; }
    if cw.z > wa { ia = 2u; wa = cw.z; }
    if cw.w > wa { ia = 3u; wa = cw.w; }
    var ib = 0u;
    var wb = -1.0;
    if ia != 0u && cw.x > wb { ib = 0u; wb = cw.x; }
    if ia != 1u && cw.y > wb { ib = 1u; wb = cw.y; }
    if ia != 2u && cw.z > wb { ib = 2u; wb = cw.z; }
    if ia != 3u && cw.w > wb { ib = 3u; wb = cw.w; }
    let total = max(wa + wb, 0.000001);
    wa = wa / total;
    wb = wb / total;
    let sa = slots[ia];
    let sb = slots[ib];

    // the game's LOD weights: Chebyshev distance from the camera against the radii
    // scaled by min(1 / fov^2, 999.99994), fov the vertical field of view
    let rel = abs(in.world_position.xyz - view.world_position);
    let cheb = max(rel.x, max(rel.y, rel.z));
    let fov = 2.0 * atan(1.0 / view.clip_from_view[1][1]);
    let k = min(1.0 / (fov * fov), LOD_SCALE_MAX);
    let t0 = 1.0 - saturate((cheb - LOD0_RADIUS * k) / LOD0_WIDTH);
    let t1 = 1.0 - saturate((cheb - LOD1_RADIUS * k) / LOD1_WIDTH);

    // first pass: the bands the gradient picks; they say which displacement tiles bend
    // the gradient and how far (f2), and how far the lighting normal bends (f3)
    let p1 = pick_layers(sa, sb, wa, wb, s);
    var d = vec3<f32>(0.0, 0.0, 0.0);
    var f2 = 0.0;
    var f3 = 0.0;
    if splat.slope.w > 0.0 {
        d = displacement(world_xz, disp_mip, entry_tile(p1.e1), entry_tile(p1.e2), p1.w1, p1.w2);
        f2 = entry_f2(p1.e1) * p1.w1 + entry_f2(p1.e2) * p1.w2;
        f3 = entry_f3(p1.e1) * p1.w1 + entry_f3(p1.e2) * p1.w2;
    }
    let dv = vec2<f32>(d.x, -d.y);
    // second pass: the bent gradient chooses the bands again
    let gv = -g + f2 * dv;
    let s2 = min(length(gv), 1.0) * splat.slope.y;
    let p2 = pick_layers(sa, sb, wa, wb, s2);

    // Lod0 (near repeat), Lod1 (far repeat), Lod2 (colour table), mixed by t0 and t1;
    // the atlas samples are skipped where their weight is 0
    var colour = p2.c1 * p2.w1 + p2.c2 * p2.w2;
    if t1 > 0.0 {
        var lod1 = layer_colour(p2.e1, world_xz, splat.origin.w, far_mip) * p2.w1
            + layer_colour(p2.e2, world_xz, splat.origin.w, far_mip) * p2.w2;
        if t0 > 0.0 {
            let lod0 = layer_colour(p2.e1, world_xz, splat.origin.z, near_mip) * p2.w1
                + layer_colour(p2.e2, world_xz, splat.origin.z, near_mip) * p2.w2;
            lod1 = mix(lod1, lod0, t0);
        }
        colour = mix(colour, lod1, t1);
    }

    // the lighting normal: the gradient bent by 1.5 f3 of the displacement, then the
    // normal atlas at 4 t0
    let lg = (-g + 1.5 * f3 * dv) * unit;
    let n_detail = normalize(vec3<f32>(lg.x, 1.0, lg.y));
    var bent = n_detail;
    if t0 > 0.0 && splat.slope.z > 0.0 {
        let nt = layer_normal(p2.e1, world_xz, near_mip) * p2.w1
            + layer_normal(p2.e2, world_xz, near_mip) * p2.w2;
        bent = bend_normal(n_detail, nt, splat.slope.z * t0);
    }
    pbr_input.N = bent;
    // t11: AO (x) and the two horizon heights (y, w) at world.xz / 32768 + 0.5
    let t11 = textureSample(ao_tex, weight_sampler, ao_uv(world_xz));
    // the hollows of the displacement map and the AO map darken the ambient term only
    pbr_input.diffuse_occlusion = pbr_input.diffuse_occlusion
        * (1.0 - 0.6 * f3 * (1.0 - d.z) * 0.5) * t11.x;

    pbr_input.material.base_color = vec4<f32>(colour, 1.0);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // the Sun's direct term times shadow * (AO / 2 + 1/2)
    out.color = vec4<f32>(
        out.color.rgb + sun_correction(pbr_input, sun_factor(t11, horizon.cb0, horizon.sun)), out.color.a);
    // the hemisphere part of the native ambient (the uniform AmbientLight is its mean)
    let diffuse = calculate_diffuse_color(
        pbr_input.material.base_color.rgb, pbr_input.material.metallic,
        pbr_input.material.specular_transmission, pbr_input.material.diffuse_transmission);
    out.color = vec4<f32>(
        out.color.rgb + ambient_tilt(diffuse, pbr_input.N, horizon.tilt, pbr_input.diffuse_occlusion, view.exposure),
        out.color.a);
    // the native atmospheric fog (haze_fog.wgsl); Bevy's own fog is off for this material
    let haze_clip = view.clip_from_view * (view.view_from_world * vec4<f32>(in.world_position.xyz, 1.0));
    out.color = vec4<f32>(
        haze_fog(out.color.rgb, haze_clip, in.world_position.y, horizon.fog_a, horizon.fog_b, horizon.fog_colour),
        out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
