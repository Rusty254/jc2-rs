//! Passive fatal ragdoll. This deliberately does not emulate Havok powered profiles.
use crate::{
    character::{Hero, HeroModel},
    coords::{mat_to_bevy, qs_to_bevy},
    rig::Animator,
};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::{animation::Skeleton, ragdoll::Ragdoll};
use std::sync::Arc;

#[derive(Resource)]
pub struct RagdollAsset(pub Arc<Ragdoll>);
/// Captured before fatal-downing clears attachment and controller motion.
#[derive(Component, Clone, Copy)]
pub struct PendingRagdoll {
    pub origin: Vec3,
    pub linear: Vec3,
    pub angular: Vec3,
}
#[derive(Component)]
pub struct PhysicsPose;
#[derive(Component)]
pub struct FallbackPose;
#[derive(Component)]
struct Owned {
    hero: Entity,
    joint: bool,
}
#[derive(Clone)]
struct Binding {
    body: usize,
    bone: usize,
    body_to_bone: Mat4,
}
#[derive(Component)]
struct Assembly {
    bodies: Vec<Entity>,
    bindings: Vec<Binding>,
    model: Entity,
    hips_bone: usize,
    model_from_hips: Mat4,
    retained: Vec<Transform>,
    layers: CollisionLayers,
    gravity: GravityScale,
    rigid_body: RigidBody,
}

pub struct RagdollPlugin;
impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .add_systems(PreUpdate, cleanup)
            .add_systems(
                PostUpdate,
                reconstruct
                    .before(crate::rig::animate)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                activate.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}
fn load(mut commands: Commands, game: Res<crate::Game>) {
    match game
        .fs
        .read_inflated("triplemapped_normal_ragdoll.brd")
        .and_then(|bytes| Ragdoll::parse(&bytes))
    {
        Ok(asset) => {
            commands.insert_resource(RagdollAsset(Arc::new(asset)));
        }
        Err(e) => warn!("passive ragdoll unavailable; retaining downed fallback: {e}"),
    }
}
fn rest_world(skeleton: &Skeleton) -> Vec<Mat4> {
    let mut result = Vec::with_capacity(skeleton.bones.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let parent = bone.parent.map_or(Mat4::IDENTITY, |p| result[p]);
        result.push(parent * qs_to_bevy(&skeleton.reference_pose[i]).to_matrix());
    }
    result
}
fn bindings(asset: &Ragdoll, render: &Skeleton) -> Result<Vec<Binding>, String> {
    let mapper = asset
        .mappers
        .iter()
        .find(|m| {
            m.skeletons[0] == asset.skeleton_index
                && asset.skeletons[m.skeletons[1]].skeleton.name == render.name
        })
        .ok_or("no exact ragdoll-to-render mapper")?;
    if !mapper.chains.is_empty() {
        return Err("chain mappings unsupported".into());
    }
    let target = &asset.skeletons[mapper.skeletons[1]].skeleton;
    let rest = rest_world(render);
    let source_rest = rest_world(&asset.skeletons[mapper.skeletons[0]].skeleton);
    let mut result = Vec::new();
    for m in &mapper.simple {
        let body = asset.bone_to_body[m.bones[0] as usize];
        if body < 0 {
            return Err("mapper bone has no body".into());
        }
        let name = &target.bones[m.bones[1] as usize].name;
        let bone = render
            .bone_index(name)
            .ok_or("mapped render bone missing")?;
        let body_rest = mat_to_bevy(&asset.bodies[body as usize].collision.transform).to_matrix();
        let mapped_rest = source_rest[m.bones[0] as usize] * qs_to_bevy(&m.a_from_b).to_matrix();
        // aFromB maps target coordinates into source-bone coordinates. Reject a
        // different rest skeleton rather than silently retargeting it by names.
        if !mapped_rest.abs_diff_eq(rest[bone], 0.002) {
            return Err(format!("mapper/render rest pose mismatch for {name}"));
        }
        result.push(Binding {
            body: body as usize,
            bone,
            body_to_bone: body_rest.inverse() * mapped_rest,
        });
    }
    if result.len() != asset.bodies.len() {
        return Err("incomplete body mapping".into());
    }
    Ok(result)
}
fn point_velocity(pending: PendingRagdoll, point: Vec3) -> Vec3 {
    pending.linear + pending.angular.cross(point - pending.origin)
}
fn axis(index: u8) -> Option<Vec3> {
    match index {
        0 => Some(Vec3::X),
        1 => Some(Vec3::Y),
        2 => Some(Vec3::Z),
        _ => None,
    }
}
fn make_joint(
    world: &mut World,
    hero: Entity,
    joint: &jc2_formats::ragdoll::Joint,
    bodies: &[Entity],
    poses: &[Transform],
) -> Result<Entity, String> {
    let frames = joint.frames.as_ref().ok_or("joint frames missing")?;
    let mut frames = frames.map(|f| mat_to_bevy(&f));
    let [a, b] = joint.body_indices;
    // Animated poses are not constraint-projected. Reconcile anchors at the midpoint
    // before simulation; bases/limits retain their decoded reference frames.
    let pa = poses[a].transform_point(frames[0].translation);
    let pb = poses[b].transform_point(frames[1].translation);
    let pivot = (pa + pb) * 0.5;
    frames[0].translation = poses[a].to_matrix().inverse().transform_point3(pivot);
    frames[1].translation = poses[b].to_matrix().inverse().transform_point3(pivot);
    let owner = Owned { hero, joint: true };
    let frame = |i: usize| bevy::math::Isometry3d::new(frames[i].translation, frames[i].rotation);
    if joint.data_class == "hkpLimitedHingeConstraintData" {
        let limit = joint
            .limits
            .iter()
            .find(|l| l.enabled && l.atom_class.contains("AngLimit"))
            .ok_or("hinge limit missing")?;
        let hinge = limit
            .axes
            .iter()
            .find(|(n, _)| n == "limitAxis")
            .map(|(_, v)| *v)
            .and_then(axis)
            .ok_or("hinge axis unsupported")?;
        let mut j = RevoluteJoint::new(bodies[a], bodies[b])
            .with_local_frame1(frame(0))
            .with_local_frame2(frame(1))
            .with_hinge_axis(hinge);
        j.angle_limit = Some(AngleLimit::new(limit.min, limit.max));
        Ok(world.spawn((j, JointCollisionDisabled, owner)).id())
    } else if joint.data_class == "hkpRagdollConstraintData" {
        let twist = joint
            .limits
            .iter()
            .find(|l| l.enabled && l.atom_class.contains("Twist"))
            .ok_or("twist limit missing")?;
        let twist_axis = twist
            .axes
            .iter()
            .find(|(n, _)| n == "twistAxisInA" || n == "twistAxis")
            .map(|(_, v)| *v)
            .and_then(axis)
            .ok_or("twist axis unsupported")?;
        let cone = joint
            .limits
            .iter()
            .find(|l| l.enabled && l.atom_class.contains("Cone"))
            .ok_or("cone limit missing")?;
        let mut j = SphericalJoint::new(bodies[a], bodies[b])
            .with_local_frame1(frame(0))
            .with_local_frame2(frame(1))
            .with_twist_axis(twist_axis);
        j.twist_limit = Some(AngleLimit::new(twist.min, twist.max));
        // Circular Avian cone; authored plane constraint and powered motors are not reproduced.
        j.swing_limit = Some(AngleLimit::new(-cone.max.abs(), cone.max.abs()));
        Ok(world.spawn((j, JointCollisionDisabled, owner)).id())
    } else {
        Err(format!("unsupported constraint {}", joint.data_class))
    }
}
fn clear_hero_tethers(world: &mut World, hero: Entity) {
    let linked: Vec<_> = world
        .query::<(Entity, &crate::stunts::Tether)>()
        .iter(world)
        .filter(|(_, t)| t.ends.iter().any(|e| e.body == hero || e.owner == hero))
        .map(|(e, _)| e)
        .collect();
    for e in linked {
        world.despawn(e);
    }
    // An incomplete first shot is visually connected to Rico's hand; completed
    // object-to-object cables have independent ownership and remain untouched.
    if let Some(mut state) = world.get_resource_mut::<crate::stunts::TetherState>()
        && let Some(end) = state.pending.take()
        && let Some(anchor) = end.anchor
    {
        world.despawn(anchor);
    }
}
fn activate(world: &mut World) {
    let Some(asset) = world.get_resource::<RagdollAsset>().map(|a| a.0.clone()) else {
        return;
    };
    let pending: Vec<_> = world
        .query::<(Entity, &PendingRagdoll)>()
        .iter(world)
        .map(|(e, p)| (e, *p))
        .collect();
    for (hero, request) in pending {
        world.entity_mut(hero).remove::<PendingRagdoll>();
        if world.get::<Assembly>(hero).is_some()
            || !world.get::<Hero>(hero).is_some_and(|h| h.downed)
        {
            continue;
        }
        clear_hero_tethers(world, hero);
        let Some(an) = world.get::<Animator>(hero) else {
            continue;
        };
        let Ok(mapping) = bindings(&asset, &an.rig.skeleton) else {
            warn!("unsupported render mapper; retaining rigid downed fallback");
            world.entity_mut(hero).insert(FallbackPose);
            continue;
        };
        let Some(hips_bone) = an.rig.skeleton.bone_index("Hips") else {
            world.entity_mut(hero).insert(FallbackPose);
            continue;
        };
        let joints = an.joints.clone();
        let retained = joints
            .iter()
            .map(|&e| world.get::<Transform>(e).copied())
            .collect::<Option<Vec<_>>>();
        let Some(retained) = retained else { continue };
        // These are the last propagated animated world poses, untouched by fatal cleanup.
        let mut poses = vec![Transform::IDENTITY; asset.bodies.len()];
        let mut valid = true;
        for binding in &mapping {
            if let Some(g) = world.get::<GlobalTransform>(joints[binding.bone]) {
                poses[binding.body] =
                    Transform::from_matrix(g.to_matrix() * binding.body_to_bone.inverse());
            } else {
                valid = false;
            }
        }
        let model = world
            .get::<Children>(hero)
            .and_then(|c| c.iter().find(|&e| world.get::<HeroModel>(e).is_some()));
        let Some(model) = model.filter(|_| valid) else {
            continue;
        };
        let Some(hips_world) = world
            .get::<GlobalTransform>(joints[hips_bone])
            .map(|g| g.to_matrix())
        else {
            continue;
        };
        let Some(model_world) = world.get::<GlobalTransform>(model).map(|g| g.to_matrix()) else {
            continue;
        };
        let model_from_hips = hips_world.inverse() * model_world;
        let mut bodies = Vec::new();
        for (i, body) in asset.bodies.iter().enumerate() {
            let mut collision = body.collision.clone();
            collision.transform = jc2_formats::placement::Transform3::IDENTITY;
            let leaves = crate::physics::colliders_from_body(&collision);
            if leaves.is_empty() {
                valid = false;
                break;
            }
            let collider = Collider::compound(
                leaves
                    .into_iter()
                    .map(|(p, r, c)| (Position(p), Rotation(r), c))
                    .collect(),
            );
            let inverse = body.inverse_inertia_and_mass;
            let inertia = Vec3::new(inverse[0].recip(), inverse[1].recip(), inverse[2].recip());
            if !inertia.is_finite() || inverse[3] <= 0.0 {
                valid = false;
                break;
            }
            let pose = poses[i];
            let center = Vec3::from_array(body.center_of_mass_local[..3].try_into().unwrap());
            // Reserve bit 31 for ragdolls: self contacts off, world/default contacts on.
            let e = world
                .spawn((
                    (
                        RigidBody::Dynamic,
                        collider,
                        pose,
                        Position(pose.translation),
                        Rotation(pose.rotation),
                        LinearVelocity(point_velocity(request, pose.transform_point(center))),
                        AngularVelocity(request.angular),
                    ),
                    (
                        Mass(inverse[3].recip()),
                        AngularInertia::new(inertia),
                        CenterOfMass(center),
                        NoAutoMass,
                        NoAutoAngularInertia,
                        NoAutoCenterOfMass,
                    ),
                    LinearDamping(body.damping[0]),
                    AngularDamping(body.damping[1]),
                    CollisionLayers::new(1u32 << 31, !(1u32 << 31)),
                    SleepingDisabled,
                    Owned { hero, joint: false },
                    Name::new(format!("passive {}", body.name)),
                ))
                .id();
            bodies.push(e);
        }
        let mut constraints = Vec::new();
        if valid {
            for j in &asset.joints {
                match make_joint(world, hero, j, &bodies, &poses) {
                    Ok(e) => constraints.push(e),
                    Err(e) => {
                        warn!("passive ragdoll: {e}");
                        valid = false;
                        break;
                    }
                }
            }
        }
        if !valid {
            for e in constraints {
                world.despawn(e);
            }
            for e in bodies {
                world.despawn(e);
            }
            world.entity_mut(hero).insert(FallbackPose);
            continue;
        }
        let layers = world
            .get::<CollisionLayers>(hero)
            .copied()
            .unwrap_or_default();
        let gravity = world.get::<GravityScale>(hero).copied().unwrap_or_default();
        let rigid_body = world
            .get::<RigidBody>(hero)
            .copied()
            .unwrap_or(RigidBody::Dynamic);
        world.entity_mut(hero).insert((
            PhysicsPose,
            Assembly {
                bodies,
                bindings: mapping,
                model,
                hips_bone,
                model_from_hips,
                retained,
                layers,
                gravity,
                rigid_body,
            },
            RigidBody::Kinematic,
            CollisionLayers::NONE,
            GravityScale(0.0),
            LinearVelocity::ZERO,
            AngularVelocity::ZERO,
        ));
        info!(
            "passive ragdoll: 14 bodies / 13 joints, circular cone approximation; no powered motors"
        );
    }
}
fn reconstruct(world: &mut World) {
    let heroes: Vec<_> = world
        .query_filtered::<Entity, With<Assembly>>()
        .iter(world)
        .collect();
    for hero in heroes {
        if !world.get::<Hero>(hero).is_some_and(|h| h.downed) {
            continue;
        }
        let assembly = world.get::<Assembly>(hero).unwrap();
        let Some(an) = world.get::<Animator>(hero) else {
            continue;
        };
        let skeleton = an.rig.skeleton.clone();
        let joints = an.joints.clone();
        let retained = assembly.retained.clone();
        let model = assembly.model;
        let hips_bone = assembly.hips_bone;
        let model_from_hips = assembly.model_from_hips;
        let mut driven = vec![None; skeleton.bones.len()];
        for binding in &assembly.bindings {
            let body = assembly.bodies[binding.body];
            if let (Some(p), Some(r)) = (world.get::<Position>(body), world.get::<Rotation>(body)) {
                driven[binding.bone] =
                    Some(Mat4::from_rotation_translation(r.0, p.0) * binding.body_to_bone);
            }
        }
        // Follow hips for camera/world streaming, but never parent or translate bodies.
        let Some(hips_world) = driven[hips_bone] else {
            continue;
        };
        let at = hips_world.w_axis.truncate();
        if let Some(mut t) = world.get_mut::<Transform>(hero) {
            t.translation = at;
        }
        if let Some(mut p) = world.get_mut::<Position>(hero) {
            p.0 = at;
        }
        let retained_model_world = hips_world * model_from_hips;
        let Some(root) = world.get::<Transform>(hero).copied() else {
            continue;
        };
        let Some(model_local) = world.get::<Transform>(model).copied() else {
            continue;
        };
        let model_world = root.to_matrix() * model_local.to_matrix();
        let mut globals = Vec::<Mat4>::with_capacity(joints.len());
        for (i, bone) in skeleton.bones.iter().enumerate() {
            let parent = bone.parent.map_or(model_world, |p| globals[p]);
            let retained_parent = bone.parent.map_or(retained_model_world, |p| globals[p]);
            let global = driven[i].unwrap_or(retained_parent * retained[i].to_matrix());
            let local = Transform::from_matrix(parent.inverse() * global);
            if let Some(mut t) = world.get_mut::<Transform>(joints[i]) {
                *t = local;
            }
            globals.push(global);
        }
    }
}
fn cleanup(world: &mut World) {
    let fallback: Vec<_> = world
        .query_filtered::<Entity, With<FallbackPose>>()
        .iter(world)
        .filter(|&e| !world.get::<Hero>(e).is_some_and(|h| h.downed))
        .collect();
    for e in fallback {
        world.entity_mut(e).remove::<FallbackPose>();
    }
    let retired: Vec<_> = world
        .query::<(Entity, &Owned)>()
        .iter(world)
        .filter(|(_, o)| !world.get::<Hero>(o.hero).is_some_and(|h| h.downed))
        .map(|(e, o)| (e, o.joint))
        .collect();
    // Remove constraint entities first. Their observers update Avian's graph before bodies go.
    for &(e, joint) in &retired {
        if joint {
            world.despawn(e);
        }
    }
    for &(e, joint) in &retired {
        if !joint {
            world.despawn(e);
        }
    }
    let heroes: Vec<_> = world
        .query_filtered::<Entity, With<Assembly>>()
        .iter(world)
        .filter(|&e| !world.get::<Hero>(e).is_some_and(|h| h.downed))
        .collect();
    for hero in heroes {
        let assembly = world.entity_mut(hero).take::<Assembly>().unwrap();
        world.entity_mut(hero).remove::<PhysicsPose>().insert((
            assembly.rigid_body,
            assembly.layers,
            assembly.gravity,
            LinearVelocity::ZERO,
            AngularVelocity::ZERO,
        ));
    }
}
#[cfg(test)]
mod tests;
