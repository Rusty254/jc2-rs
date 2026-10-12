//! The native point-light term (JC2-221): selection of the frame's lights, their falloff and the
//! GPU data the surface shaders read (`point_lights.wgsl`). Research:
//! `research/formats/point-light-model.md` sections 2-4, 9.2 and 10, `light-tables-next.md`.
//! It replaces the JC2-139 policy of Bevy `PointLight`s with an inverse-square fall-off.
//!
//! * **Native (probe-verified):** the sort key `trunc(max(r^2, 90000) * 300) / trunc(max(d^2, 1))
//!   + 300 * priority` (0x00970120), the descending stable insertion, the 300 m cull, the
//!   sphere-against-cone view test, the first-64 rule with the colour thresholds (0x0091dcc0),
//!   the colour `clamp(c * S * (1 - d^2 / 90000), 0, 8)`, the shader window
//!   `colour * saturate(1 - d^2 / r^2)^2 * saturate(N . L)` added to the irradiance before the
//!   albedo product and fog, the 4-lights-per-4 m-cell grid, effect light keys (type 7
//!   components), placed `CDynamicLightObject` lamps (no time-of-day test, dimmed by the
//!   gradient scale `S` only).
//! * **Inferred / non-native (each labelled where used):** the camera cone's vertical FOV
//!   reading, the grid origin snapping, cell membership and the weight used to replace a fifth
//!   light, the clock being the Bevy frame delta, the array order that breaks key ties, which
//!   effect each blast spawns.
//! * **Not covered (Gaps):** per-object PL0 / PL2 / PL4 lights, point specular, vehicle,
//!   siren, cutscene, traffic-light and fill-light producers, lamp disable events, flashers.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::render::render_resource::ShaderType;
use bevy::render::storage::ShaderBuffer;
use jc2_formats::point_light::{DynamicLight, LightTrack};

/// Lights sent to the GPU per frame (loop limit of 0x0091dd80).
pub const MAX_LIGHTS: usize = 64;
/// Squared camera distance at and above which a light is dropped (0x00970120, float 90000).
pub const CULL_D2: f32 = 90_000.0;
/// Per-channel colour cap (float 0x00ebf270).
pub const COLOUR_CAP: f32 = 8.0;
/// A light whose every channel ends below this is dropped (float 0x00f66934).
pub const COLOUR_FLOOR: f32 = 0.01;
/// A light whose radius is at or below this is dropped.
pub const RADIUS_FLOOR: f32 = 0.001;
/// Grid cells per side and their size in metres (0x0094bc10).
pub const GRID: usize = 128;
pub const CELL: f32 = 4.0;
/// Lights a cell can hold.
pub const PER_CELL: usize = 4;

/// Fixed handles of the two shared buffers, so a material needs no binding sweep.
pub const LIGHT_BUFFER: Handle<ShaderBuffer> = uuid_handle!("6a2f0b6e-7a46-4c0e-9a1d-221c0de5a001");
pub const CELL_BUFFER: Handle<ShaderBuffer> = uuid_handle!("6a2f0b6e-7a46-4c0e-9a1d-221c0de5a002");

/// A light before selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub pos: Vec3,
    /// `diffuse * multiplier * fade intensity`, before the per-hour scale and the distance fade.
    pub colour: Vec3,
    pub radius: f32,
    /// The signed short at light `+2`.
    pub priority: i16,
}

/// A light that won a slot, its colour final (scale, distance fade and cap applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selected {
    pub pos: Vec3,
    pub colour: Vec3,
    pub radius: f32,
}

/// The frame's selected list as the CPU keeps it (native: the record list of `0x0091dcc0`, read
/// by the grid builder and by the particle draw `0x0095fb60`). Refreshed every frame by
/// `update_point_lights`.
#[derive(Resource, Default, Debug, Clone)]
pub struct FrameLights(pub Vec<Selected>);

/// The camera cone of the view test (0x00970120 / 0x00927830; `CameraCone::new` documents the
/// inferred parts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraCone {
    pub apex: Vec3,
    pub axis: Vec3,
    /// `tan(half-angle)`
    pub tan: f32,
    /// `1 / cos(half-angle)`
    pub sec: f32,
    pub near: f32,
}

impl CameraCone {
    /// The cone subtends the screen **diagonal**: `tan(half) = tan(fov / 2) * sqrt(1 + a^2)`,
    /// `a` = 16/9 when the camera's `+0x634` field exceeds 1 (taken as: the aspect is wide)
    /// else 4/3 (native, decompiler-read). The vertical reading of `fov` is **inferred**.
    pub fn new(apex: Vec3, forward: Vec3, fov_y: f32, wide: bool, near: f32) -> Self {
        let a: f32 = if wide { 16.0 / 9.0 } else { 4.0 / 3.0 };
        let tan = (fov_y * 0.5).tan() * (1.0 + a * a).sqrt();
        Self {
            apex,
            axis: forward.normalize_or(Vec3::NEG_Z),
            tan,
            sec: (1.0 + tan * tan).sqrt(),
            near,
        }
    }

    /// The exact sphere-against-cone test: `f + r > near` and `lat^2 < (tan f + r sec)^2`.
    pub fn contains(&self, pos: Vec3, r: f32) -> bool {
        let v = pos - self.apex;
        let f = v.dot(self.axis);
        let lat2 = (v - self.axis * f).length_squared();
        let lim = self.tan * f + r * self.sec;
        f + r > self.near && lat2 < lim * lim
    }
}

/// The ranking key stored at light `+0x70` (0x00970120): unsigned 32-bit, every conversion
/// truncates. Radius only matters above 300 m (the floor `max(r^2, 90000)`).
pub fn sort_key(radius: f32, d2: f32, priority: i16) -> u32 {
    let num = (radius * radius).max(CULL_D2) * 300.0;
    let num = num as u32;
    let den = (d2.max(1.0) as u32).max(1);
    (num / den).wrapping_add((i32::from(priority) * 300) as u32)
}

/// Selection of the frame's lights (native, model doc 10.1): the candidate array is walked from
/// the last index down, lights outside 300 m or the cone are not listed, the survivors are
/// inserted by `new_key > element_key` (descending, an equal key goes after the existing
/// ones), then the list is read from the front until 64 lights pass the colour and radius
/// thresholds. `scale` is the gradient scalar `S` (times the manager's 1.0).
pub fn select(cands: &[Candidate], cone: &CameraCone, scale: f32) -> Vec<Selected> {
    let mut list: Vec<(u32, usize, f32)> = Vec::new();
    for (i, c) in cands.iter().enumerate().rev() {
        let d2 = c.pos.distance_squared(cone.apex);
        if d2 >= CULL_D2 || !cone.contains(c.pos, c.radius) {
            continue;
        }
        let key = sort_key(c.radius, d2, c.priority);
        let at = list.partition_point(|(k, _, _)| key <= *k);
        list.insert(at, (key, i, d2.max(1.0)));
    }
    let mut out = Vec::with_capacity(MAX_LIGHTS);
    for (_, i, d2) in list {
        let c = &cands[i];
        let fade = 1.0 - d2 * (1.0 / CULL_D2);
        let colour = (c.colour * (scale * fade)).clamp(Vec3::ZERO, Vec3::splat(COLOUR_CAP));
        if c.radius <= RADIUS_FLOOR || colour.max_element() < COLOUR_FLOOR {
            continue;
        }
        out.push(Selected {
            pos: c.pos,
            colour,
            radius: c.radius,
        });
        if out.len() == MAX_LIGHTS {
            break;
        }
    }
    out
}

/// The shader's per-light term (`point_lights.wgsl`): `colour * saturate(1 - d^2 / r^2)^2 *
/// saturate(N . L / |L|)` at `p` with unit normal `n`.
#[cfg_attr(not(test), allow(dead_code))] // CPU mirror of point_lights.wgsl, checked by the tests
pub fn light_term(l: &Selected, p: Vec3, n: Vec3) -> Vec3 {
    let d = l.pos - p;
    let d2 = d.length_squared();
    let a = (1.0 - d2 / (l.radius * l.radius)).clamp(0.0, 1.0);
    l.colour * (a * a) * (n.dot(d) / d2.max(1.0e-8).sqrt()).clamp(0.0, 1.0)
}

/// The grid corner (world x, z): the camera cell snapped to 4 m, 64 cells either way
/// (**inferred**: the original keeps `cb0[7]` as the grid origin, its snapping is not decoded).
pub fn grid_origin(cam: Vec3) -> Vec2 {
    let half = (GRID as f32) * CELL * 0.5;
    Vec2::new(
        ((cam.x - half) / CELL).floor() * CELL,
        ((cam.z - half) / CELL).floor() * CELL,
    )
}

/// The 128 x 128 `LightIndex` cells, 4 byte-sized ids (light index + 1) per u32. A cell takes the
/// lights that can reach it; the fifth replaces the dimmest by `luminance * (1 - d^2 / r^2)^2`
/// (native rule; **inferred**: distance measured from the cell centre in the horizontal plane,
/// a light reaches a cell whose centre is within `r` plus the cell half-diagonal).
pub fn build_cells(sel: &[Selected], origin: Vec2) -> Vec<u32> {
    let mut ids = vec![[0u8; PER_CELL]; GRID * GRID];
    let mut weight = vec![[0.0f32; PER_CELL]; GRID * GRID];
    let reach_pad = CELL * std::f32::consts::FRAC_1_SQRT_2 * 2.0 * 0.5;
    for (i, l) in sel.iter().enumerate() {
        let lum = 0.3 * l.colour.x + 0.59 * l.colour.y + 0.11 * l.colour.z;
        let r = l.radius + reach_pad;
        let lo = ((Vec2::new(l.pos.x, l.pos.z) - Vec2::splat(r) - origin) / CELL).floor();
        let hi = ((Vec2::new(l.pos.x, l.pos.z) + Vec2::splat(r) - origin) / CELL).floor();
        let (x0, x1) = (lo.x.max(0.0) as usize, hi.x.min(GRID as f32 - 1.0).max(-1.0));
        let (z0, z1) = (lo.y.max(0.0) as usize, hi.y.min(GRID as f32 - 1.0).max(-1.0));
        if x1 < 0.0 || z1 < 0.0 {
            continue;
        }
        for cz in z0..=(z1 as usize) {
            for cx in x0..=(x1 as usize) {
                let centre = origin + (Vec2::new(cx as f32, cz as f32) + 0.5) * CELL;
                let d2 = centre.distance_squared(Vec2::new(l.pos.x, l.pos.z));
                if d2 > r * r {
                    continue;
                }
                let w = lum * (1.0 - d2 / (l.radius * l.radius)).clamp(0.0, 1.0).powi(2);
                let cell = cz * GRID + cx;
                let slot = (0..PER_CELL).find(|k| ids[cell][*k] == 0).or_else(|| {
                    let (k, least) = (0..PER_CELL)
                        .map(|k| (k, weight[cell][k]))
                        .min_by(|a, b| a.1.total_cmp(&b.1))?;
                    (w > least).then_some(k)
                });
                if let Some(k) = slot {
                    ids[cell][k] = (i + 1) as u8;
                    weight[cell][k] = w;
                }
            }
        }
    }
    ids.iter()
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[derive(ShaderType, Clone, Copy, Default)]
struct GpuLight {
    pos_w: Vec4,
    colour: Vec4,
}

#[derive(ShaderType, Clone, Copy)]
struct GpuLights {
    origin: Vec4,
    lights: [GpuLight; MAX_LIGHTS],
}

impl GpuLights {
    fn new(sel: &[Selected], origin: Vec2) -> Self {
        let mut g = Self {
            origin: Vec4::new(origin.x, origin.y, CELL, sel.len() as f32),
            lights: [GpuLight::default(); MAX_LIGHTS],
        };
        for (slot, l) in g.lights.iter_mut().zip(sel) {
            slot.pos_w = l.pos.extend(1.0 / (l.radius * l.radius));
            slot.colour = l.colour.extend(0.0);
        }
        g
    }
}

/// What the last frame did (for the HUD / tests).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct PointLightStats {
    pub candidates: usize,
    pub selected: usize,
}

/// A placed `CDynamicLightObject` (native lamp). Lit all day: only the gradient scale `S` dims
/// it by day (model doc 9.2). `fade_in` is the intensity ramp of `0x00778a30` (decompiler-read).
#[derive(Component, Clone, Copy, Debug)]
pub struct LampLight {
    pub pos: Vec3,
    pub colour: Vec3,
    pub radius: f32,
    pub priority: i16,
    pub fade_in: f32,
    pub age: f32,
}

impl LampLight {
    pub fn from_native(d: &DynamicLight, pos: Vec3) -> Self {
        Self {
            pos,
            colour: Vec3::from(d.colour()),
            radius: d.radius,
            priority: d.priority,
            fade_in: d.fade_in,
            age: 0.0,
        }
    }

    pub fn intensity(&self) -> f32 {
        if self.fade_in <= 0.0 {
            1.0
        } else {
            (self.age / self.fade_in).clamp(0.0, 1.0)
        }
    }
}

/// A keyed effect light (muzzle / explosion): follows its entity, dies after the last key.
#[derive(Component, Clone, Debug)]
pub struct EffectLight {
    pub track: Arc<LightTrack>,
    pub clock: f32,
}

/// The effect light tracks loaded from the retail effect files.
#[derive(Resource, Default)]
pub struct NativeLightEffects {
    tracks: HashMap<String, Vec<Arc<LightTrack>>>,
}

/// Effects preloaded at start-up. `missile_explosion` is the **non-native stand-in** for every
/// blast (which effect each blast source spawns is the effect graph's job, not decoded here).
pub const PRELOAD: &[&str] = &["missile_explosion", "grenade_explosion"];
/// The effect every blast of the game spawns today (non-native mapping, Gap).
pub const BLAST_EFFECT: &str = "missile_explosion";

impl NativeLightEffects {
    pub fn insert(&mut self, name: &str, tracks: Vec<LightTrack>) {
        self.tracks
            .insert(name.to_string(), tracks.into_iter().map(Arc::new).collect());
    }

    /// Spawn the effect's light components at `at` (the effect transform).
    pub fn spawn(&self, commands: &mut Commands, name: &str, at: Vec3) {
        for t in self.tracks.get(name).into_iter().flatten() {
            commands.spawn((
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
                EffectLight {
                    track: t.clone(),
                    clock: 0.0,
                },
                Name::new(format!("light {name}")),
            ));
        }
    }
}

fn load_effects(game: Option<Res<crate::Game>>, mut effects: ResMut<NativeLightEffects>) {
    let Some(game) = game else { return };
    for name in PRELOAD {
        match jc2_formats::point_light::load_effect_lights(&game.fs, name) {
            Ok(t) => effects.insert(name, t),
            Err(e) => warn!("point lights: effect {name} not read ({e})"),
        }
    }
}

fn make_buffers(buffers: Option<ResMut<Assets<ShaderBuffer>>>) {
    let Some(mut buffers) = buffers else { return };
    let _ = buffers.insert(
        LIGHT_BUFFER.id(),
        ShaderBuffer::from(GpuLights::new(&[], Vec2::ZERO)),
    );
    let _ = buffers.insert(
        CELL_BUFFER.id(),
        ShaderBuffer::new(&vec![0u8; GRID * GRID * 4], RenderAssetUsages::default()),
    );
}

fn tick_lamps(time: Res<Time>, mut lamps: Query<&mut LampLight>) {
    let dt = time.delta_secs();
    for mut l in &mut lamps {
        if l.age < l.fade_in {
            l.age += dt;
        }
    }
}

fn tick_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut EffectLight)>,
) {
    // The clock is the frame delta (0x004d3690, inferred; slow motion unresolved, Gap)
    let dt = time.delta_secs();
    for (e, mut l) in &mut q {
        l.clock += dt;
        if l.track.evaluate(l.clock).is_none() {
            commands.entity(e).despawn();
        }
    }
}

/// Selects the frame's lights and writes the GPU buffers.
#[allow(clippy::too_many_arguments)]
fn update_point_lights(
    atmosphere: Option<Res<crate::atmosphere::AtmosphereState>>,
    cams: Query<(&GlobalTransform, Option<&Projection>), With<Camera3d>>,
    lamps: Query<&LampLight>,
    effects: Query<(&GlobalTransform, &EffectLight)>,
    buffers: Option<ResMut<Assets<ShaderBuffer>>>,
    mut stats: ResMut<PointLightStats>,
    mut frame: ResMut<FrameLights>,
    mut cands: Local<Vec<Candidate>>,
    mut last: Local<Option<(Vec<Selected>, Vec2)>>,
    mut lamps_off: Local<Option<bool>>,
) {
    // `JC2_NO_LAMP_LIGHTS=1`: placed lamps give no light (before / after captures only)
    let lamps_off = *lamps_off.get_or_insert_with(|| std::env::var_os("JC2_NO_LAMP_LIGHTS").is_some());
    let Some((cam, projection)) = cams.iter().next() else {
        return;
    };
    let (fov, near, wide) = match projection {
        Some(Projection::Perspective(p)) => (p.fov, p.near, p.aspect_ratio > 1.0),
        _ => (1.0, 0.1, true),
    };
    let cone = CameraCone::new(cam.translation(), cam.forward().as_vec3(), fov, wide, near);
    cands.clear();
    for l in lamps.iter().filter(|_| !lamps_off) {
        // cheap pre-cull: the same 300 m test `select` repeats
        if l.pos.distance_squared(cone.apex) >= CULL_D2 {
            continue;
        }
        let i = l.intensity();
        if i < 1.0 / 256.0 {
            continue;
        }
        cands.push(Candidate {
            pos: l.pos,
            colour: l.colour * i,
            radius: l.radius,
            priority: l.priority,
        });
    }
    for (tf, e) in &effects {
        if let Some((colour, radius)) = e.track.evaluate(e.clock) {
            cands.push(Candidate {
                pos: tf.translation() + Vec3::from(e.track.offset),
                colour: Vec3::from(colour),
                radius,
                priority: e.track.priority,
            });
        }
    }
    let scale = atmosphere.map_or(1.0, |a| a.point_light_factor());
    let sel = select(&cands, &cone, scale);
    stats.candidates = cands.len();
    stats.selected = sel.len();
    frame.0.clone_from(&sel);
    // `JC2_LIGHT_DEBUG=1`: log the selection when it changes size (diagnostics only)
    if std::env::var_os("JC2_LIGHT_DEBUG").is_some() && last.as_ref().is_none_or(|(s, _)| s.len() != sel.len()) {
        info!(
            "point lights: {} candidates in range, {} selected, camera {:?}, lamps total {}",
            cands.len(),
            sel.len(),
            cone.apex,
            lamps.iter().count()
        );
    }
    let origin = grid_origin(cone.apex);
    let Some(mut buffers) = buffers else { return };
    if last.as_ref().is_some_and(|(s, o)| *s == sel && *o == origin) {
        return;
    }
    if let Some(mut b) = buffers.get_mut(&LIGHT_BUFFER) {
        b.set_data(GpuLights::new(&sel, origin));
    }
    if let Some(mut b) = buffers.get_mut(&CELL_BUFFER) {
        let cells = build_cells(&sel, origin);
        b.data = Some(cells.iter().flat_map(|c| c.to_le_bytes()).collect());
    }
    *last = Some((sel, origin));
}

/// Embeds and loads `point_lights.wgsl` once; every plugin whose shader imports
/// `jc2::point_lights` calls this.
pub fn ensure_shader(app: &mut App) {
    if !app.world().contains_resource::<PointLightShader>() {
        bevy::asset::embedded_asset!(app, "point_lights.wgsl");
        let shader: Handle<Shader> = bevy::asset::load_embedded_asset!(app, "point_lights.wgsl");
        app.insert_resource(PointLightShader(shader));
    }
}

#[derive(Resource)]
struct PointLightShader(#[allow(dead_code)] Handle<Shader>);

pub struct PointLightPlugin;

impl Plugin for PointLightPlugin {
    fn build(&self, app: &mut App) {
        ensure_shader(app);
        app.init_resource::<NativeLightEffects>()
            .init_resource::<PointLightStats>()
            .init_resource::<FrameLights>()
            .add_systems(Startup, (make_buffers, load_effects))
            .add_systems(Update, (tick_lamps, tick_effects))
            .add_systems(
                PostUpdate,
                update_point_lights.after(TransformSystems::Propagate),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cone() -> CameraCone {
        CameraCone::new(Vec3::ZERO, Vec3::NEG_Z, 1.0, true, 0.1)
    }

    fn lamp(x: f32, z: f32, colour: f32, radius: f32, priority: i16) -> Candidate {
        Candidate {
            pos: Vec3::new(x, 2.0, z),
            colour: Vec3::splat(colour),
            radius,
            priority,
        }
    }

    #[test]
    fn the_key_reproduces_the_documented_examples() {
        // model doc 10.1: priority 0 at 50 / 100 / 200 / 299 m -> 10,800 / 2,700 / 675 / 302
        let k = |d: f32| sort_key(11.0, d * d, 0);
        assert_eq!((k(50.0), k(100.0), k(200.0), k(299.0)), (10_800, 2_700, 675, 302));
        // radius is irrelevant below 300 m, a 586 m blast gets the larger numerator
        assert_eq!(sort_key(5.0, 2500.0, 0), sort_key(150.0, 2500.0, 0));
        assert!(sort_key(586.0, 2500.0, 0) > sort_key(150.0, 2500.0, 0));
        // priority adds 300 per step; the night fill light (99) adds 29,700
        assert_eq!(sort_key(11.0, 2500.0, 99) - sort_key(11.0, 2500.0, 0), 29_700);
        // d^2 below 1 counts as 1
        assert_eq!(sort_key(11.0, 0.0, 0), 27_000_000);
    }

    #[test]
    fn the_cone_is_the_exact_sphere_test() {
        let c = cone();
        assert!(c.contains(Vec3::new(0.0, 0.0, -50.0), 1.0));
        // behind the camera by more than the radius: out; overlapping the apex plane: in
        assert!(!c.contains(Vec3::new(0.0, 0.0, 20.0), 5.0));
        assert!(c.contains(Vec3::new(0.0, 0.0, 3.0), 5.0));
        // far to the side outside the diagonal half angle: out, but its radius can bring it in
        assert!(!c.contains(Vec3::new(200.0, 0.0, -10.0), 1.0));
        assert!(c.contains(Vec3::new(200.0, 0.0, -10.0), 250.0));
        // tan(half) = tan(fov/2) * sqrt(1 + 16/9^2) for a 1 radian FOV
        assert!((c.tan - 0.5463 * (1.0 + 256.0 / 81.0f32).sqrt()).abs() < 1e-3);
    }

    #[test]
    fn nearest_lights_win_and_ties_keep_the_higher_array_index_first() {
        // 70 lamps in a row ahead; only 64 are sent and they are the 64 nearest
        let cands: Vec<_> = (0..70)
            .map(|i| lamp(0.0, -10.0 - 2.0 * i as f32, 3.0, 11.0, 0))
            .collect();
        let sel = select(&cands, &cone(), 1.0);
        assert_eq!(sel.len(), 64);
        assert_eq!(sel[0].pos.z, -10.0);
        assert_eq!(sel[63].pos.z, -136.0);
        // equal keys: the array is walked backwards and an equal key goes after, so the
        // higher index is listed first
        let a = lamp(10.0, -30.0, 1.0, 11.0, 0);
        let b = lamp(-10.0, -30.0, 2.0, 11.0, 0);
        let sel = select(&[a, b], &cone(), 1.0);
        assert_eq!(sel[0].pos.x, -10.0);
    }

    #[test]
    fn priority_beats_distance_and_the_thresholds_drop_lights() {
        // a priority-5 light 150 m away ties a priority-0 lamp at 100 m (doc: 300 per step)
        let near = lamp(0.0, -100.0, 3.0, 11.0, 0);
        let far = lamp(0.0, -150.0, 3.0, 11.0, 5);
        assert_eq!(
            sort_key(11.0, 150.0 * 150.0 + 4.0, 5) / 10,
            sort_key(11.0, 100.0 * 100.0 + 4.0, 0) / 10
        );
        let sel = select(&[near, far], &cone(), 1.0);
        assert_eq!(sel.len(), 2);
        // colour: scale, (1 - d^2 / 90000) fade, cap 8
        let bright = lamp(0.0, -3.0, 100.0, 11.0, 0);
        let s = select(&[bright], &cone(), 1.0);
        assert_eq!(s[0].colour, Vec3::splat(COLOUR_CAP));
        // by day the gradient scale 0.25 applies to a lamp that burns all day
        let day = select(&[lamp(0.0, -30.0, 4.0, 11.0, 0)], &cone(), 0.25);
        let want = 4.0 * 0.25 * (1.0 - (900.0 + 4.0) / CULL_D2);
        assert!((day[0].colour.x - want).abs() < 1e-4);
        // dropped: dim (< 0.01), zero radius, past 300 m, behind the camera
        let none = select(
            &[
                lamp(0.0, -20.0, 0.005, 11.0, 0),
                lamp(0.0, -20.0, 3.0, 0.0, 0),
                lamp(0.0, -310.0, 3.0, 11.0, 0),
                lamp(0.0, 50.0, 3.0, 11.0, 0),
            ],
            &cone(),
            1.0,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn the_window_is_zero_at_the_radius_and_cosine_weighted() {
        let l = Selected {
            pos: Vec3::new(0.0, 3.0, 0.0),
            colour: Vec3::new(3.5, 3.5, 2.1),
            radius: 11.0,
        };
        // straight below the lamp: a = 1 - 9/121, N.L = 1
        let t = light_term(&l, Vec3::ZERO, Vec3::Y);
        let a = 1.0 - 9.0 / 121.0;
        assert!((t.x - 3.5 * a * a).abs() < 1e-5);
        // exactly at the radius: nothing; a face turned away: nothing
        assert_eq!(light_term(&l, Vec3::new(11.0, 3.0, 0.0), Vec3::X), Vec3::ZERO);
        assert_eq!(light_term(&l, Vec3::ZERO, Vec3::NEG_Y), Vec3::ZERO);
        // no inverse-square: halving the distance does not quadruple the light
        let near = light_term(&l, Vec3::new(0.0, 2.0, 0.0), Vec3::Y).x;
        let far = light_term(&l, Vec3::new(0.0, 1.0, 0.0), Vec3::Y).x;
        assert!(near / far < 1.2, "{near} {far}");
    }

    #[test]
    fn the_grid_holds_four_lights_and_replaces_the_dimmest() {
        let origin = grid_origin(Vec3::ZERO);
        assert_eq!(origin, Vec2::new(-256.0, -256.0));
        let sel: Vec<_> = (0..5)
            .map(|i| Selected {
                pos: Vec3::new(0.5, 2.0, 0.5),
                colour: Vec3::splat(1.0 + i as f32),
                radius: 8.0,
            })
            .collect();
        let cells = build_cells(&sel, origin);
        // the cell holding the point (0.5, 0.5)
        let cx = ((0.5 - origin.x) / CELL) as usize;
        let cell = cells[cx * GRID + cx];
        let ids: Vec<u8> = cell.to_le_bytes().to_vec();
        assert_eq!(ids.iter().filter(|i| **i != 0).count(), 4);
        // the dimmest (light 1, id 1) was replaced by the fifth (id 5)
        assert!(ids.contains(&5) && !ids.contains(&1));
        // far from every light: empty
        assert_eq!(cells[0], 0);
    }

    /// The shared shader parses and validates on its own, and the bindings and cell layout match
    /// the Rust side (64 lights of 32 bytes after a 16-byte header).
    #[test]
    fn the_shared_shader_validates_and_matches_the_buffer_layout() {
        let src = include_str!("point_lights.wgsl")
            .lines()
            .filter(|l| !l.starts_with("#define_import_path"))
            .map(|l| l.replace("#{MATERIAL_BIND_GROUP}", "2"))
            .collect::<Vec<_>>()
            .join("
");
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let mut v = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        v.validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        assert!(src.contains("@binding(120)") && src.contains("@binding(121)"));
        use bevy::render::render_resource::ShaderType;
        assert_eq!(GpuLights::min_size().get(), 16 + 32 * 64);
    }

    #[test]
    fn effect_lights_expire_and_lamps_fade_in() {
        let t = LightTrack {
            offset: [0.0; 3],
            looping: false,
            priority: 1,
            keys: vec![
                jc2_formats::point_light::LightKey {
                    time: 0.0,
                    colour: [8.0, 5.2, 1.6],
                    radius: Some(10.0),
                    clq: [1.0; 3],
                },
                jc2_formats::point_light::LightKey {
                    time: 0.2,
                    colour: [0.0; 3],
                    radius: Some(10.0),
                    clq: [1.0; 3],
                },
            ],
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .add_systems(Update, (tick_lamps, tick_effects));
        let fx = app
            .world_mut()
            .spawn(EffectLight {
                track: Arc::new(t),
                clock: 0.0,
            })
            .id();
        app.update();
        app.update();
        assert!(app.world().get::<EffectLight>(fx).is_some(), "alive at 0.1 s");
        app.update();
        app.update();
        assert!(app.world().get::<EffectLight>(fx).is_none(), "gone after the last key");
        let lamp = app
            .world_mut()
            .spawn(LampLight {
                pos: Vec3::ZERO,
                colour: Vec3::ONE,
                radius: 5.0,
                priority: 0,
                fade_in: 0.2,
                age: 0.0,
            })
            .id();
        assert_eq!(app.world().get::<LampLight>(lamp).unwrap().intensity(), 0.0);
        app.world_mut().get_mut::<LampLight>(lamp).unwrap().age = 0.1;
        assert!((app.world().get::<LampLight>(lamp).unwrap().intensity() - 0.5).abs() < 1e-6);
    }
}
