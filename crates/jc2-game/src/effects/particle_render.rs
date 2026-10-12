//! Draws the effect particle emitters ([`super::particle_sim`]): one camera-facing billboard mesh
//! per emitter, rebuilt every frame, back-to-front inside the mesh.
//!
//! # Native (research `particle-units-next.md` sections 1, 4, 6; `particle-consumers-next.md`)
//! * **size is a half-extent**: a corner sits at `(+-1, +-1) * size` from the particle centre (the
//!   quad edge is `2 * size`), rotated by the roll and placed on the camera right / up axes; the
//!   texture top (`v = 0`) is up (vertex shader, bundle offset 246068);
//! * velocity stretch: `size_y = size * min(base + hypot(vx, vy) * k, max)` for the view-space
//!   velocity, and with flag bit12 the roll is `-atan2(vx, vy)` (`0xa5bc30`);
//! * pixel stage: `colour = clamp01(4 * tex.rgb * lit * particle_colour)`,
//!   `alpha = clamp01(tex.a * particle_alpha * distance_fade * near_fade)`; the default blend is
//!   `colour * a' + dst * (1 - a')` with `a' = a^1.18`; flag bit7 leaves use the blend
//!   `INV_DEST_COLOR / ONE` with the source `colour * 1.35 * a^2` ([`ParticleMaterial`],
//!   `particle_quad.wgsl`);
//! * `distance_fade = 1 - max(0, (D - 0.8 R) / (0.2 R))` with `D` the camera distance of the
//!   system's source and `R` the leaf's maximum distance; `near_fade = clamp01((d - start) /
//!   length)`; a particle with `alpha * fades < 0.01` is culled.
//!
//! * textures are sampled as raw bytes (UNORM view): the native name callback picks the sRGB view
//!   only for `_dif` names (`texture-policy-runtime.md`) and particle textures have none.
//!
//! # Approximations (labelled)
//! * the full atlas cell is the quad; the fitted 8-gon of the executable tables (doc section 1, phase
//!   2) is not drawn (alpha mass outside the polygon is 0.1 to 1.5 percent for the verified tables);
//! * lighting (vertex stage `cb3[5]`, `cb3[6]`, `cb3[7..14]`; producers `0xa6fc10`, `0x0095fb60`,
//!   `0x0091dcc0`, see [`ParticleLighting`]): `lit = r0 + max(bit6, emissive) * (1 - r0)` with `r0 =
//!   ambient + (bit7 ? 0 : 1) * sum of up to four lights`; the vertex light term is per particle;
//! * the initial roll and roll rate are not read (unknown hashes): the roll is 0 unless bit12;
//! * the blend of bit7 leaves is documented native, but that the pixel shader at bundle offset
//!   6734720 is the one used for them is **inferred** (doc section 4);
//! * soft-particle depth attenuation and the distortion pass are not ported (distortion leaves are
//!   simulated but not drawn); fog is Bevy's `DistanceFog` for the default blend, none for bit7;
//! * draw order between emitters is Bevy's transparent sort on the emitter's mean particle position.

use super::EffectHost;
use super::particle_sim::Emitter;
use crate::point_lights::{FrameLights, Selected};
use crate::sky_light::SkyLighting;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use std::collections::{HashMap, HashSet};

#[derive(Component)]
pub struct EffectParticleMesh(pub u64);

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleParams {
    /// x: 1 for the screen-blend (flag bit7) leaves, 0 for the default blend.
    pub mode: Vec4,
}

/// The particle quad material: the native pixel stage and blend states (see the module header).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(ParticleMaterialKey)]
pub struct ParticleMaterial {
    #[uniform(0)]
    pub params: ParticleParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    pub screen: bool,
}

impl Material for ParticleMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("particle_quad.wgsl")).with_source("embedded"),
        )
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // transparent phase, depth test on, no depth write; the blend is replaced in `specialize`
        if self.screen {
            AlphaMode::Add
        } else {
            AlphaMode::Blend
        }
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let state = if key.bind_group_data.screen {
            // native bit7 blend: SrcBlend = INV_DEST_COLOR, DestBlend = ONE (`0x957db0(.., 10, 2, ..)`)
            let c = BlendComponent {
                src_factor: BlendFactor::OneMinusDst,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            };
            BlendState { color: c, alpha: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add } }
        } else {
            // native default: SRC1_ALPHA / INV_SRC1_ALPHA with alpha raised to 1.18 in the shader
            BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::SrcAlpha,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
            }
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(state);
            }
        }
        // the quads face the camera by construction; never cull
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Bevy's per-material pipeline key: the blend choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParticleMaterialKey {
    screen: bool,
}

impl From<&ParticleMaterial> for ParticleMaterialKey {
    fn from(m: &ParticleMaterial) -> Self {
        Self { screen: m.screen }
    }
}

#[derive(Resource)]
struct ParticleShader(#[allow(dead_code)] Handle<Shader>);

/// Register the material and its embedded shader.
pub fn add_particle_material(app: &mut App) {
    bevy::asset::embedded_asset!(app, "particle_quad.wgsl");
    let shader: Handle<Shader> = load_embedded_asset!(app, "particle_quad.wgsl");
    app.insert_resource(ParticleShader(shader))
        .add_plugins(MaterialPlugin::<ParticleMaterial>::default());
}

#[derive(Resource, Default)]
pub struct ParticleDraw {
    meshes: HashMap<u64, (Entity, Handle<Mesh>)>,
    materials: HashMap<(String, bool), Handle<ParticleMaterial>>,
    /// Texture members already confirmed loaded (logged once).
    loaded: HashSet<String>,
    /// Particles drawn last frame (diagnostics).
    pub drawn: usize,
    pub emitters_drawn: usize,
    /// Distortion emitters skipped last frame.
    pub distortion_skipped: usize,
}

fn empty_mesh() -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
    m.insert_indices(Indices::U32(Vec::new()));
    m
}

/// Maximum lights the particle vertex stage reads (`0x0095fb60` is called with a count of 4: the
/// constants `cb3[7..14]` hold four `{position, 1 / r^2}` / `{colour}` pairs).
pub const PARTICLE_LIGHTS: usize = 4;
/// Extra reach added to a light's radius when a system collects lights (`0x0095fb60`'s float
/// argument, pushed as `0x40a00000` by the draw `0x00a5bc30`).
pub const LIGHT_GATHER_PAD: f32 = 5.0;

/// The lighting inputs of the particle vertex stage (native, research `particle-units-next.md`
/// section 4 and the Gap 5 closure there).
///
/// * `ambient` is `cb3[5].xyz`: the draw `0x00a5bc30` reads the three bytes at data `+0x29c..0x29e`
///   as `byte * 0.003921569 * 4`. The bytes are written every frame for every particle data
///   object by `0x00a6fc10` through `0x00a6e6c0` (`byte = trunc(clamp01(v * 0.25) * 255)`), with
///   `v = 0.75 * (sky_ambient + ground_ambient) + 0.75 * sun_colour * |light_dir.y|`, where the
///   ambients are `cb0[10].xyz` / `cb0[11].xyz` (`0x0093afc0` on the render engine) and the sun
///   colour is the main light's `+0x18` (`0x004813d0`, `cb0[3]`) and `light_dir.y` the float at
///   `+0x264` of the landscape object `[0x01191c48] + 0x2c` (the main light direction, which is
///   the sun or the moon).
/// * `lights` is the frame's managed-light list (`0x0091dcc0`: the first 64 records of the sorted
///   list, `{pos, radius, colour * fade, 0}`); a system takes the first four whose centre is
///   within `radius + 5` of the system source (`0x0095fb60`, in list order).
#[derive(Clone, Debug, Default)]
pub struct ParticleLighting {
    pub ambient: Vec3,
    pub lights: Vec<Selected>,
    /// Capture aid (`JC2_PARTICLE_UNLIT=1`, before / after pictures only): `lit = 1` for every
    /// leaf, the behaviour before the lighting was ported. Not native.
    pub unlit: bool,
}

/// `cb3[5].xyz` from the sky/ground ambient, the sun colour and the light direction's y
/// (`0x00a6fc10` + `0x00a6e6c0` + the byte reads of `0x00a5bc30`).
pub fn ambient_constant(sky: Vec3, ground: Vec3, sun: Vec3, light_dir_y: f32) -> Vec3 {
    let v = (sky + ground) * 0.75 + sun * (0.75 * light_dir_y.abs());
    v.map(|c| {
        let byte = ((c * 0.25).clamp(0.0, 1.0) * 255.0) as i32 as f32;
        byte * 0.003_921_569 * 4.0
    })
}

/// The up-to-four lights a system with its source at `source` collects (`0x0095fb60`).
pub fn gather_lights(all: &[Selected], source: Vec3) -> Vec<Selected> {
    let mut out = Vec::with_capacity(PARTICLE_LIGHTS);
    for l in all {
        let reach = l.radius + LIGHT_GATHER_PAD;
        if l.pos.distance_squared(source) < reach * reach {
            out.push(*l);
            if out.len() == PARTICLE_LIGHTS {
                break;
            }
        }
    }
    out
}

/// The per-particle `lit` factor of the vertex stage (bundle offset 246068): the light sum is
/// `colour * saturate(1 - d^2 / r^2)^4` per light (two squarings of the window), the sum is
/// scaled by `cb3[6].x` (0 for flag bit7 leaves) and added to the ambient, then
/// `lit = r0 + max(bit6, emissive) * (1 - r0)` with no clamp of `r0`.
pub fn particle_lit(
    lighting: &ParticleLighting,
    lights: &[Selected],
    centre: Vec3,
    bit6: bool,
    bit7: bool,
    emissive: f32,
) -> Vec3 {
    if lighting.unlit {
        return Vec3::ONE;
    }
    let mut sum = Vec3::ZERO;
    for l in lights {
        let a = (1.0 - l.pos.distance_squared(centre) * (1.0 / (l.radius * l.radius))).clamp(0.0, 1.0);
        let a2 = a * a;
        sum += l.colour * (a2 * a2);
    }
    let weight = if bit7 { 0.0 } else { 1.0 };
    let r0 = sum * weight + lighting.ambient;
    let f = emissive.max(if bit6 { 1.0 } else { 0.0 });
    r0 + (Vec3::ONE - r0) * f
}

/// One billboard of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub centre: Vec3,
    /// Half-extents in metres (`size_x`, `size_y`).
    pub half: Vec2,
    /// Roll in radians (positive turns counter-clockwise as seen by the viewer).
    pub roll: f32,
    /// Particle colour (leaf colour times `lit`); the shader multiplies it by the texture and 4.
    pub colour: [f32; 3],
    /// Alpha before the texture and the 1.18 power: `alpha * distance_fade * near_fade`.
    pub alpha: f32,
    pub uv: [f32; 4],
}

/// Positions, uvs, colours, indices.
type MeshData = (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>);

/// The quads of one emitter for a camera at `cam_pos` with the given right / up axes: size
/// (half-extent), velocity stretch, culls and fades of the native vertex stage (`0xa5bc30`).
pub fn emitter_quads(
    e: &Emitter,
    cam_pos: Vec3,
    right: Vec3,
    up: Vec3,
    lighting: &ParticleLighting,
) -> Vec<Quad> {
    let sys = &e.system;
    let source = e.matrix.w_axis.truncate();
    let range = if sys.max_distance > 0.0 { sys.max_distance } else { 800.0 };
    let distance_fade = if range < 0.01 {
        1.0
    } else {
        1.0 - ((cam_pos.distance(source) - 0.8 * range) / (0.2 * range)).max(0.0)
    };
    if distance_fade <= 0.0 {
        return Vec::new();
    }
    let [stretch_base, stretch_max, stretch_k] = sys.stretch;
    let stretched = stretch_max > 1.0 || e.flags.align_velocity;
    let [near_start, near_len] = sys.near_fade;
    let lights = gather_lights(&lighting.lights, source);
    let mut quads = Vec::with_capacity(e.particles.len());
    for p in &e.particles {
        if !(p.size > 0.0 && p.alpha > 0.0) {
            continue;
        }
        let centre = e.world_position(p);
        let near = if near_start == 0.0 && near_len == 0.0 {
            1.0
        } else {
            ((cam_pos.distance(centre) - near_start) / near_len).clamp(0.0, 1.0)
        };
        let alpha = p.alpha * distance_fade * near;
        // the vertex stage culls a particle whose faded alpha is below 0.0025 * 4
        if alpha < 0.01 {
            continue;
        }
        let mut half = Vec2::splat(p.size);
        let mut roll = 0.0;
        if stretched {
            let v = e.world_velocity(p);
            let (vx, vy) = (v.dot(right), v.dot(up));
            half.y = p.size * (stretch_base + vx.hypot(vy) * stretch_k).min(stretch_max);
            if e.flags.align_velocity {
                roll = -vx.atan2(vy);
            }
        }
        let lit = particle_lit(
            lighting,
            &lights,
            centre,
            e.flags.bit6,
            e.flags.additive_hint,
            sys.initial_emissive,
        );
        quads.push(Quad {
            centre,
            half,
            roll,
            colour: (Vec3::from(sys.colour) * lit).to_array(),
            alpha,
            uv: e.cell_uv(p),
        });
    }
    quads
}

/// Camera-facing quad corners in the order top-left, top-right, bottom-right, bottom-left
/// (matching UV `[u0,v0]`, `[u1,v0]`, `[u1,v1]`, `[u0,v1]`: v grows downwards). A corner with the
/// cell-relative uv `(cu, cv)` is at `ox = (2 cu - 1) * hx`, `oy = (2 cv - 1) * hy`, then
/// `rx = ox cos + oy sin`, `ry = ox sin - oy cos`, placed at `centre + right * rx + up * ry`
/// (native vertex stage), so `v = 0` is up.
pub fn billboard_mesh_data(
    quads: &[Quad],
    origin: Vec3,
    right: Vec3,
    up: Vec3,
    cam: Vec3,
) -> MeshData {
    let mut q: Vec<&Quad> = quads.iter().collect();
    // far to near so overlapping smoke blends correctly inside one mesh
    q.sort_by(|a, b| {
        let da = a.centre.distance_squared(cam);
        let db = b.centre.distance_squared(cam);
        db.total_cmp(&da)
    });
    let mut pos = Vec::with_capacity(q.len() * 4);
    let mut uvs = Vec::with_capacity(q.len() * 4);
    let mut col = Vec::with_capacity(q.len() * 4);
    let mut idx = Vec::with_capacity(q.len() * 6);
    for quad in q {
        let c = quad.centre - origin;
        let (sin, cos) = quad.roll.sin_cos();
        let base = pos.len() as u32;
        for (cu, cv) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let ox = (2.0 * cu - 1.0) * quad.half.x;
            let oy = (2.0 * cv - 1.0) * quad.half.y;
            let rx = ox * cos + oy * sin;
            let ry = ox * sin - oy * cos;
            pos.push((c + right * rx + up * ry).to_array());
            uvs.push([
                quad.uv[0] + cu * (quad.uv[2] - quad.uv[0]),
                quad.uv[1] + cv * (quad.uv[3] - quad.uv[1]),
            ]);
            col.push([quad.colour[0], quad.colour[1], quad.colour[2], quad.alpha.clamp(0.0, 1.0)]);
        }
        idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    (pos, uvs, col, idx)
}

#[allow(clippy::too_many_arguments)]
pub fn draw_particles(
    mut commands: Commands,
    host: Res<EffectHost>,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut draw: ResMut<ParticleDraw>,
    mut xf: Query<(&mut Transform, &mut Visibility), With<EffectParticleMesh>>,
    time: Res<Time>,
    sky: Option<Res<SkyLighting>>,
    atmosphere: Option<Res<crate::atmosphere::AtmosphereState>>,
    frame_lights: Option<Res<FrameLights>>,
    mut last_log: Local<f32>,
) {
    let Some(cam) = cameras.iter().next() else {
        return;
    };
    let (right, up, cam_pos) = (cam.right().as_vec3(), cam.up().as_vec3(), cam.translation());
    let Ok(mgr) = host.particles.lock() else {
        return;
    };
    // Without the sky state (headless / viewer) the ambient is 0: not a native value.
    let lighting = ParticleLighting {
        ambient: sky.as_ref().map_or(Vec3::ZERO, |sky| {
            ambient_constant(
                sky.state.sky_ambient,
                sky.state.ground_ambient,
                sky.state.sun,
                atmosphere.as_ref().map_or(1.0, |a| a.sun_dir.y),
            )
        }),
        lights: frame_lights.map(|f| f.0.clone()).unwrap_or_default(),
        unlit: std::env::var_os("JC2_PARTICLE_UNLIT").is_some(),
    };
    let mut seen = HashSet::new();
    let (mut drawn, mut emitters, mut distortion) = (0, 0, 0);
    for (id, e) in mgr.sim_emitters() {
        seen.insert(id);
        if e.flags.distortion {
            // the distortion pass (refraction by a normal-map texture) is not ported
            distortion += 1;
            continue;
        }
        let quads = emitter_quads(e, cam_pos, right, up, &lighting);
        let entry = draw.meshes.get(&id).cloned();
        if quads.is_empty() {
            if let Some((ent, _)) = entry
                && let Ok((_, mut vis)) = xf.get_mut(ent)
            {
                *vis = Visibility::Hidden;
            }
            continue;
        }
        let material = draw
            .materials
            .entry((e.texture_member.clone(), e.flags.additive_hint))
            .or_insert_with(|| {
                // raw bytes: the native colour-view policy (`texture-policy-runtime.md`, name
                // callback of `gammacorrect.bin`) selects the sRGB view only for `_dif` names and
                // no particle texture has that suffix; the scene target does the sRGB encode.
                let tex: Handle<Image> = assets
                    .load_builder()
                    .with_settings(|s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false)
                    .load(format!("jc2://particles.blz/{}", e.texture_member));
                materials.add(ParticleMaterial {
                    params: ParticleParams {
                        mode: Vec4::new(f32::from(u8::from(e.flags.additive_hint)), 0.0, 0.0, 0.0),
                    },
                    texture: tex,
                    screen: e.flags.additive_hint,
                })
            })
            .clone();
        if draw.loaded.len() < 4096
            && !draw.loaded.contains(&e.texture_member)
            && let Some(img) = materials.get(&material).and_then(|m| images.get(&m.texture))
        {
            info!(
                "particle texture loaded: particles.blz/{} {}x{} {:?}",
                e.texture_member,
                img.texture_descriptor.size.width,
                img.texture_descriptor.size.height,
                img.texture_descriptor.format
            );
            draw.loaded.insert(e.texture_member.clone());
        }
        if time.elapsed_secs() - *last_log > 1.0 {
            let q0 = quads[0];
            debug!(
                "emitter {id}: tex {} screen {} n {} first pos {:?} half {:?} alpha {}",
                e.texture_member, e.flags.additive_hint, quads.len(), q0.centre, q0.half, q0.alpha
            );
        }
        let origin = quads.iter().map(|q| q.centre).sum::<Vec3>() / quads.len() as f32;
        let (pos, uvs, col, idx) = billboard_mesh_data(&quads, origin, right, up, cam_pos);
        drawn += quads.len();
        emitters += 1;
        let handle = match entry {
            Some((ent, h)) => {
                if let Ok((mut t, mut vis)) = xf.get_mut(ent) {
                    t.translation = origin;
                    *vis = Visibility::Inherited;
                }
                h
            }
            None => {
                let h = meshes.add(empty_mesh());
                let ent = commands
                    .spawn((
                        EffectParticleMesh(id),
                        Mesh3d(h.clone()),
                        MeshMaterial3d(material),
                        Transform::from_translation(origin),
                        NoFrustumCulling,
                    ))
                    .id();
                draw.meshes.insert(id, (ent, h.clone()));
                h
            }
        };
        if let Some(mut mesh) = meshes.get_mut(&handle) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
            mesh.insert_indices(Indices::U32(idx));
        }
    }
    drop(mgr);
    if drawn > 0 && time.elapsed_secs() - *last_log > 1.0 {
        *last_log = time.elapsed_secs();
        let first = draw.meshes.values().next().map(|m| m.0);
        let at = first.and_then(|e| xf.get(e).ok()).map(|(t, _)| t.translation);
        debug!(
            "particles drawn: {drawn} in {emitters} emitters; camera {cam_pos:?}; first mesh at {at:?}"
        );
    }
    draw.drawn = drawn;
    draw.emitters_drawn = emitters;
    draw.distortion_skipped = distortion;
    // emitters that no longer exist
    let gone: Vec<u64> = draw
        .meshes
        .keys()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in gone {
        if let Some((ent, h)) = draw.meshes.remove(&id) {
            commands.entity(ent).despawn();
            meshes.remove(h.id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(centre: Vec3, half: Vec2, roll: f32, alpha: f32, uv: [f32; 4]) -> Quad {
        Quad {
            centre,
            half,
            roll,
            colour: [0.5, 0.25, 1.0],
            alpha,
            uv,
        }
    }

    #[test]
    fn quads_are_twice_the_size_face_the_camera_and_sort_far_to_near() {
        let quads = [
            quad(Vec3::new(0.0, 0.0, -1.0), Vec2::splat(1.0), 0.0, 0.5, [0.0, 0.0, 0.5, 0.5]),
            quad(Vec3::new(0.0, 0.0, -9.0), Vec2::splat(1.0), 0.0, 1.0, [0.5, 0.5, 1.0, 1.0]),
        ];
        let (pos, uv, col, idx) =
            billboard_mesh_data(&quads, Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::ZERO);
        assert_eq!((pos.len(), uv.len(), col.len(), idx.len()), (8, 8, 8, 12));
        // the far quad (alpha 1, z=-9) comes first
        assert_eq!(col[0][3], 1.0);
        assert_eq!(col[0][..3], [0.5, 0.25, 1.0]);
        // a half-extent of 1 is an edge of 2: top-left (-1, +1), bottom-right (+1, -1)
        assert_eq!(pos[0], [-1.0, 1.0, -9.0]);
        assert_eq!(pos[2], [1.0, -1.0, -9.0]);
        // top-left uses (u0, v0)
        assert_eq!(uv[0], [0.5, 0.5]);
        assert_eq!(uv[2], [1.0, 1.0]);
        assert!(idx.iter().all(|i| (*i as usize) < pos.len()));
    }

    fn light(x: f32, radius: f32, colour: Vec3) -> Selected {
        Selected { pos: Vec3::new(x, 0.0, 0.0), colour, radius }
    }

    #[test]
    fn the_ambient_constant_is_the_byte_quantised_hemisphere_sum() {
        // 0.75 * (0.2 + 0.1) + 0.75 * 0.4 * 0.5 = 0.375: byte trunc(0.09375 * 255) = 23
        let a = ambient_constant(Vec3::splat(0.2), Vec3::splat(0.1), Vec3::splat(0.4), -0.5);
        assert!((a.x - 23.0 * 0.003_921_569 * 4.0).abs() < 1e-6, "{a:?}");
        // values above 4 clamp to the byte 255 = 4.0
        let big = ambient_constant(Vec3::splat(9.0), Vec3::ZERO, Vec3::ZERO, 0.0);
        assert!((big.y - 4.0).abs() < 1e-5);
        assert_eq!(ambient_constant(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1.0), Vec3::ZERO);
    }

    #[test]
    fn a_system_takes_the_first_four_lights_within_radius_plus_five() {
        let all: Vec<Selected> = (0..6).map(|i| light(i as f32, 1.0, Vec3::ONE)).collect();
        let got = gather_lights(&all, Vec3::ZERO);
        assert_eq!(got.len(), 4);
        assert_eq!(got[3].pos.x, 3.0);
        // reach is radius + 5, strictly less: a light 6 m away with radius 1 is out
        assert!(gather_lights(&[light(6.0, 1.0, Vec3::ONE)], Vec3::ZERO).is_empty());
        assert_eq!(gather_lights(&[light(5.9, 1.0, Vec3::ONE)], Vec3::ZERO).len(), 1);
    }

    #[test]
    fn lit_follows_the_vertex_stage_chain() {
        let lighting = ParticleLighting { ambient: Vec3::splat(0.25), lights: Vec::new(), unlit: false };
        let l = [light(0.0, 10.0, Vec3::new(2.0, 1.0, 0.5))];
        // at the light the window is 1; at half the radius a = 0.75 and the term is a^4
        let at = |x: f32, bit6: bool, bit7: bool, e: f32| {
            particle_lit(&lighting, &l, Vec3::new(x, 0.0, 0.0), bit6, bit7, e)
        };
        let full = at(0.0, false, false, 0.0);
        assert!((full - Vec3::new(2.25, 1.25, 0.75)).abs().max_element() < 1e-6, "{full:?}");
        let half = at(5.0, false, false, 0.0);
        let a4 = 0.75f32.powi(4);
        assert!((half.x - (0.25 + 2.0 * a4)).abs() < 1e-6);
        // outside the radius only the ambient remains
        assert_eq!(at(11.0, false, false, 0.0), Vec3::splat(0.25));
        // bit7 drops the light sum, bit6 forces lit = 1, emissive blends toward 1
        assert_eq!(at(0.0, false, true, 0.0), Vec3::splat(0.25));
        assert_eq!(at(0.0, true, false, 0.0), Vec3::ONE);
        assert!((at(11.0, false, false, 0.5).x - 0.625).abs() < 1e-6);
    }

    #[test]
    fn roll_turns_counter_clockwise_and_stretch_lengthens_y() {
        // a quarter turn: the top-left corner (-1, +1) goes to the bottom-left (-1, -1)
        let q = [quad(Vec3::ZERO, Vec2::new(1.0, 1.0), std::f32::consts::FRAC_PI_2, 1.0, [0.0, 0.0, 1.0, 1.0])];
        let (pos, ..) = billboard_mesh_data(&q, Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        assert!((Vec3::from_array(pos[0]) - Vec3::new(-1.0, -1.0, 0.0)).length() < 1e-5, "{:?}", pos[0]);
        // stretched half-extents
        let q = [quad(Vec3::ZERO, Vec2::new(0.5, 2.0), 0.0, 1.0, [0.0, 0.0, 1.0, 1.0])];
        let (pos, ..) = billboard_mesh_data(&q, Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        assert_eq!(pos[0], [-0.5, 2.0, 0.0]);
        assert_eq!(pos[2], [0.5, -2.0, 0.0]);
    }
}
