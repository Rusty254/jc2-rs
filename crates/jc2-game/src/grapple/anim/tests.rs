//! Headless traces of the grapple's animation graph: every state the body (layer 0) and the
//! left arm (layer 2) pass through, with the clip each one plays. Run with `--nocapture`
//! to read them; with `JC2_TEST_DIR` the clip lengths are the retail ones, without it a
//! fixed stand-in length is used (the sequences do not depend on it).

use super::*;
use crate::rig::Rig;
use jc2_formats::GameFs;

/// Retail clip lengths when a game folder is set, else half a second for every clip.
fn durations() -> impl Fn(Clip) -> f32 {
    let rig = std::env::var("JC2_TEST_DIR")
        .ok()
        .and_then(|d| GameFs::open(d).ok())
        .and_then(|fs| Rig::load(&fs, "biped.bsk"));
    move |c| match &rig {
        Some(r) => r.duration(c),
        None => 0.5,
    }
}

/// Step the clock until `seconds` have passed.
fn run(a: &mut GrappleAnim, seconds: f32, dur: &dyn Fn(Clip) -> f32) {
    let dt = 1.0 / 64.0;
    let mut t = 0.0;
    while t < seconds {
        a.tick(dt, dur);
        t += dt;
    }
}

fn states(a: &GrappleAnim, layer: u8) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in a.steps.iter().filter(|s| s.layer == layer && s.accepted) {
        if out.last() != Some(&s.to) {
            out.push(s.to);
        }
    }
    out
}

fn show(name: &str, a: &GrappleAnim) {
    println!("--- {name}\n{}\n", a.trace());
}

// ------------------------------------------------------------ the selector

#[test]
fn the_selector_follows_the_native_table() {
    let pick = |t, wire| ActionSet::default().select(t, wire);
    assert_eq!(pick(HookTarget::Terrain, false), Choice::Reel);
    assert_eq!(pick(HookTarget::Character, false), Choice::Pull);
    assert_eq!(pick(HookTarget::Object(1), false), Choice::Pull);
    assert_eq!(pick(HookTarget::Object(2), false), Choice::Reel);
    for k in [0u8, 3, 4, 7, 127] {
        assert_eq!(pick(HookTarget::Object(k), false), Choice::Neither, "category {k}");
    }
    // a second hook switches Reel, Pull and the parachute action off, whatever it hit
    for t in [HookTarget::Terrain, HookTarget::Character, HookTarget::Object(2)] {
        let mut s = ActionSet::default();
        s.select(HookTarget::Terrain, false);
        assert!(s.is(SetAction::ReelIn));
        assert_eq!(s.select(t, true), Choice::Dual);
        assert!(!s.is(SetAction::ReelIn) && !s.is(SetAction::PullGrappled));
        assert!(!s.is(SetAction::DeployParachute));
        assert!(s.is(SetAction::FireHook));
    }
    // never both at once, and only FireHook at spawn
    let s = ActionSet::default();
    assert_eq!(s.enabled, [true, false, false, false, false, false]);
}

#[test]
fn mass_is_never_compared() {
    use crate::grapple::class::{ClassInput, classify};
    // a 5000 kg body authored light is pulled, a 1 kg one authored heavy is an anchor
    let t = |category, mass| {
        hook_target(Some(classify(ClassInput {
            dynamic: true,
            category: Some(category),
            mass: Some(mass),
            ..Default::default()
        })))
    };
    assert_eq!(t(1, 5000.0), HookTarget::Object(1));
    assert_eq!(t(2, 1.0), HookTarget::Object(2));
    assert_eq!(hook_target(None), HookTarget::Terrain);
}

#[test]
fn the_arrival_classes_use_the_device_angles() {
    assert!((cos_floor() - 0.6428).abs() < 1e-3 && (cos_ceiling() + 0.7660).abs() < 1e-3);
    assert_eq!(classify_arrival(1.0), ArrivalClass::Floor);
    assert_eq!(classify_arrival(cos_floor()), ArrivalClass::Floor);
    assert_eq!(classify_arrival(cos_floor() - 1e-4), ArrivalClass::Wall);
    assert_eq!(classify_arrival(0.0), ArrivalClass::Wall);
    assert_eq!(classify_arrival(cos_ceiling() + 1e-4), ArrivalClass::Wall);
    assert_eq!(classify_arrival(cos_ceiling()), ArrivalClass::Ceiling);
    assert_eq!(classify_arrival(-1.0), ArrivalClass::Ceiling);
    // the old 0.7 / -0.7 cut-offs called a 0.68 slope a wall and a -0.72 slope a wall
    assert_eq!(classify_arrival(0.68), ArrivalClass::Floor);
    assert_eq!(classify_arrival(-0.74), ArrivalClass::Wall);
}

#[test]
fn the_reel_start_blend_input() {
    assert_eq!(reel_start_blend(0.0), 0.0);
    assert!((reel_start_blend(REEL_START_ANGLE / 2.0) - 0.5).abs() < 1e-6);
    assert_eq!(reel_start_blend(3.0), 1.0);
    assert_eq!(reel_start_blend(-3.0), -1.0);
}

// ------------------------------------------------------------ the graph, state by state

/// Hook a floor from the ground: `S_REEL_START` (take-off) -> `S_REEL_FLIGHT` ->
/// `S_REEL_END_LAND`; the arm raises, waits, hits, holds, then drops back.
#[test]
fn trace_reel_from_the_ground_to_a_floor() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    assert!(a.fire());
    run(&mut a, 1.0, &dur);
    let at = a.attach(HookTarget::Terrain, false, BodyCtx::Ground);
    assert_eq!(at.choice, Choice::Reel);
    assert!(!at.air_pull);
    assert_eq!(a.body, BodyState::ReelStart);
    assert_eq!(a.arm, ArmState::Grapple);
    run(&mut a, 1.0, &dur);
    assert_eq!(a.body, BodyState::ReelFlight, "the take-off ends into the flight");
    assert!(a.actions.is(SetAction::DeployParachute), "flight enables the parachute action");
    assert!(!a.actions.is(SetAction::ReelIn), "reel start switches Reel off");
    assert!(a.arrive(ArrivalClass::Floor, BodyCtx::Air));
    assert_eq!(a.body, BodyState::ReelEndLand);
    assert!(!a.actions.is(SetAction::DeployParachute));
    a.detach();
    run(&mut a, 2.0, &dur);
    assert_eq!(a.body, BodyState::Free);
    assert_eq!(a.arm, ArmState::Idle);
    show("reel from the ground to a floor", &a);
    assert_eq!(
        states(&a, 0),
        ["S_REEL_START", "S_REEL_FLIGHT", "S_REEL_END_LAND", "(free)"]
    );
    assert_eq!(
        states(&a, 2),
        [
            "LA_S_RAISE_GRAPPLE",
            "LA_S_WAITING_FOR_HOOK",
            "LA_S_HOOK_HIT",
            "LA_S_GRAPPLE",
            "LA_S_GRAPPLE_TO_IDLE",
            "LA_S_IDLE"
        ]
    );
}

/// Hooked from the air, onto a wall: straight into `S_REEL_FLIGHT`, the vertical impact,
/// the reeled-in idle; `Space` bounces off (`S_REELED_IN_JUMP`).
#[test]
fn trace_reel_from_the_air_to_a_wall_and_jump() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    a.attach(HookTarget::Terrain, false, BodyCtx::Air);
    assert_eq!(a.body, BodyState::ReelFlight);
    run(&mut a, 0.5, &dur);
    assert!(a.arrive(ArrivalClass::Wall, BodyCtx::Air));
    assert_eq!(a.body, BodyState::ReelImpactVertical);
    assert!(a.actions.is(SetAction::ReeledInJump) && a.actions.is(SetAction::ReeledInRelease));
    run(&mut a, 3.0, &dur);
    assert_eq!(a.body, BodyState::ReeledInIdle, "the impact ends into the hang");
    assert!(a.reeled_in_jump(BodyCtx::Air));
    assert_eq!(a.body, BodyState::ReeledInJump);
    a.detach();
    run(&mut a, 3.0, &dur);
    assert_eq!(a.body, BodyState::Free);
    show("reel from the air to a wall, jump", &a);
    assert_eq!(
        states(&a, 0),
        [
            "S_REEL_FLIGHT",
            "S_REEL_IMPACT_VERTICAL",
            "S_REELED_IN_IDLE",
            "S_REELED_IN_JUMP",
            "(free)"
        ]
    );
}

/// The wall's `Ctrl`: `S_REELED_IN_RELEASE_DROP`. Pressed while the impact still plays it
/// cuts the impact short (inferred).
#[test]
fn trace_release_from_a_wall() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    a.attach(HookTarget::Object(2), false, BodyCtx::Air);
    a.arrive(ArrivalClass::Wall, BodyCtx::Air);
    // straight away, mid-impact
    assert!(a.reeled_in_release(BodyCtx::Air));
    assert_eq!(a.body, BodyState::ReeledInReleaseDrop);
    a.detach();
    assert_eq!(a.arm, ArmState::GrappleToIdle);
    run(&mut a, 3.0, &dur);
    assert_eq!(a.body, BodyState::Free);
    assert_eq!(a.arm, ArmState::Idle);
    show("release from a wall", &a);
    assert!(a.trace().contains("input during impact"));
}

/// A ceiling: `S_REEL_IMPACT_HORIZONTAL` -> `S_REELED_IN_TO_HANG` (0.9 speed) ->
/// `S_HANGSTUNT_IDLE`; `Ctrl`: `S_HANG_TO_FALL`.
#[test]
fn trace_reel_to_a_ceiling_hang_and_release() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    a.attach(HookTarget::Terrain, false, BodyCtx::Air);
    run(&mut a, 0.5, &dur);
    assert!(a.arrive(ArrivalClass::Ceiling, BodyCtx::Air));
    assert_eq!(a.body, BodyState::ReelImpactHorizontal);
    assert!(a.actions.is(SetAction::ReeledInRelease));
    assert!(!a.actions.is(SetAction::ReeledInJump), "a ceiling has no jump");
    run(&mut a, 6.0, &dur);
    assert_eq!(a.body, BodyState::HangStuntIdle);
    // the jump is not an action of a ceiling hang
    assert!(!a.reeled_in_jump(BodyCtx::Air));
    assert!(a.reeled_in_release(BodyCtx::Air));
    assert_eq!(a.body, BodyState::HangToFall);
    a.detach();
    run(&mut a, 3.0, &dur);
    assert_eq!(a.body, BodyState::Free);
    show("reel to a ceiling, hang, release", &a);
    assert_eq!(
        states(&a, 0),
        [
            "S_REEL_FLIGHT",
            "S_REEL_IMPACT_HORIZONTAL",
            "S_REELED_IN_TO_HANG",
            "S_HANGSTUNT_IDLE",
            "S_HANG_TO_FALL",
            "(free)"
        ]
    );
}

/// Pull on the ground (a soldier, a barrel): the left arm alone; the body is not touched.
#[test]
fn trace_pull_on_the_ground() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    let at = a.attach(HookTarget::Character, false, BodyCtx::Ground);
    assert_eq!(at.choice, Choice::Pull);
    assert!(!at.air_pull);
    assert_eq!(a.body, BodyState::Free, "no body action on the ground");
    assert_eq!(a.arm, ArmState::GrapplePull);
    assert!(a.actions.is(SetAction::PullGrappled));
    // the wire ends (the thing arrived): the detach is refused by the pull state, the clip
    // plays out
    a.detach();
    assert_eq!(a.arm, ArmState::GrapplePull);
    run(&mut a, 3.0, &dur);
    assert_eq!(a.arm, ArmState::Idle);
    show("pull on the ground", &a);
    assert!(states(&a, 0).is_empty());
    assert_eq!(
        states(&a, 2),
        [
            "LA_S_RAISE_GRAPPLE",
            "LA_S_WAITING_FOR_HOOK",
            "LA_S_HOOK_HIT",
            "LA_S_GRAPPLE_PULL",
            "LA_S_IDLE"
        ]
    );
}

/// Pull in the air: `ACT_GRAPPLE_PULL` on layer 0 (`S_PULL_IN_AIR_FLIGHT`, the reel-flight
/// clip), the arm goes idle; at the end `ACT_FALL`.
#[test]
fn trace_pull_in_the_air() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    let at = a.attach(HookTarget::Object(1), false, BodyCtx::Air);
    assert_eq!(at.choice, Choice::Pull);
    assert!(at.air_pull);
    assert_eq!(a.body, BodyState::PullInAirFlight);
    assert_eq!(a.body.clip(), BodyState::ReelFlight.clip(), "the same clip as the reel flight");
    // `LA_ACT_IDLE` is posted, but `LA_S_HOOK_HIT` has no edge for it (only `LA_S_GRAPPLE`
    // does): the arm keeps holding out at the hook until the detach (Gap: the order of the
    // arm's hit-finish and the pull start inside the original frame is not decoded)
    assert!(matches!(a.arm, ArmState::HookHit | ArmState::Grapple));
    run(&mut a, 1.0, &dur);
    a.end_air_pull(BodyCtx::Air);
    assert_eq!(a.body, BodyState::Free);
    show("pull in the air", &a);
    assert_eq!(states(&a, 0), ["S_PULL_IN_AIR_FLIGHT", "(free)"]);
    // swimming dives accept the pull too; a stunt position does not
    let mut s = GrappleAnim::default();
    assert!(s.pull_start(BodyCtx::Swim));
    let mut st = GrappleAnim::default();
    assert!(!st.pull_start(BodyCtx::Stunt));
}

/// A miss: raise, wait, `LA_ACT_HOOK_MISSED`, recoil, idle; the body never moves.
#[test]
fn trace_a_miss() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 1.0, &dur);
    a.miss();
    assert_eq!(a.arm, ArmState::HookMissed);
    run(&mut a, 2.0, &dur);
    assert_eq!(a.arm, ArmState::Idle);
    show("a miss", &a);
    assert_eq!(
        states(&a, 2),
        ["LA_S_RAISE_GRAPPLE", "LA_S_WAITING_FOR_HOOK", "LA_S_HOOK_MISSED", "LA_S_IDLE"]
    );
    assert!(states(&a, 0).is_empty());
}

/// `ACT_DETACH` mid-flight (the cancel): `S_REEL_FLIGHT` -> fall, the arm drops back.
#[test]
fn trace_detach_during_the_reel() {
    let dur = durations();
    for ctx in [BodyCtx::Ground, BodyCtx::Air] {
        let mut a = GrappleAnim::recording();
        a.fire();
        run(&mut a, 0.8, &dur);
        a.attach(HookTarget::Terrain, false, ctx);
        run(&mut a, 0.05, &dur);
        assert!(a.owns_body());
        a.detach();
        assert_eq!(a.body, BodyState::Free);
        assert_eq!(a.arm, ArmState::GrappleToIdle);
        assert_eq!(a.actions, ActionSet::default(), "the cancel restores the initial set");
        show(&format!("detach during the reel ({ctx:?})"), &a);
    }
}

/// The second hook: `LA_ACT_DUAL_GRAPPLE`, Reel and Pull off.
#[test]
fn trace_the_second_hook() {
    let dur = durations();
    let mut a = GrappleAnim::recording();
    a.fire();
    run(&mut a, 0.8, &dur);
    a.attach(HookTarget::Terrain, false, BodyCtx::Air);
    let two = a.attach(HookTarget::Terrain, true, BodyCtx::Air);
    assert_eq!(two.choice, Choice::Dual);
    assert_eq!(a.arm, ArmState::DualGrapple);
    run(&mut a, 2.0, &dur);
    assert_eq!(a.arm, ArmState::Idle);
    show("the second hook", &a);
}

/// A hit the device does nothing with (category 0): the hook holds, nothing starts.
#[test]
fn trace_an_object_with_no_usable_category() {
    let mut a = GrappleAnim::recording();
    a.fire();
    a.tick(1.0, &|_| 0.5);
    let at = a.attach(HookTarget::Object(0), false, BodyCtx::Air);
    assert_eq!(at.choice, Choice::Neither);
    assert_eq!(a.body, BodyState::Free);
    assert_eq!(a.arm, ArmState::HookHit);
}

/// The reel from a stunt position, from the sea and from a hang start in the flight.
#[test]
fn reel_starts_by_what_the_body_is_doing() {
    for (ctx, first) in [
        (BodyCtx::Ground, BodyState::ReelStart),
        (BodyCtx::Air, BodyState::ReelFlight),
        (BodyCtx::Swim, BodyState::ReelFlight),
        (BodyCtx::Stunt, BodyState::ReelFlight),
    ] {
        let mut a = GrappleAnim::default();
        assert!(a.reel_start(ctx));
        assert_eq!(a.body, first, "{ctx:?}");
    }
    // a vertical impact is also offered from the plain fall
    assert_eq!(
        body_accepts(BodyState::Free, BodyCtx::Air, BodyAction::ReelImpactVertical),
        Some(BodyState::ReelImpactVertical)
    );
    // nothing lands from the ground
    assert_eq!(body_accepts(BodyState::Free, BodyCtx::Ground, BodyAction::ReelLand), None);
}

/// The clip bound to each state (`kc001_base.asb` / `kc001_leftarm.asb`, probe report).
#[test]
fn the_states_play_the_native_clips() {
    let body = [
        (BodyState::ReelStart, "reel_in_start.ban", 0.1),
        (BodyState::ReelFlight, "grpl_reel_flight_part2.ban", 0.2),
        (BodyState::PullInAirFlight, "grpl_reel_flight_part2.ban", 0.2),
        (BodyState::ReelEndLand, "grpl_reel_hori_land.ban", 0.4),
        (BodyState::ReelImpactVertical, "grpl_reel_impact.ban", 0.07),
        (BodyState::ReeledInIdle, "grpl_reeled_timeblend.ban", 0.0),
        (BodyState::ReeledInJump, "grpl_reel_bounce.ban", 0.0),
        (BodyState::ReeledInReleaseDrop, "grpl_reeled_release.ban", 0.0),
        (BodyState::ReelImpactHorizontal, "reel_impact_horizontal.ban", 0.07),
        (BodyState::ReeledInToHang, "reel_impact_to_hang.ban", 0.0),
        (BodyState::HangStuntIdle, "heli_default_timeblend_hori.ban", 0.2),
        (BodyState::HangToFall, "heli_default_exit_to_fall.ban", 0.0),
    ];
    for (s, file, blend) in body {
        assert_eq!(s.clip().map(Clip::file), Some(file), "{}", s.name());
        assert!((s.blend() - blend).abs() < 1e-6, "{} blend", s.name());
    }
    assert_eq!(BodyState::ReeledInToHang.speed(), 0.9);
    let arm = [
        (ArmState::RaiseGrapple, "grpl_fire_ani_con.ban", 0.2),
        (ArmState::WaitingForHook, "grpl_fire_aim_con.ban", 0.0),
        (ArmState::HookHit, "grpl_fire_aim_con.ban", 0.0),
        (ArmState::Grapple, "grpl_fire_aim_con.ban", 0.0),
        (ArmState::GrapplePull, "grpl_idle_to_pull.ban", 0.14),
        (ArmState::GrappleDetach, "la_grpl_idle_recoil.ban", 0.2),
        (ArmState::HookMissed, "la_grpl_idle_recoil.ban", 0.2),
        (ArmState::GrappleToIdle, "la_grpl_idle_recoil.ban", 0.2),
        (ArmState::DualGrapple, "grpl_fire_dualhook_con.ban", 0.07),
    ];
    for (s, file, blend) in arm {
        assert_eq!(s.clip().map(Clip::file), Some(file), "{}", s.name());
        assert!((s.blend() - blend).abs() < 1e-6, "{} blend", s.name());
    }
    assert_eq!(VictimState::Grappled.clip().map(Clip::file), Some("stumble_grpl.ban"));
    assert_eq!(
        VictimState::RemoveGrapplingHook.clip().map(Clip::file),
        Some("stumble_grpl_release.ban")
    );
}

/// Every clip a grapple state names loads from the retail archives (needs JC2_TEST_DIR).
#[test]
fn the_grapple_clips_load_and_have_length() {
    let Some(rig) = std::env::var("JC2_TEST_DIR")
        .ok()
        .and_then(|d| GameFs::open(d).ok())
        .and_then(|fs| Rig::load(&fs, "biped.bsk"))
    else {
        return;
    };
    let all_body = [
        BodyState::ReelStart,
        BodyState::ReelFlight,
        BodyState::ReelEndLand,
        BodyState::ReelImpactVertical,
        BodyState::ReeledInIdle,
        BodyState::ReeledInJump,
        BodyState::ReeledInReleaseDrop,
        BodyState::ReelImpactHorizontal,
        BodyState::ReeledInToHang,
        BodyState::HangStuntIdle,
        BodyState::HangToFall,
    ];
    println!("clip lengths (s):");
    for s in all_body {
        let c = s.clip().unwrap();
        let d = rig.duration(c);
        println!("  {:<28} {:<34} {d:.3}", s.name(), c.file());
        assert!(d > 0.0, "{}", s.name());
    }
    for s in [
        ArmState::RaiseGrapple,
        ArmState::HookHit,
        ArmState::GrapplePull,
        ArmState::HookMissed,
        ArmState::DualGrapple,
    ] {
        let c = s.clip().unwrap();
        let d = rig.duration(c);
        let tracks = rig.clips[&c].anim.track_names.len();
        println!("  {:<28} {:<34} {d:.3}  ({tracks} tracks)", s.name(), c.file());
        assert!(d > 0.0, "{}", s.name());
    }
}

// ------------------------------------------------------------ the victim

#[test]
fn the_grappled_counter_and_the_npc_states() {
    let mut c = GrappledCount::default();
    assert!(c.not_grappled() && !c.atleast_once());
    c.hooked();
    assert!(c.atleast_once() && !c.not_grappled());
    for _ in 0..41 {
        c.hooked();
    }
    assert_eq!(c.0, 42);
    assert!(c.atleast_once());
    c.hooked();
    assert!(!c.atleast_once(), "43 is beyond the band");
    let mut z = GrappledCount(1);
    z.unhooked();
    z.unhooked();
    assert_eq!(z.0, 0, "never below zero");

    // S_AI_BASIC_NAVIGATION -T_HIT_BY_GRAPPLINGHOOK-> S_GRAPPLED -> S_REMOVE_GRAPPLINGHOOK -> idle
    let mut s = VictimState::Free;
    let mut c = GrappledCount::default();
    let mut log = vec![s.name()];
    let mut step = |s: &mut VictimState, c: GrappledCount, done: bool| {
        *s = s.update(c, done);
        if log.last() != Some(&s.name()) {
            log.push(s.name());
        }
    };
    step(&mut s, c, true);
    assert_eq!(s, VictimState::Free);
    c.hooked();
    step(&mut s, c, true);
    assert_eq!(s, VictimState::Grappled);
    assert!(s.dies_in_place(c), "a grappled victim dies in place (S_DEAD)");
    step(&mut s, c, true);
    assert_eq!(s, VictimState::Grappled);
    c.unhooked();
    step(&mut s, c, false);
    assert_eq!(s, VictimState::RemoveGrapplingHook);
    step(&mut s, c, false);
    assert_eq!(s, VictimState::RemoveGrapplingHook, "waits for the upper body");
    step(&mut s, c, true);
    assert_eq!(s, VictimState::Free);
    println!("--- victim\n{}\n", log.join(" -> "));
    assert_eq!(
        VictimState::Grappled.force_release(),
        VictimState::Free,
        "ACT_FORCE_RELEASE_GRAPPLINGHOOK"
    );
}
