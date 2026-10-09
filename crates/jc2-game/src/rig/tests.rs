use super::*;
use crate::poses::{Arms, Gait, Stow, WeaponClass, drive_upper_body};

#[test]
fn turning_round_is_half_a_turn_about_the_vertical() {
    let q = Quat::from_euler(EulerRot::YXZ, 0.4, -0.3, 0.2);
    let t = QsTransform {
        translation: [0.1, 0.9, -0.2],
        rotation: q.to_array(),
        scale: [1.0; 3],
    };
    let r = turn_round(&t);
    let expect = Quat::from_rotation_y(std::f32::consts::PI) * q;
    let got = Quat::from_array(r.rotation);
    assert!(got.dot(expect).abs() > 0.9999, "{got:?} vs {expect:?}");
    let tr = Vec3::from(r.translation);
    assert!((tr - Vec3::new(-0.1, 0.9, 0.2)).length() < 1e-6);
}

/// Rico's rig with every clip; `None` without JC2_TEST_DIR.
fn rico() -> Option<Rig> {
    let dir = std::env::var("JC2_TEST_DIR").ok()?;
    let fs = GameFs::open(dir).ok()?;
    Rig::load(&fs, "biped.bsk")
}

fn animator(rig: &Rig) -> Animator {
    let n = rig.skeleton.bones.len();
    Animator::new(rig.0.clone(), vec![Entity::PLACEHOLDER; n])
}

fn bone(rig: &Rig, name: &str) -> usize {
    rig.skeleton
        .bone_index(name)
        .unwrap_or_else(|| panic!("no bone {name}"))
}

/// Every clip the game asks for is a `.ban` that loads, and its tracks land on the
/// skeleton's bones. Needs JC2_TEST_DIR.
#[test]
fn every_clip_maps_to_an_existing_ban() {
    let Some(rig) = rico() else { return };
    for &clip in Clip::ALL {
        let c = rig
            .clips
            .get(&clip)
            .unwrap_or_else(|| panic!("{clip:?}: {} did not load", clip.file()));
        assert!(c.anim.duration > 0.0, "{clip:?}");
        let mapped = c.bones.iter().filter(|b| b.is_some()).count();
        assert!(
            mapped * 10 >= c.bones.len() * 9,
            "{clip:?} ({}): {mapped} of {} tracks on bones",
            clip.file(),
            c.bones.len()
        );
    }
}

/// The pelvis's left-to-right axis in every clip must point the way it does in the
/// idle, or that clip has him facing backwards (`grpl_hang.ban` did, 180° out).
/// Needs JC2_TEST_DIR.
#[test]
fn every_clip_faces_the_same_way() {
    let Some(rig) = rico() else { return };
    let (l, r) = (bone(&rig, "LeftUpLeg"), bone(&rig, "RightUpLeg"));
    let hips = bone(&rig, "Hips");
    let mut scratch = Vec::new();
    let mut idle_axis = None;
    let mut order: Vec<Clip> = vec![Clip::Idle];
    order.extend(Clip::ALL.iter().copied().filter(|c| *c != Clip::Idle));
    for clip in order {
        if clip == Clip::Dive {
            continue; // lies flat
        }
        let Some(c) = rig.clips.get(&clip) else {
            continue;
        };
        // the upper-body layers leave the hips to the base clip
        if !c.bones.contains(&Some(hips)) {
            continue;
        }
        let Some(bones) = clip_bones(&rig, clip, c.anim.duration * 0.5, &mut scratch) else {
            continue;
        };
        let local: Vec<Transform> = (0..rig.skeleton.bones.len())
            .map(|i| qs_to_bevy(&bones[i].unwrap_or(rig.skeleton.reference_pose[i])))
            .collect();
        let global = model_space(&rig.skeleton, &local);
        let axis = (global[r].translation - global[l].translation).normalize();
        let idle = *idle_axis.get_or_insert(axis);
        // the strafes are side-on takes: the body turned against the way they travel
        let idle = Quat::from_rotation_y(clip.body_yaw()) * idle;
        // (the hips tilt in a crouch and turn in a roll or a strafe: same facing, a
        // wider cone)
        let limit = match clip {
            Clip::Crouch | Clip::LandRoll | Clip::LandHard | Clip::Reel => 0.5,
            Clip::WalkStrafeLeft | Clip::WalkStrafeRight => 0.7,
            // the heavy landing twists the hips 60° to his right as he goes down, and the
            // soldiers stand and walk side-on, the gun across them
            Clip::LandHeavy | Clip::NpcIdle => 0.2,
            Clip::NpcWalk => 0.5,
            _ => 0.8,
        };
        assert!(
            axis.dot(idle) > limit,
            "{clip:?} ({}): left-right axis {axis:?}, the idle's is {idle:?}",
            clip.file()
        );
    }
    let idle = idle_axis.expect("no clips loaded");
    // he faces −z (`coords::MODEL_FORWARD`) with his right leg on +x, his right
    // (`coords::MODEL_RIGHT`): the same frame as the vehicles' (nose −z, the driver's
    // left −x)
    assert!(
        idle.dot(crate::coords::MODEL_RIGHT) > 0.8,
        "left-to-right axis {idle:?}"
    );
}

/// Step an animator `seconds` at 60 Hz.
fn run_for(an: &mut Animator, seconds: f32) {
    let n = (seconds * 60.0).round() as usize;
    for _ in 0..n.max(1) {
        an.step(1.0 / 60.0);
    }
}

/// The weapon's frame in the model's space when it is held in the right hand.
fn gun_frame(rig: &Rig, an: &Animator) -> Transform {
    let global = model_space(&rig.skeleton, &an.local);
    let (_, mount) = Stow::Hand.mount(None).unwrap();
    global[bone(rig, "AttachHandRight")] * mount
}

/// The gun sits in the palm with its barrel along the hand in every pose, and in the
/// two-handed poses the left hand is on the handguard: on the barrel's line, ahead of
/// the grip — which is what fixes the turn `weapon_in_right_hand` gives the model.
/// Needs JC2_TEST_DIR.
#[test]
fn the_gun_follows_the_hand_and_the_left_hand_holds_the_handguard() {
    let Some(rig) = rico() else { return };
    let hand = bone(&rig, "RightHand");
    let left = bone(&rig, "AttachHandLeft");
    let cases: [(Clip, Option<Clip>, bool); 7] = [
        (Clip::Idle, None, false),
        (Clip::Run, Some(Clip::Wield1HRun), false),
        (Clip::Idle, Some(Clip::Aim1H), false),
        (Clip::Idle, Some(Clip::Wield2HIdle), true),
        (Clip::Run, Some(Clip::Wield2HRun), true),
        (Clip::Idle, Some(Clip::Aim2H), true),
        (Clip::Run, Some(Clip::Aim2HLow), true),
    ];
    for (base, layer, two_handed) in cases {
        let mut an = animator(&rig);
        an.play(base);
        an.set_layer(Slot::Aim, layer, 0.05, false);
        run_for(&mut an, 0.6);
        let global = model_space(&rig.skeleton, &an.local);
        let gun = gun_frame(&rig, &an);
        // the grip is in the palm: a few centimetres from the wrist bone
        let grip_from_wrist = gun.translation.distance(global[hand].translation);
        assert!(
            (0.03..0.1).contains(&grip_from_wrist),
            "{base:?}+{layer:?}: grip {grip_from_wrist} m from the wrist"
        );
        // the barrel runs along the hand (its +x), the gun's top on the thumb's side
        let barrel = gun.rotation * Vec3::NEG_Z;
        let along = global[hand].rotation * Vec3::X;
        assert!(barrel.dot(along) > 0.99, "{base:?}+{layer:?}");
        if two_handed {
            // the off hand: on the barrel's line within 9 cm, 20 to 45 cm ahead
            let to_left = global[left].translation - gun.translation;
            let ahead = to_left.dot(barrel);
            let off_line = (to_left - barrel * ahead).length();
            assert!(
                (0.2..0.45).contains(&ahead) && off_line < 0.12,
                "{base:?}+{layer:?}: left hand {ahead} m ahead, {off_line} m off the line"
            );
            // and above it, under the handguard: the gun's up points from grip to hand
            let up = gun.rotation * Vec3::Y;
            assert!(
                to_left.dot(up) > 0.0,
                "{base:?}+{layer:?}: the gun is upside down"
            );
        }
    }
}

fn angle(a: Vec3, b: Vec3) -> f32 {
    a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos()
}

/// Aiming, the barrel points along the aim within 10° for every class, standing and on
/// the move, across the range the spine can turn. Needs JC2_TEST_DIR.
#[test]
fn the_barrel_points_along_the_aim() {
    let Some(rig) = rico() else { return };
    // (beyond these the legs turn: `character::ground_facing`)
    let dirs = [
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-0.5, 0.0, -1.0),
        Vec3::new(0.55, 0.0, -1.0),
        Vec3::new(0.0, 0.4, -1.0),
        Vec3::new(0.3, -0.45, -1.0),
    ];
    for class in [
        WeaponClass::Pistol,
        WeaponClass::Smg,
        WeaponClass::Rifle,
        WeaponClass::Shotgun,
        WeaponClass::Rpg,
    ] {
        for (base, gait) in [(Clip::Idle, Gait::Idle), (Clip::Run, Gait::Run)] {
            for dir in dirs {
                let mut an = animator(&rig);
                an.play(base);
                let mut arms = Arms {
                    class: Some(class),
                    aim_dir: dir.normalize(),
                    ..default()
                };
                for _ in 0..40 {
                    arms.aim_for = 1.0;
                    drive_upper_body(&mut an, &mut arms, gait, Quat::IDENTITY, false, 1.0 / 60.0);
                    an.step(1.0 / 60.0);
                }
                assert!(an.aim.weight > 0.99);
                let barrel = gun_frame(&rig, &an).rotation * Vec3::NEG_Z;
                let off = angle(barrel, dir).to_degrees();
                assert!(
                    off < 10.0,
                    "{class:?} {base:?} aiming {dir:?}: the barrel points {barrel:?}, {off:.1}° off"
                );
            }
        }
    }
}

/// While he runs and aims, the legs keep the run clip untouched; the spine turns
/// towards the aim, and no further than its limit (beyond it the legs have to follow:
/// `character::ground_facing`). Needs JC2_TEST_DIR.
#[test]
fn the_upper_body_aims_while_the_legs_run() {
    let Some(rig) = rico() else { return };
    let legs = [
        "LeftUpLeg",
        "RightUpLeg",
        "LeftLeg",
        "RightLeg",
        "LeftFoot",
        "Hips",
    ]
    .map(|n| bone(&rig, n));
    let mut plain = animator(&rig);
    plain.play(Clip::Run);
    let mut aiming = animator(&rig);
    aiming.play(Clip::Run);
    let mut arms = Arms {
        class: Some(WeaponClass::Rifle),
        // 50° to his right
        aim_dir: Vec3::new(0.766, 0.0, -0.643),
        ..default()
    };
    let mut yaw = 0.0;
    for _ in 0..60 {
        arms.aim_for = 1.0;
        drive_upper_body(
            &mut aiming,
            &mut arms,
            Gait::Run,
            Quat::IDENTITY,
            false,
            1.0 / 60.0,
        );
        aiming.step(1.0 / 60.0);
        plain.step(1.0 / 60.0);
        yaw = aiming.aim.applied.0;
    }
    assert_eq!(aiming.clip, Clip::Run);
    assert_eq!(aiming.layer(Slot::Aim).clip, Some(Clip::Aim2H));
    for b in legs {
        let (a, p) = (aiming.local[b], plain.local[b]);
        assert!(
            a.rotation.dot(p.rotation).abs() > 0.9999
                && a.translation.distance(p.translation) < 1e-4,
            "{} moved with the aim",
            rig.skeleton.bones[b].name
        );
    }
    // turning right is a negative yaw about +y
    assert!(yaw < -0.1, "the spine turned {yaw} rad");
    let spine1 = bone(&rig, "Spine1");
    assert!(
        aiming.local[spine1]
            .rotation
            .dot(plain.local[spine1].rotation)
            .abs()
            < 0.999,
        "the spine did not turn"
    );
    // straight behind him: the spine stops at its limit
    arms.aim_dir = Vec3::new(0.05, 0.0, 1.0).normalize();
    for _ in 0..30 {
        arms.aim_for = 1.0;
        drive_upper_body(
            &mut aiming,
            &mut arms,
            Gait::Run,
            Quat::IDENTITY,
            false,
            1.0 / 60.0,
        );
        aiming.step(1.0 / 60.0);
    }
    assert!(aiming.aim.applied.0.abs() <= AIM_YAW_LIMIT + 1e-4);
    // and the aim lets go when the gun comes down
    arms.aim_for = 0.0;
    for _ in 0..30 {
        drive_upper_body(
            &mut aiming,
            &mut arms,
            Gait::Run,
            Quat::IDENTITY,
            false,
            1.0 / 60.0,
        );
        aiming.step(1.0 / 60.0);
    }
    assert_eq!(aiming.aim.weight, 0.0);
    assert_eq!(aiming.aim.applied, (0.0, 0.0));
}

/// A shot plays the recoil over the aim; a reload plays its clip stretched to the
/// data's reload time and is gone when the reload is. Needs JC2_TEST_DIR.
#[test]
fn shots_recoil_and_reloads_last_the_reload_time() {
    let Some(rig) = rico() else { return };
    let mut an = animator(&rig);
    an.play(Clip::Idle);
    let mut arms = Arms {
        class: Some(WeaponClass::Pistol),
        aim_dir: Vec3::NEG_Z,
        interval: 0.12,
        ..default()
    };
    let dt = 1.0 / 60.0;
    let tick = |an: &mut Animator, arms: &mut Arms| {
        arms.aim_for = arms.aim_for.max(0.5);
        drive_upper_body(an, arms, Gait::Idle, Quat::IDENTITY, false, dt);
        an.step(dt);
    };
    for _ in 0..10 {
        tick(&mut an, &mut arms);
    }
    assert_eq!(an.layer(Slot::Aim).clip, Some(Clip::Aim1H));
    assert_eq!(an.layer(Slot::Action).clip, None);
    arms.shots += 1;
    tick(&mut an, &mut arms);
    tick(&mut an, &mut arms);
    let recoil = an.layer(Slot::Action);
    assert_eq!(recoil.clip, Some(Clip::Recoil1H));
    assert!(recoil.weight > 0.0);
    // the arm moves off the aim pose with the kick
    let arm = bone(&rig, "RightArm");
    let mut still = animator(&rig);
    still.play(Clip::Idle);
    still.set_layer(Slot::Aim, Some(Clip::Aim1H), 0.01, false);
    run_for(&mut still, 0.2);
    for _ in 0..6 {
        tick(&mut an, &mut arms);
    }
    assert!(
        an.local[arm].rotation.dot(still.local[arm].rotation).abs() < 0.9995,
        "no kick"
    );
    // the Beretta's reload: 0.8 s in weapons.bin, the clip is 1 s long
    for _ in 0..60 {
        tick(&mut an, &mut arms);
    }
    arms.reload_time = 0.8;
    arms.reloads += 1;
    tick(&mut an, &mut arms);
    let r = an.layer(Slot::Action);
    assert_eq!(r.clip, Some(Clip::ReloadPistol));
    assert!((r.rate - rig.duration(Clip::ReloadPistol) / 0.8).abs() < 1e-3);
    let mut t = dt;
    while an.layer(Slot::Action).clip == Some(Clip::ReloadPistol) && t < 3.0 {
        tick(&mut an, &mut arms);
        t += dt;
    }
    assert!((0.75..0.9).contains(&t), "the reload played {t} s");
}

/// A throw, the hook and a layer switch crossfade instead of popping: no bone jumps
/// more than a few degrees from one frame to the next. Needs JC2_TEST_DIR.
#[test]
fn layers_crossfade_without_pops() {
    let Some(rig) = rico() else { return };
    let mut an = animator(&rig);
    an.play(Clip::Run);
    let mut arms = Arms {
        class: Some(WeaponClass::Rifle),
        aim_dir: Vec3::NEG_Z,
        ..default()
    };
    let dt = 1.0 / 60.0;
    let mut last: Option<Vec<Transform>> = None;
    for frame in 0..240 {
        let gait = match frame {
            0..60 => Gait::Run,
            60..120 => Gait::Idle,
            _ => Gait::Walk,
        };
        if frame == 90 {
            arms.throws += 1;
        }
        if frame == 150 {
            arms.aim_for = 1.0;
        }
        if frame == 200 {
            arms.hooks += 1;
        }
        let base = match gait {
            Gait::Run => Clip::Run,
            Gait::Idle => Clip::Idle,
            _ => Clip::Walk,
        };
        an.play_blend(base, 0.2);
        drive_upper_body(&mut an, &mut arms, gait, Quat::IDENTITY, false, dt);
        an.step(dt);
        let global = model_space(&rig.skeleton, &an.local);
        if let Some(prev) = &last {
            for name in ["Spine1", "Head", "RightHand", "LeftHand"] {
                // (the throw itself is fast: the left hand is left out while it flies)
                if name == "LeftHand" && (90..150).contains(&frame) {
                    continue;
                }
                let b = bone(&rig, name);
                let jump = prev[b].translation.distance(global[b].translation);
                // (raising the gun to aim takes the game's 0.07 s: quick, not a pop)
                assert!(jump < 0.2, "frame {frame}: {name} jumped {jump} m");
            }
        }
        last = Some(global);
    }
}

/// A left-arm throw must stay masked while another action fades it out.
#[test]
fn a_replaced_throw_keeps_its_mask_and_playback_rate() {
    let Some(rig) = rico() else { return };
    let mut an = animator(&rig);
    let mut plain = animator(&rig);
    an.play(Clip::Run);
    plain.play(Clip::Run);
    an.play_once_masked(Slot::Action, Clip::Throw, 0.5, 0.1, "LeftShoulder");
    for _ in 0..20 {
        an.step(1.0 / 60.0);
        plain.step(1.0 / 60.0);
    }
    let old_time = an.layer(Slot::Action).time;
    an.play_once(Slot::Action, Clip::ReloadRifle, 1.0, 0.2);
    an.step(1.0 / 60.0);
    plain.step(1.0 / 60.0);
    let previous = an.layer(Slot::Action).prev.unwrap();
    assert_eq!(previous.mask, rig.skeleton.bone_index("LeftShoulder"));
    assert!((previous.time - old_time - 0.5 / 60.0).abs() < 1e-5);
    for name in ["Hips", "LeftUpLeg", "RightUpLeg", "LeftFoot", "RightFoot"] {
        let b = bone(&rig, name);
        assert!(
            an.local[b].rotation.dot(plain.local[b].rotation).abs() > 0.9999,
            "{name} moved when the throw was replaced"
        );
        assert!(
            an.local[b].translation.distance(plain.local[b].translation) < 1e-4,
            "{name} moved when the throw was replaced"
        );
    }
}
