//! Adapter: `Decal` components (file type 3, native `CECD_VisualDecal` 0x00740e70, instance
//! `CECI_VisualDecal` vftable 0x00eba490) onto Bevy forward decals.
//!
//! Native (research `effects-pipeline.md` section 1.3 and 6, `explosion-effects.md` section 3):
//! `Start` 0x00573150 hands the decal to the decal manager (`CDecalManager` vftable 0x00eba00c) and
//! `IsAlive` is always false (0x00a414e0), so a decal outlives the effect that made it. The
//! variation holds the diffuse / specular / normal texture paths (`#0338eafb`, `#9045606f`,
//! `#f0f75c7f`, `Decals/*.dds`), `lifetime` (10 s bullet marks, 100 s scorch), `size` (4 floats),
//! `#7bb996a0` / `#99bfeaf1` (2 floats each) and `#9b006d73` (an int).
//!
//! The manager's internals (projection volume, atlas addressing, fade, count cap, render path) are
//! the open gap task f7426df4; what follows is labelled by how well the data supports it:
//!
//! * **Inferred from the data, not traced to the consumer:** `#7bb996a0` / `#99bfeaf1` are the
//!   min / max uv corners of the texture cell (the retail bullet decals are cells of the 256 x 128
//!   `BulletImpacts` sheet: `(0, .5)..(.25, 1)` and `(0, 0)..(.25, .5)`; explosion decals use
//!   `(0, 0)..(1, 1)`; reading the second pair as a scale would reach uv 1.5);
//!   `size` = (width, height, depth, ?) in metres: the bullet marks are 0.25 / 0.4 m wide with 0.3
//!   depth, the scorch 4.1 to 6 m. The fourth value is unread.
//! * **Non-native:** the renderer is Bevy's `ForwardDecal` (depth-prepass projection, standard lit
//!   material, blend), not the original decal shader; the specular and normal textures and
//!   `#9b006d73` are carried but unused; the decal is removed when its lifetime is over (the
//!   original fade-out is unknown); at most [`MAX_DECALS`] live decals (the original cap is unknown).

use super::component::{ComponentInit, EffectComponent};
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::prelude::*;
use jc2_formats::property::{Node, Variant};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Variation keys (hashes of the unnamed fields).
pub const KEY_DIFFUSE: u32 = 0x0338_eafb;
pub const KEY_SPECULAR: u32 = 0x9045_606f;
pub const KEY_NORMAL: u32 = 0xf0f7_5c7f;
pub const KEY_UV_MIN: u32 = 0x7bb9_96a0;
pub const KEY_UV_MAX: u32 = 0x99bf_eaf1;
pub const KEY_UNKNOWN_INT: u32 = 0x9b00_6d73;

/// Non-native cap on live decals; the oldest is removed first.
pub const MAX_DECALS: usize = 256;
/// Non-native: Bevy's forward-decal depth blend distance (metres).
const DEPTH_FADE: f32 = 1.0;

/// The fields of one decal variation.
#[derive(Clone, Debug, PartialEq)]
pub struct DecalParams {
    pub diffuse: String,
    pub specular: String,
    pub normal: String,
    /// Seconds before removal.
    pub lifetime: f32,
    /// (width, height, depth, unread), see the module docs.
    pub size: Vec4,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// `#9b006d73`, meaning unknown.
    pub unknown_int: i32,
}

impl DecalParams {
    pub fn from_node(n: &Node) -> Self {
        let s = |k: u32| match n.value_hash(k) {
            Some(Variant::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let v2 = |k: u32, d: Vec2| match n.value_hash(k) {
            Some(Variant::Vec2(v)) => Vec2::from(*v),
            _ => d,
        };
        Self {
            diffuse: s(KEY_DIFFUSE),
            specular: s(KEY_SPECULAR),
            normal: s(KEY_NORMAL),
            lifetime: n.float("lifetime").unwrap_or(0.0),
            size: match n.value("size") {
                Some(Variant::Vec4(v)) => Vec4::from(*v),
                _ => Vec4::ZERO,
            },
            uv_min: v2(KEY_UV_MIN, Vec2::ZERO),
            uv_max: v2(KEY_UV_MAX, Vec2::ONE),
            unknown_int: match n.value_hash(KEY_UNKNOWN_INT) {
                Some(Variant::Int(i)) => *i,
                _ => 0,
            },
        }
    }

    /// The asset path of the diffuse texture (`jc2://` reads the archive member by name).
    pub fn diffuse_asset(&self) -> String {
        format!("jc2://{}", self.diffuse.replace('\\', "/").to_ascii_lowercase())
    }

    /// Decal volume scale for the forward decal (x width, y depth along the normal, z height).
    pub fn volume(&self) -> Vec3 {
        Vec3::new(self.size.x, self.size.z, self.size.y)
    }
}

/// A request from a decal component to the ECS side.
#[derive(Clone, Debug)]
pub struct DecalOp {
    pub params: DecalParams,
    /// The effect matrix at `Start`; its Y axis is the projection direction.
    pub matrix: Mat4,
}

/// The outbox shared by the decal components and [`apply_decal_ops`].
#[derive(Default)]
pub struct DecalQueue {
    ops: Mutex<Vec<DecalOp>>,
    /// Decals started (counted even when no renderer drains them).
    pub started: AtomicU64,
}

impl DecalQueue {
    fn push(&self, op: DecalOp) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(op);
        }
        self.started.fetch_add(1, Ordering::Relaxed);
    }
    pub fn drain(&self) -> Vec<DecalOp> {
        self.ops.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
    }
}

pub struct DecalComponent {
    params: DecalParams,
    queue: Arc<DecalQueue>,
    matrix: Mat4,
    enabled: bool,
}

impl DecalComponent {
    pub fn new(params: DecalParams, queue: Arc<DecalQueue>) -> Self {
        Self { params, queue, matrix: Mat4::IDENTITY, enabled: true }
    }

    /// The constructor registered for kind 3.
    pub fn from_init(init: &ComponentInit, queue: Arc<DecalQueue>) -> Box<dyn EffectComponent> {
        Box::new(Self::new(DecalParams::from_node(&init.variation.node), queue))
    }
}

impl EffectComponent for DecalComponent {
    /// 0x00573150 hands the decal to the manager; a decal without a texture or lifetime is
    /// not placed (inferred), and does not count as started.
    fn start(&mut self) -> bool {
        if !self.enabled || self.params.diffuse.is_empty() {
            return false;
        }
        self.queue.push(DecalOp { params: self.params.clone(), matrix: self.matrix });
        true
    }
    fn stop(&mut self, _immediate: bool) {}
    /// Always false (0x00a414e0): the decal outlives the effect.
    fn is_alive(&self) -> bool {
        false
    }
    fn update(&mut self, _dt: f32) {}
    fn set_transform(&mut self, m: &Mat4) {
        self.matrix = *m;
    }
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// The world transform of a decal from the effect matrix: its axes are orthonormalised, the
/// volume (`DecalParams::volume`) becomes the scale. A degenerate matrix keeps only its position.
pub fn decal_transform(matrix: &Mat4, volume: Vec3) -> Transform {
    let x = matrix.x_axis.truncate().normalize_or_zero();
    let y = matrix.y_axis.truncate().normalize_or_zero();
    let rotation = if x == Vec3::ZERO || y == Vec3::ZERO {
        Quat::IDENTITY
    } else {
        let z = x.cross(y).normalize_or_zero();
        if z == Vec3::ZERO {
            Quat::IDENTITY
        } else {
            Quat::from_mat3(&Mat3::from_cols(y.cross(z), y, z))
        }
    };
    Transform {
        translation: matrix.w_axis.truncate(),
        rotation,
        scale: volume,
    }
}

/// A live decal entity.
#[derive(Component)]
pub struct EffectDecal {
    pub remaining: f32,
}

/// Decals in spawn order (for the cap) and the shared materials per texture and cell.
#[derive(Resource, Default)]
pub struct DecalDraw {
    pub live: VecDeque<Entity>,
    pub spawned: u64,
    /// `JC2_EFFECT_NO_DECAL_DRAW=1`: decals are consumed but not spawned (before/after captures).
    pub disabled: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn apply_decal_ops(
    mut commands: Commands,
    host: Res<super::EffectHost>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<ForwardDecalMaterial<StandardMaterial>>>,
    mut draw: ResMut<DecalDraw>,
    mut decals: Query<(Entity, &mut EffectDecal)>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    let mut expired = Vec::new();
    for (e, mut d) in &mut decals {
        d.remaining -= dt;
        if d.remaining <= 0.0 {
            commands.entity(e).despawn();
            expired.push(e);
        }
    }
    if !expired.is_empty() {
        draw.live.retain(|e| !expired.contains(e));
    }
    for op in host.decals.drain() {
        let p = &op.params;
        if p.lifetime <= 0.0 || draw.disabled {
            continue;
        }
        let tex: Handle<Image> = assets.load(p.diffuse_asset());
        let material = materials.add(ForwardDecalMaterial {
            base: StandardMaterial {
                base_color_texture: Some(tex),
                uv_transform: bevy::math::Affine2::from_scale_angle_translation(
                    p.uv_max - p.uv_min,
                    0.0,
                    p.uv_min,
                ),
                perceptual_roughness: 1.0,
                reflectance: 0.0,
                alpha_mode: AlphaMode::Blend,
                ..default()
            },
            extension: ForwardDecalMaterialExt { depth_fade_factor: DEPTH_FADE },
        });
        let entity = commands
            .spawn((
                ForwardDecal,
                MeshMaterial3d(material),
                decal_transform(&op.matrix, p.volume()),
                EffectDecal { remaining: p.lifetime },
                Name::new("effect decal"),
            ))
            .id();
        draw.live.push_back(entity);
        draw.spawned += 1;
        while draw.live.len() > MAX_DECALS {
            if let Some(old) = draw.live.pop_front() {
                commands.entity(old).try_despawn();
            }
        }
    }
}
