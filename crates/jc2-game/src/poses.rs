//! Guns in the hands: which clips Rico and the soldiers carry, aim, fire and reload each
//! weapon class with, and where a gun sits on the body.
//!
//! The classes come from `weapons.bin`'s modifiers (`WEAPON_MODIFIER_DUALWIELD_PISTOL`,
//! `AI_MODIFIER_2H`, …) and the clips from the game's upper-body animation set
//! (`kc001_upperbody.asb`: `UB_ANIM_AIM_AND_FIRE` is `one_handed_rgt_aim.ban` with
//! `one_handed_rgt_recoil.ban`, `UB_ANIM_AIM_AND_FIRE_2H_LOW` is `rifle_aim_con_low` with
//! `rifle_recoil_con_low`, …; `research/formats/animation.md`). [`drive_upper_body`]
//! turns an [`Arms`] (what the gun is doing: aiming where, shots fired, reloads, throws)
//! into the animator's layers and its aim constraint.

use crate::rig::{Animator, Clip, Slot};
use bevy::prelude::*;
use jc2_formats::weapons::Weapon;

/// How a weapon is held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WeaponClass {
    /// One-handed: the Beretta, the revolver, the one-hand shotgun.
    #[default]
    Pistol,
    /// One-handed too (`WEAPON_MODIFIER_DUALWIELD_SMG`): the Scorpion, the bubble gun.
    Smg,
    /// Two-handed: the assault rifle, the sniper, the machine gun, the grenade launcher.
    Rifle,
    /// The pump shotgun: two-handed, pumped between shots.
    Shotgun,
    /// On the shoulder: the rocket launchers.
    Rpg,
}

impl WeaponClass {
    /// From the weapon's hand modifier (`#62d2f184`) and the AI's class (`#4fdb09b7`).
    pub fn of(w: &Weapon) -> Self {
        let ai = w.ai.as_ref().map_or("", |a| a.modifier.as_str());
        let hand = w.hand.as_str();
        if ai.contains("RPG") {
            WeaponClass::Rpg
        } else if hand.contains("DUALWIELD_SMG") {
            WeaponClass::Smg
        } else if hand.contains("DUALWIELD_PISTOL") || hand.contains("1H_SHOTGUN") {
            WeaponClass::Pistol
        } else if ai.contains("SHOTGUN") {
            WeaponClass::Shotgun
        } else if ai.contains("PISTOL") {
            WeaponClass::Pistol
        } else if ai.contains("SMG") {
            WeaponClass::Smg
        } else if !ai.is_empty() || hand.contains("GRENADE_LAUNCHER") {
            WeaponClass::Rifle
        } else if w.projectile == 4 {
            WeaponClass::Rpg
        } else {
            WeaponClass::Pistol
        }
    }

    pub fn one_handed(self) -> bool {
        matches!(self, WeaponClass::Pistol | WeaponClass::Smg)
    }

    /// The upper-body clip that carries it in a gait (`None`: the base clip's own arms,
    /// as the one-handed guns are carried standing and walking).
    pub fn wield(self, gait: Gait) -> Option<Clip> {
        use WeaponClass::*;
        match (self, gait) {
            (_, Gait::Crouch) => Some(match self {
                Pistol | Smg => Clip::Crouch1H,
                Rifle | Shotgun => Clip::Crouch2H,
                Rpg => Clip::CrouchRpg,
            }),
            (Pistol | Smg, _) => None,
            (Rifle | Shotgun, Gait::Walk) => Some(Clip::Wield2HWalk),
            (Rifle | Shotgun, Gait::Run | Gait::Sprint) => Some(Clip::Wield2HRun),
            (Rifle | Shotgun, _) => Some(Clip::Wield2HIdle),
            (Rpg, Gait::Walk) => Some(Clip::WieldRpgWalk),
            (Rpg, Gait::Run | Gait::Sprint) => Some(Clip::WieldRpgRun),
            (Rpg, _) => Some(Clip::WieldRpgIdle),
        }
    }

    /// After a fight, a one-handed gun is carried up (`UB_ANIM_WIELDING_*_1H_ALERT`).
    pub fn wield_alert(self, gait: Gait) -> Option<Clip> {
        match (self.one_handed(), gait) {
            (true, Gait::Idle) => Some(Clip::Wield1HAlert),
            (true, Gait::Run | Gait::Sprint) => Some(Clip::Wield1HRun),
            _ => self.wield(gait),
        }
    }

    /// The aim pose (`low`: crouched on a roof, the rifles' `_LOW` set).
    pub fn aim(self, low: bool) -> Clip {
        match self {
            WeaponClass::Pistol | WeaponClass::Smg => Clip::Aim1H,
            WeaponClass::Rifle | WeaponClass::Shotgun if low => Clip::Aim2HLow,
            WeaponClass::Rifle | WeaponClass::Shotgun => Clip::Aim2H,
            WeaponClass::Rpg => Clip::AimRpg,
        }
    }

    /// The recoil played on each shot (none for the launcher in the data).
    pub fn recoil(self, low: bool) -> Option<Clip> {
        match self {
            WeaponClass::Pistol | WeaponClass::Smg => Some(Clip::Recoil1H),
            WeaponClass::Rifle | WeaponClass::Shotgun if low => Some(Clip::Recoil2HLow),
            WeaponClass::Rifle | WeaponClass::Shotgun => Some(Clip::Recoil2H),
            WeaponClass::Rpg => None,
        }
    }

    /// `UB_ANIM_RELOAD_RH` (pistol, SMG), `_RIFLE_RH`, `_SHOTGUN_RH`, `UB_ANIM_RELOADING_RPG`.
    pub fn reload(self) -> Clip {
        match self {
            WeaponClass::Pistol | WeaponClass::Smg => Clip::ReloadPistol,
            WeaponClass::Rifle => Clip::ReloadRifle,
            WeaponClass::Shotgun => Clip::ReloadShotgun,
            WeaponClass::Rpg => Clip::ReloadRpg,
        }
    }

    /// The soldiers' aim and recoil (`npc_*_con`), with Rico's where they have none.
    pub fn npc_aim(self) -> Clip {
        match self {
            WeaponClass::Pistol => Clip::Aim1H,
            WeaponClass::Smg => Clip::NpcAimSmg,
            WeaponClass::Rifle | WeaponClass::Shotgun => Clip::NpcAimRifle,
            WeaponClass::Rpg => Clip::AimRpg,
        }
    }

    pub fn npc_recoil(self) -> Option<Clip> {
        match self {
            WeaponClass::Pistol => Some(Clip::Recoil1H),
            WeaponClass::Smg => Some(Clip::NpcRecoilSmg),
            WeaponClass::Rifle | WeaponClass::Shotgun => Some(Clip::NpcRecoilRifle),
            WeaponClass::Rpg => None,
        }
    }

    /// The bones the aim turns and their shares (the `.acsb` constraint weights, made
    /// shares of one turn: 2H `Spine1` 100 (the low set `Spine` 15, `Spine1` 30), the
    /// RPG `Spine` 60 `Spine1` 100, one-handed the right shoulder — here the chest takes a
    /// little of it, the shoulder joint the rest, which skins better than the clavicle).
    pub fn aim_chain(self) -> &'static [(&'static str, f32)] {
        match self {
            WeaponClass::Pistol | WeaponClass::Smg => &[("Spine1", 0.35), ("RightArm", 0.65)],
            WeaponClass::Rifle | WeaponClass::Shotgun => &[("Spine", 0.3), ("Spine1", 0.7)],
            WeaponClass::Rpg => &[("Spine", 0.4), ("Spine1", 0.6)],
        }
    }

    /// How long the gun takes to come down after the aim (`TransitionTimes`: 1H aim to
    /// idle 0.27 s, 2H 0.44 s).
    pub fn aim_out(self) -> f32 {
        if self.one_handed() { 0.27 } else { 0.44 }
    }

    pub fn npc_reload(self) -> Clip {
        match self {
            WeaponClass::Pistol => Clip::NpcReload,
            other => other.reload(),
        }
    }
}

/// What the legs are doing, as far as the gun is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gait {
    #[default]
    Idle,
    Walk,
    Run,
    Sprint,
    /// Crouched on a vehicle.
    Crouch,
    /// Jumping or falling.
    Air,
    /// On the wire, or hanging under a helicopter: one hand is on the line.
    Hang,
    /// Under the canopy: the left hand on the handle.
    Chute,
    Swim,
    /// In a seat.
    Seated,
}

impl Gait {
    pub fn moving(self) -> bool {
        matches!(self, Gait::Walk | Gait::Run | Gait::Sprint)
    }

    /// Whether a gun of `class` is in his hand (else it is stowed): two-handed guns go on
    /// the back while a hand holds the wire or the chute, nothing is held swimming or in
    /// a seat.
    pub fn holds(self, class: WeaponClass) -> bool {
        match self {
            Gait::Swim | Gait::Seated => false,
            Gait::Hang | Gait::Chute => class.one_handed(),
            _ => true,
        }
    }
}

/// What a character's gun is doing; written by `combat.rs` (Rico's trigger, the
/// soldiers' bursts), turned into animation by [`drive_upper_body`].
#[derive(Component, Debug, Clone, Default)]
pub struct Arms {
    /// The gun in the right hand (`None`: empty-handed).
    pub class: Option<WeaponClass>,
    /// Where to aim (world space) while `aim_for` lasts (s).
    pub aim_dir: Vec3,
    pub aim_for: f32,
    /// Counters: every change plays the clip once.
    pub shots: u32,
    pub reloads: u32,
    /// The reload's length (`weapons.bin` `reloadtime`): the reload clip is played to fit.
    pub reload_time: f32,
    pub throws: u32,
    pub hooks: u32,
    /// The seconds between shots of the gun in hand (a recoil restarts no faster).
    pub interval: f32,
    /// Seconds he stays on his guard after aiming: the one-handed gun is carried up.
    pub alert_for: f32,
    /// The gait it was last animated in (where the guns go: `stowage`).
    pub gait: Gait,
    pub(crate) seen: [u32; 4],
}

impl Arms {
    pub fn aiming(&self) -> bool {
        self.aim_for > 0.0 && self.class.is_some()
    }
}

/// How long the gun stays up after a shot (s).
pub const AIM_AFTER_SHOT: f32 = 1.2;
/// How long he stays alert after aiming (s).
const ALERT: f32 = 5.0;
/// Seconds to fade the carrying, aiming and finger layers in and out (the `.asb`:
/// navigation 0.24 s, the rifle idle 0.21; 0.07 to raise the gun to aim, 0.07–0.2 into a
/// reload).
const WIELD_BLEND: f32 = 0.24;
const AIM_IN: f32 = 0.07;
const RECOIL_BLEND: f32 = 0.04;
const RELOAD_BLEND: f32 = 0.15;

/// Set the animator's layers from `arms` for a body in `gait`; `model_rot` turns the
/// model's space into the world's (for the aim). `npc`: the soldiers' clips.
pub fn drive_upper_body(
    an: &mut Animator,
    arms: &mut Arms,
    gait: Gait,
    model_rot: Quat,
    npc: bool,
    dt: f32,
) {
    arms.aim_for = (arms.aim_for - dt).max(0.0);
    arms.alert_for = (arms.alert_for - dt).max(0.0);
    arms.gait = gait;
    let held = arms.class.filter(|c| gait.holds(*c));
    let aiming = held.is_some() && arms.aim_for > 0.0 && gait != Gait::Swim;
    if aiming {
        arms.alert_for = ALERT;
    }
    let moving = gait.moving();
    let low = gait == Gait::Crouch;
    // carrying (under the chute the caller holds the turn axis on this slot)
    if gait != Gait::Chute {
        let wield = held.and_then(|c| {
            if arms.alert_for > 0.0 {
                c.wield_alert(gait)
            } else {
                c.wield(gait)
            }
        });
        an.set_layer(Slot::Wield, wield, WIELD_BLEND, moving);
    }
    // aiming: the pose, and the spine turned onto the target
    match held.filter(|_| aiming) {
        Some(c) => {
            let clip = if npc { c.npc_aim() } else { c.aim(low) };
            an.set_layer(Slot::Aim, Some(clip), AIM_IN, false);
            an.set_aim_chain(c.aim_chain());
            an.aim.target = 1.0;
            an.aim.dir = (model_rot.inverse() * arms.aim_dir).normalize_or(Vec3::NEG_Z);
        }
        None => {
            let out = arms.class.map_or(0.3, |c| c.aim_out());
            an.set_layer(Slot::Aim, None, out, false);
            an.aim.target = 0.0;
        }
    }
    // shots, reloads, throws, the hook
    if arms.shots != arms.seen[0] {
        arms.seen[0] = arms.shots;
        if let Some(c) = held {
            let recoil = if npc { c.npc_recoil() } else { c.recoil(low) };
            let busy = an.layer(Slot::Action);
            // a recoil under way restarts no faster than the gun fires (an SMG's bursts
            // keep it at the kick instead of pinning it at frame 0)
            let restart = match busy.clip {
                Some(r) if Some(r) == recoil => busy.time >= arms.interval.clamp(0.05, 0.12),
                Some(other) => !is_reload(other),
                None => true,
            };
            if let Some(r) = recoil
                && restart
            {
                an.play_once(Slot::Action, r, 1.0, RECOIL_BLEND);
            }
        }
    }
    if arms.reloads != arms.seen[1] {
        arms.seen[1] = arms.reloads;
        if let Some(c) = held {
            let clip = if npc { c.npc_reload() } else { c.reload() };
            let d = an.rig.duration(clip);
            // the clip is stretched to the data's reload time
            let rate = if arms.reload_time > 0.05 && d > 0.0 {
                d / arms.reload_time
            } else {
                1.0
            };
            an.play_once(Slot::Action, clip, rate, RELOAD_BLEND);
        }
    }
    if arms.throws != arms.seen[2] {
        arms.seen[2] = arms.throws;
        // the left arm throws (the left-arm layer: masked)
        if gait == Gait::Chute {
            an.play_once(Slot::Action, Clip::ThrowChute, 1.0, 0.1);
        } else {
            an.play_once_masked(Slot::Action, Clip::Throw, 1.0, 0.1, "LeftShoulder");
        }
    }
    if arms.hooks != arms.seen[3] {
        arms.seen[3] = arms.hooks;
        let clip = if matches!(gait, Gait::Air | Gait::Chute | Gait::Hang) {
            Clip::GrappleFireAir
        } else {
            Clip::GrappleFire
        };
        // the left arm only: it may go over a recoil or a reload of the right
        if !an
            .layer(Slot::Action)
            .clip
            .is_some_and(|c| is_reload(c) || c == Clip::Throw)
        {
            an.play_once(Slot::Action, clip, 1.0, 0.06);
        }
    }
    // the fingers round the grip
    let fingers = held.map(|_| Clip::FingersRight);
    an.set_layer(Slot::Fingers, fingers, 0.15, false);
}

pub(crate) fn is_reload(c: Clip) -> bool {
    matches!(
        c,
        Clip::ReloadPistol
            | Clip::ReloadRifle
            | Clip::ReloadShotgun
            | Clip::ReloadRpg
            | Clip::NpcReload
    )
}

/// Where a gun sits: in a hand, or stowed by the original's bone slots (JC2-092,
/// `research/formats/weapon-ownership-next.md`): holder slot 0 on `LeftUpLeg`, slot 1 on
/// `RightUpLeg`, slot 2 (two-handed) on `Harness`. The transforms are the exact native
/// ones, built in `weapon_attach`.
///
/// Which pickup lands in which slot is **unsupported** (the slot-insertion writer was not
/// found): the first one-handed gun takes slot 1 (`RightUpLeg`), the second slot 0, a
/// two-handed gun slot 2 -- the reconstruction's earlier order, kept, not proven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stow {
    /// `AttachHandRight` (0x19): every draw state but the dual-wield left gun.
    Hand,
    /// `AttachHandLeft` (0x18): slot 0 when dual-wielding (draw state 4). The runtime has
    /// no dual-wield state yet; the mapping is exercised by the tests only.
    #[allow(dead_code)]
    HandLeft,
    /// Slot 2: `Harness` (0x4d).
    Harness,
    /// Slot 1: `RightUpLeg` (7).
    RightUpLeg,
    /// Slot 0: `LeftUpLeg` (4).
    LeftUpLeg,
    Hidden,
}

impl Stow {
    /// The `NBones::EBoneIndex` value the original writes to `weapon+0x254`.
    pub fn bone(self) -> Option<usize> {
        use crate::weapon_attach::*;
        match self {
            Stow::Hand => Some(ATTACH_HAND_RIGHT),
            Stow::HandLeft => Some(ATTACH_HAND_LEFT),
            Stow::Harness => Some(HARNESS),
            Stow::RightUpLeg => Some(RIGHT_UP_LEG),
            Stow::LeftUpLeg => Some(LEFT_UP_LEG),
            Stow::Hidden => None,
        }
    }

    /// Whether a hand holds it.
    pub fn in_hand(self) -> bool {
        matches!(self, Stow::Hand | Stow::HandLeft)
    }

    /// The bone it hangs from (by name) and the gun's transform in that bone's frame:
    /// the native correction for the bone slot and this weapon's settings.
    pub fn mount(
        self,
        mounts: Option<&jc2_formats::weapon_mounts::WeaponMounts>,
    ) -> Option<(&'static str, Transform)> {
        let bone = self.bone()?;
        Some((
            crate::weapon_attach::bone_name(bone)?,
            crate::weapon_attach::bone_local(bone, mounts),
        ))
    }
}

/// Where each of the guns carried goes: the one in use in the hand (if the gait lets a
/// hand hold it, else stowed), the others in the first free stowage of their kind —
/// one two-handed gun on the harness, two pistols on the thighs — and the rest hidden.
pub fn stowage(classes: &[WeaponClass], current: usize, gait: Gait) -> Vec<Stow> {
    let mut out = vec![Stow::Hidden; classes.len()];
    let mut back = false;
    let mut thighs = [false; 2];
    let free = |c: WeaponClass, back: &mut bool, thighs: &mut [bool; 2]| {
        if c.one_handed() {
            if !thighs[0] {
                thighs[0] = true;
                Stow::RightUpLeg
            } else if !thighs[1] {
                thighs[1] = true;
                Stow::LeftUpLeg
            } else {
                Stow::Hidden
            }
        } else if !*back {
            *back = true;
            Stow::Harness
        } else {
            Stow::Hidden
        }
    };
    if let Some(&c) = classes.get(current) {
        out[current] = if gait.holds(c) {
            Stow::Hand
        } else if gait == Gait::Seated {
            Stow::Hidden
        } else {
            free(c, &mut back, &mut thighs)
        };
    }
    // the last two-handed gun used first: the order of the slots
    for (i, &c) in classes.iter().enumerate() {
        if i != current && gait != Gait::Seated {
            out[i] = free(c, &mut back, &mut thighs);
        }
    }
    out
}

/// FALLBACK only, for a weapon whose settings author no V (`weapon_attach::emission_point`
/// is the original origin): the muzzle of a model in its own frame, the front of its
/// bounding box, at the
/// bore's height (11 cm above the grip on every hand gun looked at).
pub fn muzzle_of(bbox_min: Vec3, bbox_max: Vec3) -> Vec3 {
    Vec3::new(0.0, 0.11f32.min(bbox_max.y * 0.8), bbox_min.z)
}

/// Debug only (`--pose`): hold Rico in one state for a screenshot, whatever the physics
/// says — the base clip and the gait the gun follows.
#[derive(Resource, Clone, Debug)]
pub struct PoseOverride {
    pub clip: Clip,
    pub gait: Gait,
    /// Exercise the real zero-health lifecycle while holding the respawn timer.
    pub downed: bool,
}

/// Debug only: `--pose <state> [--weapon N] [--aim] [--firing] [--pose-view DEG]`.
#[derive(Resource, Clone, Debug)]
pub struct PoseDemo {
    pub pose: Option<PoseOverride>,
    /// 1-based, as the keys.
    pub weapon: Option<usize>,
    pub aim: bool,
    pub firing: bool,
    /// The camera's yaw round him (degrees; 0 behind him, 90 his right side, 180 the
    /// front) and pitch.
    pub view: f32,
    pub pitch: f32,
    /// Soldiers to call in a few metres ahead of him (they see him and shoot; he does
    /// not die meanwhile).
    pub soldiers: u32,
}

impl PoseOverride {
    /// The `--pose` names.
    pub fn named(name: &str) -> Option<Self> {
        let (clip, gait) = match name.to_ascii_lowercase().as_str() {
            "idle" => (Clip::Idle, Gait::Idle),
            "down" => (Clip::Idle, Gait::Seated),
            "walk" => (Clip::Walk, Gait::Walk),
            "walk1h" => (Clip::Walk1H, Gait::Walk),
            "run" => (Clip::Run, Gait::Run),
            "sprint" => (Clip::Sprint, Gait::Sprint),
            "strafe" => (Clip::RunStrafeLeft, Gait::Run),
            "jump" => (Clip::Jump, Gait::Air),
            "fall" => (Clip::Fall, Gait::Air),
            "dive" => (Clip::Dive, Gait::Air),
            "land" => (Clip::Land, Gait::Idle),
            "landheavy" => (Clip::LandHeavy, Gait::Idle),
            "roll" => (Clip::LandRoll, Gait::Idle),
            "hang" => (Clip::Hang, Gait::Hang),
            "reel" => (Clip::Reel, Gait::Hang),
            "climb" => (Clip::GrappleClimb, Gait::Hang),
            "chute" => (Clip::Chute, Gait::Chute),
            "swim" => (Clip::Swim, Gait::Swim),
            "crouch" => (Clip::Crouch, Gait::Crouch),
            "seated" => (Clip::Drive, Gait::Seated),
            _ => return None,
        };
        Some(Self {
            clip,
            gait,
            downed: name.eq_ignore_ascii_case("down"),
        })
    }
}

pub struct PoseDemoPlugin(pub PoseDemo);

impl Plugin for PoseDemoPlugin {
    fn build(&self, app: &mut App) {
        if let Some(p) = &self.0.pose {
            app.insert_resource(p.clone());
        }
        app.insert_resource(self.0.clone())
            .add_systems(PreUpdate, demo_buttons.after(bevy::input::InputSystems))
            .add_systems(Update, demo_soldiers)
            .add_systems(Update, demo_downed.before(crate::combat::hero_down))
            .add_systems(
                Update,
                demo_aim
                    .after(crate::combat::hero_arms)
                    .after(crate::combat::fire)
                    .before(crate::character::hero_animation),
            );
    }
}

fn demo_downed(
    pose: Option<Res<PoseOverride>>,
    mut arsenal: Option<ResMut<crate::combat::Arsenal>>,
    mut heroes: Query<&mut crate::combat::Health, With<crate::character::Hero>>,
) {
    if pose.is_none_or(|pose| !pose.downed) {
        return;
    }
    if let Ok(mut health) = heroes.single_mut() {
        health.hp = 0.0;
    }
    if let Some(arsenal) = arsenal.as_mut() {
        arsenal.down_for = Some(0.6);
    }
}

/// The trigger and the aim button, held (semi-automatics are clicked every other frame).
fn demo_buttons(
    demo: Res<PoseDemo>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    if demo.aim || demo.firing {
        buttons.press(MouseButton::Right);
    }
    if demo.firing {
        if (*frame).is_multiple_of(2) {
            buttons.press(MouseButton::Left);
        } else {
            buttons.release(MouseButton::Left);
        }
    }
}

/// `--pose-soldiers N`: a few seconds in, N soldiers a dozen metres ahead of him (with
/// the roster's guns), and Rico kept alive.
#[allow(clippy::type_complexity)]
fn demo_soldiers(
    demo: Res<PoseDemo>,
    time: Res<Time>,
    world: Option<Res<crate::world::WorldConfig>>,
    roster: Option<Res<crate::npc::EnemyRoster>>,
    npcs: Option<ResMut<crate::npc::Npcs>>,
    mut heroes: Query<(
        &crate::character::Hero,
        &Transform,
        Option<&mut crate::combat::Health>,
    )>,
    mut called: Local<bool>,
) {
    let Ok((hero, tf, health)) = heroes.single_mut() else {
        return;
    };
    if let Some(mut h) = health {
        h.hp = h.max;
    }
    if *called || demo.soldiers == 0 || time.elapsed_secs() < 6.0 {
        return;
    }
    let (Some(world), Some(roster), Some(mut npcs)) = (world, roster, npcs) else {
        return;
    };
    *called = true;
    let ahead = crate::coords::facing_dir(hero.facing);
    let side = Vec3::new(-ahead.z, 0.0, ahead.x);
    for i in 0..demo.soldiers {
        let at = tf.translation
            + ahead * 12.0
            + side * (i as f32 - (demo.soldiers as f32 - 1.0) / 2.0) * 3.0;
        npcs.reinforce(&roster, at, at, world.tile, 7 + i * 13);
    }
}

/// The gun he holds, the aim straight ahead of where he faced at the start, the camera
/// held at the asked angle round him.
fn demo_aim(
    demo: Res<PoseDemo>,
    arsenal: Option<ResMut<crate::combat::Arsenal>>,
    mut orbit: ResMut<crate::character::OrbitCamera>,
    mut heroes: Query<(&crate::character::Hero, &mut Arms)>,
) {
    if let (Some(mut a), Some(w)) = (arsenal, demo.weapon)
        && w >= 1
        && w <= a.guns.len()
        && a.current != w - 1
    {
        a.current = w - 1;
    }
    let Ok((hero, mut arms)) = heroes.single_mut() else {
        return;
    };
    // he faces −z at the start: aim along it, a little down the range
    arms.aim_dir = Vec3::new(0.0, -0.05, -1.0).normalize();
    if demo.aim || demo.firing {
        arms.aim_for = arms.aim_for.max(0.3);
    }
    orbit.yaw = hero.facing + demo.view.to_radians();
    orbit.pitch = demo.pitch.to_radians();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stowage_puts_one_gun_in_hand_and_the_rest_on_the_body() {
        use WeaponClass::*;
        let classes = [Pistol, Smg, Shotgun, Rifle, Rpg];
        let s = stowage(&classes, 3, Gait::Run);
        assert_eq!(s[3], Stow::Hand);
        assert_eq!(&s[..3], &[Stow::RightUpLeg, Stow::LeftUpLeg, Stow::Harness]);
        assert_eq!(s[4], Stow::Hidden);
        // on the wire the rifle goes on the harness
        let s = stowage(&classes, 3, Gait::Hang);
        assert_eq!(s[3], Stow::Harness);
        assert_eq!(s[0], Stow::RightUpLeg);
        // the pistol stays in the hand under the chute
        assert_eq!(stowage(&classes, 0, Gait::Chute)[0], Stow::Hand);
        assert!(
            stowage(&classes, 0, Gait::Seated)
                .iter()
                .all(|s| *s == Stow::Hidden)
        );
    }

    #[test]
    fn mounts_are_rotations_on_the_native_bones() {
        let want = [
            (Stow::Hand, "AttachHandRight"),
            (Stow::HandLeft, "AttachHandLeft"),
            (Stow::Harness, "Harness"),
            (Stow::RightUpLeg, "RightUpLeg"),
            (Stow::LeftUpLeg, "LeftUpLeg"),
        ];
        for (s, bone) in want {
            let (name, t) = s.mount(None).unwrap();
            assert_eq!(name, bone);
            assert!((t.rotation.length() - 1.0).abs() < 1e-4, "{s:?}");
            assert!(Mat3::from_quat(t.rotation).determinant() > 0.99, "{s:?}");
        }
        assert!(Stow::Hidden.mount(None).is_none());
        // the held right-hand rotation is the one the reconstruction already used
        let (_, hand) = Stow::Hand.mount(None).unwrap();
        assert!(hand.rotation.dot(crate::rig::weapon_in_right_hand()).abs() > 0.99999);
    }
}
