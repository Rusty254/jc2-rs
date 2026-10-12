//! Opt-in smoke fixture (`JC2_EFFECT_FIXTURE=1`, or `=name1,name2` to choose the one-shot and the
//! looping effect): plays an explosion and a looping effect through the real lifecycle and logs the
//! per-component adapter state twice a second, then the diagnostic counters once both are over.
//! The default picks (`missile_explosion`; the first looping effect with a particle or light
//! component) are fixture content, not part of the spawner.

use super::system::{EffectController, EffectHandle, EffectSystem, PlayRequest};
use bevy::prelude::*;
use jc2_formats::effect::kind;

#[derive(Resource, Clone, Debug)]
pub struct FixtureConfig {
    pub one_shot: String,
    pub looping: Option<String>,
    /// `JC2_EFFECT_FIXTURE_DELAY` seconds of app time before the effects start (the world needs
    /// time to load before a screenshot); default 0.
    pub delay: f32,
    /// `JC2_EFFECT_FIXTURE_HOLD` seconds the looping effect runs before its graceful stop; default 6.
    pub hold: f32,
    /// `JC2_EFFECT_FIXTURE_DIST` metres in front of the camera; default 25.
    pub dist: f32,
    /// `JC2_EFFECT_FIXTURE_MOVE` metres per second the looping effect's source travels along +X
    /// (a moving smoke source for captures); default 0.
    pub move_speed: f32,
}

impl FixtureConfig {
    pub fn from_env(v: &str) -> Self {
        let mut it = v.split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "1");
        Self {
            one_shot: it.next().unwrap_or("missile_explosion").to_string(),
            looping: it.next().map(str::to_string),
            delay: env_f32("JC2_EFFECT_FIXTURE_DELAY", 0.0),
            hold: env_f32("JC2_EFFECT_FIXTURE_HOLD", 6.0),
            dist: env_f32("JC2_EFFECT_FIXTURE_DIST", 25.0),
            move_speed: env_f32("JC2_EFFECT_FIXTURE_MOVE", 0.0),
        }
    }
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[derive(Default)]
pub struct FixtureState {
    started: bool,
    done: bool,
    t: f32,
    last_print: f32,
    stopped: bool,
    one_shot: Option<EffectHandle>,
    looping: Option<EffectController>,
    looping_base: Vec3,
}

fn fmt_components(fx: &EffectSystem, e: &impl super::system::AsEffect) -> String {
    fx.components(e)
        .iter()
        .map(|c| {
            format!(
                "k{}:{}/{}{}{}",
                c.kind,
                c.uid,
                c.variation,
                if c.alive { " alive" } else { "" },
                if c.adapter { "" } else { " stub" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn run_fixture(
    time: Res<Time>,
    real: Res<Time<Real>>,
    cfg: Res<FixtureConfig>,
    fx: Option<ResMut<EffectSystem>>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut st: Local<FixtureState>,
) {
    let Some(mut fx) = fx else { return };
    if st.done {
        return;
    }
    let dt = time.delta_secs();
    // real time, so it lines up with `--exit-after` (also real time) however long the world took to load
    if !st.started && real.elapsed_secs() < cfg.delay {
        return;
    }
    if !st.started {
        st.started = true;
        let at = cams
            .iter()
            .next()
            .map_or(Vec3::ZERO, |c| c.translation() + c.forward().as_vec3() * cfg.dist);
        match fx.id_of(&cfg.one_shot) {
            Some(id) => {
                st.one_shot = fx.play(PlayRequest::new(id).at_point(at));
                info!("effect fixture: one-shot {} (id {id}) -> {:?}", cfg.one_shot, st.one_shot);
            }
            None => warn!("effect fixture: no effect named {}", cfg.one_shot),
        }
        let looping_id = match &cfg.looping {
            Some(n) => fx.id_of(n),
            None => (0..fx.slot_count()).find(|&i| {
                fx.description(i).is_some_and(|d| {
                    d.settings.looping
                        && d.components
                            .values()
                            .any(|c| c.kind == kind::PARTICLE || c.kind == kind::LIGHT)
                })
            }),
        };
        if let Some(id) = looping_id
            && let Some(c) = fx.create(id)
        {
            st.looping_base = at + Vec3::X * (cfg.dist * 0.25);
            fx.set_transform(&c, Mat4::from_translation(st.looping_base));
            fx.play_controller(&c, &[], false);
            info!("effect fixture: looping effect id {id}");
            st.looping = Some(c);
        }
    }
    st.t += dt;
    if cfg.move_speed != 0.0
        && !st.stopped
        && let Some(c) = st.looping.clone()
    {
        let at = st.looping_base + Vec3::X * (cfg.move_speed * st.t);
        fx.set_transform(&c, Mat4::from_translation(at));
    }
    if st.t - st.last_print >= 0.5 {
        st.last_print = st.t;
        if let Some(h) = st.one_shot {
            info!(
                "effect fixture t={:.1} one-shot state {:?} [{}]",
                st.t,
                fx.state(&h),
                fmt_components(&fx, &h)
            );
        }
        if let Some(c) = &st.looping {
            info!(
                "effect fixture t={:.1} looping state {:?} [{}]",
                st.t,
                fx.state(c),
                fmt_components(&fx, c)
            );
        }
    }
    if st.t > cfg.hold && !st.stopped {
        st.stopped = true;
        if let Some(c) = st.looping.clone() {
            fx.stop(&c, false);
            info!("effect fixture: looping effect stopped (graceful)");
        }
    }
    let one_over = st.one_shot.is_none_or(|h| fx.state(&h).is_none());
    let loop_over = st
        .looping
        .as_ref()
        .is_none_or(|c| fx.state(c).is_none_or(|s| s == 0));
    if st.stopped && one_over && loop_over {
        st.done = true;
        info!("effect fixture finished: {:?}", fx.diagnostics());
    }
}

/// Opt-in decal capture fixture (`JC2_DECAL_FIXTURE=<delay seconds>`): ray-casts down onto the
/// ground `JC2_DECAL_FIXTURE_DIST` (default 8) metres in front of the camera and plays a grenade
/// explosion scorch there plus a row of bullet-impact decals (hook_impact, surfaces 0..8) beside
/// it, aligned to the surface normal. Fixture content, not part of the spawner.
#[allow(clippy::too_many_arguments)]
pub fn run_decal_fixture(
    time: Res<Time>,
    fx: Option<ResMut<EffectSystem>>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    cam_view: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    spatial: avian3d::prelude::SpatialQuery,
    draw: Res<super::decal::DecalDraw>,
    mut done: Local<bool>,
    mut logged: Local<u64>,
) {
    use avian3d::prelude::SpatialQueryFilter;
    if draw.spawned != *logged {
        *logged = draw.spawned;
        info!("decal fixture: {} decals spawned, {} live", draw.spawned, draw.live.len());
    }
    let Some(mut fx) = fx else { return };
    let delay = env_f32("JC2_DECAL_FIXTURE", 0.0);
    if *done || time.elapsed_secs() < delay {
        return;
    }
    let Some(cam) = cams.iter().next() else { return };
    let dist = env_f32("JC2_DECAL_FIXTURE_DIST", 8.0);
    let forward = cam.forward().as_vec3();
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
    let right = flat.cross(Vec3::Y);
    let ground = |p: Vec3| {
        spatial
            .cast_ray(
                p + Vec3::Y * 60.0,
                Dir3::NEG_Y,
                120.0,
                true,
                &SpatialQueryFilter::default(),
            )
            .map(|h| (p + Vec3::Y * 60.0 - Vec3::Y * h.distance, h.normal))
    };
    let base = cam.translation() + flat * dist;
    let Some((p, n)) = ground(base) else { return };
    *done = true;
    let frame = |p: Vec3, n: Vec3| {
        Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, n.normalize_or(Vec3::Y)), p)
    };
    if let Some(id) = fx.id_of("grenade_explosion") {
        fx.play(PlayRequest::new(id).at(frame(p, n)));
    }
    if let Some(id) = fx.id_of("hook_impact") {
        for s in 0..8u32 {
            let q = base + right * (2.5 + 0.6 * s as f32) - flat * 2.0;
            if let Some((q, qn)) = ground(q) {
                fx.play(
                    PlayRequest::new(id)
                        .at(frame(q, qn))
                        .interaction(jc2_formats::effect::Interaction::surface(s)),
                );
            }
        }
    }
    let screen = cam_view.iter().next().and_then(|(c, t)| c.world_to_viewport(t, p).ok());
    info!("decal fixture: scorch and bullet marks at {p:?} normal {n:?} camera {:?} screen {screen:?}", cam.translation());
}
