//! Guns: Rico's arsenal from `settings\weapons.bin` and the soldiers who shoot back.
//! Hitscan along the camera's aim with each weapon's range, damage, pellets and push; a
//! tracer from the muzzle; rockets fly as projectiles and grenades are thrown
//! (`explosion.rs`). Soldiers take damage, die and fall; within sight and range they
//! turn on Rico — or the vehicle he is in — and fire with their rifle's burst delays.
//! Vehicles carry the health from `vehicletypes.bin` and burn out at zero. Rico goes
//! down at zero health and gets up at the start a few seconds later.
//!
//! Every gun has a magazine and a reserve (`ammo.rs`): firing spends rounds, an empty
//! magazine reloads by itself (or `R`), nothing fires during the reload. Soldiers carry
//! the weapon their class rolled (`npc.rs`, `enemytypespawnsettings.bin`) and fire it in
//! bursts of the AI block's length, pausing to reload.
//!
//! Controls on foot: left mouse fires (with the cursor grabbed, `Tab`, or while
//! holding the right button to aim); `1`–`5` pick the Beretta, the SMG, the shotgun,
//! the assault rifle and the rocket launcher (`6`–`9` the guns picked up since); `R`
//! reloads; `G` throws a grenade.

mod accuracy;
use accuracy::{PlayerAccuracy, ShotRandom};

use crate::Game;
use crate::ammo::WeaponState;
use crate::character::{HEIGHT, Hero};
use crate::coords::yaw_toward;
use crate::destruction::StructureHits;
use crate::explosion::{Blast, ExplosionPlugin, Fx, Grenade, Projectile, body_of};
use crate::fleet::Container;
use crate::npc::{Npc, Soldier};
use crate::physics::release_passenger;
use crate::poses::{AIM_AFTER_SHOT, Arms, Gait, Stow, WeaponClass, stowage};
use crate::rbm_loader::RbmAsset;
use crate::rig::{Animator, Clip};
use crate::stunts::OnStunt;
use crate::vehicle::{CameraMode, Enterable, Occupied};
use crate::weapon_attach::{WeaponAttachPlugin, WeaponMountTable, emission_point};
use crate::world::{FlyCamera, Piece, SpawnPoint};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::weapons::{AiUse, Throwable, Upgrades, Weapon};
use std::collections::HashMap;

/// Rico's guns at the start, by the table's names. The time between shots
/// (`firerate`) and the magazine come from `weapon_upgrades.bin`, whether holding the
/// button keeps firing from `weapons.bin` (`#01dbd94a`).
const ROSTER: [&str; 5] = [
    "Beretta Handgun",
    "Scorpion_SMG",
    "Shotgun (pump action)",
    "Assault",
    "RocketLauncher",
];
/// The most guns he carries (`1`–`9`).
pub const MAX_GUNS: usize = 9;
/// Time between shots when the upgrade table has none (s).
const FALLBACK_INTERVAL: f32 = 0.2;
const TRACER_SECONDS: f32 = 0.06;
/// What a soldier without a class (spawned by a test) carries; the village's get theirs
/// from `enemytypespawnsettings.bin`.
const FALLBACK_SOLDIER_WEAPON: &str = "Assault";
/// Their chance to hit at point-blank range, falling off with distance (tuned). Within a
/// burst every shot rolls, with the chance spread so a burst lands about two hits at
/// most.
const NPC_HIT_CHANCE: f32 = 0.55;
const HITS_PER_BURST: f32 = 2.0;
/// Health of a soldier without a class (the grunt's `health` in
/// `enemytypespawnsettings.bin`, for soldiers spawned by tests).
const NPC_HEALTH: f32 = 100.0;
const HEAL_AFTER: f32 = 6.0;
const HEAL_RATE: f32 = 10.0;
pub(crate) const RESPAWN_SECONDS: f32 = 4.0;
/// How long the dead lie there (s).
const CORPSE_SECONDS: f32 = 30.0;
const GRENADE_INTERVAL: f32 = 1.0;
/// `#e56e25f4` values that fly as projectiles: rockets and grenade-launcher rounds.
const ROCKET: i32 = 4;
const LOBBED: i32 = 61;

#[derive(Component, Debug, Clone, Copy)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { hp: max, max }
    }
}

/// Fallen: how long ago.
#[derive(Component)]
pub struct Dead(pub f32);

#[derive(Component)]
struct Tracer(f32);

/// Which of Rico's guns have their model spawned.
#[derive(Component)]
pub(crate) struct Holster(Vec<bool>);

/// One of Rico's guns (index into `Arsenal::guns`): where it is mounted, and its
/// muzzle in its own frame (the settings' V; `poses::muzzle_of` only when none is
/// authored), and its transform in the bone's frame (`weapon_attach::bone_local`).
#[derive(Component)]
pub(crate) struct GunModel {
    pub slot: usize,
    pub stow: Stow,
    pub muzzle: Vec3,
    pub base: Transform,
}

/// A soldier's gun model is in his hand (or he has none to show).
#[derive(Component)]
struct SoldierArmed;

/// A soldier's gun model, and its muzzle in its own frame.
#[derive(Component)]
struct SoldierGunModel {
    muzzle: Vec3,
}

/// The soldier's gun model entity.
#[derive(Component)]
struct SoldierGunOf(Entity);

#[derive(Component)]
struct HealthHud;

/// A soldier's gun: which one (an index into `Arsenal::soldier_guns`), its rounds, the
/// seconds until the next shot and the shots left in the burst.
#[derive(Component, Debug)]
pub struct Gunner {
    pub gun: usize,
    pub ammo: WeaponState,
    pub next: f32,
    pub burst_left: u32,
}

pub struct Gun {
    pub weapon: Weapon,
    /// Its upgrade level (0 … 5, `weapon_upgrades.bin`; the HUD's stars).
    pub level: usize,
    /// Damage a hit (a pellet) does: the upgrade level's `damage`, else the table's.
    pub damage: f32,
    pub interval: f32,
    pub automatic: bool,
    pub ammo: WeaponState,
    /// How it is held (`poses.rs`).
    pub class: WeaponClass,
    model: Option<Handle<RbmAsset>>,
}

/// How soldiers use a weapon: the AI block, the time between shots in a burst and a
/// full gun's rounds.
pub struct SoldierGun {
    pub weapon: Weapon,
    pub class: WeaponClass,
    pub ai: AiUse,
    pub interval: f32,
    pub ammo: WeaponState,
}

#[derive(Resource)]
pub struct Arsenal {
    pub guns: Vec<Gun>,
    pub current: usize,
    pub grenade: Throwable,
    grenade_model: Option<Handle<RbmAsset>>,
    /// Grenades carried, and the most he can (`#7b655f19`, assumed).
    pub grenades: u32,
    pub max_grenades: u32,
    /// The whole weapon table, the upgrade table and `max_ammo_N`, for guns picked up.
    pub table: Vec<Weapon>,
    upgrades: Upgrades,
    max_ammo: Vec<u32>,
    /// Upgrade levels bought at the Black Market, by `weapons.bin` name (absent: 0).
    pub levels: HashMap<String, usize>,
    /// The weapons soldiers carry, by name → index in `soldier_guns`.
    pub soldier_guns: Vec<SoldierGun>,
    soldier_index: HashMap<String, usize>,
    hero_health: f32,
    /// `vehicletypes.bin`: container → health.
    vehicle_health: HashMap<String, f32>,
    cooldown: f32,
    grenade_cooldown: f32,
    rng: u32,
    player_accuracy: PlayerAccuracy,
    accuracy_recovery: f32,
    shot_random: ShotRandom,
    tracer_mesh: Handle<Mesh>,
    tracer_hot: Handle<StandardMaterial>,
    tracer_enemy: Handle<StandardMaterial>,
    flash_mesh: Handle<Mesh>,
    flash_material: Handle<StandardMaterial>,
    /// The soldiers' gun models, by `soldier_guns` index (loaded on first use).
    soldier_models: HashMap<usize, Option<Handle<RbmAsset>>>,
    /// Hits and kills, for the HUD and the tests.
    pub hits: u32,
    pub kills: u32,
    pub down_for: Option<f32>,
    /// Trigger pulls so far, and where soldiers fired from since the sound last looked
    /// (`audio.rs`).
    pub shots: u32,
    pub enemy_shots: Vec<Vec3>,
    /// Seconds since Rico was last hit; he heals once out of the fight.
    since_hurt: f32,
}

impl Arsenal {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn gun(&self) -> &Gun {
        &self.guns[self.current]
    }

    /// A full gun of `name` for Rico, sized from the tables; `None` for names the table
    /// does not have.
    fn make_gun(&self, assets: Option<&AssetServer>, name: &str) -> Option<Gun> {
        let weapon = self.table.iter().find(|w| w.name == name)?.clone();
        Some(gun_from(
            weapon,
            &self.upgrades,
            &self.max_ammo,
            assets.and_then(|a| general_model(a, &self.table_model(name))),
            self.level_of(name),
        ))
    }

    /// The upgrade level of a weapon (by `weapons.bin` name).
    pub fn level_of(&self, name: &str) -> usize {
        self.levels.get(name).copied().unwrap_or(0)
    }

    /// The upgrade table.
    #[cfg(test)]
    pub fn upgrades(&self) -> &Upgrades {
        &self.upgrades
    }

    /// Upgrade a weapon to `level`: a gun he carries takes the level's damage, fire
    /// interval and magazine at once (keeping its rounds); one he picks up later comes at
    /// that level.
    pub fn set_level(&mut self, name: &str, level: usize) {
        let level = level.min(5);
        self.levels.insert(name.to_string(), level);
        let Some(i) = self.guns.iter().position(|g| g.weapon.name == name) else {
            return;
        };
        let fresh = gun_from(
            self.guns[i].weapon.clone(),
            &self.upgrades,
            &self.max_ammo,
            None,
            level,
        );
        let g = &mut self.guns[i];
        g.level = level;
        g.damage = fresh.damage;
        g.interval = fresh.interval;
        g.ammo
            .resize(fresh.ammo.magazine_size, fresh.ammo.max_reserve);
        info!(
            "{name} upgraded to level {level}: {} damage, {} rounds, a shot every {} s",
            g.damage, g.ammo.magazine_size, g.interval
        );
    }

    fn table_model(&self, name: &str) -> String {
        self.table
            .iter()
            .find(|w| w.name == name)
            .map(|w| w.model.clone())
            .unwrap_or_default()
    }

    /// Rounds in one magazine of `name` (for pickups counted in magazines).
    pub fn magazine_of(&self, name: &str) -> u32 {
        match self.guns.iter().find(|g| g.weapon.name == name) {
            Some(g) => g.ammo.magazine_size,
            None => self
                .make_gun(None, name)
                .map_or(1, |g| g.ammo.magazine_size),
        }
    }

    /// A weapon picked up with `rounds` of ammunition: tops up the reserve of the gun he
    /// has, or adds it (with its magazine full and the rounds in reserve) while there
    /// is room. The rounds taken, or `None` when nothing came of it.
    pub fn pick_up(
        &mut self,
        assets: Option<&AssetServer>,
        name: &str,
        rounds: u32,
    ) -> Option<u32> {
        if let Some(g) = self.guns.iter_mut().find(|g| g.weapon.name == name) {
            let took = g.ammo.add(rounds);
            return (took > 0).then_some(took);
        }
        if self.guns.len() >= MAX_GUNS {
            return None;
        }
        let mut g = self.make_gun(assets, name)?;
        g.ammo.reserve = 0;
        let took = g.ammo.add(rounds);
        info!("rico picks up a {} ({} rounds)", g.weapon.name, took);
        self.guns.push(g);
        Some(took)
    }

    /// Grenades picked up; how many fitted.
    pub fn pick_up_grenades(&mut self, n: u32) -> u32 {
        let take = n.min(self.max_grenades.saturating_sub(self.grenades));
        self.grenades += take;
        take
    }

    /// The index of a soldier weapon by name (the fallback rifle when unknown).
    pub fn soldier_gun(&self, name: &str) -> usize {
        self.soldier_index
            .get(name)
            .or_else(|| self.soldier_index.get(FALLBACK_SOLDIER_WEAPON))
            .copied()
            .unwrap_or(0)
    }
}

fn gun_from(
    weapon: Weapon,
    upgrades: &Upgrades,
    max_ammo: &[u32],
    model: Option<Handle<RbmAsset>>,
    level: usize,
) -> Gun {
    let up = upgrades.get(&weapon.name, level);
    let interval = up
        .map(|u| u.fire_interval)
        .filter(|i| *i > 0.0)
        .unwrap_or(FALLBACK_INTERVAL);
    let damage = up
        .map(|u| u.damage)
        .filter(|d| *d > 0.0)
        .unwrap_or(weapon.damage);
    Gun {
        ammo: WeaponState::for_weapon_at(&weapon, upgrades, max_ammo, level),
        level,
        damage,
        interval,
        automatic: weapon.automatic,
        class: WeaponClass::of(&weapon),
        model,
        weapon,
    }
}

fn soldier_gun_from(weapon: Weapon, upgrades: &Upgrades) -> SoldierGun {
    let ai = weapon.ai.clone().unwrap_or(AiUse {
        min_range: 0.0,
        max_range: 50.0,
        min_burst_delay: 0.5,
        max_burst_delay: 1.5,
        magazine_size: None,
        min_burst_length: 0.0,
        max_burst_length: 0.0,
        fire_interval: None,
        modifier: String::new(),
    });
    let interval = ai
        .fire_interval
        .or_else(|| upgrades.get(&weapon.name, 0).map(|u| u.fire_interval))
        .filter(|i| *i > 0.0)
        .unwrap_or(FALLBACK_INTERVAL);
    // the AI block's magazine on the hand guns, else the player's
    let magazine = ai
        .magazine_size
        .or_else(|| upgrades.get(&weapon.name, 0).map(|u| u.magazine))
        .filter(|m| (1..1000).contains(m))
        .unwrap_or(20);
    // soldiers never run dry: a reserve is not modelled for them, only the reloads
    let mut ammo = WeaponState::new(magazine, 0, weapon.reload_time);
    ammo.reserve = u32::MAX / 2;
    ammo.max_reserve = u32::MAX / 2;
    SoldierGun {
        class: WeaponClass::of(&weapon),
        weapon,
        ai,
        interval,
        ammo,
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExplosionPlugin,
            crate::destruction::DestructionPlugin,
            crate::pickups::PickupPlugin,
        ))
        .add_plugins(WeaponAttachPlugin)
        .add_systems(Startup, setup_arsenal)
        .add_systems(
            Update,
            (
                give_health,
                hero_down.before(crate::character::hero_input),
                attach_guns,
                arm_soldiers,
                choose_gun,
                reload,
                fire,
                hero_arms,
                throw_grenade,
                soldiers_fire,
                deaths,
                tracers,
                combat_hud,
            )
                .chain()
                .after(crate::world::GameplaySourcesReconciled),
        );
    }
}

/// A model in `general.blz` by its table path (`Weapons\WEA02\WEA02_LOD1-WEA02.rbm`).
fn general_model(assets: &AssetServer, path: &str) -> Option<Handle<RbmAsset>> {
    let file = jc2_formats::placement::bare_name(path).to_ascii_lowercase();
    file.ends_with(".rbm")
        .then(|| assets.load::<RbmAsset>(format!("jc2://general.blz/{file}")))
}

fn fallback_weapon(name: &str) -> Weapon {
    Weapon {
        id: 2,
        name: name.into(),
        short_name: String::new(),
        model: String::new(),
        damage: 19.0,
        range: 100.0,
        velocity: 750.0,
        pellets: 1,
        force: 1.0,
        accuracy: 1.0,
        aim_type: 0,
        max_bullet_spread_degrees: 0.0,
        projectile: 10,
        reload_time: 1.0,
        ammo_type: 1,
        unlimited_ammo: false,
        automatic: false,
        ai: None,
        hand: String::new(),
    }
}

fn setup_arsenal(
    mut commands: Commands,
    game: Res<Game>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let data = game.fs.read_inflated("weapons.bin").ok();
    let weapons = data
        .as_ref()
        .and_then(|d| jc2_formats::weapons::parse(d).ok())
        .unwrap_or_default();
    let throwables = data
        .as_ref()
        .and_then(|d| jc2_formats::weapons::throwables(d).ok())
        .unwrap_or_default();
    let upgrades = game
        .fs
        .read_inflated("weapon_upgrades.bin")
        .ok()
        .and_then(|d| Upgrades::parse(&d).ok())
        .unwrap_or_default();
    let max_ammo = game
        .fs
        .read_inflated("ammunition.bin")
        .ok()
        .and_then(|d| jc2_formats::weapons::ammunition(&d).ok())
        .unwrap_or_default();
    let find = |name: &str| {
        weapons
            .iter()
            .find(|w| w.name == name)
            .cloned()
            .unwrap_or_else(|| fallback_weapon(name))
    };
    let guns: Vec<Gun> = ROSTER
        .iter()
        .map(|name| {
            let weapon = find(name);
            let model = general_model(&assets, &weapon.model);
            gun_from(weapon, &upgrades, &max_ammo, model, 0)
        })
        .collect();
    // every hand-held gun the soldiers might carry (the ones with an AI block), and the
    // fallback rifle even without the table
    let mut soldier_guns: Vec<SoldierGun> = weapons
        .iter()
        .filter(|w| w.ai.is_some())
        .map(|w| soldier_gun_from(w.clone(), &upgrades))
        .collect();
    if !soldier_guns
        .iter()
        .any(|g| g.weapon.name == FALLBACK_SOLDIER_WEAPON)
    {
        soldier_guns.push(soldier_gun_from(find(FALLBACK_SOLDIER_WEAPON), &upgrades));
    }
    let soldier_index: HashMap<String, usize> = soldier_guns
        .iter()
        .enumerate()
        .map(|(i, g)| (g.weapon.name.clone(), i))
        .collect();
    let grenade = throwables
        .iter()
        .find(|t| t.id == "WEA33")
        .cloned()
        .unwrap_or(Throwable {
            id: "WEA33".into(),
            class: "CHandGrenade".into(),
            model: "Weapons\\WEA33\\WEA33_LOD1-WEA33.rbm".into(),
            damage: 500.0,
            impulse: 1000.0,
            range: 5.0,
            detonation_time: 3.5,
            speed: 40.0,
            max_speed: 25.0,
            max_count: 10,
        });
    let max_grenades = if grenade.max_count > 0 {
        grenade.max_count
    } else {
        10
    };
    let hero_health = game
        .fs
        .read_inflated("player.bin")
        .ok()
        .and_then(|d| jc2_formats::property::PropertyFile::parse_any(&d).ok())
        .and_then(|f| {
            let mut hp = None;
            for n in &f.nodes {
                n.walk(&mut |n| {
                    if hp.is_none() {
                        hp = n.float("health");
                    }
                });
            }
            hp
        })
        .unwrap_or(400.0);
    let vehicle_health: HashMap<String, f32> = game
        .fs
        .read_inflated("vehicletypes.bin")
        .ok()
        .and_then(|d| jc2_formats::types::vehicle_types(&d).ok())
        .unwrap_or_default()
        .iter()
        .map(|t| (t.container().to_ascii_lowercase(), t.health))
        .collect();
    info!(
        "arsenal: {}; grenade {} damage over {} m, {} carried; {} soldier weapons; rico {} hp; {} vehicle types",
        guns.iter()
            .map(|g| format!(
                "{} {} dmg {}/{} every {} s",
                g.weapon.name, g.damage, g.ammo.magazine, g.ammo.reserve, g.interval
            ))
            .collect::<Vec<_>>()
            .join(", "),
        grenade.damage,
        grenade.range,
        max_grenades,
        soldier_guns.len(),
        hero_health,
        vehicle_health.len()
    );
    let unlit = |c: Color| StandardMaterial {
        base_color: c,
        emissive: c.to_linear() * 8.0,
        unlit: true,
        ..default()
    };
    info!(
        "accuracy: regular player recovery/loss and two-stage scatter; native AI, assisted-target and weapon-modifier overrides are not implemented by this path; grapple/parachute/stunt riding retain existing scatter"
    );
    commands.insert_resource(Arsenal {
        guns,
        current: 0,
        grenade_model: general_model(&assets, &grenade.model),
        grenade,
        grenades: max_grenades,
        max_grenades,
        table: weapons,
        upgrades,
        max_ammo,
        levels: HashMap::new(),
        soldier_guns,
        soldier_index,
        hero_health,
        vehicle_health,
        cooldown: 0.0,
        grenade_cooldown: 0.0,
        rng: 0x9e37_79b9,
        player_accuracy: PlayerAccuracy::default(),
        accuracy_recovery: game
            .fs
            .read_inflated("player.bin")
            .ok()
            .and_then(|d| jc2_formats::weapons::player_accuracy_recovery(&d).ok())
            .filter(|rate| rate.is_finite() && *rate >= 0.0)
            .unwrap_or(0.5),
        shot_random: ShotRandom::default(),
        tracer_mesh: meshes.add(Cylinder::new(1.0, 1.0)),
        tracer_hot: materials.add(unlit(Color::srgb(1.0, 0.85, 0.4))),
        tracer_enemy: materials.add(unlit(Color::srgb(1.0, 0.45, 0.3))),
        flash_mesh: meshes.add(flash_mesh()),
        flash_material: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.8, 0.45, 0.9),
            emissive: LinearRgba::rgb(14.0, 8.0, 2.5),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: None,
            double_sided: true,
            ..default()
        }),
        soldier_models: HashMap::new(),
        hits: 0,
        kills: 0,
        down_for: None,
        shots: 0,
        enemy_shots: Vec::new(),
        since_hurt: 99.0,
    });
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        // on foot: above Rico's two-line HUD in the bottom left (`combat_hud` moves it to
        // the bottom right in a vehicle, whose HUD takes the left)
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(HUD_ABOVE_HERO),
            ..default()
        },
        HealthHud,
        crate::hud::DevText,
    ));
}

/// Pixels from the bottom for the combat HUD on foot: clear of Rico's two lines.
const HUD_ABOVE_HERO: f32 = 56.0;

/// Rico, the soldiers and the vehicles get their health when they appear.
#[allow(clippy::type_complexity)]
fn give_health(
    mut commands: Commands,
    arsenal: Option<Res<Arsenal>>,
    heroes: Query<Entity, (With<Hero>, Without<Health>)>,
    npcs: Query<(Entity, &Npc, Option<&Soldier>), Without<Health>>,
    vehicles: Query<(Entity, &Container), Without<Health>>,
) {
    let Some(a) = arsenal else { return };
    for e in &heroes {
        commands.entity(e).insert(Health::new(a.hero_health));
    }
    for (e, n, soldier) in &npcs {
        let (hp, gun) = match soldier {
            Some(s) => (s.health, a.soldier_gun(&s.weapon)),
            None => (NPC_HEALTH, a.soldier_gun(FALLBACK_SOLDIER_WEAPON)),
        };
        let ammo = a
            .soldier_guns
            .get(gun)
            .map(|g| g.ammo.clone())
            .unwrap_or_else(|| WeaponState::new(20, 0, 1.0));
        commands.entity(e).insert((
            Health::new(hp),
            Gunner {
                gun,
                ammo,
                next: 1.5 + (n.facing.abs() % 1.0),
                burst_left: 0,
            },
        ));
    }
    for (e, c) in &vehicles {
        let hp = a.vehicle_health.get(&c.0).copied().unwrap_or(2000.0);
        commands.entity(e).insert(Health::new(hp));
    }
}

/// Rico's guns as their models arrive: the one in use in his right hand, the others on
/// his back and thighs (`poses::stowage`), moved between the two as he draws and stows.
#[allow(clippy::type_complexity)]
pub(crate) fn attach_guns(
    mut commands: Commands,
    arsenal: Option<Res<Arsenal>>,
    mounts: Res<WeaponMountTable>,
    rbms: Res<Assets<RbmAsset>>,
    mut heroes: Query<(Entity, &Animator, Option<&Arms>, Option<&mut Holster>), With<Hero>>,
    mut models: Query<(Entity, &mut GunModel, &mut Transform, &mut Visibility), Without<Hero>>,
) {
    let Some(a) = arsenal else { return };
    let Ok((hero, an, arms, holster)) = heroes.single_mut() else {
        return;
    };
    let gait = arms.map_or(Gait::Idle, |a| a.gait);
    let classes: Vec<WeaponClass> = a.guns.iter().map(|g| g.class).collect();
    let stows = stowage(&classes, a.current, gait);
    let skeleton = &an.rig.skeleton;
    // the native bone slot of a stow, only on a skeleton ordered like biped.bsk
    let mount = |stow: Stow, slot: usize| {
        let m = a.guns.get(slot).and_then(|g| mounts.0.get(&g.weapon.name));
        let (_, tf) = stow.mount(m)?;
        let b = crate::weapon_attach::resolve_bone(skeleton, stow.bone()?)?;
        Some((an.joints[b], tf))
    };
    for (e, mut gm, mut tf, mut vis) in &mut models {
        let want = stows.get(gm.slot).copied().unwrap_or(Stow::Hidden);
        if want == gm.stow {
            continue;
        }
        gm.stow = want;
        match mount(want, gm.slot) {
            Some((joint, at)) => {
                commands.entity(e).insert(ChildOf(joint));
                gm.base = at;
                *tf = at;
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    let mut done = match holster {
        Some(h) => h.0.clone(),
        None => Vec::new(),
    };
    let before = done.clone();
    // guns picked up since get their slot
    done.resize(a.guns.len(), false);
    for (i, g) in a.guns.iter().enumerate() {
        if done[i] {
            continue;
        }
        let Some(rbm) = g.model.as_ref().and_then(|h| rbms.get(h)) else {
            continue;
        };
        let stow = stows.get(i).copied().unwrap_or(Stow::Hidden);
        let (parent, at, vis) = match mount(stow, i) {
            Some((joint, at)) => (joint, at, Visibility::Inherited),
            None => (hero, Transform::IDENTITY, Visibility::Hidden),
        };
        let own = mounts.0.get(&g.weapon.name);
        let root = commands
            .spawn((
                at,
                vis,
                GunModel {
                    slot: i,
                    stow,
                    muzzle: emission_point(own, rbm.bbox_min, rbm.bbox_max),
                    base: at,
                },
                Name::new(g.weapon.name.clone()),
                ChildOf(parent),
            ))
            .id();
        for b in &rbm.blocks {
            commands.spawn((
                Mesh3d(b.mesh.clone()),
                b.material_component(),
                Transform::IDENTITY,
                ChildOf(root),
            ));
        }
        done[i] = true;
    }
    if done != before || before.is_empty() {
        commands.entity(hero).insert(Holster(done));
    }
}

/// Soldiers get their gun's model in the right hand once it has loaded, and the
/// `Arms` the aim, recoil and reload layers follow (`npc.rs`).
#[allow(clippy::type_complexity)]
fn arm_soldiers(
    mut commands: Commands,
    arsenal: Option<ResMut<Arsenal>>,
    assets: Res<AssetServer>,
    mounts: Res<WeaponMountTable>,
    rbms: Res<Assets<RbmAsset>>,
    soldiers: Query<
        (Entity, &Gunner, &Animator),
        (With<Npc>, Without<SoldierArmed>, Without<Dead>),
    >,
) {
    let Some(mut a) = arsenal else { return };
    for (e, gunner, an) in &soldiers {
        let Some(sg) = a.soldier_guns.get(gunner.gun) else {
            continue;
        };
        let arms = Arms {
            class: Some(sg.class),
            interval: sg.interval,
            reload_time: sg.weapon.reload_time,
            ..default()
        };
        let path = sg.weapon.model.clone();
        let weapon_name = sg.weapon.name.clone();
        let handle = a
            .soldier_models
            .entry(gunner.gun)
            .or_insert_with(|| general_model(&assets, &path))
            .clone();
        let hand = an.rig.skeleton.bone_index("AttachHandRight");
        let (Some(h), Some(hand)) = (handle, hand) else {
            // no model to show (a vehicle gun's .lod): the poses all the same
            commands.entity(e).insert((SoldierArmed, arms));
            continue;
        };
        let Some(rbm) = rbms.get(&h) else {
            if matches!(
                assets.get_load_state(h.id()),
                Some(bevy::asset::LoadState::Failed(_))
            ) {
                commands.entity(e).insert((SoldierArmed, arms));
            }
            continue;
        };
        // held on AttachHandRight like Rico's (0x005e3100); no stow or support IK for
        // soldiers: their holder ownership is unproven (`weapon_attach::UNSUPPORTED`)
        let own = mounts.0.get(&weapon_name);
        let root = commands
            .spawn((
                crate::weapon_attach::bone_local(crate::weapon_attach::ATTACH_HAND_RIGHT, own),
                Visibility::default(),
                SoldierGunModel {
                    muzzle: emission_point(own, rbm.bbox_min, rbm.bbox_max),
                },
                Name::new("soldier gun"),
                ChildOf(an.joints[hand]),
            ))
            .id();
        for b in &rbm.blocks {
            commands.spawn((
                Mesh3d(b.mesh.clone()),
                b.material_component(),
                Transform::IDENTITY,
                ChildOf(root),
            ));
        }
        commands
            .entity(e)
            .insert((SoldierArmed, arms, SoldierGunOf(root)));
    }
}

/// Rico's arms follow his gun: the class in hand, the aim (the right button, or for a
/// while after a shot) at what the crosshair is on, and the reloads as they start.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hero_arms(
    buttons: Res<ButtonInput<MouseButton>>,
    mode: Res<CameraMode>,
    arsenal: Option<Res<Arsenal>>,
    spatial: SpatialQuery,
    mut heroes: Query<(Entity, &Hero, &Transform, &mut Arms), Without<Dead>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    mut was_reloading: Local<bool>,
) {
    let Some(a) = arsenal else { return };
    let Ok((hero, h, tf, mut arms)) = heroes.single_mut() else {
        return;
    };
    let g = a.gun();
    arms.class = Some(g.class);
    arms.interval = g.interval;
    let reloading = g.ammo.reloading;
    if let Some(t) = reloading
        && !*was_reloading
    {
        arms.reloads += 1;
        arms.reload_time = t;
    }
    *was_reloading = reloading.is_some();
    let on_foot = *mode == CameraMode::Hero && h.in_vehicle.is_none();
    if on_foot && buttons.pressed(MouseButton::Right) {
        arms.aim_for = arms.aim_for.max(0.2);
    }
    if arms.aim_for > 0.0
        && let Ok(cam) = cams.single()
    {
        arms.aim_dir = aim_direction(&spatial, hero, tf, cam, g.weapon.range);
    }
}

/// From his chest to what the crosshair is on (a far point when nothing is).
fn aim_direction(
    spatial: &SpatialQuery,
    hero: Entity,
    tf: &Transform,
    cam: &Transform,
    range: f32,
) -> Vec3 {
    let reach = range + cam.translation.distance(tf.translation);
    let filter = SpatialQueryFilter::from_excluded_entities([hero]);
    let point = spatial
        .cast_ray(cam.translation, cam.forward(), reach, true, &filter)
        .map_or(cam.translation + cam.forward() * reach, |h| {
            cam.translation + cam.forward() * h.distance
        });
    let chest = tf.translation + Vec3::Y * 0.5;
    let d = point - chest;
    // a target right under his nose would aim the gun at his feet: keep the camera's
    if d.length() < 2.0 {
        cam.forward().as_vec3()
    } else {
        d.normalize()
    }
}

/// The muzzle flash: a star of three crossed quads along −z (scaled per gun).
fn flash_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for k in 0..3 {
        let a = k as f32 * std::f32::consts::PI / 3.0;
        let side = Vec3::new(a.cos(), a.sin(), 0.0) * 0.07;
        let base = pos.len() as u32;
        pos.extend([
            (-side).to_array(),
            side.to_array(),
            (side * 0.3 + Vec3::NEG_Z * 0.3).to_array(),
            (-side * 0.3 + Vec3::NEG_Z * 0.3).to_array(),
        ]);
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let n = pos.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; n])
    .with_inserted_indices(Indices::U32(idx))
}

/// A flash at a muzzle, pointing along `dir`, with a brief light.
fn spawn_flash(
    commands: &mut Commands,
    a: &Arsenal,
    at: Vec3,
    dir: Vec3,
    size: f32,
    mount: Option<(Entity, Vec3)>,
) {
    let rot = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize_or(Vec3::NEG_Z));
    let transform = match mount {
        // A gun child follows this frame's animated hand when transforms propagate.
        Some((_, local_muzzle)) => Transform::from_translation(local_muzzle)
            .with_rotation(Quat::from_rotation_z(at.x * 37.0))
            .with_scale(Vec3::splat(size)),
        None => Transform::from_translation(at)
            .with_rotation(rot * Quat::from_rotation_z(at.x * 37.0))
            .with_scale(Vec3::splat(size)),
    };
    let flash = commands
        .spawn((
            Mesh3d(a.flash_mesh.clone()),
            MeshMaterial3d(a.flash_material.clone()),
            transform,
            Tracer(FLASH_SECONDS),
            bevy::light::NotShadowCaster,
            Name::new("muzzle flash"),
        ))
        .with_child((
            PointLight {
                color: Color::srgb(1.0, 0.75, 0.4),
                intensity: 40_000.0,
                range: 8.0,
                shadow_maps_enabled: false,
                ..default()
            },
            // scaled each frame by the native per-hour point-light factor (atmosphere.rs)
            crate::atmosphere::PointLightBase(40_000.0),
        ))
        .id();
    if let Some((gun, _)) = mount {
        commands.entity(flash).insert(ChildOf(gun));
    }
}

const FLASH_SECONDS: f32 = 0.05;

/// The muzzle of the gun in Rico's hand and the way its barrel points, when its model
/// is there.
fn gun_muzzle(
    a: &Arsenal,
    guns: &Query<(Entity, &GunModel, &GlobalTransform)>,
) -> Option<(Vec3, Vec3, Entity, Vec3)> {
    guns.iter()
        .find(|(_, g, _)| g.slot == a.current && g.stow == Stow::Hand)
        .map(|(e, g, gt)| {
            // V in the weapon's world; the direction is that frame's -Z row
            let frame = crate::weapon_attach::emission_frame(&gt.compute_transform(), g.muzzle);
            (frame.origin, frame.direction, e, g.muzzle)
        })
}

fn choose_gun(keys: Res<ButtonInput<KeyCode>>, arsenal: Option<ResMut<Arsenal>>) {
    let Some(mut a) = arsenal else { return };
    if a.down_for.is_some() {
        return;
    }
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) && i < a.guns.len() && a.current != i {
            // a reload under way is dropped with the gun
            let old = a.current;
            a.guns[old].ammo.reloading = None;
            a.current = i;
            a.cooldown = a.cooldown.max(0.3);
            info!("rico: {}", a.guns[i].weapon.name);
        }
    }
}

/// The gun in hand reloads: by itself when the magazine is empty, or on `R` (on foot;
/// in a car `R` rights it, in the free camera it goes back to the start).
fn reload(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    arsenal: Option<ResMut<Arsenal>>,
    heroes: Query<&Hero>,
) {
    let Some(mut a) = arsenal else { return };
    if a.down_for.is_some() {
        return;
    }
    let on_foot =
        *mode == CameraMode::Hero && heroes.single().is_ok_and(|h| h.in_vehicle.is_none());
    let i = a.current;
    let ammo = &mut a.guns[i].ammo;
    if on_foot && keys.just_pressed(KeyCode::KeyR) && ammo.start_reload() {
        info!("rico: reloading");
    }
    ammo.tick(time.delta_secs());
}

fn spawn_tracer(commands: &mut Commands, a: &Arsenal, from: Vec3, to: Vec3, enemy: bool) {
    commands.spawn((
        Mesh3d(a.tracer_mesh.clone()),
        MeshMaterial3d(if enemy {
            a.tracer_enemy.clone()
        } else {
            a.tracer_hot.clone()
        }),
        crate::character::segment_transform(from, to, 0.012),
        Tracer(TRACER_SECONDS),
    ));
}

/// Apply a hit: damage what has health, push what moves. True when it killed.
fn hit(
    entity: Entity,
    point: Vec3,
    dir: Vec3,
    damage: f32,
    push: f32,
    healths: &mut Query<&mut Health, Without<Dead>>,
    bodies: &mut Query<(&RigidBody, Forces)>,
) -> bool {
    let mut killed = false;
    if let Ok(mut h) = healths.get_mut(entity) {
        let was = h.hp;
        h.hp -= damage;
        killed = was > 0.0 && h.hp <= 0.0;
    }
    // only what the physics moves (a kinematic soldier would fly off at the velocity)
    if let Ok((body, mut f)) = bodies.get_mut(entity)
        && body.is_dynamic()
    {
        f.apply_linear_impulse_at_point(dir * push, point);
    }
    killed
}

/// Where Rico's gun is, roughly: right hand, chest high.
fn muzzle(tf: &Transform, h: &Hero) -> Vec3 {
    // his right hand, where the gun is (`AttachHandRight`): +x right, −z ahead
    tf.translation + Quat::from_rotation_y(h.facing) * Vec3::new(0.3, 0.5, -0.5)
}

type FireHero = (
    Entity,
    &'static Hero,
    &'static Transform,
    Option<&'static mut Arms>,
    Option<&'static OnStunt>,
);

#[allow(clippy::too_many_arguments)]
pub(crate) fn fire(
    mut commands: Commands,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    mode: Res<CameraMode>,
    arsenal: Option<ResMut<Arsenal>>,
    fx: Option<Res<Fx>>,
    spatial: SpatialQuery,
    mut heroes: Query<FireHero, Without<Dead>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    guns: Query<(Entity, &GunModel, &GlobalTransform)>,
    colliders: Query<&ColliderOf>,
    mut healths: Query<&mut Health, Without<Dead>>,
    mut bodies: Query<(&RigidBody, Forces)>,
    pieces: Query<&Piece>,
    mut structures: ResMut<StructureHits>,
) {
    let Some(mut a) = arsenal else { return };
    a.cooldown -= time.delta_secs();
    let automatic = a.gun().automatic;
    let trigger = if automatic {
        buttons.pressed(MouseButton::Left)
    } else {
        buttons.just_pressed(MouseButton::Left)
    };
    let Ok((hero, h, tf, arms, stunt)) = heroes.single_mut() else {
        return;
    };
    let regular_handheld = h.in_vehicle.is_none()
        && h.grapple.is_none()
        && !h.parachute
        && !h.swimming
        && h.stunt.is_none()
        && stunt.is_none();
    let maximum = accuracy::regular_player_maximum(&a.gun().weapon);
    let recovery = a.accuracy_recovery;
    if regular_handheld {
        a.player_accuracy
            .update(hero, time.delta_secs(), recovery, maximum);
    }
    if *mode != CameraMode::Hero || !trigger || a.cooldown > 0.0 {
        return;
    }
    // hanging from the hook (as in the original) or under the chute one hand is free:
    // one-handed guns only; reeling needs the hand
    let one_hand_free = h.grapple.is_some() || h.parachute;
    if h.in_vehicle.is_some()
        || h.swimming
        || (h.grapple.is_some() && !h.hanging)
        || (one_hand_free && !a.gun().class.one_handed())
    {
        return;
    }
    let Ok(cam) = cams.single() else { return };
    let i = a.current;
    if !a.guns[i].ammo.fire() {
        // empty or reloading: the reload system refills it
        return;
    }
    a.cooldown = a.gun().interval;
    a.shots += 1;
    let w = a.gun().weapon.clone();
    let damage = a.gun().damage;
    let upgrade = a.upgrades.get(&w.name, a.gun().level);
    let angle = upgrade.map_or(0.0, |u| u.aim_spread_degrees.to_radians())
        * (1.0 - a.player_accuracy.value);
    let shot_aim = if regular_handheld {
        a.shot_random.aim_direction(cam.forward().as_vec3(), angle)
    } else {
        cam.forward().as_vec3()
    };
    // Native006ec6e0 decrements after aim sampling, once before the pellet loop.
    if regular_handheld {
        a.player_accuracy
            .fired(upgrade.map_or(0.0, |u| u.accuracy_loss), maximum);
    }
    let aim_frame = accuracy::aim_frame(shot_aim);
    let gun_origin = gun_muzzle(&a, &guns);
    let (muzzle, barrel) = gun_origin
        .map(|(at, dir, _, _)| (at, dir))
        .unwrap_or((muzzle(tf, h), Quat::from_rotation_y(h.facing) * Vec3::NEG_Z));
    if let Some(mut arms) = arms {
        arms.shots += 1;
        arms.aim_for = AIM_AFTER_SHOT;
        arms.aim_dir = aim_direction(&spatial, hero, tf, cam, w.range);
    }
    let size = match a.gun().class {
        WeaponClass::Pistol => 0.9,
        WeaponClass::Smg => 1.0,
        WeaponClass::Rpg => 2.2,
        _ => 1.4,
    };
    spawn_flash(
        &mut commands,
        &a,
        muzzle,
        barrel,
        size,
        gun_origin.map(|(_, _, gun, local)| (gun, local)),
    );
    let filter = SpatialQueryFilter::from_excluded_entities([hero]);
    let reach = w.range + cam.translation.distance(tf.translation);
    if w.projectile == ROCKET || w.projectile == LOBBED {
        // aim the round from the muzzle at what the crosshair is on
        let aim = spatial
            .cast_ray(
                cam.translation,
                Dir3::new_unchecked(shot_aim),
                reach,
                true,
                &filter,
            )
            .map_or(cam.translation + shot_aim * reach, |h| {
                cam.translation + shot_aim * h.distance
            });
        let base = (aim - muzzle).normalize_or(cam.forward().as_vec3());
        let dir = if regular_handheld {
            a.shot_random.pellet_direction(
                accuracy::aim_frame(base),
                w.max_bullet_spread_degrees.to_radians(),
            )
        } else {
            base
        };
        let Some(fx) = fx else { return };
        commands.spawn((
            Mesh3d(fx.rocket.clone()),
            MeshMaterial3d(fx.rocket_material.clone()),
            Transform::from_translation(muzzle + dir * 0.6)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir)),
            Projectile {
                velocity: dir * w.velocity,
                gravity: if w.projectile == LOBBED { 9.81 } else { 0.0 },
                life: w.range / w.velocity.max(1.0) + 0.5,
                blast: Blast {
                    at: Vec3::ZERO,
                    radius: a.grenade.range,
                    damage,
                    impulse: a.grenade.impulse,
                },
                owner: hero,
            },
            Name::new("rocket"),
        ));
        return;
    }
    for _ in 0..w.pellets {
        let direction = if regular_handheld {
            a.shot_random
                .pellet_direction(aim_frame, w.max_bullet_spread_degrees.to_radians())
        } else {
            // Preserve existing special-state approximation until native modifier ownership is proven.
            let spread =
                if w.pellets > 1 { 0.08 } else { 0.012 } * (2.0 - w.accuracy.clamp(0.0, 1.0));
            cam.forward().as_vec3()
                + Vec3::new(a.rand() - 0.5, a.rand() - 0.5, a.rand() - 0.5) * spread
        };
        let Ok(dir) = Dir3::new(direction) else {
            continue;
        };
        // from the camera, through what is in front of Rico
        let end = match spatial.cast_ray(cam.translation, dir, reach, true, &filter) {
            Some(hit_data) => {
                let p = cam.translation + dir * hit_data.distance;
                let target = body_of(hit_data.entity, &colliders);
                if healths.contains(target) {
                    a.hits += 1;
                }
                if let Ok(piece) = pieces.get(target) {
                    structures.queue.push((*piece, damage));
                }
                if hit(
                    target,
                    p,
                    dir.as_vec3(),
                    damage,
                    w.force * 60.0,
                    &mut healths,
                    &mut bodies,
                ) {
                    a.kills += 1;
                }
                p
            }
            None => cam.translation + dir * reach,
        };
        spawn_tracer(&mut commands, &a, muzzle, end, false);
    }
}

/// `G`: a grenade, thrown along the aim with a little loft, on its fuse.
#[allow(clippy::too_many_arguments)]
fn throw_grenade(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<CameraMode>,
    arsenal: Option<ResMut<Arsenal>>,
    fx: Option<Res<Fx>>,
    rbms: Res<Assets<RbmAsset>>,
    mut heroes: Query<(&Hero, &Transform, &LinearVelocity, Option<&mut Arms>), Without<Dead>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
) {
    let Some(mut a) = arsenal else { return };
    a.grenade_cooldown -= time.delta_secs();
    if *mode != CameraMode::Hero
        || !keys.just_pressed(KeyCode::KeyG)
        || a.grenade_cooldown > 0.0
        || a.grenades == 0
    {
        return;
    }
    let (Ok((h, tf, lin, arms)), Ok(cam), Some(fx)) = (heroes.single_mut(), cams.single(), fx)
    else {
        return;
    };
    if h.in_vehicle.is_some() {
        return;
    }
    if let Some(mut arms) = arms {
        arms.throws += 1;
    }
    a.grenade_cooldown = GRENADE_INTERVAL;
    a.grenades -= 1;
    let g = &a.grenade;
    // `speed` is the throw; `max_speed` caps it — a lob, not a pitch
    let speed = g.speed.min(g.max_speed) * 0.7;
    let dir = (cam.forward().as_vec3() + Vec3::Y * 0.35).normalize();
    let at = muzzle(tf, h) + Vec3::Y * 0.3;
    let mut e = commands.spawn((
        RigidBody::Dynamic,
        Collider::sphere(0.06),
        Mass(0.6),
        Restitution::new(0.35),
        Friction::new(0.8),
        LinearVelocity(lin.0 + dir * speed),
        AngularVelocity(Vec3::new(6.0, 0.0, 3.0)),
        Transform::from_translation(at),
        Visibility::default(),
        Grenade {
            fuse: g.detonation_time,
            blast: Blast {
                at: Vec3::ZERO,
                radius: g.range,
                damage: g.damage,
                impulse: g.impulse,
            },
        },
        Name::new("grenade"),
    ));
    match a.grenade_model.as_ref().and_then(|m| rbms.get(m)) {
        Some(rbm) => {
            for b in &rbm.blocks {
                e.with_child((Mesh3d(b.mesh.clone()), b.material_component()));
            }
        }
        None => {
            e.insert((
                Mesh3d(fx.sphere.clone()),
                MeshMaterial3d(fx.rocket_material.clone()),
            ));
        }
    }
}

/// Soldiers who see Rico turn to him and shoot — at him, or at what he drives.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn soldiers_fire(
    mut commands: Commands,
    time: Res<Time>,
    arsenal: Option<ResMut<Arsenal>>,
    spatial: SpatialQuery,
    colliders: Query<&ColliderOf>,
    mut soldiers: Query<
        (
            Entity,
            &mut Npc,
            &Transform,
            &mut Gunner,
            Option<&mut Arms>,
            Option<&SoldierGunOf>,
        ),
        (Without<Dead>, Without<crate::npc::Friendly>),
    >,
    soldier_guns: Query<(&SoldierGunModel, &GlobalTransform)>,
    mut heroes: Query<(Entity, &Hero, &Transform, &mut Health), (Without<Npc>, Without<Container>)>,
    mut rides: Query<
        (&GlobalTransform, &mut Health),
        (With<Container>, Without<Hero>, Without<Npc>),
    >,
) {
    let Some(mut a) = arsenal else { return };
    let Ok((hero_e, hero, htf, mut hh)) = heroes.single_mut() else {
        return;
    };
    if hh.hp <= 0.0 {
        return;
    }
    let (target_e, target) = match hero.in_vehicle {
        Some(v) => match rides.get(v) {
            Ok((t, _)) => (v, t.translation() + Vec3::Y * 0.5),
            Err(_) => return,
        },
        None => (hero_e, htf.translation + Vec3::Y * 0.4),
    };
    let dt = time.delta_secs();
    for (e, mut npc, tf, mut gun, mut arms, gun_of) in &mut soldiers {
        let was_reloading = gun.ammo.reloading.is_some();
        gun.ammo.tick(dt);
        if let Some(arms) = arms.as_mut()
            && !was_reloading
            && let Some(t) = gun.ammo.reloading
        {
            arms.reloads += 1;
            arms.reload_time = t;
        }
        if gun.ammo.reloading.is_some() {
            gun.burst_left = 0;
        }
        let eye = tf.translation + Vec3::Y * 0.6;
        let to = target - eye;
        let d = to.length();
        let Some(sg) = a.soldier_guns.get(gun.gun) else {
            continue;
        };
        let (sight, interval, damage, pellets) = (
            sg.ai.max_range,
            sg.interval,
            sg.weapon.damage,
            sg.weapon.pellets.max(1),
        );
        let (lo, hi) = (sg.ai.min_burst_delay, sg.ai.max_burst_delay);
        let (blo, bhi) = (sg.ai.min_burst_length, sg.ai.max_burst_length);
        if d > sight {
            npc.alert = false;
            continue;
        }
        // line of sight
        let Ok(dir) = Dir3::new(to) else { continue };
        let filter = SpatialQueryFilter::from_excluded_entities([e]);
        let seen = spatial
            .cast_ray(eye, dir, d + 3.0, true, &filter)
            .is_some_and(|h| body_of(h.entity, &colliders) == target_e);
        if !seen {
            continue;
        }
        npc.alert = true;
        npc.facing = yaw_toward(to);
        if let Some(arms) = arms.as_mut() {
            arms.aim_for = arms.aim_for.max(0.5);
            arms.aim_dir = to / d.max(1e-3);
        }
        gun.next -= dt;
        if gun.next > 0.0 {
            continue;
        }
        if !gun.ammo.fire() {
            // reloading: try again shortly
            gun.next = 0.1;
            continue;
        }
        if gun.burst_left == 0 {
            // a new burst: as many shots as its length allows, at least one
            let len = blo + (bhi - blo) * a.rand();
            gun.burst_left = ((len / interval).round() as u32).max(1);
        }
        let in_burst = gun.burst_left as f32;
        gun.burst_left -= 1;
        gun.next = if gun.burst_left > 0 {
            interval
        } else {
            lo + (hi - lo) * a.rand()
        };
        let chance =
            NPC_HIT_CHANCE * (1.0 - d / sight * 0.7) * (HITS_PER_BURST / in_burst).min(1.0);
        // every pellet rolls
        let hits = (0..pellets).filter(|_| a.rand() < chance).count() as f32;
        let hit_it = hits > 0.0;
        let aim = if hit_it {
            target
        } else {
            target + Vec3::new(a.rand() - 0.5, a.rand() * 0.8, a.rand() - 0.5) * 2.5
        };
        if hit_it {
            if target_e == hero_e {
                hh.hp -= damage * hits;
                a.since_hurt = 0.0;
            } else if let Ok((_, mut vh)) = rides.get_mut(target_e) {
                vh.hp -= damage * hits;
            }
        }
        let gun_origin = gun_of.and_then(|g| {
            soldier_guns
                .get(g.0)
                .ok()
                .map(|(model, gt)| (g.0, model, gt))
        });
        let (muzzle, barrel) = gun_origin
            .map(|(_, g, gt)| (g, gt))
            .map(|(g, gt)| (gt.transform_point(g.muzzle), gt.rotation() * Vec3::NEG_Z))
            .unwrap_or((
                eye + Quat::from_rotation_y(npc.facing) * Vec3::new(0.25, 0.0, -0.5),
                to / d.max(1e-3),
            ));
        if let Some(arms) = arms.as_mut() {
            arms.shots += 1;
        }
        spawn_flash(
            &mut commands,
            &a,
            muzzle,
            barrel,
            1.2,
            gun_origin.map(|(gun, model, _)| (gun, model.muzzle)),
        );
        spawn_tracer(&mut commands, &a, muzzle, aim, true);
        if a.enemy_shots.len() < 32 {
            a.enemy_shots.push(muzzle);
        }
    }
}
/// Soldiers at zero health fall and stay down a while.
#[allow(clippy::type_complexity)]
fn deaths(
    mut commands: Commands,
    time: Res<Time>,
    fresh: Query<(Entity, &Health, &Children), (With<Npc>, Without<Dead>)>,
    mut dead: Query<(Entity, &mut Dead, &Children), With<Npc>>,
    mut models: Query<&mut Transform, Without<Npc>>,
    mut animators: Query<&mut Animator>,
) {
    for (e, h, _) in &fresh {
        if h.hp <= 0.0 {
            commands
                .entity(e)
                .insert(Dead(0.0))
                .remove::<(Collider, RigidBody)>();
            if let Ok(mut an) = animators.get_mut(e) {
                an.rate = 1.0;
                an.play(Clip::Fall);
            }
        }
    }
    let dt = time.delta_secs();
    for (e, mut d, children) in &mut dead {
        d.0 += dt;
        // topple backwards (towards +z, the model faces −z) over half a second, then lie
        let k = (d.0 / 0.5).min(1.0);
        for c in children.iter() {
            if let Ok(mut m) = models.get_mut(c) {
                let yaw = m.rotation.to_euler(EulerRot::YXZ).0;
                m.rotation = Quat::from_rotation_y(yaw)
                    * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2 * k);

                m.translation.y = -HEIGHT / 2.0 + 0.15 * k;
            }
        }
        if d.0 > CORPSE_SECONDS {
            commands.entity(e).despawn();
        }
    }
}

/// Rico down: he lies there, then gets up at the start, healed. Dead he is no longer in a
/// vehicle or on its roof: he comes out beside it (the way `E` takes him out) and the
/// vehicle is left empty.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn hero_down(
    mut commands: Commands,
    time: Res<Time>,
    arsenal: Option<ResMut<Arsenal>>,
    spawn: Option<Res<SpawnPoint>>,
    ragdoll: Option<Res<crate::ragdoll::RagdollAsset>>,
    mut mode: ResMut<CameraMode>,
    mut heroes: Query<(
        Entity,
        &mut Health,
        &mut Transform,
        &mut LinearVelocity,
        &mut Hero,
        &mut Position,
        &mut Visibility,
        Option<&mut crate::grapple::GrappleRig>,
        Option<&crate::stunts::WireSwing>,
        Has<crate::stunts::StuntTransfer>,
    )>,
    cars: Query<
        (
            &Transform,
            &LinearVelocity,
            Option<&Enterable>,
            Option<&AngularVelocity>,
            Option<&ComputedCenterOfMass>,
        ),
        Without<Hero>,
    >,
) {
    let Some(mut a) = arsenal else { return };
    let Ok((
        hero_entity,
        mut h,
        mut tf,
        mut lin,
        mut hero,
        mut pos,
        mut vis,
        mut grapple_rig,
        wire_swing,
        transferring,
    )) = heroes.single_mut()
    else {
        return;
    };
    if h.hp > 0.0 {
        // healing once out of the fight
        a.since_hurt += time.delta_secs();
        if a.since_hurt > HEAL_AFTER {
            h.hp = (h.hp + HEAL_RATE * time.delta_secs()).min(h.max);
        }
        return;
    }
    if !hero.downed && ragdoll.is_some() {
        let mut request = crate::ragdoll::PendingRagdoll {
            origin: tf.translation,
            linear: lin.0,
            angular: Vec3::ZERO,
        };
        if let Some(support) = hero.in_vehicle.or(hero.stunt)
            && let Ok((support_tf, support_lin, _, angular, center)) = cars.get(support)
        {
            request.origin = support_tf.transform_point(center.map_or(Vec3::ZERO, |c| c.0));
            request.linear = support_lin.0;
            request.angular = angular.map_or(Vec3::ZERO, |v| v.0);
        }
        if transferring {
            // Transfer velocity includes authored-frame motion at the hero COM.
            request.origin = tf.translation;
            request.linear = lin.0;
        }
        if let Some(wire) = wire_swing {
            // swing_step already gives world velocity at Rico's capsule centre,
            // including the moving pivot. Do not add vehicle translation/angular
            // motion at that centre again; only limb offsets inherit omega x r.
            request.origin = wire.pos;
            request.linear = wire.vel;
        }
        commands.entity(hero_entity).insert(request);
    }
    let t = a.down_for.get_or_insert(0.0);
    *t += time.delta_secs();
    hero.downed = true;
    commands.entity(hero_entity).insert(Dead(*t));
    // Cancel flight, pull, retract and ledge state, not only the Hero anchor.
    if let Some(rig) = grapple_rig.as_mut() {
        if let Some(pull) = rig.pull {
            commands
                .entity(pull.body)
                .try_remove::<crate::grapple::Hooked>();
        }
        **rig = crate::grapple::GrappleRig::default();
    }
    if let Some(car) = hero.in_vehicle {
        // out beside the car, with its velocity; nobody drives it now
        let (at, v) = match cars.get(car) {
            Ok((car_tf, car_lin, enterable, _, _)) => {
                commands.entity(car).remove::<Occupied>();
                let reach = enterable.map_or(1.5, |e| e.reach);
                (
                    if ragdoll.is_some() {
                        tf.translation
                    } else {
                        car_tf.translation - car_tf.rotation * Vec3::X * (reach + 1.2) + Vec3::Y
                    },
                    car_lin.0,
                )
            }
            Err(_) => (tf.translation, Vec3::ZERO),
        };
        tf.translation = at;
        pos.0 = at;
        lin.0 = v;
        release_passenger(&mut commands, hero_entity, at, v);
        *vis = Visibility::Inherited;
        hero.in_vehicle = None;
        hero.grounded = false;
        if *mode == CameraMode::Drive {
            *mode = CameraMode::Hero;
        }
        info!("rico: dead, out of the vehicle");
    }
    if hero.stunt.is_some() {
        // off the roof he stood on, at the roof's speed
        commands.entity(hero_entity).remove::<(
            OnStunt,
            crate::stunts::WireSwing,
            crate::stunts::CurrentStuntSpot,
            crate::stunts::StuntTransfer,
            crate::stunts::WireTilt,
        )>();
        release_passenger(&mut commands, hero_entity, pos.0, lin.0);
        hero.stunt = None;
        hero.grounded = false;
        info!("rico: dead, off the roof");
    }
    lin.0 = Vec3::new(0.0, lin.y.min(0.0), 0.0);
    hero.grapple = None;
    hero.parachute = false;
    hero.hanging = false;
    hero.stunt_clip = None;
    hero.swimming = false;
    hero.speed = 0.0;
    hero.aim_yaw = None;
    hero.air_time = 0.0;
    commands
        .entity(hero_entity)
        .remove::<crate::stunts::HookedVehicle>();
    *vis = Visibility::Inherited;
    if *t > RESPAWN_SECONDS
        && let Some(s) = spawn
    {
        h.hp = h.max;
        let p = s.0 + Vec3::new(3.0, HEIGHT / 2.0 + 0.3, 0.0);
        tf.translation = p;
        pos.0 = p;
        lin.0 = Vec3::ZERO;
        a.down_for = None;
        a.since_hurt = 0.0;
        hero.downed = false;
        hero.grounded = false;
        // put on the ground there, not under a base's floor (character::settle_spawn)
        commands
            .entity(hero_entity)
            .remove::<Dead>()
            .insert(crate::character::Settling::default());
        info!("rico: back on his feet at the start");
    }
}

fn tracers(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Tracer)>) {
    for (e, mut t) in &mut q {
        t.0 -= time.delta_secs();
        if t.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

fn combat_hud(
    mode: Res<CameraMode>,
    arsenal: Option<Res<Arsenal>>,
    heroes: Query<(&Hero, &Health)>,
    mut hud: Query<(&mut Text, &mut Node), With<HealthHud>>,
    heat: Option<Res<crate::heat::Heat>>,
) {
    let on_foot = heroes
        .single()
        .is_ok_and(|(h, _)| h.in_vehicle.is_none() && *mode == CameraMode::Hero);
    let (Ok((_, health)), Some(a)) = (heroes.single(), arsenal) else {
        return;
    };
    let heat = match heat {
        Some(h) if h.level() > 0 => format!("   HEAT {}", h.level()),
        _ => String::new(),
    };
    for (mut t, mut node) in &mut hud {
        // on foot above Rico's HUD on the left; in a vehicle bottom right, health only
        let (left, right, bottom) = if on_foot {
            (Val::Px(10.0), Val::Auto, Val::Px(HUD_ABOVE_HERO))
        } else {
            (Val::Auto, Val::Px(12.0), Val::Px(10.0))
        };
        if node.left != left || node.right != right || node.bottom != bottom {
            node.left = left;
            node.right = right;
            node.bottom = bottom;
        }
        **t = if health.hp <= 0.0 {
            "DOWN — back at the start in a moment".to_string()
        } else if !on_foot {
            format!("health {:3.0}{heat}", health.hp)
        } else {
            let g = a.gun();
            let ammo = if g.ammo.unlimited {
                format!("{}/unlimited", g.ammo.magazine)
            } else {
                format!("{}/{}", g.ammo.magazine, g.ammo.reserve)
            };
            format!(
                "health {:3.0}   {} {ammo}{}   [{}/{}]   grenades {}{heat}   {} hits, {} down\n\
                 LMB fire   R reload   1-{} weapon   G grenade",
                health.hp,
                g.weapon.label(),
                if g.ammo.reloading.is_some() {
                    "  reloading"
                } else if g.ammo.is_empty() {
                    "  empty"
                } else {
                    ""
                },
                a.current + 1,
                a.guns.len(),
                a.grenades,
                a.hits,
                a.kills,
                a.guns.len(),
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod downed;

    #[test]
    fn authored_accuracy_actual_fire_decrements_once_per_round_not_pellet() {
        let Some((mut app, _, spawn)) = crate::fleet::testing::app(
            &["weapons.bin", "weapon_upgrades.bin", "player.bin"],
            |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<StructureHits>()
                    .add_plugins(ExplosionPlugin)
                    .add_systems(Startup, setup_arsenal)
                    .add_systems(Update, fire);
            },
        ) else {
            return;
        };
        let hero = app
            .world_mut()
            .spawn((Hero::default(), Transform::from_translation(spawn)))
            .id();
        app.world_mut().spawn((
            Transform::from_translation(spawn + Vec3::Y * 1.0),
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
        ));
        app.world_mut().insert_resource(CameraMode::Hero);
        app.update();
        // Isolate the shot boundary, so frame recovery cannot hide duplicate decrements.
        app.world_mut().resource_mut::<Arsenal>().accuracy_recovery = 0.0;
        for gun in [0, 2, 3] {
            // pistol, multi-pellet shotgun, automatic rifle
            let mut a = app.world_mut().resource_mut::<Arsenal>();
            a.current = gun;
            a.cooldown = 0.0;
            a.player_accuracy.value = 0.8;
            let w = a.gun().weapon.clone();
            let loss = a.upgrades.get(&w.name, 0).unwrap().accuracy_loss;
            let before = a.shots;
            let mut tracers = app.world_mut().query_filtered::<Entity, With<Tracer>>();
            let count = tracers.iter(app.world()).count();
            let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            input.release(MouseButton::Left);
            input.clear();
            input.press(MouseButton::Left);
            app.update();
            let a = app.world().resource::<Arsenal>();
            assert_eq!(a.shots, before + 1);
            assert!((a.player_accuracy.value - (0.8 - loss).max(0.0)).abs() < 1e-6);
            assert_eq!(
                tracers.iter(app.world()).count() - count,
                w.pellets as usize + 1 // One muzzle-flash root also carries Tracer lifetime.
            );
            if gun == 3 {
                // Held automatic input has no just_pressed edge, but still spends one round.
                app.world_mut().resource_mut::<Arsenal>().cooldown = 0.0;
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .clear();
                app.update();
                let a = app.world().resource::<Arsenal>();
                assert_eq!(a.shots, before + 2);
                assert!((a.player_accuracy.value - (0.8 - 2.0 * loss)).abs() < 1e-6);
            }
            // A rejected dry shot cannot change accuracy or the shot counter.
            let a = app.world().resource::<Arsenal>();
            let before_dry = a.shots;
            let value = a.player_accuracy.value;
            let mut a = app.world_mut().resource_mut::<Arsenal>();
            a.cooldown = 0.0;
            a.guns[gun].ammo.magazine = 0;
            let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            input.release(MouseButton::Left);
            input.clear();
            input.press(MouseButton::Left);
            app.update();
            let a = app.world().resource::<Arsenal>();
            assert_eq!(a.shots, before_dry);
            assert_eq!(a.player_accuracy.value, value);
        }
        app.world_mut().despawn(hero);
        let replacement = app
            .world_mut()
            .spawn((Hero::default(), Transform::from_translation(spawn)))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert_eq!(app.world().resource::<Arsenal>().player_accuracy.value, 0.0);
        // Original special controller policies are unproven: do not apply ordinary loss.
        for on_roof in [false, true] {
            app.world_mut()
                .get_mut::<Hero>(replacement)
                .unwrap()
                .parachute = !on_roof;
            if on_roof {
                app.world_mut().entity_mut(replacement).insert(OnStunt {
                    vehicle: hero,
                    local: Vec3::ZERO,
                    kind: jc2_formats::vehicle::StuntKind::Roof,
                    wire: None,
                });
            }
            let mut a = app.world_mut().resource_mut::<Arsenal>();
            a.current = 0;
            a.cooldown = 0.0;
            a.guns[0].ammo.magazine = 1;
            a.player_accuracy.value = 0.37;
            let before = a.shots;
            let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            input.release(MouseButton::Left);
            input.clear();
            input.press(MouseButton::Left);
            app.update();
            let a = app.world().resource::<Arsenal>();
            assert_eq!(a.shots, before + 1);
            assert_eq!(a.player_accuracy.value, 0.37);
        }
        // A roof launcher reaches the projectile branch: preserve both direction and RNG.
        let mut a = app.world_mut().resource_mut::<Arsenal>();
        a.current = 4;
        a.cooldown = 0.0;
        a.guns[4].ammo.magazine = 1;
        a.player_accuracy.value = 0.37;
        let before = a.shots;
        let random = a.shot_random.clone();
        let weapon = a.gun().weapon.clone();
        let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        input.release(MouseButton::Left);
        input.clear();
        input.press(MouseButton::Left);
        app.update();
        let a = app.world().resource::<Arsenal>();
        assert_eq!(a.shots, before + 1);
        assert_eq!(a.player_accuracy.value, 0.37);
        assert_eq!(a.shot_random, random);
        let mut projectiles = app.world_mut().query::<&Projectile>();
        let projectile = projectiles.single(app.world()).unwrap();
        let expected = Vec3::new(-0.3, 0.5, -(weapon.range + 0.5)).normalize();
        assert!(projectile.velocity.normalize().distance(expected) < 1e-5);
        assert!((projectile.velocity.length() - weapon.velocity).abs() < 1e-4);
        assert_eq!(projectile.owner, replacement);
    }

    #[test]
    fn hits_hurt_and_kill() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, PhysicsPlugins::default()));
        let target = app.world_mut().spawn(Health::new(100.0)).id();
        let mut sys = bevy::ecs::system::SystemState::<(
            Query<&mut Health, Without<Dead>>,
            Query<(&RigidBody, Forces)>,
        )>::new(app.world_mut());
        let mut killed = 0;
        for _ in 0..6 {
            let (mut h, mut b) = sys.get_mut(app.world_mut()).unwrap();
            if hit(target, Vec3::ZERO, Vec3::Z, 19.0, 1.0, &mut h, &mut b) {
                killed += 1;
            }
        }
        // 19 damage a shot: six shots to drop 100, and it dies once
        assert_eq!(killed, 1);
        assert!(app.world().get::<Health>(target).unwrap().hp <= 0.0);
    }

    /// Headless: a soldier 20 m ahead sees Rico and shoots him; Rico puts six Beretta
    /// rounds into him and he falls. Needs JC2_TEST_DIR with Rico and `weapons.bin`.
    #[test]
    fn rico_and_a_soldier_trade_shots() {
        use crate::character::{CharacterPlugin, OrbitCamera};
        use bevy::input::mouse::AccumulatedMouseMotion;
        let Some((mut app, ground, spawn)) = crate::fleet::testing::app(
            &["mc01_lod1-rico.rbm", "weapons.bin", "general.blz"],
            |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins((CharacterPlugin, CombatPlugin));
                app.world_mut().spawn((
                    Transform::from_xyz(-250.0, 0.0, -8955.0),
                    FlyCamera {
                        yaw: 0.0,
                        pitch: 0.0,
                        speed: 1.0,
                    },
                ));
            },
        ) else {
            return;
        };
        let _ = (ground, spawn);
        let settle = |app: &mut App| {
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .bypass_change_detection()
                .clear();
        };
        // (Rico's model loads on other threads: wait for him by the wall clock)
        crate::fleet::testing::wait_for_hero(&mut app);
        {
            let mut cam = app.world_mut().resource_mut::<OrbitCamera>();
            cam.yaw = 0.0;
            cam.pitch = 0.0;
        }
        for _ in 0..30 {
            settle(&mut app);
        }
        let mut q = app
            .world_mut()
            .query_filtered::<&Transform, With<FlyCamera>>();
        let cam = *q.single(app.world()).unwrap();
        let mut fwd = cam.forward().as_vec3();
        fwd.y = 0.0;
        let at = cam.translation + fwd.normalize() * 20.0 - Vec3::Y * 0.4;
        let soldier = app
            .world_mut()
            .spawn((
                RigidBody::Kinematic,
                Collider::capsule(0.3, HEIGHT - 0.6),
                Transform::from_translation(at),
                Npc::guard(at, 0.0),
                // a grunt with the commonest grunt's gun
                Soldier {
                    elite: false,
                    weapon: "Beretta Handgun".into(),
                    health: 100.0,
                },
            ))
            .with_child(Transform::default())
            .id();
        // he shoots for a while
        for _ in 0..64 * 6 {
            settle(&mut app);
        }
        let mut hq = app.world_mut().query_filtered::<&Health, With<Hero>>();
        let h = *hq.single(app.world()).unwrap();
        assert_eq!(h.max, 400.0);
        assert!(h.hp < h.max, "the soldier never hit Rico");
        assert!(app.world().get::<Npc>(soldier).unwrap().alert);
        // Rico fires back
        for _ in 0..8 {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            settle(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            for _ in 0..20 {
                settle(&mut app);
            }
        }
        let a = app.world().resource::<Arsenal>();
        assert!(a.hits >= 6, "hits {}", a.hits);
        assert_eq!(a.kills, 1);
        // eight rounds out of the Beretta's twelve; the reserve untouched
        let b = &a.guns[0].ammo;
        assert_eq!(a.guns[0].weapon.name, "Beretta Handgun");
        if a.table.len() > 40 {
            assert_eq!((b.magazine, b.reserve, b.magazine_size), (4, 180, 12));
        }
        assert!(app.world().get::<Dead>(soldier).is_some());
        assert!(app.world().get::<Collider>(soldier).is_none());
    }

    /// Headless: Rico's gun is in his right hand (the hand's attach bone, turned as
    /// `poses::Stow::Hand` says), aiming points its barrel where the camera aims, a shot
    /// plays the recoil, and drawing the rifle puts the pistol in its thigh holster.
    #[test]
    fn rico_holds_aims_and_stows_his_guns() {
        use crate::character::{CharacterPlugin, OrbitCamera};
        use crate::rig::{Joint, Slot};
        use bevy::input::mouse::AccumulatedMouseMotion;
        let Some((mut app, _, _)) = crate::fleet::testing::app(
            &["mc01_lod1-rico.rbm", "weapons.bin", "general.blz"],
            |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins((CharacterPlugin, CombatPlugin));
                app.world_mut().spawn((
                    Transform::from_xyz(-250.0, 0.0, -8955.0),
                    FlyCamera {
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
        // the guns' models load on other threads
        crate::fleet::testing::wait_until(&mut app, "the gun in his hand", |app| {
            let mut q = app.world_mut().query::<&GunModel>();
            q.iter(app.world())
                .any(|g| g.slot == 0 && g.stow == Stow::Hand)
        });
        let buttons = |app: &mut App, right: bool, left: bool| {
            let mut b = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            b.bypass_change_detection().clear();
            if right {
                b.press(MouseButton::Right);
            } else {
                b.release(MouseButton::Right);
            }
            if left {
                b.press(MouseButton::Left);
            } else {
                b.release(MouseButton::Left);
            }
            app.update();
        };
        for _ in 0..5 {
            buttons(&mut app, false, false);
        }
        // the gun's frame is the hand's attach bone's, turned
        let hand_joint = {
            let mut q = app.world_mut().query::<(Entity, &Joint, &Name)>();
            q.iter(app.world())
                .find(|(_, _, n)| n.as_str() == "AttachHandRight")
                .map(|(e, _, _)| e)
                .expect("no hand joint")
        };
        let gun_global = |app: &mut App, slot: usize| {
            let mut q = app.world_mut().query::<(&GunModel, &GlobalTransform)>();
            q.iter(app.world())
                .find(|(g, _)| g.slot == slot)
                .map(|(g, t)| (g.stow, t.compute_transform()))
                .unwrap()
        };
        let (_, mount) = Stow::Hand.mount(None).unwrap();
        let hand = app
            .world()
            .get::<GlobalTransform>(hand_joint)
            .unwrap()
            .compute_transform();
        let (stow, gun) = gun_global(&mut app, 0);
        assert_eq!(stow, Stow::Hand);
        let expect = hand * mount;
        assert!(gun.translation.distance(expect.translation) < 1e-3);
        assert!(gun.rotation.dot(expect.rotation).abs() > 0.9999);
        // First shot from the lowered idle: the flash follows the newly animated
        // muzzle, rather than the previous frame's propagated hand transform.
        buttons(&mut app, false, true);
        let mut flashes = app
            .world_mut()
            .query::<(&Name, &GlobalTransform, &ChildOf)>();
        let (_, flash, parent) = flashes
            .iter(app.world())
            .find(|(name, _, _)| name.as_str() == "muzzle flash")
            .expect("first shot has no attached muzzle flash");
        let gun_model = app.world().get::<GunModel>(parent.parent()).unwrap();
        let gun_transform = app.world().get::<GlobalTransform>(parent.parent()).unwrap();
        assert!(
            flash
                .translation()
                .distance(gun_transform.transform_point(gun_model.muzzle))
                < 1e-4
        );
        let flash_direction = flash.rotation() * Vec3::NEG_Z;
        let gun_direction = gun_transform.rotation() * Vec3::NEG_Z;
        assert!(flash_direction.dot(gun_direction) > 0.9999);
        buttons(&mut app, false, false);
        // aim 40° to his left and a little up: the barrel follows within 10°
        {
            let mut cam = app.world_mut().resource_mut::<OrbitCamera>();
            cam.yaw += 0.7;
            cam.pitch = 0.1;
        }
        for _ in 0..60 {
            buttons(&mut app, true, false);
        }
        let aim = {
            let mut q = app.world_mut().query::<&Arms>();
            let a = q.single(app.world()).unwrap();
            assert!(a.aiming());
            a.aim_dir
        };
        let (_, gun) = gun_global(&mut app, 0);
        let barrel = gun.rotation * Vec3::NEG_Z;
        let off = barrel.angle_between(aim).to_degrees();
        assert!(off < 10.0, "barrel {barrel:?}, aim {aim:?}: {off:.1}° off");
        // a shot: the recoil plays, the flash shows
        buttons(&mut app, true, true);
        buttons(&mut app, true, false);
        {
            let mut q = app.world_mut().query::<&Animator>();
            let an = q.single(app.world()).unwrap();
            assert_eq!(an.layer(Slot::Action).clip, Some(Clip::Recoil1H));
        }
        let mut q = app.world_mut().query::<&Name>();
        assert!(q.iter(app.world()).any(|n| n.as_str() == "muzzle flash"));
        // the assault rifle: in the hand; the pistol goes to the right thigh
        app.world_mut().resource_mut::<Arsenal>().current = 3;
        crate::fleet::testing::wait_until(&mut app, "the rifle in his hand", |app| {
            let mut q = app.world_mut().query::<&GunModel>();
            q.iter(app.world())
                .any(|g| g.slot == 3 && g.stow == Stow::Hand)
        });
        buttons(&mut app, false, false);
        assert_eq!(gun_global(&mut app, 0).0, Stow::RightUpLeg);
        let mut q = app.world_mut().query::<&Arms>();
        assert_eq!(
            q.single(app.world()).unwrap().class,
            Some(WeaponClass::Rifle)
        );
    }

    /// Headless: a rocket flies into a parked helicopter and goes off; enough blasts burn
    /// it out (charred, no longer enterable); a thrown grenade bounces and goes off on
    /// its 3.5 s fuse. Needs JC2_TEST_DIR with Rico, `weapons.bin`, `vehicletypes.bin` and
    /// the chopper.
    #[test]
    fn rockets_grenades_and_a_burnt_out_chopper() {
        use crate::character::CharacterPlugin;
        use crate::explosion::{Blasts, Grenade, Wreck};
        use crate::fleet::{Place, testing};
        use crate::vehicle::Enterable;
        use bevy::input::mouse::AccumulatedMouseMotion;
        let Some((mut app, _, _)) = testing::app(
            &[
                "mc01_lod1-rico.rbm",
                "weapons.bin",
                "vehicletypes.bin",
                "arve.v062_chopper.ee",
            ],
            |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins((CharacterPlugin, CombatPlugin, crate::heli::HeliPlugin));
                app.world_mut().spawn((
                    Transform::from_xyz(-250.0, 0.0, -8955.0),
                    FlyCamera {
                        yaw: 0.0,
                        pitch: 0.0,
                        speed: 1.0,
                    },
                ));
            },
        ) else {
            return;
        };
        let heli = testing::spawn(
            &mut app,
            "arve.v062_chopper.ee",
            Place::NearStart(Vec3::ZERO),
        );
        let step = |app: &mut App, n: usize| {
            for _ in 0..n {
                app.update();
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .bypass_change_detection()
                    .clear();
            }
        };
        crate::fleet::testing::wait_for_hero(&mut app);
        step(&mut app, 120);
        let hp = *app
            .world()
            .get::<Health>(heli)
            .expect("the chopper has health");
        assert_eq!(hp.max, 4000.0);
        // a rocket from 40 m out
        let at = app.world().get::<Transform>(heli).unwrap().translation;
        let from = at + Vec3::new(40.0, 1.0, 0.0);
        let rocket = {
            let a = app.world().resource::<Arsenal>();
            a.guns[4].weapon.clone()
        };
        assert_eq!(rocket.name, "RocketLauncher");
        let owner = app.world_mut().spawn_empty().id();
        app.world_mut().spawn((
            Transform::from_translation(from),
            Projectile {
                velocity: (at + Vec3::Y * 1.0 - from).normalize() * rocket.velocity,
                gravity: 0.0,
                life: 3.0,
                blast: Blast {
                    at: Vec3::ZERO,
                    radius: 5.0,
                    damage: rocket.damage,
                    impulse: 1000.0,
                },
                owner,
            },
        ));
        step(&mut app, 60);
        assert_eq!(app.world().resource::<Blasts>().count, 1);
        let after = app.world().get::<Health>(heli).unwrap().hp;
        assert!(after < hp.max - 200.0, "{after}");
        // enough to burn it out
        for _ in 0..8 {
            app.world_mut().resource_mut::<Blasts>().queue.push(Blast {
                at: at + Vec3::Y,
                radius: 5.0,
                damage: 525.0,
                impulse: 0.0,
            });
            step(&mut app, 2);
        }
        step(&mut app, 4);
        assert!(app.world().get::<Wreck>(heli).is_some());
        assert!(app.world().get::<Enterable>(heli).is_none());
        // a grenade: G, then the fuse
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        step(&mut app, 1);
        let mut q = app.world_mut().query::<&Grenade>();
        assert_eq!(q.iter(app.world()).count(), 1);
        let before = app.world().resource::<Blasts>().count;
        step(&mut app, (64.0 * 3.7) as usize);
        let mut q = app.world_mut().query::<&Grenade>();
        assert_eq!(q.iter(app.world()).count(), 0);
        assert_eq!(app.world().resource::<Blasts>().count, before + 1);
    }
}
