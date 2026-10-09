use super::*;
use crate::rig::{SUPPORT_REACH_LIMIT, two_bone_solve};
use jc2_formats::weapon_mounts::{
    HARNESS_DEFAULT_ROTATION, HARNESS_DEFAULT_TRANSLATION, HarnessCorrection,
};

// ---- the probe's flat row-major row-vector model (weapon_ownership_probe.py), as an oracle ----

type Flat = [f32; 16];
const IDENT: Flat = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

fn rot(r1: f32, r2: f32, r3: f32) -> Flat {
    let (a, b, c, d, e, f) = (r1.sin(), r1.cos(), r2.sin(), r2.cos(), r3.sin(), r3.cos());
    [
        e * c * a + f * b,
        e * d,
        e * c * b - f * a,
        0.,
        f * c * a - e * b,
        f * d,
        f * c * b + e * a,
        0.,
        d * a,
        -c,
        d * b,
        0.,
        0.,
        0.,
        0.,
        1.,
    ]
}

fn mul(a: &Flat, b: &Flat) -> Flat {
    let mut out = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    out
}

fn tr(mut m: Flat, t: [f32; 3]) -> Flat {
    m[12..15].copy_from_slice(&t);
    m
}

fn point(v: [f32; 3], m: &Flat) -> [f32; 3] {
    [0, 1, 2].map(|c| v[0] * m[c] + v[1] * m[4 + c] + v[2] * m[8 + c] + m[12 + c])
}

/// The native flat matrix of an engine transform.
fn flat(t: &Transform) -> Flat {
    let (x, y, z) = (
        t.rotation * Vec3::X,
        t.rotation * Vec3::Y,
        t.rotation * Vec3::Z,
    );
    let p = t.translation;
    [
        x.x, x.y, x.z, 0., y.x, y.y, y.z, 0., z.x, z.y, z.z, 0., p.x, p.y, p.z, 1.,
    ]
}

fn close(a: &Flat, b: &Flat) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 2e-4)
}

fn oracle_world(bone: usize, m: &WeaponMounts, bone_m: &Flat, char_m: &Flat) -> Flat {
    let corr = match bone {
        0x18 => tr(rot(FRAC_PI_2, 0., -FRAC_PI_2), [0.; 3]),
        0x19 if m.right_hand_rotation => tr(rot(-FRAC_PI_2, 0., FRAC_PI_2), [0.; 3]),
        1 | 4 => tr(rot(0., -1.7, 0.), [-0.11, 0.06, 0.06]),
        7 => tr(rot(0.2, -1.7, 0.), [0.14, 0.03, 0.06]),
        0x4d => m.harness.map_or(IDENT, |h| {
            let r = h.rotation_radians;
            tr(rot(r[0], r[1], r[2]), h.translation)
        }),
        _ => IDENT,
    };
    mul(&mul(&corr, bone_m), char_m)
}

// ---- fixtures: the rows of weapon-sockets-next.md / weapon-ownership-next.md ----

fn beretta() -> WeaponMounts {
    WeaponMounts {
        id: 2,
        emission_v: Some([0.0, 0.11, -0.35]),
        hand_a: AuthoredTransform::default(),
        hand_b: AuthoredTransform::default(),
        two_handed: false,
        right_hand_rotation: true,
        harness: None,
    }
}

fn assault() -> WeaponMounts {
    WeaponMounts {
        id: 11,
        emission_v: Some([-0.03, 0.115, -0.75]),
        hand_a: AuthoredTransform {
            translation: [-0.06, 0.08, -0.35],
            rotation_half_turns: [0.0, 0.0, 0.92],
        },
        hand_b: AuthoredTransform::default(),
        two_handed: true,
        right_hand_rotation: true,
        harness: Some(HarnessCorrection {
            translation: HARNESS_DEFAULT_TRANSLATION,
            rotation_radians: HARNESS_DEFAULT_ROTATION,
        }),
    }
}

fn launcher() -> WeaponMounts {
    WeaponMounts {
        id: 16,
        emission_v: Some([0.0, 0.1, 0.7]),
        hand_a: AuthoredTransform {
            translation: [-0.1, 0.06, 0.1],
            rotation_half_turns: [0.7, -0.2, -0.15],
        },
        hand_b: AuthoredTransform::default(),
        two_handed: true,
        right_hand_rotation: true,
        harness: Some(HarnessCorrection {
            translation: [0.05, 0.175, 0.07],
            rotation_radians: [1.8, 1.5, 1.5],
        }),
    }
}

/// The probe's non-commuting bone and character matrices.
fn bone_and_char() -> ((Flat, Transform), (Flat, Transform)) {
    let b = tr(rot(0.3, -0.4, 0.5), [0.2, 1.3, -0.1]);
    let c = tr(rot(1.1, 0.0, 0.0), [10.0, 2.0, -5.0]);
    let t = |m: &Flat| {
        Transform::from_translation(Vec3::new(m[12], m[13], m[14])).with_rotation(Quat::from_mat3(
            &Mat3::from_cols(
                Vec3::new(m[0], m[1], m[2]),
                Vec3::new(m[4], m[5], m[6]),
                Vec3::new(m[8], m[9], m[10]),
            ),
        ))
    };
    ((b, t(&b)), (c, t(&c)))
}

/// Runtime world of a weapon on `bone`: char * bone * bone_local (column form of
/// `Correction * Bone * CharWorld`).
fn runtime_world(bone: usize, m: &WeaponMounts) -> (Transform, Flat) {
    let ((bf, bt), (cf, ct)) = bone_and_char();
    (
        ct * bt * bone_local(bone, Some(m)),
        oracle_world(bone, m, &bf, &cf),
    )
}

#[test]
fn the_native_rotation_formula_matches_the_probe_axes() {
    // right hand: model X -> bone +Y, Y -> -Z, Z -> -X; left hand mirrored
    let r = hand_rotation(false);
    assert!((r * Vec3::X - Vec3::Y).length() < 1e-5);
    assert!((r * Vec3::Y - Vec3::NEG_Z).length() < 1e-5);
    assert!((r * Vec3::Z - Vec3::NEG_X).length() < 1e-5);
    let l = hand_rotation(true);
    assert!((l * Vec3::X - Vec3::NEG_Y).length() < 1e-5);
    assert!((l * Vec3::Y - Vec3::NEG_Z).length() < 1e-5);
    assert!((l * Vec3::Z - Vec3::X).length() < 1e-5);
    // and the rotation the reconstruction already used
    assert!(r.dot(crate::rig::weapon_in_right_hand()).abs() > 0.99999);
}

#[test]
#[allow(clippy::approx_constant)]
fn half_turns_are_converted_exactly_once() {
    let a = authored_transform(&assault().hand_a);
    // Assault .92 half-turns = 2.890265 rad about the third angle
    let expect = native_rotation([0.0, 0.0, 2.890265]);
    assert!(a.rotation.dot(expect).abs() > 0.999_999);
    // converting twice (x pi again) or not at all would differ
    let twice = native_rotation([0.0, 0.0, 0.92 * PI * PI]);
    let never = native_rotation([0.0, 0.0, 0.92]);
    assert!(a.rotation.dot(twice).abs() < 0.9999);
    assert!(a.rotation.dot(never).abs() < 0.9999);
    // launcher [.7,-.2,-.15] -> [2.199115,-.628319,-.471239]
    let l = authored_transform(&launcher().hand_a);
    assert!(
        l.rotation
            .dot(native_rotation([2.199115, -0.628319, -0.471239]))
            .abs()
            > 0.999_999
    );
    assert_eq!(l.translation, Vec3::new(-0.1, 0.06, 0.1));
    // zero rotation is the identity
    assert!(
        authored_transform(&beretta().hand_a)
            .rotation
            .dot(Quat::IDENTITY)
            > 0.999_999
    );
    // radians stay radians: the harness 3.14 is a literal, not pi
    let h = bone_local(HARNESS, Some(&assault()));
    assert!(h.rotation.dot(native_rotation([1.5, -1.9, 3.14])).abs() > 0.999_999);
}

#[test]
fn held_world_matches_the_probe_for_pistol_rifle_and_launcher() {
    for (m, name) in [
        (beretta(), "beretta"),
        (assault(), "assault"),
        (launcher(), "rpg"),
    ] {
        let (world, oracle) = runtime_world(ATTACH_HAND_RIGHT, &m);
        assert!(close(&flat(&world), &oracle), "{name}");
        // the wrong composition order is not equal
        let ((_, bt), (_, ct)) = bone_and_char();
        let wrong = ct * bt * Transform::IDENTITY;
        assert!(!close(&flat(&wrong), &oracle), "{name}");
        // V in the world
        let v = m.emission_v.unwrap();
        let p = point(v, &oracle);
        let q = emission_frame(&world, Vec3::from(v)).origin;
        assert!((q - Vec3::from(p)).length() < 2e-4, "{name} V");
    }
}

#[test]
fn dual_wield_puts_slot_0_on_the_left_hand() {
    let slots = [true, true, false, false, false];
    let (state, bones) = draw(&slots, 4).unwrap();
    assert_eq!(state, 4);
    assert_eq!(bones, vec![(0, ATTACH_HAND_LEFT), (1, ATTACH_HAND_RIGHT)]);
    assert_eq!(active_index(&slots, 4), 1);
    assert_eq!(a_target_owner(&slots, 4), None);
    let m = beretta();
    let (left, oracle_l) = runtime_world(ATTACH_HAND_LEFT, &m);
    assert!(close(&flat(&left), &oracle_l));
    let (right, oracle_r) = runtime_world(ATTACH_HAND_RIGHT, &m);
    assert!(close(&flat(&right), &oracle_r));
    // the barrel points +x in the right bone and -x in the left one
    assert!(
        (bone_local(ATTACH_HAND_RIGHT, Some(&m)).rotation * Vec3::NEG_Z - Vec3::X).length() < 1e-5
    );
    assert!(
        (bone_local(ATTACH_HAND_LEFT, Some(&m)).rotation * Vec3::NEG_Z - Vec3::NEG_X).length()
            < 1e-5
    );
}

#[test]
fn stow_lands_on_the_native_bones_with_the_exact_corrections() {
    assert_eq!(stow_bone(0), Some(LEFT_UP_LEG));
    assert_eq!(stow_bone(1), Some(RIGHT_UP_LEG));
    assert_eq!(stow_bone(2), Some(HARNESS));
    assert_eq!(stow_bone(3), None);
    for (bone, m, name) in [
        (LEFT_UP_LEG, beretta(), "pistol slot 0"),
        (RIGHT_UP_LEG, beretta(), "pistol slot 1"),
        (HARNESS, assault(), "assault harness"),
        (HARNESS, launcher(), "launcher harness"),
    ] {
        let (world, oracle) = runtime_world(bone, &m);
        assert!(close(&flat(&world), &oracle), "{name}");
    }
    // the exact data: thigh corrections, per-weapon harness overrides
    let left = bone_local(LEFT_UP_LEG, None);
    assert_eq!(left.translation, Vec3::new(-0.11, 0.06, 0.06));
    assert!(left.rotation.dot(native_rotation([0.0, -1.7, 0.0])).abs() > 0.999_999);
    let right = bone_local(RIGHT_UP_LEG, None);
    assert_eq!(right.translation, Vec3::new(0.14, 0.03, 0.06));
    assert!(right.rotation.dot(native_rotation([0.2, -1.7, 0.0])).abs() > 0.999_999);
    let rpg = bone_local(HARNESS, Some(&launcher()));
    assert_eq!(rpg.translation, Vec3::new(0.05, 0.175, 0.07));
    assert!(rpg.rotation.dot(native_rotation([1.8, 1.5, 1.5])).abs() > 0.999_999);
    // no corrections authored: a null pointer pair is the identity
    assert_eq!(bone_local(HARNESS, Some(&beretta())), Transform::IDENTITY);
    assert_eq!(bone_local(0x40, None), Transform::IDENTITY);
}

#[test]
fn slerp_smoothing_uses_the_native_factors() {
    assert_eq!(slerp_t(HIPS), Some(0.4));
    assert_eq!(slerp_t(LEFT_UP_LEG), Some(0.4));
    assert_eq!(slerp_t(RIGHT_UP_LEG), Some(0.5));
    assert_eq!(slerp_t(HARNESS), Some(0.3));
    assert_eq!(slerp_t(ATTACH_HAND_RIGHT), None);
    let mut s = StowSmoothing::default();
    let a = Quat::from_rotation_y(0.0);
    let b = Quat::from_rotation_y(1.0);
    // the first sample seeds the cache, the next lags by t
    assert!(s.filter(HARNESS, a).dot(a) > 0.999_999);
    let next = s.filter(HARNESS, b);
    assert!(next.dot(a.slerp(b, 0.3)) > 0.999_999);
    // the caches are separate per case, and shared by whoever asks
    assert!(s.filter(RIGHT_UP_LEG, b).dot(b) > 0.999_999);
    // a hand bone is not smoothed, and a disabled filter is the identity map
    assert_eq!(s.filter(ATTACH_HAND_RIGHT, b), b);
    s.enabled = false;
    assert_eq!(s.filter(HARNESS, a), a);
}

#[test]
fn the_holder_rejects_stale_and_absent_slots() {
    let none = [false; HOLDER_SLOTS];
    for state in 0..=6 {
        assert!(draw(&none, state).is_none(), "empty holder, state {state}");
        assert_eq!(active_index(&none, state), 5);
        assert_eq!(a_target_owner(&none, state), None);
    }
    // a state whose slot is empty: a no-op
    let pistol_only = [true, false, false, false, false];
    assert!(draw(&pistol_only, 3).is_none());
    assert!(draw(&pistol_only, 4).is_none());
    assert_eq!(active_index(&pistol_only, 3), 5);
    assert_eq!(
        draw(&pistol_only, 1).unwrap().1,
        vec![(0, ATTACH_HAND_RIGHT)]
    );
    // the A target belongs to slot 2 in state 3 only
    let rifle = [false, false, true, false, false];
    assert_eq!(a_target_owner(&rifle, 3), Some(2));
    assert_eq!(a_target_owner(&rifle, 1), None);
    assert_eq!(a_target_owner(&rifle, 2), None);
    assert_eq!(active_index(&rifle, 3), 2);
    // a bone that is not in the table, or the wrong skeleton order, is not resolved
    assert_eq!(bone_name(0x40), None);
    let sk = jc2_formats::animation::Skeleton {
        name: "civilian".into(),
        bones: vec![],
        reference_pose: vec![],
    };
    assert_eq!(resolve_bone(&sk, HARNESS), None);
}

#[test]
fn the_emission_origin_is_v_in_the_weapon_world_and_the_direction_its_minus_z() {
    let m = assault();
    let (world, _) = runtime_world(ATTACH_HAND_RIGHT, &m);
    let v = Vec3::from(m.emission_v.unwrap());
    let f = emission_frame(&world, v);
    assert!((f.origin - world.transform_point(v)).length() < 1e-5);
    assert!((f.direction - world.rotation * Vec3::NEG_Z).length() < 1e-5);
    assert!((f.direction.length() - 1.0).abs() < 1e-5);
    // V is used when authored; the bounding box only when it is absent
    let (lo, hi) = (Vec3::new(-0.1, -0.1, -0.5), Vec3::new(0.1, 0.2, 0.3));
    assert_eq!(emission_point(Some(&m), lo, hi), v);
    let mut none = m.clone();
    none.emission_v = None;
    assert_eq!(
        emission_point(Some(&none), lo, hi),
        crate::poses::muzzle_of(lo, hi)
    );
}

#[test]
fn the_a_target_follows_the_weapon_world() {
    for m in [assault(), launcher()] {
        let (world, oracle) = runtime_world(ATTACH_HAND_RIGHT, &m);
        let a = authored_transform(&m.hand_a);
        let a_world = world * a;
        let a_local = tr(
            rot(
                m.hand_a.rotation_half_turns[0] * PI,
                m.hand_a.rotation_half_turns[1] * PI,
                m.hand_a.rotation_half_turns[2] * PI,
            ),
            m.hand_a.translation,
        );
        assert!(close(&flat(&a_world), &mul(&a_local, &oracle)));
    }
}

#[test]
fn the_two_bone_solver_reaches_bounds_and_rejects() {
    let (root, mid, end) = (
        Vec3::ZERO,
        Vec3::new(0.3, 0.0, 0.1),
        Vec3::new(0.6, 0.0, 0.0),
    );
    let (l1, l2) = ((mid - root).length(), (end - mid).length());
    let target = Vec3::new(0.35, 0.2, -0.15);
    let (m, e) = two_bone_solve(root, mid, end, target).unwrap();
    assert!(e.distance(target) < 1e-4);
    assert!(((m - root).length() - l1).abs() < 1e-4);
    assert!(((e - m).length() - l2).abs() < 1e-4);
    // the elbow stays on the side it was (+z)
    assert!(m.z > 0.0);
    // out of reach, but inside the limit: the arm straightens towards it
    let far = Vec3::new(l1 + l2 + 0.05, 0.0, 0.0);
    let (_, e) = two_bone_solve(root, mid, end, far).unwrap();
    assert!(e.distance(Vec3::new(l1 + l2, 0.0, 0.0)) < 2e-3);
    // beyond the limit: rejected
    assert!(
        two_bone_solve(
            root,
            mid,
            end,
            Vec3::X * (l1 + l2) * (SUPPORT_REACH_LIMIT + 0.1)
        )
        .is_none()
    );
    assert!(two_bone_solve(root, mid, end, Vec3::NAN).is_none());
}

#[test]
fn every_unresolved_item_is_an_explicit_diagnostic() {
    for id in [
        "pickup-slot-assignment",
        "character-0x728-gate",
        "ik-weighting",
        "solver-0x008f09c0",
    ] {
        assert!(UNSUPPORTED.iter().any(|u| u.id == id), "{id}");
    }
}

/// Headless, needs JC2_TEST_DIR: Rico's guns on their native bones, the stow corrections,
/// the left hand on the rifle's A and V as the muzzle.
#[test]
fn rico_guns_follow_the_native_slots() {
    use crate::character::CharacterPlugin;
    use crate::combat::CombatPlugin;
    use crate::rig::Joint;
    use bevy::input::mouse::AccumulatedMouseMotion;
    let Some((mut app, _, _)) = crate::fleet::testing::app(
        &["mc01_lod1-rico.rbm", "weapons.bin", "general.blz"],
        |app| {
            app.init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .add_plugins((CharacterPlugin, CombatPlugin));
            app.world_mut().spawn((
                Transform::from_xyz(-250.0, 0.0, -8955.0),
                crate::world::FlyCamera {
                    yaw: 0.0,
                    pitch: 0.0,
                    speed: 1.0,
                },
            ));
        },
    ) else {
        return;
    };
    crate::fleet::testing::wait_for_hero(&mut app);
    if app.world().get_resource::<crate::rig::Rig>().is_none() {
        return;
    }
    let held = |app: &mut App, slot: usize| {
        let mut q = app.world_mut().query::<&GunModel>();
        q.iter(app.world())
            .any(|g| g.slot == slot && g.stow == Stow::Hand)
    };
    crate::fleet::testing::wait_until(&mut app, "the pistol in his hand", |app| held(app, 0));
    app.world_mut().resource_mut::<StowSmoothing>().enabled = false;
    for _ in 0..5 {
        app.update();
    }
    let joint = |app: &mut App, name: &str| {
        let mut q = app.world_mut().query::<(Entity, &Joint, &Name)>();
        let e = q
            .iter(app.world())
            .find(|(_, _, n)| n.as_str() == name)
            .map(|(e, _, _)| e)
            .unwrap_or_else(|| panic!("no joint {name}"));
        app.world()
            .get::<GlobalTransform>(e)
            .unwrap()
            .compute_transform()
    };
    let gun = |app: &mut App, slot: usize| {
        let mut q = app.world_mut().query::<(&GunModel, &GlobalTransform)>();
        q.iter(app.world())
            .find(|(g, _)| g.slot == slot)
            .map(|(g, t)| (g.stow, g.muzzle, t.compute_transform()))
            .unwrap()
    };
    let mounts = app.world().resource::<WeaponMountTable>().0.clone();
    let beretta = mounts.get("Beretta Handgun").expect("no Beretta mounts");
    let assault_m = mounts.get("Assault").expect("no Assault mounts");
    let same = |a: &Transform, b: &Transform| {
        a.translation.distance(b.translation) < 1e-3 && a.rotation.dot(b.rotation).abs() > 0.9999
    };
    // the pistol: AttachHandRight + R(-pi/2,0,pi/2); the muzzle is V in the world
    let hand = joint(&mut app, "AttachHandRight");
    let (stow, muzzle, pistol) = gun(&mut app, 0);
    assert_eq!(stow, Stow::Hand);
    assert!(same(
        &pistol,
        &(hand * bone_local(ATTACH_HAND_RIGHT, Some(beretta)))
    ));
    assert_eq!(muzzle, Vec3::new(0.0, 0.11, -0.35));
    let frame = emission_frame(&pistol, muzzle);
    assert!((frame.origin - hand.transform_point(hand_rotation(false) * muzzle)).length() < 1e-3);
    // draw the rifle: held, the pistol on a thigh, the left hand on A
    app.world_mut().resource_mut::<Arsenal>().current = 3;
    crate::fleet::testing::wait_until(&mut app, "the rifle in his hand", |app| held(app, 3));
    for _ in 0..150 {
        app.update();
    }
    assert_eq!(
        app.world().resource::<Arsenal>().guns[3].weapon.name,
        "Assault"
    );
    let thigh = joint(&mut app, "RightUpLeg");
    let (stow, _, pistol) = gun(&mut app, 0);
    assert_eq!(stow, Stow::RightUpLeg);
    assert!(same(
        &pistol,
        &(thigh * bone_local(RIGHT_UP_LEG, Some(beretta)))
    ));
    let (_, muzzle, rifle) = gun(&mut app, 3);
    assert_eq!(muzzle, Vec3::new(-0.03, 0.115, -0.75));
    let hand = joint(&mut app, "AttachHandRight");
    assert!(same(
        &rifle,
        &(hand * bone_local(ATTACH_HAND_RIGHT, Some(assault_m)))
    ));
    {
        let mut q = app.world_mut().query_filtered::<&Animator, With<Hero>>();
        let an = q.single(app.world()).unwrap();
        eprintln!(
            "clip {:?}: the pose left the support hand {:.3} m from A",
            an.clip, an.support.moved
        );
        assert!(an.support.moved < 0.3, "moved {:.3}", an.support.moved);
        let (target, error) = an
            .support
            .solved
            .expect("the left hand has no solution on the rifle");
        assert!(error < 0.03, "left hand {error:.3} m from A at {target:?}");
    }
    // stow the first two-handed gun (slot 2): Harness with its own correction
    let two_handed_name = app.world().resource::<Arsenal>().guns[2]
        .weapon
        .name
        .clone();
    let two_handed_m = mounts.get(&two_handed_name).expect("no mounts for slot 2");
    assert!(two_handed_m.two_handed);
    app.world_mut().resource_mut::<Arsenal>().current = 0;
    crate::fleet::testing::wait_until(&mut app, "the pistol in his hand again", |app| held(app, 0));
    for _ in 0..5 {
        app.update();
    }
    let harness = joint(&mut app, "Harness");
    let (stow, _, rifle) = gun(&mut app, 2);
    assert_eq!(stow, Stow::Harness);
    assert!(same(
        &rifle,
        &(harness * bone_local(HARNESS, Some(two_handed_m)))
    ));
    {
        let mut q = app.world_mut().query_filtered::<&Animator, With<Hero>>();
        let an = q.single(app.world()).unwrap();
        assert!(an.support.hand.is_none() && an.support.solved.is_none());
    }
    // smoothing back on: the slerp lags the bone a little, it does not detach the gun
    app.world_mut().resource_mut::<StowSmoothing>().enabled = true;
    for _ in 0..5 {
        app.update();
    }
    let harness = joint(&mut app, "Harness");
    let (_, _, rifle) = gun(&mut app, 2);
    let expect = harness * bone_local(HARNESS, Some(two_handed_m));
    assert!(rifle.translation.distance(expect.translation) < 0.06);
    assert!(rifle.rotation.dot(expect.rotation).abs() > 0.99);
}
