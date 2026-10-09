//! Opt-in original MC smoke leaf. Native indexed updates/age/frame/local space
//! are implemented; RNG, birth direction/scatter, fitted quads and material remain
//! labeled preview approximations. There are no gameplay damage hooks.
use bevy::prelude::*;
use jc2_formats::particle::{
    FIXTURE_EFFECT, FIXTURE_LEAF, Modifier, ModifierKind, ParticleFixture, ParticleSystem,
};
use std::collections::{BTreeMap, BTreeSet};

const EVENT_WORK_LIMIT: usize = 4096;
const OWNER_LIMIT: usize = 64;
#[derive(Clone, Debug)]
struct Emission {
    rate: f32,
    lifetime: f32,
    speed: f32,
    initial_alpha: f32,
    initial_size: [f32; 2],
    initial_frame: [f32; 2],
    alpha_scale: f32,
    size_scale: f32,
    max_particles: usize,
    modifiers: Vec<Modifier>,
}
impl Emission {
    fn decoded(s: &ParticleSystem) -> Result<Self, &'static str> {
        if s.atlas != [8, 8] || s.initial_frame[0] + s.initial_frame[1] >= 64.0 {
            return Err("only the researched 8x8 atlas and in-range birth cells are supported");
        }
        if s.system_lifetime != 0.0 || s.lifetime_variation != 0.0 || s.space != 2 {
            return Err(
                "only zero system/variation lifetime and native local space2 are supported",
            );
        }
        if s.emission_speed != [0.4, 0.4] {
            return Err("unresearched birth speed configuration");
        }
        let config = Self {
            rate: s.emission_rate,
            lifetime: s.lifetime,
            // +Y at fixed .4 is still a diagnostic birth-direction policy.
            speed: 0.4,
            initial_alpha: s.initial_alpha,
            initial_size: s.initial_size,
            initial_frame: s.initial_frame,
            alpha_scale: 1.0, // native instance defaults, not a guessed root field
            size_scale: 1.0,
            max_particles: s.max_particles as usize,
            modifiers: s.execution_modifiers().cloned().collect(),
        };
        if !config.valid() {
            return Err("particle configuration outside bounded native fixture range");
        }
        for m in &config.modifiers {
            if let ModifierKind::Animation { lower, upper, .. } = m.kind
                && (lower > upper || upper >= u32::from(s.atlas[0]) * u32::from(s.atlas[1]))
            {
                return Err("particle animation bounds outside atlas");
            }
        }
        Ok(config)
    }
    fn valid(&self) -> bool {
        self.rate.is_finite()
            && (0.0..=10000.0).contains(&self.rate)
            && self.lifetime.is_finite()
            && (0.001..=60.0).contains(&self.lifetime)
            && self.speed.is_finite()
            && (0.0..=128.0).contains(&self.speed)
            && self.initial_alpha.is_finite()
            && (0.0..=1.0).contains(&self.initial_alpha)
            && self
                .initial_size
                .iter()
                .all(|v| v.is_finite() && (0.0..=100.0).contains(v))
            && self
                .initial_frame
                .iter()
                .all(|v| v.is_finite() && (0.0..=65535.0).contains(v))
            && [self.alpha_scale, self.size_scale]
                .iter()
                .all(|v| v.is_finite() && (0.0..=100.0).contains(v))
            && self.modifiers.iter().all(|m| {
                m.start.is_finite()
                    && m.end.is_finite()
                    && match m.kind {
                        ModifierKind::Animation {
                            lower, upper, rate, ..
                        } => lower <= upper && rate.is_finite() && rate.abs() <= 10000.0,
                        ModifierKind::Size(v)
                        | ModifierKind::Opacity(v)
                        | ModifierKind::Damping(v) => v.is_finite() && v.abs() <= 10000.0,
                        ModifierKind::Force(v) => {
                            v.iter().all(|v| v.is_finite() && v.abs() <= 10000.0)
                        }
                        ModifierKind::Unsupported => true,
                    }
            })
    }
}
#[derive(Clone, Debug)]
struct Source {
    config: Emission,
    seed: u64,
    accumulator: f32,
    ordinal: u64,
    active: bool,
    local: Vec3,
    previous: Transform,
}
#[derive(Clone, Debug, PartialEq)]
struct Particle {
    owner: u64,
    ordinal: u64,
    age: f32,
    position: Vec3,
    velocity: Vec3,
    size: f32,
    alpha: f32,
    frame: f32,
}
fn window(age: f32, start: f32, end: f32) -> bool {
    (start <= 0.0 && end >= 1.0) || (start <= age && age < end)
}
impl Particle {
    fn at(&self, source: &Source) -> Vec3 {
        source.previous.transform_point(self.position)
    }
    fn update(&mut self, dt: f32, source: &Emission) {
        self.age += (1.0 / source.lifetime) * dt;
        self.position += self.velocity * dt;
        for m in &source.modifiers {
            if !window(self.age, m.start, m.end) {
                continue;
            }
            match m.kind {
                ModifierKind::Animation {
                    lower,
                    upper,
                    rate,
                    looping,
                } => {
                    let lower = lower as f32;
                    let upper = upper as f32;
                    self.frame += dt * rate;
                    if looping {
                        if self.frame < lower {
                            self.frame += upper - lower;
                        } else if self.frame >= upper {
                            self.frame -= upper - lower;
                        }
                    }
                    self.frame = self.frame.clamp(lower, upper);
                }
                ModifierKind::Size(coefficient) => {
                    let coefficient = if source.size_scale < 1.0 {
                        coefficient * source.size_scale
                    } else {
                        coefficient
                    };
                    self.size += dt * coefficient;
                }
                ModifierKind::Opacity(coefficient) => {
                    self.alpha =
                        source.alpha_scale * (self.alpha + dt * coefficient).clamp(0.0, 1.0);
                }
                ModifierKind::Force(force) => self.velocity += Vec3::from_array(force) * dt,
                ModifierKind::Damping(coefficient) => {
                    self.velocity *= 1.0 - (dt * coefficient).min(1.0)
                }
                ModifierKind::Unsupported => {}
            }
        }
    }
}
#[derive(Resource, Debug)]
struct ParticlePool {
    clock: f64,
    global_cap: usize,
    owner_cap: usize,
    sources: BTreeMap<u64, Source>,
    particles: Vec<Particle>,
    dropped_budget: u64,
    dropped_authored: u64,
    dropped_work: u64,
}
impl ParticlePool {
    fn new(global_cap: usize, owner_cap: usize) -> Self {
        Self {
            clock: 0.0,
            global_cap,
            owner_cap,
            sources: BTreeMap::new(),
            particles: Vec::new(),
            dropped_budget: 0,
            dropped_authored: 0,
            dropped_work: 0,
        }
    }
    fn start(
        &mut self,
        owner: u64,
        config: Emission,
        seed: u64,
        local: Vec3,
        frame: Transform,
    ) -> Result<(), &'static str> {
        if !config.valid() {
            return Err("invalid particle emission configuration");
        }
        if !self.sources.contains_key(&owner) && self.sources.len() >= OWNER_LIMIT {
            return Err("particle owner budget exhausted");
        }
        if !finite_frame(&frame) || !local.is_finite() {
            return Err("nonfinite particle source transform");
        }
        self.remove(owner);
        self.sources.insert(
            owner,
            Source {
                config,
                seed,
                accumulator: 0.0,
                ordinal: 1,
                active: true,
                local,
                previous: frame,
            },
        );
        Ok(())
    }
    fn stop(&mut self, owner: u64, drain: bool) {
        if let Some(source) = self.sources.get_mut(&owner) {
            source.active = false;
        }
        if !drain {
            self.particles.retain(|p| p.owner != owner);
        }
    }
    fn remove(&mut self, owner: u64) {
        self.sources.remove(&owner);
        self.particles.retain(|p| p.owner != owner);
    }
    /// Native Euler update deliberately depends on dt partition. Survivors are
    /// checked/updated before births. Native strict accumulator>spacing and fmod
    /// phase is retained, including the equality boundary. Overload drops consume
    /// ordinals without queuing bursts; global source scheduling is stable policy.
    fn advance(&mut self, dt: f64, frames: &BTreeMap<u64, Transform>) -> Result<(), &'static str> {
        if !dt.is_finite() || !(0.0..=1_000_000.0).contains(&dt) || !(self.clock + dt).is_finite() {
            return Err("invalid or unrepresentable bounded particle time step");
        }
        if dt > 0.0 && self.clock + dt <= self.clock {
            return Err("particle clock cannot represent time step");
        }
        let step = dt as f32;
        // Validate counter arithmetic before any mutation, including a huge stall.
        for s in self
            .sources
            .values()
            .filter(|s| s.active && s.config.rate > 0.0)
        {
            let spacing = 1.0 / s.config.rate;
            let due = (f64::from(s.accumulator + step) / f64::from(spacing))
                .ceil()
                .max(1.0)
                - 1.0;
            if !due.is_finite()
                || due >= u64::MAX as f64
                || s.ordinal
                    .checked_add(due as u64)
                    .and_then(|n| n.checked_add(EVENT_WORK_LIMIT as u64))
                    .and_then(|n| n.checked_add(1))
                    .is_none()
            {
                return Err("particle birth count exceeds representable event clock");
            }
        }
        let removed: Vec<_> = self
            .sources
            .keys()
            .filter(|id| frames.get(id).is_none_or(|f| !finite_frame(f)))
            .copied()
            .collect();
        for id in removed {
            self.remove(id);
        }
        self.particles.retain(|p| p.age < 1.0 && p.size > 0.0);
        for p in &mut self.particles {
            p.update(step, &self.sources[&p.owner].config);
        }
        let mut work = 0;
        for (owner, s) in &mut self.sources {
            s.previous = frames[owner];
            if !s.active || s.config.rate == 0.0 {
                continue;
            }
            let spacing = 1.0 / s.config.rate;
            s.accumulator += step;
            while s.accumulator > spacing && work < EVENT_WORK_LIMIT {
                s.accumulator -= spacing;
                let ordinal = s.ordinal;
                s.ordinal += 1; // preflight guarantees space for this whole step
                work += 1;
                let owner_count = self.particles.iter().filter(|p| p.owner == *owner).count();
                if owner_count >= s.config.max_particles {
                    self.dropped_authored = self.dropped_authored.saturating_add(1);
                    continue;
                }
                if self.particles.len() >= self.global_cap || owner_count >= self.owner_cap {
                    self.dropped_budget = self.dropped_budget.saturating_add(1);
                    continue;
                }
                let random = random01(s.seed, ordinal);
                let angle = random * std::f32::consts::TAU;
                // Deterministic local disk/+Y birth direction remain approximations.
                let offset = Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.035;
                let frame_random = (random01(s.seed ^ 0xabc, ordinal) * 32768.0) as u32;
                self.particles.push(Particle {
                    owner: *owner,
                    ordinal,
                    age: 0.0,
                    position: s.local + offset,
                    velocity: Vec3::Y * s.config.speed,
                    size: s.config.initial_size[0]
                        + random01(s.seed ^ 0xdef, ordinal) * s.config.initial_size[1],
                    alpha: s.config.initial_alpha,
                    frame: s.config.initial_frame[0]
                        + if s.config.initial_frame[1] > 0.0 {
                            (frame_random % (s.config.initial_frame[1] as u32 + 1)) as f32
                        } else {
                            0.0
                        },
                });
            }
            if s.accumulator > spacing {
                let dropped = (f64::from(s.accumulator) / f64::from(spacing)).ceil() as u64 - 1;
                self.dropped_work = self.dropped_work.saturating_add(dropped);
                s.ordinal = s
                    .ordinal
                    .checked_add(dropped)
                    .ok_or("particle ordinal overflow")?;
            }
            s.accumulator %= spacing;
        }
        self.clock += dt;
        Ok(())
    }
}
fn finite_frame(f: &Transform) -> bool {
    f.translation.is_finite() && f.rotation.is_finite() && f.scale.is_finite()
}
fn random01(seed: u64, ordinal: u64) -> f32 {
    let mut x = seed.wrapping_add(ordinal.wrapping_mul(0x9e3779b97f4a7c15));
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((x ^ (x >> 31)) >> 40) as f32 / 16777216.0
}

#[derive(Resource)]
pub struct ParticleViewerConfig {
    pub leaf: String,
    pub frame: Option<u32>,
    pub move_source: bool,
    pub seed: u64,
    pub global_cap: usize,
    pub owner_cap: usize,
    pub stop_after: Option<f64>,
}
pub struct ParticleViewerPlugin;
impl Plugin for ParticleViewerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (controls, tick, draw).chain());
    }
}
#[derive(Component)]
struct ParticleOwner;
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewText;
#[derive(Component)]
struct DrawnParticle(u64, u64, Handle<StandardMaterial>);
#[derive(Resource)]
struct Preview {
    emission: Emission,
    owner: Option<Entity>,
    moving: bool,
    stopped: bool,
    meshes: Vec<Handle<Mesh>>,
    material: Handle<StandardMaterial>,
    texture: Handle<Image>,
    texture_ready: bool,
}
fn setup(
    mut commands: Commands,
    game: Res<crate::Game>,
    cfg: Res<ParticleViewerConfig>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let fixture = match ParticleFixture::load(&game.fs, FIXTURE_EFFECT, &cfg.leaf) {
        Ok(f) => f,
        Err(e) => {
            error!("particle fixture: {e}");
            commands.write_message(AppExit::error());
            return;
        }
    };
    let emission = match Emission::decoded(&fixture.system) {
        Ok(s) => s,
        Err(e) => {
            error!("particle fixture: {e}");
            commands.write_message(AppExit::error());
            return;
        }
    };
    if let Some(frame) = cfg.frame
        && let Err(e) = fixture.system.preview_uv(frame)
    {
        error!("particle fixture: {e}");
        commands.write_message(AppExit::error());
        return;
    }
    for modifier in &emission.modifiers {
        if matches!(modifier.kind, ModifierKind::Unsupported) {
            warn!(
                "particle indexed modifier {} is retained but unsupported",
                modifier.class
            );
        }
    }
    info!(
        "particle fixture {FIXTURE_EFFECT} -> {FIXTURE_LEAF}.psmb -> {}; rate={} life={} atlas={:?}; {} indexed modifiers evaluated, local space2; child {:?} not emitted; static diagnostic frame={:?} seed={} budgets={}/{}; RNG/+Y birth/full-cell geometry/unlit blend/host job dt remain approximations; no gameplay replacement",
        fixture.texture_member,
        emission.rate,
        emission.lifetime,
        fixture.system.atlas,
        emission.modifiers.len(),
        fixture.system.child_link,
        cfg.frame,
        cfg.seed,
        cfg.global_cap,
        cfg.owner_cap
    );
    let atlas_count = u32::from(fixture.system.atlas[0]) * u32::from(fixture.system.atlas[1]);
    let cell_meshes = (0..atlas_count)
        .map(|cell| {
            // Full unit cell geometry is explicit; fitted native corners are unknown.
            let mut mesh = Mesh::from(Rectangle::new(1.0, 1.0));
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                fixture.system.preview_uv(cell).unwrap().to_vec(),
            );
            meshes.add(mesh)
        })
        .collect();
    let texture: Handle<Image> =
        assets.load(format!("jc2://particles.blz/{}", fixture.texture_member));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(texture.clone()),
        base_color: Color::WHITE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(ClearColor(Color::srgb(0.12, 0.15, 0.19)));
    commands.spawn((
        Camera3d::default(),
        PreviewCamera,
        Transform::from_xyz(0.0, 0.2, 0.9).looking_at(Vec3::new(0.0, 0.07, 0.0), Vec3::Y),
    ));
    let frame = Transform::IDENTITY;
    let owner = commands
        .spawn((ParticleOwner, frame, GlobalTransform::default()))
        .id();
    let mut pool = ParticlePool::new(cfg.global_cap.min(4096), cfg.owner_cap.min(4096));
    pool.start(
        owner.to_bits(),
        emission.clone(),
        cfg.seed,
        Vec3::ZERO,
        frame,
    )
    .expect("finite fixture source");
    commands.insert_resource(pool);
    commands.insert_resource(Preview {
        emission,
        owner: Some(owner),
        moving: cfg.move_source,
        stopped: false,
        meshes: cell_meshes,
        material,
        texture,
        texture_ready: false,
    });
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(17.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            top: Val::Px(16.0),
            ..default()
        },
        PreviewText,
    ));
}
fn controls(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ParticleViewerConfig>,
    preview: Option<ResMut<Preview>>,
    pool: Option<ResMut<ParticlePool>>,
    mut owners: Query<&mut Transform, With<ParticleOwner>>,
) {
    let (Some(mut preview), Some(mut pool)) = (preview, pool) else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyM) {
        preview.moving = !preview.moving;
    }
    if let Some(owner) = preview.owner {
        if keys.just_pressed(KeyCode::Space) {
            pool.stop(owner.to_bits(), true);
            preview.stopped = true;
        }
        if keys.just_pressed(KeyCode::KeyX) {
            commands.entity(owner).despawn();
            preview.owner = None;
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        let owner = if let Some(owner) = preview.owner {
            owner
        } else {
            let owner = commands
                .spawn((
                    ParticleOwner,
                    Transform::IDENTITY,
                    GlobalTransform::default(),
                ))
                .id();
            preview.owner = Some(owner);
            owner
        };
        let frame = owners
            .get_mut(owner)
            .map(|f| *f)
            .unwrap_or(Transform::IDENTITY);
        if let Err(e) = pool.start(
            owner.to_bits(),
            preview.emission.clone(),
            cfg.seed,
            Vec3::ZERO,
            frame,
        ) {
            error!("particle restart: {e}");
        }
        preview.stopped = false;
    }
}
fn tick(
    time: Res<Time>,
    cfg: Res<ParticleViewerConfig>,
    preview: Option<ResMut<Preview>>,
    pool: Option<ResMut<ParticlePool>>,
    mut owners: Query<(Entity, &mut Transform), With<ParticleOwner>>,
) {
    let (Some(mut preview), Some(mut pool)) = (preview, pool) else {
        return;
    };
    let dt = time.delta_secs_f64();
    let mut frames = BTreeMap::new();
    for (entity, mut frame) in &mut owners {
        if preview.moving {
            let phase = (pool.clock + dt) as f32;
            frame.translation.x = (phase * 0.8).sin() * 0.22;
            frame.rotation = Quat::from_rotation_y(phase * 0.5);
            frame.scale.x = 1.0 + (phase * 0.8).sin() * 0.15;
        }
        frames.insert(entity.to_bits(), *frame);
    }
    // Split a frame at the configured stop time: no extra whole-frame births.
    if let Some(at) = cfg
        .stop_after
        .filter(|at| *at <= pool.clock + dt && !preview.stopped)
    {
        let first = (at - pool.clock).clamp(0.0, dt);
        let alpha = if dt == 0.0 { 1.0 } else { (first / dt) as f32 };
        let middle = frames
            .iter()
            .map(|(id, current)| {
                let previous = pool.sources.get(id).map(|s| s.previous).unwrap_or(*current);
                (
                    *id,
                    Transform {
                        translation: previous.translation.lerp(current.translation, alpha),
                        rotation: previous.rotation.slerp(current.rotation, alpha),
                        scale: previous.scale.lerp(current.scale, alpha),
                    },
                )
            })
            .collect();
        if let Err(e) = pool.advance(first, &middle) {
            error!("particle step: {e}");
        }
        if let Some(owner) = preview.owner {
            pool.stop(owner.to_bits(), true);
        }
        preview.stopped = true;
        if let Err(e) = pool.advance(dt - first, &frames) {
            error!("particle drain: {e}");
        }
    } else if let Err(e) = pool.advance(dt, &frames) {
        error!("particle step: {e}");
    }
}
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    preview: Option<ResMut<Preview>>,
    pool: Option<Res<ParticlePool>>,
    cfg: Option<Res<ParticleViewerConfig>>,
    images: Option<Res<Assets<Image>>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cameras: Query<&Transform, With<PreviewCamera>>,
    mut existing: Query<
        (Entity, &DrawnParticle, &mut Transform, &mut Mesh3d),
        Without<PreviewCamera>,
    >,
    mut text: Query<&mut Text, With<PreviewText>>,
) {
    let (Some(mut preview), Some(pool)) = (preview, pool) else {
        return;
    };
    if !preview.texture_ready
        && let Some(image) = images
            .as_ref()
            .and_then(|images| images.get(&preview.texture))
    {
        preview.texture_ready = true;
        info!(
            "particle fixture DDS loaded: particles.blz/fumefx_bomb_01.dds {}x{} {:?}",
            image.texture_descriptor.size.width,
            image.texture_descriptor.size.height,
            image.texture_descriptor.format
        );
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let live: BTreeMap<_, _> = pool
        .particles
        .iter()
        .map(|p| ((p.owner, p.ordinal), p))
        .collect();
    let mut seen = BTreeSet::new();
    let cell = |p: &Particle| -> usize {
        cfg.as_ref()
            .and_then(|c| c.frame)
            .unwrap_or(p.frame.round_ties_even() as u32) as usize
    };
    for (entity, key, mut transform, mut mesh) in &mut existing {
        if let Some(particle) = live.get(&(key.0, key.1)) {
            transform.translation = particle.at(&pool.sources[&particle.owner]);
            transform.rotation = camera.rotation;
            transform.scale = Vec3::splat(particle.size);
            mesh.0 = preview.meshes[cell(particle)].clone();
            if let Some(mut material) = materials.get_mut(&key.2) {
                material.base_color = Color::srgba(1.0, 1.0, 1.0, particle.alpha);
            }
            seen.insert((key.0, key.1));
        } else {
            materials.remove(key.2.id());
            commands.entity(entity).despawn();
        }
    }
    for (key, particle) in live {
        if !seen.contains(&key) {
            let mut material = materials
                .get(&preview.material)
                .cloned()
                .unwrap_or_default();
            material.base_color = Color::srgba(1.0, 1.0, 1.0, particle.alpha);
            let material = materials.add(material);
            commands.spawn((
                DrawnParticle(key.0, key.1, material.clone()),
                Mesh3d(preview.meshes[cell(particle)].clone()),
                MeshMaterial3d(material),
                Transform {
                    translation: particle.at(&pool.sources[&particle.owner]),
                    rotation: camera.rotation,
                    scale: Vec3::splat(particle.size),
                },
            ));
        }
    }
    if let Ok(mut text) = text.single_mut() {
        let cells = pool.particles.iter().map(cell).collect::<BTreeSet<_>>();
        let first_alpha = pool.particles.first().map_or(0.0, |p| p.alpha);
        let alpha = pool
            .particles
            .iter()
            .map(|p| p.alpha)
            .fold([first_alpha, first_alpha], |[lo, hi], v| {
                [lo.min(v), hi.max(v)]
            });
        let source_x = preview
            .owner
            .and_then(|id| pool.sources.get(&id.to_bits()))
            .map_or(0.0, |s| s.previous.translation.x);
        **text = format!(
            "{FIXTURE_LEAF} | indexed native updates, local space2 | 25/s, life 1s\nLive {} | budget drops {} | source-cap drops {} | stall drops {} | {} | {}\nCells {:?} | alpha {:.2}..{:.2} | source x={:.2} | {}\nSpace: stop/drain   R: restart   M: move source   X: remove owner\nApproximations: RNG/+Y births, full-cell quads, unlit blend, host job dt; no child routing",
            pool.particles.len(),
            pool.dropped_budget,
            pool.dropped_authored,
            pool.dropped_work,
            if preview.stopped {
                "draining"
            } else {
                "emitting"
            },
            if preview.texture_ready {
                "DDS loaded"
            } else {
                "DDS loading"
            },
            cells,
            alpha[0],
            alpha[1],
            source_x,
            if cfg.as_ref().is_some_and(|c| c.frame.is_some()) {
                "static diagnostic cell override"
            } else {
                "animated native cell selection"
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn emission() -> Emission {
        let modifiers = [
            (
                ModifierKind::Animation {
                    lower: 30,
                    upper: 50,
                    rate: 30.0,
                    looping: false,
                },
                0.0,
                1.0,
            ),
            (ModifierKind::Size(0.2), 0.0, 1.0),
            (ModifierKind::Opacity(4.0), 0.0, 0.2),
            (ModifierKind::Opacity(-1.168), 0.4579, 1.0),
            (ModifierKind::Force([0.0, 1.0, 0.0]), 0.0, 1.0),
            (ModifierKind::Damping(5.0), 0.0, 1.0),
        ]
        .into_iter()
        .map(|(kind, start, end)| Modifier {
            key: 0,
            class: String::new(),
            start,
            end,
            kind,
            raw: jc2_formats::property::Node::default(),
        })
        .collect();
        Emission {
            rate: 25.0,
            lifetime: 1.0,
            speed: 0.4,
            initial_alpha: 0.0,
            initial_size: [0.08, 0.04],
            initial_frame: [30.0, 10.0],
            alpha_scale: 1.0,
            size_scale: 1.0,
            max_particles: 25,
            modifiers,
        }
    }
    fn frames(x: f32) -> BTreeMap<u64, Transform> {
        BTreeMap::from([(1, Transform::from_xyz(x, 0.0, 0.0))])
    }
    #[test]
    fn native_birth_phase_newborn_suppression_and_moving_source() {
        let mut pool = ParticlePool::new(16, 12);
        pool.start(1, emission(), 7, Vec3::ZERO, Transform::IDENTITY)
            .unwrap();
        // Native strict > and final fmod discard exactly one spacing even when
        // no birth was created. This is intentionally not014 event-clock parity.
        pool.advance(f64::from(0.04f32), &frames(0.0)).unwrap();
        assert!(pool.particles.is_empty());
        pool.advance(f64::from(0.04f32), &frames(0.0)).unwrap();
        assert!(pool.particles.is_empty());
        pool.advance(
            f64::from(f32::from_bits(0.04f32.to_bits() + 1)),
            &frames(0.0),
        )
        .unwrap();
        assert_eq!(pool.particles.len(), 1);
        let p = &pool.particles[0];
        assert_eq!((p.age, p.alpha), (0.0, 0.0));
        assert!((30.0..=40.0).contains(&p.frame));
        assert!((0.08..=0.12).contains(&p.size));
        pool.stop(1, true);
        // Existing survivor is local. Rotate/scale a literal point to prove its
        // world placement changes while dt=0 preserves particle simulation state.
        pool.particles[0].position = Vec3::new(1.0, 2.0, 3.0);
        let first = Transform::from_xyz(10.0, 20.0, 30.0)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(2.0, 3.0, 4.0));
        pool.advance(0.0, &BTreeMap::from([(1, first)])).unwrap();
        assert!(
            pool.particles[0]
                .at(&pool.sources[&1])
                .distance(Vec3::new(4.0, 22.0, 42.0))
                < 1e-5
        );
        let second = Transform {
            translation: Vec3::new(20.0, 30.0, 40.0),
            ..first
        };
        pool.advance(0.0, &BTreeMap::from([(1, second)])).unwrap();
        assert_eq!(pool.particles[0].position, Vec3::new(1.0, 2.0, 3.0));
        assert!(
            pool.particles[0]
                .at(&pool.sources[&1])
                .distance(Vec3::new(14.0, 32.0, 52.0))
                < 1e-5
        );
        pool.advance(2.0, &BTreeMap::from([(1, second)])).unwrap();
        assert_eq!(pool.particles.len(), 1); // cutoff is at entry, not after advance
        pool.advance(0.0, &BTreeMap::from([(1, second)])).unwrap();
        assert!(pool.particles.is_empty());
        assert!(!pool.sources[&1].active);
    }
    #[test]
    fn instances_global_caps_zero_rate_restart_and_stall_work_are_bounded() {
        let mut pool = ParticlePool::new(3, 2);
        for id in [2, 1] {
            pool.start(id, emission(), 1, Vec3::ZERO, Transform::IDENTITY)
                .unwrap();
        }
        let two = BTreeMap::from([(1, Transform::IDENTITY), (2, Transform::IDENTITY)]);
        pool.advance(0.1, &two).unwrap();
        assert_eq!(
            pool.particles.iter().map(|p| p.owner).collect::<Vec<_>>(),
            [1, 1, 2]
        );
        assert_eq!(pool.dropped_budget, 1);
        let mut authored = ParticlePool::new(64, 64);
        authored
            .start(1, emission(), 1, Vec3::ZERO, Transform::IDENTITY)
            .unwrap();
        authored.advance(2.0, &frames(0.0)).unwrap();
        assert_eq!(authored.particles.len(), 25);
        assert!(authored.dropped_authored > 0);
        assert_eq!(authored.dropped_budget, 0);
        authored.advance(100000.0, &frames(0.0)).unwrap();
        assert!(authored.particles.iter().all(|p| {
            p.position.is_finite()
                && p.velocity.is_finite()
                && [p.age, p.size, p.alpha, p.frame]
                    .iter()
                    .all(|v| v.is_finite())
        }));
        pool.stop(1, false);
        assert_eq!(pool.particles.len(), 1);
        pool.remove(2);
        assert!(pool.particles.is_empty());
        pool.start(
            1,
            Emission {
                rate: 0.0,
                ..emission()
            },
            1,
            Vec3::ZERO,
            Transform::IDENTITY,
        )
        .unwrap();
        pool.advance(2.0, &frames(0.0)).unwrap();
        assert!(pool.particles.is_empty());
        pool.start(1, emission(), 1, Vec3::ZERO, Transform::IDENTITY)
            .unwrap();
        pool.advance(100000.0, &frames(0.0)).unwrap();
        assert!(pool.dropped_work > 2_000_000);
        assert_eq!(pool.particles.len(), 2); // same-job births remain newborns
        let state = (pool.clock, pool.particles.clone(), pool.sources[&1].ordinal);
        assert!(pool.advance(f64::NAN, &frames(0.0)).is_err());
        assert!(pool.advance(1e300, &frames(0.0)).is_err());
        assert_eq!(
            (pool.clock, pool.particles.clone(), pool.sources[&1].ordinal),
            state
        );
        let ordinal = pool.sources[&1].ordinal;
        pool.sources.get_mut(&1).unwrap().ordinal = u64::MAX - 4096;
        let clock = pool.clock;
        let particles = pool.particles.clone();
        assert!(pool.advance(0.05, &frames(0.0)).is_err());
        assert_eq!(pool.clock, clock);
        assert_eq!(pool.particles, particles);
        pool.sources.get_mut(&1).unwrap().ordinal = ordinal;
        assert!(
            pool.start(
                9,
                Emission {
                    rate: -1.0,
                    ..emission()
                },
                1,
                Vec3::ZERO,
                Transform::IDENTITY
            )
            .is_err()
        );
        assert!(
            pool.start(
                9,
                Emission {
                    lifetime: f32::INFINITY,
                    ..emission()
                },
                1,
                Vec3::ZERO,
                Transform::IDENTITY
            )
            .is_err()
        );
        pool.stop(1, true);
        pool.advance(2.0, &frames(0.0)).unwrap();
        pool.advance(0.0, &frames(0.0)).unwrap();
        pool.start(1, emission(), 1, Vec3::ZERO, Transform::IDENTITY)
            .unwrap();
        pool.advance(0.05, &frames(0.0)).unwrap();
        assert_eq!(pool.particles.len(), 1);
        assert_eq!(pool.particles[0].ordinal, 1);
        pool.advance(0.0, &BTreeMap::new()).unwrap();
        assert!(pool.sources.is_empty());
        assert!(pool.particles.is_empty());
    }
    #[test]
    fn original_native_windows_size_frame_and_euler_order() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP original native particle update: JC2_TEST_DIR absent");
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let fixture = ParticleFixture::load(&fs, FIXTURE_EFFECT, FIXTURE_LEAF).unwrap();
        let config = Emission::decoded(&fixture.system).unwrap();
        let mut p = Particle {
            owner: 1,
            ordinal: 1,
            age: 0.0,
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            size: config.initial_size[0],
            alpha: config.initial_alpha,
            frame: config.initial_frame[0],
        };
        p.update(0.125, &config);
        assert_eq!(p.age, 0.125);
        assert_eq!(p.frame, 33.75);
        assert!((p.size - 0.105).abs() < 1e-7);
        assert_eq!(p.alpha, 0.5);
        assert_eq!(p.velocity.y, 0.046875); // force then damping, not .125
        assert_eq!(p.position, Vec3::ZERO); // old velocity integration before force
        let mut partitioned = p.clone();
        p.update(0.125, &config);
        assert_eq!(p.position.y, 0.005859375);
        assert_eq!(p.alpha, 0.5); // updated age .25 excludes rise
        partitioned.update(0.0625, &config);
        partitioned.update(0.0625, &config);
        assert_ne!(p.velocity, partitioned.velocity); // native Euler partition dependence
        assert_ne!(p.alpha, partitioned.alpha); // intermediate .1875 window executed
        for (value, expected) in [
            (0.2f32, [true, false, false]),
            (0.4579f32, [false, true, true]),
        ] {
            let ages = [
                f32::from_bits(value.to_bits() - 1),
                value,
                f32::from_bits(value.to_bits() + 1),
            ];
            let (start, end) = if value == 0.2 {
                (0.0, value)
            } else {
                (value, 1.0)
            };
            assert_eq!(ages.map(|age| window(age, start, end)), expected);
        }
        p.age = 0.6;
        p.alpha = 0.5;
        p.update(0.125, &config);
        assert_eq!(p.alpha, 0.354);
        p.alpha = 0.1;
        p.age = 0.6;
        p.update(0.25, &config);
        assert_eq!(p.alpha, 0.0);
        p.age = 0.99;
        p.alpha = 0.5;
        p.frame = 49.5;
        let size = p.size;
        p.update(0.02, &config);
        assert!(p.age > 1.0);
        assert!(p.size > size); // full window bypass still executes
        assert_eq!(p.alpha, 0.5); // partial opacity end1 does not execute
        assert_eq!(p.frame, 50.0);
        for (frame, expected) in [
            (30.5f32, [30, 30, 31]),
            (31.5, [31, 32, 32]),
            (49.5, [49, 50, 50]),
        ] {
            let values = [
                f32::from_bits(frame.to_bits() - 1),
                frame,
                f32::from_bits(frame.to_bits() + 1),
            ];
            assert_eq!(
                values.map(|v| fixture.system.playback_cell(v).unwrap()),
                expected
            );
        }
    }
    #[test]
    fn blast_damage_does_not_depend_on_particle_budget() {
        use crate::combat::Health;
        use crate::explosion::{Blast, Blasts, ExplosionPlugin};
        let mut results = Vec::new();
        for budget in [0, 32] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                TransformPlugin,
                AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::scene::ScenePlugin,
                avian3d::prelude::PhysicsPlugins::default(),
            ))
            .insert_resource(crate::vehicle::CameraMode::Hero)
            .init_asset::<StandardMaterial>()
            .add_plugins(ExplosionPlugin);
            let mut pool = ParticlePool::new(budget, budget);
            pool.start(1, emission(), 5, Vec3::ZERO, Transform::IDENTITY)
                .unwrap();
            pool.advance(0.2, &frames(0.0)).unwrap();
            // Strict binary32 subtraction leaves the fifth interval below spacing.
            assert_eq!(pool.particles.len(), if budget == 0 { 0 } else { 4 });
            app.insert_resource(pool);
            let victim = app
                .world_mut()
                .spawn((
                    GlobalTransform::from_translation(Vec3::X * 2.5),
                    Health::new(100.0),
                ))
                .id();
            app.world_mut().resource_mut::<Blasts>().queue.push(Blast {
                at: Vec3::ZERO,
                radius: 5.0,
                damage: 80.0,
                impulse: 0.0,
            });
            app.finish();
            app.update();
            results.push((
                app.world().get::<Health>(victim).unwrap().hp,
                app.world().resource::<Blasts>().count,
            ));
        }
        assert_eq!(results, [(60.0, 1), (60.0, 1)]);
    }
    #[test]
    fn original_leaf_config_owner_despawn_cleanup_and_restart_generation() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP original particle runtime: JC2_TEST_DIR absent");
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let fixture = ParticleFixture::load(&fs, FIXTURE_EFFECT, FIXTURE_LEAF).unwrap();
        let config = Emission::decoded(&fixture.system).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ParticlePool::new(64, 32));
        app.insert_resource(Assets::<StandardMaterial>::default());
        app.add_systems(
            Update,
            (
                |mut pool: ResMut<ParticlePool>,
                 owners: Query<(Entity, &Transform), With<ParticleOwner>>| {
                    let frames = owners
                        .iter()
                        .map(|(id, frame)| (id.to_bits(), *frame))
                        .collect();
                    pool.advance(0.2, &frames).unwrap();
                },
                draw,
            )
                .chain(),
        );
        app.world_mut().spawn((PreviewCamera, Transform::IDENTITY));
        app.insert_resource(Preview {
            emission: config.clone(),
            owner: None,
            moving: false,
            stopped: false,
            meshes: vec![Handle::default(); 64],
            material: Handle::default(),
            texture: Handle::default(),
            texture_ready: false,
        });
        let old = app
            .world_mut()
            .spawn((ParticleOwner, Transform::IDENTITY))
            .id();
        app.world_mut()
            .resource_mut::<ParticlePool>()
            .start(
                old.to_bits(),
                config.clone(),
                3,
                Vec3::ZERO,
                Transform::IDENTITY,
            )
            .unwrap();
        app.update();
        assert_eq!(app.world().resource::<ParticlePool>().particles.len(), 4);
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 4);
        assert_eq!(
            app.world_mut()
                .query::<&DrawnParticle>()
                .iter(app.world())
                .count(),
            4
        );
        app.world_mut().despawn(old);
        app.update();
        let pool = app.world().resource::<ParticlePool>();
        assert!(pool.sources.is_empty());
        assert!(pool.particles.is_empty());
        assert_eq!(app.world().resource::<Assets<StandardMaterial>>().len(), 0);
        assert_eq!(
            app.world_mut()
                .query::<&DrawnParticle>()
                .iter(app.world())
                .count(),
            0
        );
        let new = app
            .world_mut()
            .spawn((ParticleOwner, Transform::IDENTITY))
            .id();
        assert_ne!(old.to_bits(), new.to_bits());
        app.world_mut()
            .resource_mut::<ParticlePool>()
            .start(new.to_bits(), config, 3, Vec3::ZERO, Transform::IDENTITY)
            .unwrap();
        app.update();
        let pool = app.world().resource::<ParticlePool>();
        assert_eq!(pool.particles.len(), 4);
        assert!(pool.particles.iter().all(|p| p.owner == new.to_bits()));
        // No sound-bank resources or recursive child owners are created.
        assert_eq!(pool.sources.len(), 1);
    }
}
