use super::*;
use crate::rig::{Rig, model_space};

fn rico() -> Option<Rig> {
    let dir = std::env::var("JC2_TEST_DIR").ok()?;
    let fs = jc2_formats::GameFs::open(dir).ok()?;
    Rig::load(&fs, "biped.bsk")
}

/// The animated Idle pose of the rig.
fn idle(rig: &Rig) -> Animator {
    let mut an = Animator::new(
        rig.0.clone(),
        vec![Entity::PLACEHOLDER; rig.skeleton.bones.len()],
    );
    an.step(0.0);
    an
}

/// A ray against a ground given as `height(x, z)` with a fixed normal.
fn ground(height: impl Fn(f32, f32) -> f32, normal: Vec3) -> impl FnMut(Vec3, f32) -> RayHit {
    move |start, length| {
        let h = height(start.x, start.z);
        let f = (start.y - h) / length;
        (0.0..=1.0).contains(&f).then_some((f, normal))
    }
}

fn run(
    rig: &Rig,
    an: &Animator,
    ik: &mut GroundIk,
    mut cast: impl FnMut(Vec3, f32) -> RayHit,
    calls: usize,
) -> Vec<Transform> {
    let mut out = an.local.clone();
    for _ in 0..calls {
        out = an.local.clone();
        apply(&rig.skeleton, &mut out, ik, &Transform::IDENTITY, &mut cast);
    }
    out
}

fn leg_bones(rig: &Rig, side: &str) -> LegBones {
    let b = |n: &str| rig.skeleton.bone_index(&format!("{side}{n}")).unwrap();
    LegBones {
        hip: b("UpLeg"),
        knee: b("Leg"),
        ankle: b("Foot"),
    }
}

#[test]
fn native_defaults_are_the_constructor_immediates() {
    let t = Tuning::default();
    assert_eq!(t.foot_planted_ankle_height, 0.0642);
    assert_eq!((t.max_ankle_height, t.min_ankle_height), (0.35, -0.35));
    assert_eq!((t.on_off_gain, t.foot_raised_gain), (0.3, 0.2));
    assert_eq!(
        (t.cosine_max_knee_angle, t.cosine_min_knee_angle),
        (-0.9, 0.8)
    );
}

/// Section 7 of the brief: the blend ramps 0.3, 0.51, 0.657 ... and decays 0.70, 0.49, 0.34
/// ... to zero after 13 calls (synthetic fixture, same numbers as the research probe).
#[test]
fn blend_ramps_and_decays_like_the_research_fixture() {
    let t = Tuning::default();
    let mut s = LegState::default();
    let mut seen = Vec::new();
    for _ in 0..3 {
        solve_leg(&t, &mut s, 0.1, 0.0, Some(0.0));
        seen.push(s.blend);
    }
    assert!((seen[0] - 0.3).abs() < 1e-6 && (seen[1] - 0.51).abs() < 1e-6);
    assert!((seen[2] - 0.657).abs() < 1e-5);
    s.blend = 1.0;
    let mut calls = 0;
    while s.blend > 0.0 {
        solve_leg(&t, &mut s, 0.1, 0.0, None);
        calls += 1;
        if calls == 1 {
            assert!((s.blend - 0.7).abs() < 1e-6);
        }
    }
    assert_eq!(calls, 13);
}

#[test]
fn height_limits_clamp_and_reject() {
    let t = Tuning::default();
    // ankle 0.10 above the owner: ground above +0.25 clamps at 0.35, below -0.45 rejects
    let mut s = LegState::default();
    solve_leg(&t, &mut s, 0.10, 0.0, Some(0.5));
    assert!(s.accepted && (s.offset - 0.25).abs() < 1e-6, "{s:?}");
    let mut s = LegState::default();
    solve_leg(&t, &mut s, 0.10, 0.0, Some(-0.44));
    assert!(s.accepted);
    let mut s = LegState::default();
    solve_leg(&t, &mut s, 0.10, 0.0, Some(-0.46));
    assert!(!s.accepted && s.blend == 0.0);
}

#[test]
fn smoothing_follows_the_ground_by_the_height_factor() {
    let t = Tuning::default();
    let mut s = LegState {
        blend: 0.5,
        prior: 0.0,
        ..default()
    };
    // a raised ankle (0.35) gets the lowest factor 0.2; planted (0.0642) the highest, 1.0
    solve_leg(&t, &mut s, 0.35, 0.0, Some(0.1));
    assert!((s.prior - 0.02).abs() < 1e-6, "{s:?}");
    let mut s = LegState {
        blend: 0.5,
        prior: 0.0,
        ..default()
    };
    solve_leg(&t, &mut s, 0.0642, 0.0, Some(0.1));
    assert!((s.prior - 0.1).abs() < 1e-6, "{s:?}");
}

#[test]
fn a_ray_ending_exactly_on_the_ground_is_a_miss() {
    let t = Tuning::default();
    let ankle = Vec3::new(0.0, 0.5, 0.0);
    // ground at the ray's end (ankle - 1.0)
    let mut c = ground(|_, _| -0.5, Vec3::Y);
    assert!(probe_ground(&t, ankle, ankle, &mut c).is_none());
    let mut c = ground(|_, _| -0.499, Vec3::Y);
    assert!(probe_ground(&t, ankle, ankle, &mut c).is_some());
    // flat ground 0.5 below: fraction 0.75
    let mut c = ground(|_, _| 0.0, Vec3::Y);
    let (h, _) = probe_ground(&t, ankle, ankle, &mut c).unwrap();
    assert!(h.abs() < 1e-6);
}

#[test]
fn only_the_enable_states_switch_it_on() {
    let mut walking = Hero::default();
    walking.grounded = true;
    assert!(!wants_ik(GroundIkMode::Native, &walking));
    assert!(wants_ik(GroundIkMode::AllGrounded, &walking));
    assert!(!wants_ik(GroundIkMode::Off, &walking));
    let mut roof = Hero::default();
    roof.stunt = Some(Entity::from_raw_u32(7).unwrap());
    assert!(wants_ik(GroundIkMode::Native, &roof));
    assert!(!wants_ik(GroundIkMode::Off, &roof));
}

/// Flat ground at his feet leaves the idle pose where the animation put it.
#[test]
fn flat_ground_keeps_the_pose() {
    let Some(rig) = rico() else { return };
    let an = idle(&rig);
    let mut ik = GroundIk::for_rig(&rig.skeleton);
    ik.enabled = true;
    let out = run(&rig, &an, &mut ik, ground(|_, _| 0.0, Vec3::Y), 40);
    let (a, b) = (
        model_space(&rig.skeleton, &an.local),
        model_space(&rig.skeleton, &out),
    );
    for side in ["Left", "Right"] {
        let l = leg_bones(&rig, side);
        for i in [l.knee, l.ankle] {
            let d = a[i].translation.distance(b[i].translation);
            assert!(d < 0.02, "{side} bone {i} moved {d} on flat ground");
        }
    }
}

/// A step under only the left foot: that ankle rises by the step, the other stays, the hip
/// (so the pelvis) does not move and the bones keep their lengths.
#[test]
fn a_stair_lifts_one_foot_and_not_the_other() {
    let Some(rig) = rico() else { return };
    let an = idle(&rig);
    let (l, r) = (leg_bones(&rig, "Left"), leg_bones(&rig, "Right"));
    let base = model_space(&rig.skeleton, &an.local);
    let left_x = base[l.ankle].translation.x;
    assert!(left_x * base[r.ankle].translation.x < 0.0);
    let mut ik = GroundIk::for_rig(&rig.skeleton);
    ik.enabled = true;
    let step = 0.15;
    let out = run(
        &rig,
        &an,
        &mut ik,
        ground(move |x, _| if x * left_x > 0.0 { step } else { 0.0 }, Vec3::Y),
        60,
    );
    let g = model_space(&rig.skeleton, &out);
    let rise = g[l.ankle].translation.y - base[l.ankle].translation.y;
    assert!((rise - step).abs() < 0.01, "left ankle rose {rise}");
    assert!(
        (g[r.ankle].translation.y - base[r.ankle].translation.y).abs() < 0.02,
        "right ankle moved"
    );
    assert!(g[l.hip].translation.distance(base[l.hip].translation) < 1e-5);
    assert!(
        (ik.legs[0].error - step).abs() < 1e-5 && ik.legs[1].error.abs() < 1e-5,
        "independent errors {:?}",
        ik.legs
    );
    for (leg, b) in [(0, l), (1, r)] {
        for (a, c) in [(b.hip, b.knee), (b.knee, b.ankle)] {
            let before = base[a].translation.distance(base[c].translation);
            let after = g[a].translation.distance(g[c].translation);
            assert!(
                (before - after).abs() < 1e-3,
                "leg {leg} bone length {before} -> {after}"
            );
        }
    }
}

/// On a slope the foot leans onto the ground's normal (by the slope's angle).
#[test]
fn a_slope_tilts_the_foot() {
    let Some(rig) = rico() else { return };
    let an = idle(&rig);
    let l = leg_bones(&rig, "Left");
    let base = model_space(&rig.skeleton, &an.local);
    let mut ik = GroundIk::for_rig(&rig.skeleton);
    ik.enabled = true;
    let normal = Vec3::new(-0.5, 1.0, 0.0).normalize();
    let out = run(&rig, &an, &mut ik, ground(|x, _| 0.5 * x, normal), 80);
    let g = model_space(&rig.skeleton, &out);
    let turned =
        (g[l.ankle].rotation * base[l.ankle].rotation.inverse()).angle_between(Quat::IDENTITY);
    let want = (0.5f32).atan();
    assert!((turned - want).abs() < 0.08, "tilted {turned}, slope {want}");
    for q in &g {
        assert!(q.translation.is_finite() && q.rotation.is_finite());
    }
}

/// Disabling lets the blend run out and the animated pose come back exactly.
#[test]
fn retiring_restores_the_animation() {
    let Some(rig) = rico() else { return };
    let an = idle(&rig);
    let mut ik = GroundIk::for_rig(&rig.skeleton);
    ik.enabled = true;
    let mut cast = ground(|_, _| 0.1, Vec3::Y);
    run(&rig, &an, &mut ik, &mut cast, 30);
    assert!(ik.legs[0].blend > 0.9);
    ik.enabled = false;
    let out = run(&rig, &an, &mut ik, &mut cast, 13);
    assert_eq!(ik.legs[0].blend, 0.0);
    for (a, b) in an.local.iter().zip(&out) {
        assert!(a.translation.distance(b.translation) < 1e-6);
        assert!(a.rotation.angle_between(b.rotation) < 1e-3);
    }
}

/// A target beyond the leg's reach straightens the leg and stays finite.
#[test]
fn an_unreachable_target_stays_finite() {
    let Some(rig) = rico() else { return };
    let an = idle(&rig);
    let mut ik = GroundIk::for_rig(&rig.skeleton);
    ik.enabled = true;
    ik.tuning.max_ankle_height = 5.0;
    ik.tuning.raycast_distance_up = 10.0;
    ik.tuning.raycast_distance_down = 10.0;
    let out = run(&rig, &an, &mut ik, ground(|_, _| 3.0, Vec3::Y), 60);
    for q in model_space(&rig.skeleton, &out) {
        assert!(q.translation.is_finite() && q.rotation.is_finite());
    }
}
