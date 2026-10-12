// The native stars (JC2-214; research `sky-gradient-next.md` sections 5, 10.3; `starsshader`).
//
// Mesh (stars.rs side, native layout): 2000 stars x six identical vertices, POSITION = the star's
// direction times its distance (100 .. 500), UV_0.x = its size class (2 .. 20).
// Vertex (native): the corner comes from `SV_VertexID` (x = id & 1, y = (id + 1) mod 6 < 3), the
// quad half size is `size * clip.w * (height / width / 720, 1 / 720)` in clip units, i.e.
// `size / 2` pixels of a 720-line screen scaled by height / 720; depth forced to 0 (far plane).
// Pixel (native): `star_dif.dds` rgb with alpha `cb1[0].w = min(byte / 128, 1)`; blend
// SRC_ALPHA / ONE.
//
// `frame_*` are the rows of the star-sphere rotation (the moon's orbit, `FUN_00a45260`); the
// rest-frame of that rotation is NOT decoded (see native_sky.rs).

#import bevy_pbr::mesh_view_bindings::view

struct StarParams {
    frame_x: vec4<f32>,
    frame_y: vec4<f32>,
    frame_z: vec4<f32>,
    // w = alpha
    colour: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> stars: StarParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var star_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var star_sampler: sampler;

struct StarVertex {
    @builtin(vertex_index) id: u32,
    @location(0) position: vec3<f32>,
    @location(2) size: vec2<f32>,
}

struct StarOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(v: StarVertex) -> StarOut {
    var out: StarOut;
    let id = v.id % 6u;
    var corner = vec2<f32>(-1.0, -1.0);
    if (id & 1u) != 0u { corner.x = 1.0; }
    if ((id + 1u) % 6u) < 3u { corner.y = 1.0; }
    out.uv = vec2<f32>(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);

    let world = view.world_position
        + v.position.x * stars.frame_x.xyz
        + v.position.y * stars.frame_y.xyz
        + v.position.z * stars.frame_z.xyz;
    var c = view.clip_from_world * vec4<f32>(world, 1.0);
    let width = view.viewport.z;
    let height = view.viewport.w;
    let scale = vec2<f32>((height / width) / 720.0, 1.0 / 720.0);
    c = vec4<f32>(c.xy + corner * v.size.x * c.w * scale, 0.0, c.w);
    out.clip = c;
    return out;
}

@fragment
fn fragment(in: StarOut) -> @location(0) vec4<f32> {
    let t = textureSample(star_tex, star_sampler, in.uv);
    return vec4<f32>(t.rgb, stars.colour.w);
}
