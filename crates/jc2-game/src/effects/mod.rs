//! The runtime effect spawner (research: `research/formats/effects-pipeline.md`; board task
//! e268da82). Callers pass an effect id; everything else comes from `effects/effectsettings.bin` and
//! the effect files (`jc2_formats::effect`).
//!
//! # API
//!
//! ```ignore
//! fn blast(mut fx: ResMut<effects::EffectSystem>) {
//!     // fire and forget (native Play 0x00579f50 / 0x006cd100); None when the native Play returns 0
//!     let id = 425; // an id the caller owns (weapon / vehicle record)
//!     fx.play(effects::PlayRequest::new(id).at_point(pos).velocity(v)
//!         .interaction(effects::Interaction::surface(8)));   // metal; Interaction::WATER for water
//!
//!     // persistent effect held by an owner (engine smoke, muzzle): create once, play per use
//!     let c = fx.create(id).unwrap();
//!     fx.play_controller(&c, &[], false);
//!     fx.set_transform(&c, matrix);          // every frame while it follows its owner
//!     fx.stop(&c, false);                    // graceful; true = immediate
//! }
//! ```
//!
//! Components are created at `Begin` from the interaction matrix; the adapters are registered per
//! component kind ([`default_adapters`]). Implemented: lights (kind 7, onto the existing
//! `EffectLight`) and particle emitters (kind 1: manager rules, emitter simulation and a billboard
//! renderer, see [`particle`], [`particle_sim`], [`particle_render`]), sounds (kind 0, onto the
//! project audio, see [`sound`]), sound post effects (kind 9, parsed and queued, DSP not applied),
//! decals (kind 3, forward decals, see [`decal`]), force pulses (kind 4: camera shake and rumble
//! coupling only, no impulse), camera shake (kind 8) and haptics (kind 5, counted, no device on PC;
//! see [`pulse`]). Everything else is a native-contract stub ([`component::StubComponent`]); each
//! stub start is counted in [`Diagnostics::stub_started`], never silent.
//!
//! The system is ticked in `Update` (flush, then update; doc 2.2) by [`EffectsPlugin`]. The
//! `max_instances` / throttle fields are parsed but unused (no native reader found, doc gap 1).

// The API is consumed by later per-system tasks (muzzle, impacts, explosions, vehicles, smoke);
// until they call it, the bin crate sees it as unused.
#![allow(dead_code)]

pub mod camera_shake;
pub mod component;
pub mod decal;
pub mod fixture;
pub mod light;
pub mod particle;
pub mod particle_render;
pub mod particle_sim;
pub mod pulse;
pub mod sound;
pub mod system;

#[allow(unused_imports)] // the public API for the per-system tasks
pub use component::{AdapterSet, EffectComponent};
#[allow(unused_imports)]
pub use jc2_formats::effect::Interaction;
#[allow(unused_imports)]
pub use system::{
    AsEffect, ComponentState, Diagnostics, EffectController, EffectHandle, EffectSystem,
    PlayRequest,
};

use bevy::prelude::*;
use jc2_formats::effect::kind;
use light::{HostQueue, LightComponent, LightEntities};
use decal::{DecalDraw, DecalQueue};
use particle::{ParticleComponent, ParticleManager, SharedParticles};
use pulse::{CameraShakeState, PulseQueue, PulseStats, RumbleState};
use sound::{PostEffectComponent, SoundComponent, SoundQueue};
use std::sync::{Arc, Mutex};

/// The shared state the adapters talk to the ECS and the particle manager through.
#[derive(Resource, Clone)]
pub struct EffectHost {
    pub lights: Arc<HostQueue>,
    pub particles: SharedParticles,
    /// The outbox of the sound components, drained by the audio plugin (`audio::effect_sounds`).
    pub sounds: Arc<SoundQueue>,
    /// The outbox of the decal components, drained by [`decal::apply_decal_ops`].
    pub decals: Arc<DecalQueue>,
    /// The outbox of the force pulse, camera shake and haptics components, drained by
    /// [`pulse::apply_pulse_ops`].
    pub pulses: Arc<PulseQueue>,
}

impl Default for EffectHost {
    fn default() -> Self {
        Self {
            lights: Arc::new(HostQueue::default()),
            particles: Arc::new(Mutex::new(ParticleManager::new(None))),
            sounds: Arc::new(SoundQueue::default()),
            decals: Arc::new(DecalQueue::default()),
            pulses: Arc::new(PulseQueue::default()),
        }
    }
}

/// Light, particle, sound and sound post effect adapters over `host`; every other kind stays a
/// counted stub.
pub fn default_adapters(host: &EffectHost) -> AdapterSet {
    let mut a = AdapterSet::stubs();
    let lights = host.lights.clone();
    a.register(kind::LIGHT, move |init| {
        LightComponent::from_init(init, lights.clone())
    });
    let particles = host.particles.clone();
    a.register(kind::PARTICLE, move |init| {
        ParticleComponent::from_init(init, particles.clone())
    });
    let queue = host.sounds.clone();
    a.register(kind::SOUND, move |init| SoundComponent::from_init(init, queue.clone()));
    let queue = host.sounds.clone();
    a.register(kind::SOUND_POST, move |init| {
        PostEffectComponent::from_init(init, queue.clone())
    });
    let queue = host.decals.clone();
    a.register(kind::DECAL, move |init| {
        decal::DecalComponent::from_init(init, queue.clone())
    });
    let queue = host.pulses.clone();
    a.register(kind::FORCE_PULSE, move |init| {
        pulse::OneShot::force_pulse(init, queue.clone())
    });
    let queue = host.pulses.clone();
    a.register(kind::CAMERA_SHAKE, move |init| {
        pulse::OneShot::camera_shake(init, queue.clone())
    });
    let queue = host.pulses.clone();
    a.register(kind::HAPTICS, move |init| {
        pulse::OneShot::haptics(init, queue.clone())
    });
    a
}

fn setup_effects(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    host: Res<EffectHost>,
) {
    let Some(game) = game else { return };
    if let Ok(mut p) = host.particles.lock() {
        *p = ParticleManager::from_game(&game.fs);
    }
    match EffectSystem::from_retail(game.fs.clone(), default_adapters(&host)) {
        Ok(fx) => {
            info!("effects: {} effect slots, live cap {}", fx.slot_count(), fx.diagnostics().cap);
            commands.insert_resource(fx);
        }
        Err(e) => warn!("effects: registry not read ({e}); the effect spawner is disabled"),
    }
}

pub(crate) fn tick_effects(
    time: Res<Time>,
    fx: Option<ResMut<EffectSystem>>,
    host: Res<EffectHost>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    // The births' cull distance is measured from the camera (inferred, smoke-effects.md Gap 3).
    // Emitters whose component is gone run out here (native: the manager owns them).
    if let Ok(mut p) = host.particles.lock() {
        p.set_camera(camera.iter().next().map(|t| t.translation()));
        p.update_detached(time.delta_secs());
    }
    if let Some(mut fx) = fx {
        fx.tick(time.delta_secs());
    }
}

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        // `JC2_EFFECT_NO_PARTICLE_DRAW=1` skips the particle renderer (before/after captures: the
        // particles still simulate but nothing is drawn, as in the game before the renderer existed).
        let draw = std::env::var_os("JC2_EFFECT_NO_PARTICLE_DRAW").is_none();
        app.init_resource::<EffectHost>()
            .init_resource::<LightEntities>()
            .init_resource::<particle_render::ParticleDraw>()
            .insert_resource(DecalDraw {
                disabled: std::env::var_os("JC2_EFFECT_NO_DECAL_DRAW").is_some(),
                ..default()
            })
            .init_resource::<CameraShakeState>()
            .init_resource::<RumbleState>()
            .init_resource::<PulseStats>()
            .add_systems(Startup, setup_effects)
            .add_systems(
                Update,
                (tick_effects, light::apply_host_ops).chain(),
            )
            .add_systems(
                Update,
                (decal::apply_decal_ops, pulse::apply_pulse_ops).after(tick_effects),
            );
        camera_shake::add_to(app);
        if draw {
            particle_render::add_particle_material(app);
            app.add_systems(
                Update,
                particle_render::draw_particles.after(tick_effects),
            );
        }
        if std::env::var_os("JC2_DECAL_FIXTURE").is_some() {
            app.add_systems(Update, fixture::run_decal_fixture.after(tick_effects));
        }
        if let Ok(v) = std::env::var("JC2_EFFECT_FIXTURE") {
            app.insert_resource(fixture::FixtureConfig::from_env(&v))
                .add_systems(Update, fixture::run_fixture.after(tick_effects));
        }
    }
}

#[cfg(test)]
mod tests;
