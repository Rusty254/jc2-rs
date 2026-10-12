//! Lifecycle tests: the model fixtures of `research/tools/effects_pipeline_probe.py` as Rust
//! tests (one-shot with tail, looping + graceful stop, immediate stop, mask, all components fail to
//! start, material override), the cap, controllers, and the retail files.

use super::component::{AdapterSet, ComponentInit, EffectComponent};
use super::system::*;
use super::*;
use jc2_formats::effect::{
    ComponentDesc, EffectDescription, EffectSettings, InteractionMatrix, PairMap, Variation,
};
use jc2_formats::property::{Key, Node, Variant};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Log {
    starts: AtomicUsize,
    stops_graceful: AtomicUsize,
    stops_immediate: AtomicUsize,
}

struct Scripted {
    life: Option<f32>,
    fail: bool,
    age: Option<f32>,
    stopped: bool,
    enabled: bool,
    log: Arc<Log>,
}

impl EffectComponent for Scripted {
    fn start(&mut self) -> bool {
        if !self.enabled || self.fail {
            return false;
        }
        self.log.starts.fetch_add(1, Ordering::Relaxed);
        self.age = Some(0.0);
        self.stopped = false;
        true
    }
    fn stop(&mut self, immediate: bool) {
        self.stopped = true;
        let c = if immediate { &self.log.stops_immediate } else { &self.log.stops_graceful };
        c.fetch_add(1, Ordering::Relaxed);
    }
    fn is_alive(&self) -> bool {
        match self.age {
            Some(a) if !self.stopped => self.life.is_none_or(|l| a < l),
            _ => false,
        }
    }
    fn update(&mut self, dt: f32) {
        if let Some(a) = &mut self.age {
            *a += dt;
        }
    }
    fn set_transform(&mut self, _m: &Mat4) {}
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _k: u32, _v: f32) {}
    fn set_enabled(&mut self, e: bool) {
        self.enabled = e;
    }
}

fn adapters(log: &Arc<Log>) -> AdapterSet {
    let mut a = AdapterSet::stubs();
    for kind in 0..11 {
        let log = log.clone();
        a.register(kind, move |init: &ComponentInit| {
            let n = &init.variation.node;
            let life = n.float("life").filter(|l| *l >= 0.0);
            Box::new(Scripted {
                life,
                fail: n.int("fail") == Some(1),
                age: None,
                stopped: false,
                enabled: true,
                log: log.clone(),
            })
        });
    }
    a
}

/// `(uid, kind, life seconds or None = endless, start fails)`.
type Comp = (u32, u32, Option<f32>, bool);

fn var(uid: u32, life: Option<f32>, fail: bool) -> Variation {
    let mut node = Node::default();
    node.values.push((Key::Hash(0x6a9d_fd76), Variant::Int(uid as i32)));
    node.values.push((Key::Name("life".into()), Variant::Float(life.unwrap_or(-1.0))));
    node.values.push((Key::Name("fail".into()), Variant::Int(i32::from(fail))));
    Variation { uid, node }
}

fn desc(id: u32, tail: f32, looping: bool, comps: &[Comp]) -> Arc<EffectDescription> {
    let mut components = BTreeMap::new();
    let mut default = PairMap::new();
    for &(uid, kind, life, fail) in comps {
        let mut variations = BTreeMap::new();
        variations.insert(1, var(1, life, fail));
        // variation 2 (used by the surface override): twice the life
        variations.insert(2, var(2, life.map(|l| l * 2.0), fail));
        components.insert(
            uid,
            ComponentDesc {
                uid,
                role: 100 + uid,
                kind,
                variations,
            },
        );
        default.insert(100 + uid, Some((uid, 1)));
    }
    Arc::new(EffectDescription {
        id,
        settings: EffectSettings {
            tail_time: tail,
            looping,
            ..Default::default()
        },
        components,
        matrix: InteractionMatrix {
            default,
            flagged: None,
            materials: BTreeMap::from([(8, PairMap::from([(100, Some((0, 2)))]))]),
        },
        inputs: BTreeMap::new(),
    })
}

fn system(descs: Vec<Option<Arc<EffectDescription>>>, cap: u32) -> (EffectSystem, Arc<Log>) {
    let log = Arc::new(Log::default());
    let fx = EffectSystem::new(cap, Box::new(MemorySource(descs)), adapters(&log));
    (fx, log)
}

fn one(d: Arc<EffectDescription>) -> (EffectSystem, Arc<Log>) {
    system(vec![Some(d)], 255)
}

const DT: f32 = 0.1;

#[test]
fn one_shot_runs_for_the_component_then_the_tail_then_is_freed() {
    // the tail time is compared with the whole running time (elapsed since Begin, not since the
    // tail started): a component that dies at 0.2 s with tail 0.5 s keeps the effect to 0.5 s
    let (mut fx, log) = one(desc(0, 0.5, false, &[(0, 0, Some(0.2), false)]));
    let h = fx.play(PlayRequest::new(0)).unwrap();
    // a plain Play only queues: state 0 until the flush, 4 after it
    assert_eq!(fx.state(&h), Some(STATE_DEAD));
    fx.flush();
    assert_eq!(fx.state(&h), Some(STATE_PENDING));
    assert_eq!(fx.live(), 1);
    fx.update(DT); // Begin
    assert_eq!(fx.state(&h), Some(STATE_RUNNING));
    assert_eq!(log.starts.load(Ordering::Relaxed), 1);
    let mut t = 0.0;
    let mut first = [None; 4];
    for _ in 0..200 {
        fx.tick(DT);
        t += DT;
        match fx.state(&h) {
            Some(s) if (1..=3).contains(&s) && first[s as usize].is_none() => first[s as usize] = Some(t),
            Some(_) => {}
            None => break,
        }
    }
    let (tail_at, stop_at) = (first[2].unwrap(), first[3].unwrap());
    assert!((0.2..=0.45).contains(&tail_at), "tail entered at {tail_at}");
    assert!((0.5..=0.7).contains(&stop_at), "stopping from {stop_at}");
    assert_eq!(fx.state(&h), None, "freed");
    assert_eq!(fx.live(), 0);
    // stopping state: Stop(0) was sent to the component
    assert_eq!(log.stops_graceful.load(Ordering::Relaxed), 1);
}

#[test]
fn looping_effect_runs_until_graceful_stop() {
    let (mut fx, _log) = one(desc(0, 0.3, true, &[(0, 0, Some(0.2), false)]));
    let c = fx.create(0).unwrap();
    assert_eq!(fx.state(&c), Some(STATE_DEAD));
    assert!(fx.play_controller(&c, &[], false));
    for _ in 0..100 {
        fx.tick(DT);
    }
    // the component died long ago, the looping bit keeps the effect in state 1
    assert_eq!(fx.state(&c), Some(STATE_RUNNING));
    assert!(fx.components(&c).iter().all(|s| !s.alive));
    fx.stop(&c, false);
    // elapsed >= tail: straight to the stopping state, which ends when nothing is alive
    assert_eq!(fx.state(&c), Some(STATE_STOPPING));
    fx.tick(DT);
    assert_eq!(fx.state(&c), Some(STATE_DEAD));
    assert_eq!(fx.live(), 0);
    // a controller stays valid in state 0 and can be restarted
    assert!(fx.play_controller(&c, &[], false));
    fx.tick(DT);
    assert_eq!(fx.state(&c), Some(STATE_RUNNING));
    assert_eq!(fx.live(), 1);
}

#[test]
fn graceful_stop_before_the_tail_time_goes_through_the_tail_state() {
    let (mut fx, _log) = one(desc(0, 5.0, true, &[(0, 0, None, false)]));
    let c = fx.create(0).unwrap();
    fx.play_controller(&c, &[], false);
    fx.tick(DT);
    fx.tick(DT);
    fx.stop(&c, false);
    assert_eq!(fx.state(&c), Some(STATE_TAIL));
    let mut t = 0.0;
    while fx.state(&c) == Some(STATE_TAIL) {
        fx.tick(DT);
        t += DT;
        assert!(t < 10.0);
    }
    assert!(t >= 4.5, "left the tail after {t}");
    assert_eq!(fx.state(&c), Some(STATE_STOPPING));
}

#[test]
fn immediate_stop_kills_at_once() {
    let (mut fx, log) = one(desc(0, 5.0, true, &[(0, 0, None, false)]));
    let h = fx.play(PlayRequest::new(0)).unwrap();
    fx.tick(DT);
    fx.tick(DT);
    assert_eq!(fx.live(), 1);
    fx.stop(&h, true);
    assert_eq!(fx.state(&h), Some(STATE_DEAD));
    assert_eq!(fx.live(), 0);
    assert_eq!(log.stops_immediate.load(Ordering::Relaxed), 1);
    // fire and forget: freed on the next update
    fx.tick(DT);
    assert_eq!(fx.state(&h), None);
}

#[test]
fn component_mask_disables_by_kind() {
    // kinds 0 and 7; mask bit 0 disables kind 0
    let d = desc(0, 0.1, false, &[(0, 0, Some(1.0), false), (1, 7, Some(1.0), false)]);
    let (mut fx, log) = one(d.clone());
    let h = fx.play(PlayRequest::new(0).mask(1 << 0)).unwrap();
    fx.tick(DT);
    assert_eq!(fx.state(&h), Some(STATE_RUNNING));
    assert_eq!(log.starts.load(Ordering::Relaxed), 1, "only the light started");
    // everything masked: no component starts, the effect dies at once
    let (mut fx, _) = one(d);
    let h = fx.play(PlayRequest::new(0).mask((1 << 0) | (1 << 7))).unwrap();
    fx.tick(DT);
    assert_eq!(fx.state(&h), Some(STATE_DEAD));
    assert_eq!(fx.live(), 0);
    fx.tick(DT);
    assert_eq!(fx.state(&h), None);
}

#[test]
fn effect_dies_at_once_when_no_component_starts() {
    let (mut fx, _) = one(desc(0, 0.5, false, &[(0, 0, Some(1.0), true), (1, 1, Some(1.0), true)]));
    let h = fx.play(PlayRequest::new(0)).unwrap();
    fx.tick(DT);
    assert_eq!(fx.state(&h), Some(STATE_DEAD));
    assert_eq!(fx.live(), 0);
    fx.tick(DT);
    assert_eq!(fx.state(&h), None);
    // an effect with no resolved component at all behaves the same
    let (mut fx, _) = one(desc(0, 0.5, false, &[]));
    let h = fx.play(PlayRequest::new(0)).unwrap();
    fx.tick(DT);
    assert_eq!(fx.state(&h), Some(STATE_DEAD));
}

#[test]
fn surface_interaction_chooses_the_variation_once() {
    let d = desc(0, 0.1, false, &[(0, 0, Some(1.0), false)]);
    let (mut fx, _) = one(d);
    let plain = fx.play(PlayRequest::new(0)).unwrap();
    let metal = fx.play(PlayRequest::new(0).interaction(Interaction::surface(8))).unwrap();
    // interactions begin at once (0x006cd100), the plain Play waits for the flush
    assert_eq!(fx.state(&metal), Some(STATE_RUNNING));
    assert_eq!(fx.components(&metal)[0].variation, 2);
    fx.tick(DT);
    assert_eq!(fx.components(&plain)[0].variation, 1);
    assert_eq!(fx.components(&metal)[0].variation, 2);
}

#[test]
fn live_cap_rejects_plays_when_the_counter_is_above_it() {
    let (mut fx, _) = system(vec![Some(desc(0, 0.1, true, &[(0, 0, None, false)]))], 2);
    // the counter moves at the flush: plays before it are all accepted
    let handles: Vec<_> = (0..3).map(|_| fx.play(PlayRequest::new(0))).collect();
    assert!(handles.iter().all(Option::is_some));
    fx.flush();
    assert_eq!(fx.live(), 3);
    assert!(fx.play(PlayRequest::new(0)).is_none());
    assert_eq!(fx.diagnostics().rejected_cap, 1);
    // `ja`: exactly at the cap still passes
    fx.stop(&handles[0].unwrap(), true);
    assert_eq!(fx.live(), 2);
    assert!(fx.play(PlayRequest::new(0)).is_some());
}

#[test]
fn unknown_and_file_less_ids_return_none() {
    let (mut fx, _) = system(vec![None, Some(desc(1, 0.1, false, &[(0, 0, Some(1.0), false)]))], 255);
    assert!(fx.play(PlayRequest::new(0)).is_none(), "slot without a file");
    assert!(fx.play(PlayRequest::new(7)).is_none(), "id past the slot count");
    assert!(fx.create(0).is_none());
    assert_eq!(fx.diagnostics().rejected_unknown, 3);
    assert!(fx.play(PlayRequest::new(1)).is_some());
}

#[test]
fn controller_play_restart_and_drop() {
    let (mut fx, log) = one(desc(0, 0.2, true, &[(0, 0, None, false)]));
    let c = fx.create(0).unwrap();
    assert!(fx.play_controller(&c, &[], false));
    fx.tick(DT);
    assert_eq!(fx.state(&c), Some(STATE_RUNNING));
    // running without restart: nothing happens
    assert!(!fx.play_controller(&c, &[], false));
    assert_eq!(log.starts.load(Ordering::Relaxed), 1);
    // restart: Start again on the existing component, back in state 1 with the clock cleared
    assert!(fx.play_controller(&c, &[], true));
    assert_eq!(log.starts.load(Ordering::Relaxed), 2);
    assert_eq!(fx.state(&c), Some(STATE_RUNNING));
    assert_eq!(fx.elapsed(&c), Some(0.0));
    assert_eq!(fx.live(), 1);
    // dropping the handle frees the effect on the next update and releases the counter
    let probe = c.clone();
    drop(c);
    fx.tick(DT);
    assert_eq!(fx.state(&probe), Some(STATE_RUNNING), "a clone keeps it");
    drop(probe);
    fx.tick(DT);
    assert_eq!(fx.live(), 0);
}

#[test]
fn description_timer_counts_from_the_last_begin() {
    let (mut fx, _) = one(desc(0, 0.1, true, &[(0, 0, None, false)]));
    for _ in 0..10 {
        fx.tick(DT);
    }
    assert!((fx.since_start(0) - 1.0).abs() < 1e-4, "never begun: since the start");
    fx.play(PlayRequest::new(0));
    fx.tick(DT);
    fx.tick(DT);
    fx.tick(DT);
    // Begin ran in the first of the three updates and stored 0 before it advanced further
    assert!((fx.since_start(0) - 0.2).abs() < 1e-4, "{}", fx.since_start(0));
}

#[test]
fn unimplemented_adapters_are_counted_never_silent() {
    let host = EffectHost::default();
    let d = desc(0, 0.1, false, &[(0, 10, None, false), (1, 0, None, false), (2, 2, None, false)]);
    let mut fx = EffectSystem::new(255, Box::new(MemorySource(vec![Some(d)])), default_adapters(&host));
    let h = fx.play(PlayRequest::new(0)).unwrap();
    fx.tick(DT);
    let d = fx.diagnostics();
    assert_eq!((d.created[10], d.created[0]), (1, 1));
    // the area damage class (10) is still a stub; the sound has an adapter (this fixture has no
    // bank id, so it does not start, and is not a stub start either)
    assert_eq!((d.stub_started[10], d.stub_started[0]), (1, 0));
    // kind 2 has no instance class natively
    assert_eq!(d.no_instance_class[2], 1);
    // the stubs follow the one-shot contract: never alive, so the effect ends after the tail
    assert!(fx.components(&h).iter().all(|c| !c.alive && c.adapter == (c.kind != 10)));
    for _ in 0..5 {
        fx.tick(DT);
    }
    assert_eq!(fx.state(&h), None);
}

/// The retail files through the real adapters (needs JC2_TEST_DIR).
#[test]
fn retail_effects_run_through_the_lifecycle() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    assert_eq!(fx.slot_count(), 468);
    assert_eq!(fx.diagnostics().cap, 255);
    // every id: plays (8 file-less ids refuse), runs, and the looping ones are stopped
    let mut refused = 0;
    let mut light_spawns = 0;
    let mut controllers = Vec::new();
    let mut handles = Vec::new();
    for id in 0..468 {
        let Some(d) = fx.description(id) else {
            assert!(fx.play(PlayRequest::new(id)).is_none());
            refused += 1;
            continue;
        };
        if d.settings.looping {
            // the cap may refuse while earlier tails are still running
            let Some(c) = fx.create(id) else { continue };
            fx.play_controller(&c, &[], false);
            controllers.push(c);
        } else {
            handles.extend(fx.play(PlayRequest::new(id).at_point(Vec3::new(1.0, 2.0, 3.0))));
        }
        // keep below the 255 cap while running everything
        for _ in 0..3 {
            fx.tick(DT);
        }
        light_spawns += host
            .lights
            .drain()
            .iter()
            .filter(|o| matches!(o, light::LightOp::Spawn { .. }))
            .count();
    }
    assert_eq!(refused, 8);
    assert!(light_spawns > 0, "some effect started a light");
    for c in &controllers {
        fx.stop(c, false);
    }
    // 18 retail effects (grenade_fire, km03_fire, ...) are not looping themselves but carry a
    // looping light: natively they stay alive until their owner calls Stop, so stop them too
    for h in &handles {
        fx.stop(h, false);
    }
    for _ in 0..4000 {
        fx.tick(DT);
        host.particles.lock().unwrap().update_detached(DT);
    }
    drop(controllers);
    fx.tick(DT);
    // dropped components leave their particles to the manager (the plugin steps them)
    for _ in 0..4000 {
        host.particles.lock().unwrap().update_detached(DT);
    }
    let d = fx.diagnostics();
    assert_eq!(d.live, 0, "{d:?} {:?}", fx.active());
    assert_eq!((d.strong, d.weak), (0, 0));
    assert_eq!(host.particles.lock().unwrap().live(), 0);
    assert_eq!(d.stub_started[0], 0, "sound is an adapter now");
    assert_eq!(d.stub_started[9], 0, "post effect is an adapter now");
    assert!(host.sounds.post_started.load(std::sync::atomic::Ordering::Relaxed) > 0);
    let sound_ops = host.sounds.drain();
    assert!(sound_ops.iter().any(|o| matches!(o, sound::SoundOp::Start { .. })));
    assert!(host.particles.lock().unwrap().stats.refused_unresolved_name > 0);
    let _ = host.lights.drain();
}

/// The explosion and a looping effect of the opt-in fixture end by themselves.
#[test]
fn retail_explosion_and_looping_fixture_cycle() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    let id = fx.id_of("missile_explosion").expect("missile_explosion");
    let h = fx.play(PlayRequest::new(id).at_point(Vec3::ZERO)).unwrap();
    fx.tick(DT);
    let comps = fx.components(&h);
    assert!(comps.iter().any(|c| c.kind == 7 && c.adapter), "{comps:?}");
    let mut ticks = 0;
    while fx.state(&h).is_some() {
        fx.tick(DT);
        ticks += 1;
        assert!(ticks < 2000, "never ended");
    }
    assert_eq!(fx.live(), 0);
}

/// A sound component starts the `(bank, sound)` pair of its variation; the spawner's matrix has
/// already picked the variation of the surface (`hook_impact`, 212: bank 26, sounds 3..7).
#[test]
fn retail_sound_pair_follows_the_surface() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    let mut pairs = std::collections::BTreeSet::new();
    let play = |fx: &mut EffectSystem, req: PlayRequest| {
        fx.play(req.at_point(Vec3::new(4.0, 5.0, 6.0)));
        fx.tick(DT);
        let mut got = Vec::new();
        for op in host.sounds.drain() {
            if let sound::SoundOp::Start { bank, sound, at, .. } = op {
                assert_eq!(at, Vec3::new(4.0, 5.0, 6.0), "the effect's point reaches the sound");
                got.push((bank, sound));
            }
        }
        got
    };
    for surface in [0, 1, 4, 6, 8, 9] {
        let got = play(&mut fx, PlayRequest::new(212).interaction(Interaction::surface(surface)));
        assert_eq!(got.len(), 1, "surface {surface}: {got:?}");
        pairs.extend(got);
    }
    pairs.extend(play(&mut fx, PlayRequest::new(212).interaction(Interaction::WATER)));
    assert!(pairs.len() >= 3, "surfaces give different sounds: {pairs:?}");
    // flying_hook (11): one sound, bank 26 sound 0 (grapple-tether-visuals 1.2)
    assert_eq!(play(&mut fx, PlayRequest::new(11)), vec![(26, 0)]);
}

/// Post effects are parsed from the retail fields (377: attack 0.5, gain 0.35) and queued.
#[test]
fn retail_post_effect_params() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    fx.play(PlayRequest::new(377).at_point(Vec3::ZERO));
    fx.tick(DT);
    let post: Vec<_> = host
        .sounds
        .drain()
        .into_iter()
        .filter_map(|o| match o {
            sound::SoundOp::Post { params, .. } => Some(params),
            _ => None,
        })
        .collect();
    assert_eq!(post.len(), 1);
    assert_eq!(
        (post[0].attack, post[0].release, post[0].gain),
        (0.01, 0.5, 0.35)
    );
}

/// FX-SMOKE-1 (484554ae): effect 150 (vehicle damage smoke) and 126 (fuel depot explosion) through
/// the real lifecycle: the emitter tree, the counts, and a stop that never removes particles.
#[test]
fn retail_smoke_effects_follow_the_native_emitter_lifecycle() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    let counts = |host: &EffectHost| {
        let g = host.particles.lock().unwrap();
        (g.live(), g.emitter_count(), g.particle_count())
    };

    // 150: persistent controller, unbounded; heat-haze head + two smoke leaves, one emitter each
    let c = fx.create(150).expect("effect 150");
    fx.play_controller(&c, &[], false);
    for _ in 0..240 {
        fx.tick(DT);
    }
    let (live, emitters, particles) = counts(&host);
    eprintln!("150 running: {live} {emitters} {particles}");
    assert_eq!((live, emitters), (3, 3), "head + _02 + _04");
    assert!(particles > 10);
    assert!(fx.live() >= 1, "unbounded smoke keeps the controller alive");
    // Stop(immediate): births stop, particles stay (the plugin runs the detached emitters out)
    fx.stop(&c, true);
    drop(c);
    fx.tick(DT);
    let (_, _, after) = counts(&host);
    assert!(after > 0, "an immediate stop does not kill live particles");
    for _ in 0..600 {
        host.particles.lock().unwrap().update_detached(DT);
    }
    assert_eq!(counts(&host), (0, 0, 0));

    // 126: the fuel depot explosion: a head with linked fire systems (one with a trail)
    let h = fx
        .play(PlayRequest::new(126).at_point(Vec3::new(0.0, 1.0, 0.0)))
        .unwrap();
    for _ in 0..30 {
        fx.tick(DT);
    }
    let (live, emitters, particles) = counts(&host);
    eprintln!("126 running: {live} {emitters} {particles}");
    assert!(live >= 7 && emitters >= live, "{live} {emitters}");
    assert!(particles > 0);
    fx.stop(&h, false);
    for _ in 0..4000 {
        fx.tick(DT);
        host.particles.lock().unwrap().update_detached(DT);
    }
    let _ = h;
    for _ in 0..1000 {
        host.particles.lock().unwrap().update_detached(DT);
    }
    assert_eq!(counts(&host), (0, 0, 0));
    let _ = host.lights.drain();
}

/// Decal params from the retail decals: the atlas cells, size, lifetime, and a decal leaves the
/// effect through the queue exactly once, with the effect matrix.
#[test]
fn retail_decal_params_and_queue() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    let mut fx = EffectSystem::from_retail(fs.clone(), default_adapters(&host)).unwrap();
    // grenade explosion (35): scorch, lifetime 100, full texture
    fx.play(PlayRequest::new(35).at(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))));
    fx.tick(DT);
    let ops = host.decals.drain();
    assert_eq!(ops.len(), 1, "{ops:?}");
    let p = &ops[0].params;
    assert_eq!(p.diffuse, "Decals/Explosion_DIF.dds");
    assert_eq!((p.lifetime, p.size), (100.0, Vec4::new(4.1, 4.3, 4.3, 4.1)));
    assert_eq!((p.uv_min, p.uv_max), (Vec2::ZERO, Vec2::ONE));
    assert_eq!(ops[0].matrix.w_axis.truncate(), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(fx.diagnostics().stub_started[3], 0);
    // the texture exists in the archive under the asset path the renderer uses
    assert!(fs.contains(p.diffuse_asset().trim_start_matches("jc2://")));
    // hook impact (212): per surface cells of the BulletImpacts sheet (4 x 2)
    let mut cells = std::collections::BTreeSet::new();
    for surface in 0..16 {
        fx.play(PlayRequest::new(212).interaction(Interaction::surface(surface)));
        fx.tick(DT);
        for op in host.decals.drain() {
            let p = op.params;
            assert_eq!(p.diffuse, "Decals/BulletImpacts_DIF.dds");
            assert_eq!(p.lifetime, 10.0);
            cells.insert((p.uv_min.to_array().map(f32::to_bits), p.uv_max.to_array().map(f32::to_bits)));
        }
    }
    assert!(cells.len() >= 2, "different surfaces use different cells: {cells:?}");
}

/// Every retail decal variation: min / max uv corners form a valid cell inside the sheet.
#[test]
fn retail_decal_cells_are_valid_corners() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let reg = jc2_formats::effect::EffectRegistry::load(&fs).unwrap();
    let mut n = 0;
    for id in 0..reg.slot_count() {
        let Some(d) = reg.load_description(&fs, id) else { continue };
        for c in d.components.values().filter(|c| c.kind == jc2_formats::effect::kind::DECAL) {
            for v in c.variations.values() {
                let p = decal::DecalParams::from_node(&v.node);
                assert!(!p.diffuse.is_empty() && p.lifetime > 0.0, "effect {id}: {p:?}");
                assert!(p.uv_min.cmpge(Vec2::ZERO).all(), "effect {id}: {p:?}");
                assert!(p.uv_max.cmple(Vec2::ONE).all(), "effect {id}: {p:?}");
                assert!(p.uv_max.cmpgt(p.uv_min).all(), "effect {id}: {p:?}");
                assert!(p.size.x > 0.0 && p.size.y > 0.0, "effect {id}: {p:?}");
                n += 1;
            }
        }
    }
    assert_eq!(n, 95, "95 decal variations in retail data");
}

#[test]
fn decal_transform_puts_the_effect_y_axis_on_the_decal_normal() {
    let n = Vec3::new(1.0, 1.0, 0.0).normalize();
    let m = Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, n), Vec3::new(1.0, 2.0, 3.0));
    let t = decal::decal_transform(&m, Vec3::new(2.0, 0.3, 4.0));
    assert!((t.rotation * Vec3::Y - n).length() < 1e-5);
    assert_eq!((t.translation, t.scale), (Vec3::new(1.0, 2.0, 3.0), Vec3::new(2.0, 0.3, 4.0)));
    // a degenerate matrix keeps only the position
    let degenerate = Mat4::from_cols(Vec4::ZERO, Vec4::ZERO, Vec4::ZERO, Vec4::new(1.0, 1.0, 1.0, 1.0));
    let t = decal::decal_transform(&degenerate, Vec3::ONE);
    assert_eq!((t.translation, t.rotation), (Vec3::ONE, Quat::IDENTITY));
}

/// The native camera coupling of a pulse (explosion-effects.md section 2): 0.05 inside 10 m,
/// 1/distance beyond, nothing past 3 x the largest extent.
#[test]
fn pulse_camera_shake_follows_the_native_rule() {
    let e = Vec3::splat(5.0);
    let (i, speed, decay) = pulse::pulse_shake(e, 4.0).unwrap();
    assert!((i - 0.05).abs() < 1e-6 && (speed, decay) == (1.5, -0.1));
    assert!((pulse::pulse_shake(e, 10.0).unwrap().0 - 0.05).abs() < 1e-6);
    assert!((pulse::pulse_shake(e, 14.0).unwrap().0 - 0.25 / 1.4 * 0.2).abs() < 1e-6);
    assert!(pulse::pulse_shake(e, 15.0).is_none(), "camera must be closer than 3 x extent");
    assert!(pulse::pulse_shake(Vec3::new(1.0, 7.0, 2.0), 20.0).is_some(), "largest extent");
    assert!((pulse::pulse_rumble(500.0, 0.5) - 10.0).abs() < 1e-5);
    assert!((pulse::pulse_rumble(500.0, 20.0) - 0.5).abs() < 1e-5);
}

#[test]
fn camera_shake_start_attenuates_and_the_state_keeps_the_maximum() {
    let p = pulse::ShakeParams { scale: 1.0, gain: 0.04, ..pulse::ShakeParams::DEFAULT };
    assert!((pulse::shake_intensity(&p, 5.0) - 0.04).abs() < 1e-6, "inside 10 m: no attenuation");
    assert!((pulse::shake_intensity(&p, 50.0) - 0.04 / 5.0).abs() < 1e-6);
    let flat = pulse::ShakeParams { attenuate: false, ..p };
    assert!((pulse::shake_intensity(&flat, 500.0) - 0.04).abs() < 1e-6);
    let mut s = pulse::CameraShakeState::default();
    s.set(0.05, 1.5, -0.1);
    s.set(0.01, 5.0, -0.6);
    assert_eq!((s.intensity, s.speed, s.decay), (0.05, 5.0, -0.6));
}

/// Retail field decode: the shake / pulse / haptics numbers of the effects the research lists
/// (explosion-effects.md sections 2 and 3), and the start through the adapters.
#[test]
fn retail_pulse_shake_haptics_fields() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    // cannon impact (377): pulse A 500, C 8000, extents 5
    fx.play(PlayRequest::new(377).at_point(Vec3::new(9.0, 0.0, 0.0)));
    fx.tick(DT);
    let ops = host.pulses.drain();
    assert_eq!(ops.len(), 1, "{ops:?}");
    let pulse::PulseOp::Pulse { at, params } = ops[0] else { panic!("{ops:?}") };
    assert_eq!(at, Vec3::new(9.0, 0.0, 0.0));
    assert_eq!((params.a, params.c, params.extents), (500.0, 8000.0, Vec3::splat(5.0)));
    // rumble_and_shake_large (207): shake scale 1, speed 5, decay -0.6, gain 0.08, haptics 1.0
    fx.play(PlayRequest::new(207).at_point(Vec3::ZERO));
    fx.tick(DT);
    let ops = host.pulses.drain();
    let shake: Vec<_> = ops
        .iter()
        .filter_map(|o| match o {
            pulse::PulseOp::Shake { params, .. } => Some(*params),
            _ => None,
        })
        .collect();
    assert_eq!(shake.len(), 1, "{ops:?}");
    assert_eq!((shake[0].scale, shake[0].speed, shake[0].decay, shake[0].gain), (1.0, 5.0, -0.6, 0.08));
    assert!(!shake[0].attenuate, "retail files set the #73440d3e flag to 0");
    let hap: Vec<_> = ops
        .iter()
        .filter_map(|o| match o {
            pulse::PulseOp::Haptics { params } => Some(*params),
            _ => None,
        })
        .collect();
    assert_eq!(hap.len(), 1);
    assert_eq!((hap[0].pair_a, hap[0].pair_b), ([1.0, 1.0], [0.2, 0.2]));
    let q = &host.pulses;
    assert_eq!(
        (
            q.pulses_started.load(Ordering::Relaxed),
            q.shakes_started.load(Ordering::Relaxed),
            q.haptics_started.load(Ordering::Relaxed)
        ),
        (1, 1, 1)
    );
    let d = fx.diagnostics();
    assert_eq!((d.stub_started[3], d.stub_started[4], d.stub_started[5], d.stub_started[8]), (0, 0, 0, 0));
}

/// The one-shot classes never hold the effect open and the mask switches them off.
#[test]
fn one_shot_classes_are_never_alive_and_respect_the_mask() {
    let Ok(dir) = std::env::var("JC2_TEST_DIR") else { return };
    let fs = Arc::new(jc2_formats::GameFs::open(dir).unwrap());
    let host = EffectHost::default();
    let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
    let h = fx.play(PlayRequest::new(207).at_point(Vec3::ZERO).mask(1 << 8)).unwrap();
    fx.tick(DT);
    assert!(fx.components(&h).iter().all(|c| !c.alive));
    let ops = host.pulses.drain();
    assert!(ops.iter().all(|o| !matches!(o, pulse::PulseOp::Shake { .. })), "camera shake masked off");
    assert!(ops.iter().any(|o| matches!(o, pulse::PulseOp::Haptics { .. })));
}
