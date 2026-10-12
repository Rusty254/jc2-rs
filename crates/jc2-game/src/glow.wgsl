// The distant light-source glow sprites (JC2-197; research `research/formats/glow-sprites.md`,
// JC2-136): the native `lightsource` shader pair (`DX10_Shaders_F.shader_bundle`), arithmetic
// transcribed from the probe-pinned disassembly.
//
// Mesh layout (glow.rs): six non-indexed vertices per sprite (the native mesh also stores each
// sprite six times and tells the corners apart by `SV_VertexID`), POSITION = the sprite's world
// position, UV_0.x = the size class S = 1 + type / 2 metres, COLOR = the sprite's rgb.
//
// Vertex stage (native):
//   corner  x = +1 if (id & 1) else -1;  y = +1 if ((id + 1) mod 6) < 3 else -1
//   half    = max(A * S / w, 3) / 720   (NDC-y);  NDC-x = half * B   (B = height / width)
//   z'      = c.z + S / w               (pushed away; see the depth note below)
//   t       = saturate(d * D - C);  I = t^F * E;  I < 0.002 -> clipped
//   colour  = I * rgb, alpha = I
// with C = 200 / 800, D = 1 / 800, E = 0.25, F = 1 (the block type defaults, `0x00969b50`) and
// A = cot(fovY / 2) * height / 2 (INFERRED: the native float at render-context +0xb24 is not
// identified, Gap G2 / JC2-196).
// Pixel stage (native): out = lens5.r * gain * (I * rgb, I), then alpha * 1.5.
//
// NOT native / assumed (each is a Gap in the hand-in):
// * the blend (engine modes 3 / 2, JC2-195): additive One / One is the working guess (glow.rs);
// * depth: the test is on, no write (JC2-195). The native clip-z push is in the engine's
//   depth units; this camera uses reverse-Z, so the push is subtracted (`ndc - S / w^2`) and
//   clamped; the native near / far scale is not ported, the magnitude is therefore inferred.

#import bevy_pbr::mesh_view_bindings::view

struct GlowParams {
    // x: gain (the day / night key #178dbdf8: 3.5 .. 6.0), yzw unused
    gain: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> glow: GlowParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lens: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lens_sampler: sampler;

// the block-type defaults (ctor 0x00969b50) written to cb1 by 0x0093d940
const C_FADE: f32 = 0.25;       // [+0x18] / [+0x1c] = 200 / 800
const D_FADE: f32 = 0.00125;    // 1 / [+0x1c] = 1 / 800
const E_GAIN: f32 = 0.25;       // [+0x20]
const F_POW: f32 = 1.0;         // [+0x24]
const KILL: f32 = 0.002;
const REF_HEIGHT: f32 = 720.0;
const MIN_HALF: f32 = 3.0;

struct GlowVertex {
    @builtin(vertex_index) id: u32,
    @location(0) position: vec3<f32>,
    @location(2) size: vec2<f32>,
    @location(5) colour: vec4<f32>,
}

struct GlowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // (I * rgb, I)
    @location(1) glow: vec4<f32>,
}

@vertex
fn vertex(v: GlowVertex) -> GlowOut {
    var out: GlowOut;
    let id = v.id % 6u;
    var corner = vec2<f32>(-1.0, -1.0);
    if (id & 1u) != 0u { corner.x = 1.0; }
    if ((id + 1u) % 6u) < 3u { corner.y = 1.0; }
    out.uv = corner * 0.5 + vec2<f32>(0.5);

    let s = v.size.x;
    let c = view.clip_from_world * vec4<f32>(v.position, 1.0);
    let w = c.w;
    let height = view.viewport.w;
    let width = view.viewport.z;
    // A: inferred, see the header
    let a = view.clip_from_view[1][1] * height * 0.5;
    let half_y = max(a * s / w, MIN_HALF) / REF_HEIGHT;
    let half = vec2<f32>(half_y * height / width, half_y);

    let d = distance(view.world_position, v.position);
    let t = saturate(d * D_FADE - C_FADE);
    let intensity = pow(t, F_POW) * E_GAIN;

    var pos = vec4<f32>(c.xy + corner * half * w, max(c.z - s / w, 0.0), w);
    if intensity < KILL || w <= 0.0 {
        pos.z = -2.0 * w;
    }
    out.clip = pos;
    out.glow = vec4<f32>(intensity * v.colour.rgb, intensity);
    return out;
}

@fragment
fn fragment(in: GlowOut) -> @location(0) vec4<f32> {
    let lens_r = textureSample(lens, lens_sampler, in.uv).r;
    let o = lens_r * glow.gain.x * in.glow;
    return vec4<f32>(o.rgb, o.a * 1.5);
}
