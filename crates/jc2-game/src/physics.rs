//! Milestone 4: physics on Avian3d.
//!
//! * terrain tiles become height-field colliders (`terrain_collider`),
//! * the game's own Havok collision shapes (`.pfx`, placed by each tile's `.pfi` list)
//!   become Avian colliders (`colliders_from_body`); models without one fall back to a
//!   triangle mesh built from their render mesh,
//! * `B` drops a ball from the camera to see it all work.

use crate::coords::{bevy_xyz, mat_to_bevy, to_bevy};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::{CollisionBody, CollisionShape};
use jc2_formats::placement::Transform3;
use jc2_formats::terrain::{TILE_CELLS, TILE_SIZE, TerrainTile};

pub struct JcPhysicsPlugin;

impl Plugin for JcPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
            .add_observer(never_sleep)
            .add_systems(Update, drop_ball);
    }
}

/// No body ever sleeps. This was added against the solver's "index out of bounds" panic
/// at `solver/plugin.rs:398`, blamed then on island wake-up; the real cause was seating
/// Rico with `RigidBodyDisabled`/`ColliderDisabled` (see [`stow_passenger`]), which
/// crashes with or without sleeping. Kept: it is harmless, and the open world's bodies
/// are few enough not to need sleep.
fn never_sleep(add: On<Add, RigidBody>, mut commands: Commands) {
    commands.entity(add.entity).insert(SleepingDisabled);
}

/// A passenger's collision state, kept while [`stow_passenger`] has made the body inert.
#[derive(Component, Clone, Copy, Debug)]
pub struct Stowed {
    layers: CollisionLayers,
    gravity: Option<GravityScale>,
}

/// Put a character into a vehicle: its body stays, but collides with nothing, feels no
/// gravity and stands still until [`release_passenger`]. The caller hides it and keeps it
/// on the seat.
///
/// Do not use `RigidBodyDisabled`/`ColliderDisabled` for this. In Avian 0.7.0 removing
/// them together leaves the collider's broad-phase proxy flagged `BODY_DISABLED`, so every
/// later contact of that collider starts with `STARTED_GENERATING_CONSTRAINTS` set and,
/// once its manifold count changes, is pushed into the constraint graph a second time;
/// the solver then panics at `solver/plugin.rs:398` ("index out of bounds: the len is 2
/// but the index is 2"). See `research/formats/havok.md`, "Passengers". Empty
/// [`CollisionLayers`] end the contacts through the narrow phase's ordinary
/// "AABBs disjoint" path instead.
pub fn stow_passenger(commands: &mut Commands, passenger: Entity, vehicle: Entity) {
    commands
        .entity(passenger)
        .queue(move |mut e: EntityWorldMut| {
            if e.contains::<Stowed>() {
                return;
            }
            let stowed = Stowed {
                layers: e.get::<CollisionLayers>().copied().unwrap_or_default(),
                gravity: e.get::<GravityScale>().copied(),
            };
            e.insert((
                stowed,
                CollisionLayers::NONE,
                GravityScale(0.0),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ));
            // onto the seat at once; the character's own systems keep it there
            let seat = e.world_scope(|w| w.get::<Position>(vehicle).copied());
            if let Some(seat) = seat {
                e.insert(seat);
                if let Some(mut t) = e.get_mut::<Transform>() {
                    t.translation = seat.0;
                }
            }
        });
}

/// Take a character out of a vehicle: its collision layers and gravity come back, and it
/// is placed at `position` moving at `velocity`.
pub fn release_passenger(
    commands: &mut Commands,
    passenger: Entity,
    position: Vec3,
    velocity: Vec3,
) {
    commands
        .entity(passenger)
        .queue(move |mut e: EntityWorldMut| {
            if let Some(stowed) = e.take::<Stowed>() {
                e.insert(stowed.layers);
                match stowed.gravity {
                    Some(g) => e.insert(g),
                    None => e.remove::<GravityScale>(),
                };
            }
            e.insert((Position(position), LinearVelocity(velocity)));
            if let Some(mut t) = e.get_mut::<Transform>() {
                t.translation = position;
            }
            // The broad phase only looks for new pairs around proxies that moved, and a layer
            // change alone does not count. A body's proxy usually moves anyway (the teleport,
            // falling), but mark it so a release right where he was stowed finds the ground.
            if let Some(key) = e
                .get::<avian3d::collider_tree::ColliderTreeProxyKey>()
                .copied()
                && key != avian3d::collider_tree::ColliderTreeProxyKey::PLACEHOLDER
            {
                e.world_scope(|w| {
                    if let Some(mut moved) =
                        w.get_resource_mut::<avian3d::collider_tree::MovedProxies>()
                    {
                        moved.insert(key);
                    }
                });
            }
        });
}

/// Marker for the terrain's static bodies.
#[derive(Component)]
pub struct TerrainBody;

/// A height-field collider for one tile, and where to put it.
///
/// Parry centres the field on its entity; its matrix is stored column-major with rows
/// along z and columns along x, and Avian's `Vec<Vec<f32>>` constructor flattens the
/// outer vector first, so the outer index here is **x** and the inner one is parry's
/// row, i.e. z. Bevy's z is the engine's (`coords`), so parry row `i` is the tile's cell
/// row `i`, running with z from the tile's origin `(x0, z0)`.
pub fn terrain_collider(tile: &TerrainTile, x0: f32, z0: f32) -> (Collider, Transform) {
    let n = TILE_CELLS + 1;
    let mut columns: Vec<Vec<f32>> = Vec::with_capacity(n);
    for cx in 0..n {
        columns.push((0..n).map(|i| tile.cell_height(i, cx)).collect());
    }
    let collider = Collider::heightfield(columns, Vec3::new(TILE_SIZE, 1.0, TILE_SIZE));
    let centre = bevy_xyz(x0 + TILE_SIZE / 2.0, 0.0, z0 + TILE_SIZE / 2.0);
    (collider, Transform::from_translation(centre))
}

/// Position and rotation (Bevy axes) of an engine-space transform.
pub fn transform_parts(t: &Transform3) -> (Vec3, Quat) {
    let tf = mat_to_bevy(t);
    (tf.translation, tf.rotation)
}

/// One leaf shape as an Avian collider plus its placement relative to the body.
fn leaf_collider(tf: &Transform3, leaf: &CollisionShape) -> Option<(Vec3, Quat, Collider)> {
    let (pos, rot) = transform_parts(tf);
    Some(match leaf {
        CollisionShape::Box {
            half_extents: h, ..
        } => (
            pos,
            rot,
            Collider::cuboid(2.0 * h[0], 2.0 * h[1], 2.0 * h[2]),
        ),
        CollisionShape::Sphere { radius } => (pos, rot, Collider::sphere(*radius)),
        CollisionShape::Capsule { a, b, radius } => (
            pos,
            rot,
            Collider::capsule_endpoints(*radius, to_bevy(*a), to_bevy(*b)),
        ),
        CollisionShape::Cylinder { a, b, radius } => {
            // Avian cylinders stand along y: place one at the segment's middle, turned
            // onto the segment's direction
            let (a, b) = (to_bevy(*a), to_bevy(*b));
            let axis = b - a;
            let height = axis.length();
            if height < 1e-4 {
                return None;
            }
            let turn = Quat::from_rotation_arc(Vec3::Y, axis / height);
            let mid = (a + b) * 0.5;
            (
                pos + rot * mid,
                rot * turn,
                Collider::cylinder(*radius, height),
            )
        }
        CollisionShape::ConvexHull { points, .. } => {
            let pts: Vec<Vec3> = points.iter().map(|p| to_bevy(*p)).collect();
            (pos, rot, Collider::convex_hull(pts)?)
        }
        CollisionShape::TriMesh {
            vertices,
            triangles,
        } => {
            if triangles.is_empty() {
                return None;
            }
            let v: Vec<Vec3> = vertices.iter().map(|p| to_bevy(*p)).collect();
            (pos, rot, Collider::trimesh(v, triangles.clone()))
        }
        _ => return None,
    })
}

/// Avian colliders for a Havok rigid body, each with its placement relative to the
/// body. Convex leaves are merged into one compound (one entry); triangle meshes cannot
/// live inside a parry compound, so each becomes an entry of its own.
pub fn colliders_from_body(body: &CollisionBody) -> Vec<(Vec3, Quat, Collider)> {
    let mut simple: Vec<(Vec3, Quat, Collider)> = Vec::new();
    let mut meshes: Vec<(Vec3, Quat, Collider)> = Vec::new();
    for (tf, leaf) in body.shape.flatten() {
        let tf = tf.then(&body.transform);
        let is_mesh = matches!(leaf, CollisionShape::TriMesh { .. });
        if let Some(p) = leaf_collider(&tf, &leaf) {
            if is_mesh {
                meshes.push(p);
            } else {
                simple.push(p);
            }
        }
    }
    let mut out = Vec::new();
    match simple.len() {
        0 => {}
        1 => out.push(simple.pop().unwrap()),
        _ => out.push((
            Vec3::ZERO,
            Quat::IDENTITY,
            Collider::compound(
                simple
                    .into_iter()
                    .map(|(p, r, c)| (Position(p), Rotation(r), c))
                    .collect(),
            ),
        )),
    }
    out.extend(meshes);
    out
}

#[derive(Component)]
struct Ball;

fn drop_ball(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cams: Query<&Transform, With<Camera3d>>,
    balls: Query<Entity, With<Ball>>,
) {
    if !keys.just_pressed(KeyCode::KeyB) {
        return;
    }
    let Ok(cam) = cams.single() else { return };
    // keep the scene tidy
    if balls.iter().count() > 40 {
        for e in balls.iter().take(10) {
            commands.entity(e).despawn();
        }
    }
    let radius = 0.6;
    let forward = cam.forward().as_vec3();
    commands.spawn((
        RigidBody::Dynamic,
        Collider::sphere(radius),
        Mass(80.0),
        Restitution::new(0.4),
        LinearVelocity(forward * 25.0),
        Mesh3d(meshes.add(Sphere::new(radius))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.3, 0.2),
            perceptual_roughness: 0.4,
            ..default()
        })),
        Transform::from_translation(cam.translation + forward * 3.0),
        Ball,
        Name::new("ball"),
    ));
}

/// Bundle pieces for a placed model's mesh child: a trimesh collider built from the mesh
/// once it is available.
pub fn static_mesh_collider() -> ColliderConstructor {
    ColliderConstructor::TrimeshFromMesh
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The physics plugins plus what they need without a renderer (the collider cache
    /// listens for mesh asset events).
    pub fn headless_physics() -> impl PluginGroup {
        PhysicsPlugins::default()
    }

    /// Avian without a renderer, the way its own tests run.
    pub fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            headless_physics(),
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 64.0),
        ));
        app.finish();
        app
    }

    fn flat_tile(raw: u16) -> TerrainTile {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for _ in 0..132 * 132 {
            d.extend_from_slice(&raw.to_le_bytes());
        }
        d.extend(std::iter::repeat_n(0u8, 4 * 128 * 128 * 2));
        TerrainTile::parse(&d).unwrap()
    }

    /// A ball dropped on a tile's collider must come to rest one radius above the
    /// height field — run headless through Avian's fixed step.
    #[test]
    fn ball_rests_on_terrain() {
        let tile = flat_tile(32000); // ≈ 1074 m
        let h = tile.cell_height(5, 5);
        let (collider, tf) = terrain_collider(&tile, -512.0, -9216.0);
        let mut app = headless_app();
        app.insert_resource(Gravity(Vec3::NEG_Y * 9.81));
        app.world_mut().spawn((RigidBody::Static, collider, tf));
        let ball = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::from_xyz(-300.0, h + 6.0, -9000.0),
            ))
            .id();
        for _ in 0..240 {
            app.update();
        }
        let y = app.world().get::<Transform>(ball).unwrap().translation.y;
        assert!((y - (h + 0.5)).abs() < 0.1, "ball at {y}, ground {h}");
    }

    /// A box leaf translated along +z (engine) ends up at +z in Bevy, with the right size.
    #[test]
    fn body_to_collider() {
        let body = CollisionBody {
            transform: Transform3::IDENTITY,
            shape: CollisionShape::Compound {
                children: vec![
                    CollisionShape::Transformed {
                        transform: Transform3::from_mat4x3([
                            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 3.0,
                        ]),
                        child: Box::new(CollisionShape::Box {
                            half_extents: [1.0, 0.5, 2.0],
                            radius: 0.0,
                        }),
                    },
                    CollisionShape::Sphere { radius: 0.25 },
                ],
            },
        };
        let parts = colliders_from_body(&body);
        assert_eq!(parts.len(), 1);
        let c = &parts[0].2;
        let aabb = c.aabb(parts[0].0, parts[0].1);
        assert!(
            (aabb.min.z + 0.25).abs() < 1e-4 && (aabb.max.z - 5.0).abs() < 1e-4,
            "{aabb:?}"
        );
        assert!((aabb.max.y - 1.5).abs() < 1e-4, "{aabb:?}");
        // a lone box at the origin is not wrapped in a compound
        let simple = CollisionBody {
            transform: Transform3::IDENTITY,
            shape: CollisionShape::Box {
                half_extents: [1.0, 1.0, 1.0],
                radius: 0.0,
            },
        };
        let parts = colliders_from_body(&simple);
        assert!(parts.len() == 1 && parts[0].2.shape().as_cuboid().is_some());
    }

    /// A sloped tile: heights rise with the tile's z (engine), and so with Bevy's z.
    #[test]
    fn heightfield_orientation() {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for r in 0..132 {
            for _ in 0..132 {
                d.extend_from_slice(&((r as u16) * 100).to_le_bytes());
            }
        }
        d.extend(std::iter::repeat_n(0u8, 4 * 128 * 128 * 2));
        let tile = TerrainTile::parse(&d).unwrap();
        let (collider, tf) = terrain_collider(&tile, 0.0, 0.0);
        // engine z = 500 (cell row 125) is high, z = 10 is low; Bevy's z is the same
        let mut app = headless_app();
        app.world_mut().spawn((RigidBody::Static, collider, tf));
        app.update();
        let mut q = app.world_mut().query::<&Collider>();
        let c = q.iter(app.world()).next().unwrap();
        let shape = c.shape_scaled();
        let hf = shape.as_heightfield().unwrap();
        // parry (row i, column j) = (z, x): row 125 is cell row 125 (the high end)
        let hi = hf.heights()[(125, 10)];
        let lo = hf.heights()[(3, 10)];
        assert!(hi > lo, "{hi} {lo}");
        assert!((hi - tile.cell_height(125, 10)).abs() < 1e-3, "{hi}");
        assert!((lo - tile.cell_height(3, 10)).abs() < 1e-3, "{lo}");
        // and the world position of parry's row 0 is the tile's origin edge (z0 = 0)
        let z_row0 = hf.z_at(0) + tf.translation.z;
        assert!(z_row0.abs() < 1e-3, "{z_row0}");
    }

    // ---- passengers: getting in and out of vehicles ----------------------------------

    /// How a test seats and frees Rico.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Seat {
        /// What the game did before: `RigidBodyDisabled` + `ColliderDisabled`.
        Disable,
        /// [`stow_passenger`] / [`release_passenger`].
        Helpers,
    }

    /// xorshift: deterministic "random" timing and placement.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * (self.next() % 10_000) as f32 / 10_000.0
        }
        fn frames(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next() % (hi - lo + 1) as u64) as u32
        }
    }

    /// Flat ground, a car (a compound: chassis, cabin, four wheels) and Rico's capsule
    /// standing against its side, set up the way the game spawns them.
    fn passenger_scene() -> (App, Entity, Entity) {
        let mut app = headless_app();
        app.insert_resource(Gravity(Vec3::NEG_Y * 9.81))
            .add_observer(never_sleep);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(400.0, 2.0, 400.0),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ));
        let mut parts = vec![
            (
                Vec3::new(0.0, 0.7, 0.0),
                Quat::IDENTITY,
                Collider::cuboid(2.0, 0.6, 4.4),
            ),
            (
                Vec3::new(0.0, 1.35, -0.2),
                Quat::IDENTITY,
                Collider::cuboid(1.8, 0.7, 2.2),
            ),
        ];
        for (x, z) in [(-0.85, -1.5), (0.85, -1.5), (-0.85, 1.5), (0.85, 1.5)] {
            parts.push((Vec3::new(x, 0.4, z), Quat::IDENTITY, Collider::sphere(0.4)));
        }
        let car = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::compound(parts),
                Transform::from_xyz(0.0, 0.05, 0.0),
            ))
            .id();
        // as character.rs spawns him
        let rico = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::capsule(0.3, 1.2),
                LockedAxes::ROTATION_LOCKED,
                (Mass(80.0), NoAutoMass),
                Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
                Transform::from_xyz(1.25, 0.95, 0.5),
            ))
            .id();
        for _ in 0..30 {
            app.update();
        }
        (app, car, rico)
    }

    /// The contact graph and the solver's constraint graph must agree: no pair holds
    /// more constraint handles than it has manifolds (the solver would index past the
    /// end), no pair that already has handles still waits to "start generating
    /// constraints" (it would get them all again), and no enabled body's collider is
    /// flagged disabled in the broad phase (every new pair it makes would be such a pair).
    fn contacts_consistent(world: &mut World) -> Result<(), String> {
        let mut colliders = world.query::<(
            Entity,
            &ColliderOf,
            &avian3d::collider_tree::ColliderTreeProxyKey,
        )>();
        let colliders: Vec<_> = colliders
            .iter(world)
            .map(|(e, of, k)| (e, of.body, *k))
            .collect();
        let graph = world.resource::<ContactGraph>();
        for pair in graph.iter_active() {
            let Some(edge) = graph.get_edge_by_id(pair.contact_id) else {
                continue;
            };
            let (handles, manifolds) = (edge.constraint_handles.len(), pair.manifolds.len());
            if handles > manifolds {
                return Err(format!(
                    "{:?}: {handles} constraint handles for {manifolds} manifolds",
                    pair.contact_id
                ));
            }
            if handles > 0
                && pair
                    .flags
                    .contains(ContactPairFlags::STARTED_GENERATING_CONSTRAINTS)
            {
                return Err(format!(
                    "{:?}: STARTED_GENERATING_CONSTRAINTS left on a pair already in the \
                     constraint graph",
                    pair.contact_id
                ));
            }
        }
        let trees = world.resource::<avian3d::collider_tree::ColliderTrees>();
        for (e, body, key) in colliders {
            if world.get::<RigidBodyDisabled>(body).is_some() {
                continue;
            }
            if trees.get_proxy(key).is_some_and(|p| {
                p.flags
                    .contains(avian3d::collider_tree::ColliderTreeProxyFlags::BODY_DISABLED)
            }) {
                return Err(format!(
                    "collider {e}: broad-phase proxy says BODY_DISABLED, body {body} is enabled"
                ));
            }
        }
        Ok(())
    }

    /// Rico gets into the car and out again `cycles` times, at varied moments and spots:
    /// while seated he follows the car (as `hero_physics` keeps him on the seat) and the
    /// car is pushed about; he leaves beside it, touching or slightly inside its side, and
    /// slides along it. With `check`, the contact bookkeeping is verified every frame.
    fn enter_exit(seat: Seat, seed: u64, cycles: u32, check: bool) {
        let (mut app, car, rico) = passenger_scene();
        let mut rng = Rng(seed);
        let step = |app: &mut App| {
            app.update();
            if check && let Err(e) = contacts_consistent(app.world_mut()) {
                panic!("{e}");
            }
        };
        for _ in 0..cycles {
            // in
            match seat {
                Seat::Disable => {
                    app.world_mut()
                        .entity_mut(rico)
                        .insert((RigidBodyDisabled, ColliderDisabled));
                }
                Seat::Helpers => {
                    let world = app.world_mut();
                    stow_passenger(&mut world.commands(), rico, car);
                    world.flush();
                }
            }
            let push = Vec3::new(rng.range(-4.0, 4.0), 0.0, rng.range(-4.0, 4.0));
            for _ in 0..rng.frames(1, 8) {
                let world = app.world_mut();
                let car_at = world.get::<Transform>(car).unwrap().translation;
                if car_at.length() > 60.0 || car_at.y < -1.0 {
                    world.entity_mut(car).insert((
                        Position(Vec3::new(0.0, 0.05, 0.0)),
                        Rotation::default(),
                        AngularVelocity::ZERO,
                    ));
                }
                world.get_mut::<LinearVelocity>(car).unwrap().0 = push;
                let car_at = world.get::<Transform>(car).unwrap().translation;
                world.get_mut::<Transform>(rico).unwrap().translation = car_at;
                world.get_mut::<LinearVelocity>(rico).unwrap().0 = push;
                step(&mut app);
            }
            // out: beside the car, against or a little inside its side
            let world = app.world_mut();
            let car_tf = *world.get::<Transform>(car).unwrap();
            let car_lin = world.get::<LinearVelocity>(car).unwrap().0;
            let side = if rng.next().is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            let at = car_tf.translation
                + car_tf.rotation
                    * Vec3::new(
                        side * rng.range(1.0, 1.4),
                        rng.range(0.6, 1.6),
                        rng.range(-2.4, 2.4),
                    );
            let vel = car_lin + car_tf.rotation * Vec3::new(0.0, 0.0, rng.range(-3.0, 3.0));
            world.get_mut::<Transform>(rico).unwrap().translation = at;
            world.get_mut::<LinearVelocity>(rico).unwrap().0 = vel;
            match seat {
                Seat::Disable => {
                    world
                        .entity_mut(rico)
                        .remove::<(RigidBodyDisabled, ColliderDisabled)>()
                        .insert(Position(at));
                }
                Seat::Helpers => {
                    release_passenger(&mut world.commands(), rico, at, vel);
                    world.flush();
                }
            }
            for _ in 0..rng.frames(1, 40) {
                step(&mut app);
            }
        }
    }

    /// Canary for the Avian 0.7.0 bug itself: seating Rico by disabling his body and
    /// collider, the way the game used to, corrupts the constraint graph and the solver
    /// panics. When this stops panicking, Avian has been fixed.
    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn avian_disable_reenable_still_breaks_the_solver() {
        enter_exit(Seat::Disable, 0x9E37_79B9_7F4A_7C15, 40, false);
    }

    /// The root cause, narrowed down: after `RigidBodyDisabled` and `ColliderDisabled`
    /// are removed together, the collider's broad-phase proxy still says
    /// `BODY_DISABLED` (both `Discard` observers read `Has<RigidBodyDisabled>` while the
    /// component is still there). If this starts failing, Avian has fixed it.
    #[test]
    fn avian_reenable_leaves_proxy_flagged_disabled() {
        let (mut app, _car, rico) = passenger_scene();
        let flagged = |app: &App| {
            let key = *app
                .world()
                .get::<avian3d::collider_tree::ColliderTreeProxyKey>(rico)
                .unwrap();
            let trees = app
                .world()
                .resource::<avian3d::collider_tree::ColliderTrees>();
            trees
                .get_proxy(key)
                .unwrap()
                .flags
                .contains(avian3d::collider_tree::ColliderTreeProxyFlags::BODY_DISABLED)
        };
        assert!(!flagged(&app));
        app.world_mut()
            .entity_mut(rico)
            .insert((RigidBodyDisabled, ColliderDisabled));
        app.update();
        app.world_mut()
            .entity_mut(rico)
            .remove::<(RigidBodyDisabled, ColliderDisabled)>();
        app.update();
        assert!(app.world().get::<RigidBodyDisabled>(rico).is_none());
        assert!(
            flagged(&app),
            "Avian no longer leaves the proxy flagged: the passenger workaround may be \
             unnecessary (see research/formats/havok.md, Passengers)"
        );
    }

    /// A stowed passenger sits on the seat touching nothing and invisible to rays; once
    /// released (even right there, without a teleport) he collides and stands again.
    #[test]
    fn stowed_passenger_is_inert_then_lands() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut app, car, rico) = passenger_scene();
        {
            let world = app.world_mut();
            stow_passenger(&mut world.commands(), rico, car);
            world.flush();
        }
        for _ in 0..10 {
            let world = app.world_mut();
            let car_at = world.get::<Transform>(car).unwrap().translation;
            world.get_mut::<Transform>(rico).unwrap().translation = car_at;
            app.update();
        }
        let graph = app.world().resource::<ContactGraph>();
        assert_eq!(graph.contact_pairs_with(rico).count(), 0);
        let rico_at = app.world().get::<Position>(rico).unwrap().0;
        let hit = app
            .world_mut()
            .run_system_once(move |sq: SpatialQuery| {
                let filter = SpatialQueryFilter::from_excluded_entities([car]);
                sq.cast_ray(rico_at + Vec3::X * 5.0, Dir3::NEG_X, 10.0, true, &filter)
                    .map(|h| h.entity)
            })
            .unwrap();
        assert_ne!(hit, Some(rico), "a ray hit the stowed passenger");

        // out, standing on open ground; then out again where he already is
        let at = Vec3::new(8.0, 0.92, 8.0);
        {
            let world = app.world_mut();
            release_passenger(&mut world.commands(), rico, at, Vec3::ZERO);
            world.flush();
        }
        assert!(app.world().get::<GravityScale>(rico).is_none());
        assert!(app.world().get::<Stowed>(rico).is_none());
        for _ in 0..60 {
            app.update();
        }
        let y = app.world().get::<Position>(rico).unwrap().0.y;
        assert!((y - 0.9).abs() < 0.05, "rico at {y} after release");
        for _ in 0..2 {
            let world = app.world_mut();
            stow_passenger(&mut world.commands(), rico, rico);
            world.flush();
            app.update();
            let world = app.world_mut();
            let here = world.get::<Position>(rico).unwrap().0;
            release_passenger(&mut world.commands(), rico, here, Vec3::ZERO);
            world.flush();
            for _ in 0..60 {
                app.update();
            }
            let y = app.world().get::<Position>(rico).unwrap().0.y;
            assert!((y - 0.9).abs() < 0.05, "rico at {y} after release in place");
        }
    }

    /// The same enter/exit sequence through the passenger helpers runs clean, with the
    /// contact bookkeeping checked every frame.
    #[test]
    fn passenger_enter_exit_keeps_contacts_consistent() {
        enter_exit(Seat::Helpers, 0x9E37_79B9_7F4A_7C15, 40, true);
    }

    /// 200 enter/exit cycles at random moments and spots, several seeds.
    #[test]
    fn passenger_enter_exit_soak() {
        for seed in [1, 0xDEAD_BEEF, 0x1234_5678_9ABC] {
            enter_exit(Seat::Helpers, seed, 200, true);
        }
    }
}
