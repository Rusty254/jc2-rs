//! jc2 — Just Cause 2 on Bevy.
//!
//!   jc2 [<game_dir>]                              milestone 1: a texture out of the archives
//!   jc2 [<game_dir>] --model lave.v046_buscoach.ee [--lod 1|3|all]
//!                                                 milestone 2: model viewer for a container
//!   jc2 [<game_dir>] --model lave.v046_buscoach.ee/v046-body_m_lod1.rbm
//!   jc2 [<game_dir>] --world 31,14 [--radius 1] [--lod 1] [--no-models] [--no-physics] [--flat-terrain]
//!                    [--no-vegetation] [--vegetation-lod 0] [--lodfactor 0] [--bush-density 1.0]
//!                                                 milestone 3: a piece of Panau
//!   jc2 [<game_dir>] --world 31,14 --drive lave.v014_jeep.ee
//!                                                 milestone 4: a car to drive there
//!   jc2 [<game_dir>] --world 31,14 [--no-hero]     milestone 5: Rico (walk, grapple, parachute)
//!   jc2 [<game_dir>] --world 31,14 --heli          a helicopter to fly
//!   jc2 [<game_dir>] --world 30,14 --boat          a boat on the nearest water
//!   jc2 [<game_dir>] --world 31,14 --plane         a plane (the speed plane)
//!   jc2 [<game_dir>] --world 31,14 --heli --crewed [--ride spot|wire]
//!                                                 a pilot circling (Rico under it)
//!   jc2 [<game_dir>] --world 31,14 --time 22:00 [--day-speed 60] [--no-sun-shadows]   the hour, the clock's pace
//!   jc2 [<game_dir>] --browse [--browse-select general.blz/gae06_parachute_dif.dds]
//!                                                 the asset browser (`browser/`)
//!   jc2 ... --exit-after 45 --screenshot shot.png  smoke test: save the window, then quit
//!   jc2 ... --hud-demo                                debug only: fake heat, notifications, an objective
//!   jc2 ... --pose run --weapon 4 --firing --pose-view 90   debug only: hold Rico in a state (`poses.rs`)
//!   jc2 [<game_dir>] --mission f2t10                    a mission from its restart point (`missions.rs`)
//!   (`F3` shows the developer stats; the HUD is `hud.rs`)
//!   (world viewer: `M` opens the map, a click travels there; `--open-map` opens it at start)
//!
//! `game_dir` is the folder containing JustCause2.exe; it is found automatically
//! when running from inside the Steam install, or set JC2_DIR.

mod ammo;
mod archive_source;
mod atmosphere;
mod audio;
mod audio_dsp;
mod auto_exposure;
mod batch_lod;
mod bitmap_text;
mod blackmarket;
mod boat;
mod browser;
mod chaos;
mod character;
mod chute_capture;
mod onfoot_capture;
mod rico_capture;
mod civilians;
mod clouds;
mod combat;
mod impact_fx;
mod muzzle_fx;
mod tracer_fx;
mod controls;
mod coords;
mod crew;
mod destruction;
mod effects;
mod explosion;
mod fleet;
mod foliage_alpha;
mod foliage_fixture;
mod foot_actions;
mod foot_machine;
mod forest_canopy;
mod gameplay_camera;
mod general_material;
mod glow;
mod grapple;
mod ground_ik;
mod grass;
mod haze;
mod heat;
mod heli;
mod hemisphere_material;
mod skinned_general_material;
mod horizon;
mod hud;
mod identity;
mod impostors;
mod location_activation;
mod loadscreen;
mod locations;
mod map;
mod mission_browser;
mod missions;
mod music;
mod native_sampler;
mod native_sky;
mod native_units;
mod npc;
mod ocean_inputs;
mod parachute;
mod parachute_flight;
mod particle_fixture;
mod physics;
mod pickups;
mod plane;
mod plant_lod;
mod point_lights;
mod poses;
mod post_effects;
mod post_effects_render;
mod ragdoll;
mod rbm_loader;
mod rig;
mod scenery;
mod sea;
mod sea_wave;
mod water_normal;
mod sky;
mod sky_light;
mod display;
mod smoke;
mod stunts;
mod sun_horizon;
mod sun_shadow;
mod terrain_material;
mod traffic;
mod veg_fade;
mod vegdata;
mod vehicle;
mod vehicle_capture;
mod vehicle_contact_fx;
mod vehicle_damage;
mod vehicle_death;
mod viewer;
mod weapon_attach;
mod world;
mod world_particles;

use archive_source::JcAssetReader;
use bevy::asset::io::AssetSourceBuilder;
use bevy::prelude::*;
use clap::Parser;
use jc2_formats::GameFs;
use std::path::PathBuf;
use std::sync::Arc;

/// The mounted game data, shared with the asset reader.
#[derive(Resource, Clone)]
pub struct Game {
    pub fs: Arc<GameFs>,
}

#[derive(Parser, Debug)]
#[command(
    name = "jc2",
    version,
    about = "Just Cause 2 reimplementation (work in progress)"
)]
struct Cli {
    /// Folder containing JustCause2.exe (default: auto-detect / $JC2_DIR)
    game_dir: Option<PathBuf>,
    /// Open the asset browser: a tree of every file in the archives with previews of
    /// textures, models, property files, text, sound banks and the rest
    #[arg(long)]
    browse: bool,
    /// Browser: select this file at start, e.g. `general.blz/gae06_parachute_dif.dds`
    /// (a name, `container/member`, or a hash in hex); for `--screenshot`
    #[arg(long, value_name = "FILE")]
    browse_select: Option<String>,
    /// Browser: start with this text in the filter box (a flat list of matches)
    #[arg(long, value_name = "TEXT")]
    browse_filter: Option<String>,
    /// Open the model viewer on `container.ee` or `container.ee/model.rbm`
    #[arg(long)]
    model: Option<String>,
    /// Opt-in original leaf fixture (only fx_vehicle_mc_smoke_01 is supported)
    #[arg(long, conflicts_with_all = ["world", "model", "browse", "mission"])]
    particle: Option<String>,
    /// Particle preview: diagnostic static cell override (default is native playback)
    #[arg(long, requires = "particle")]
    particle_frame: Option<u32>,
    /// Particle preview: animate a moving/rotating/scaled source for local-space checks
    #[arg(long, requires = "particle")]
    particle_move_source: bool,
    #[arg(long, default_value_t = 1, requires = "particle")]
    particle_seed: u64,
    /// Particle preview: global visual budget (zero drops every birth)
    #[arg(long, default_value_t = 64, requires = "particle")]
    particle_budget: usize,
    #[arg(long, default_value_t = 32, requires = "particle")]
    particle_owner_budget: usize,
    /// Particle preview: automatically stop emission and drain after simulation seconds
    #[arg(long, requires = "particle", value_parser = parse_particle_stop)]
    particle_stop_after: Option<f64>,
    /// Exact half-float ordinary foliage discard fixture (diagnostic only)
    #[arg(long, hide = true, conflicts_with_all = ["world", "model", "browse", "mission", "particle"])]
    foliage_check: bool,
    #[arg(long, hide = true, requires = "foliage_check")]
    foliage_check_depth: bool,
    /// Model viewer: add a matte ground plane and directional shadows for material checks
    #[arg(long, requires = "model")]
    material_check: bool,
    /// Material checks: lock camera input and save final camera/readiness beside the screenshot
    #[arg(long, requires_all = ["material_check", "screenshot"])]
    material_capture: bool,
    /// Model viewer: enable the depth prepass for material checks
    #[arg(long, requires = "model")]
    depth_prepass: bool,
    /// Model viewer: compare materials with their normal maps removed
    #[arg(long, requires = "model")]
    no_normal_maps: bool,
    /// Model viewer: primary directional light yaw in degrees
    #[arg(long, requires = "model", allow_hyphen_values = true)]
    light_yaw: Option<f32>,
    /// LOD level to show when a whole container is given: 1, 2, 3, 4 or all
    #[arg(long, default_value = "1")]
    lod: String,
    /// Model viewer: the orbit camera's yaw in degrees, fixed until you drag. 0 looks
    /// along −z from behind a vehicle (its nose is −z); 90 looks from its right side
    #[arg(long, value_name = "DEGREES", allow_hyphen_values = true)]
    camera_yaw: Option<f32>,
    /// Model viewer: the orbit camera's height angle in degrees (positive looks down)
    #[arg(long, value_name = "DEGREES", allow_hyphen_values = true)]
    camera_pitch: Option<f32>,
    /// Model viewer: leave out the doll parts whose name contains one of these
    /// (comma-separated, e.g. `hardtop,door,hatch` to look into a cabin)
    #[arg(long, value_name = "NAMES", value_delimiter = ',')]
    hide_parts: Vec<String>,
    /// Open the world viewer on terrain tile `X,Z` (0..64 each; 31,14 is a mountain village)
    #[arg(long, value_name = "X,Z")]
    world: Option<String>,
    /// Tiles kept loaded around you in each direction (1 → 3 × 3), streamed as you move
    #[arg(long, default_value_t = 1)]
    radius: u32,
    /// World viewer: terrain only, skip the placed models
    #[arg(long)]
    no_models: bool,
    /// World viewer: no colliders (and no Rico, no car)
    #[arg(long)]
    no_physics: bool,
    /// World viewer: just the free camera, no Rico
    #[arg(long)]
    no_hero: bool,
    /// World viewer: colour the terrain by height and slope instead of drawing the
    /// game's terrain materials
    #[arg(long)]
    flat_terrain: bool,
    /// World viewer: skip the procedural vegetation (trees, bushes, rocks)
    #[arg(long)]
    no_vegetation: bool,
    /// World viewer: LOD level of the vegetation models (0 = native distance selection, 1 = full detail)
    #[arg(long, default_value_t = 0)]
    vegetation_lod: usize,
    /// World viewer: LOD level of placed static objects (0 = native distance selection, 1 = full detail)
    #[arg(long, default_value_t = 0)]
    object_lod: usize,
    /// World viewer: graphics setting `/lodfactor` 0-3 (native id 0x17, "Geometric detail",
    /// default 0): vegetation fade distances and the model-LOD distance unit (1 / 1.5 / 2 / 3)
    #[arg(long, value_name = "0..3", default_value_t = 0)]
    lodfactor: u32,
    /// World viewer: share of the bushes and rocks to grow, 0..1 (trees always grow)
    #[arg(long, default_value_t = 1.0)]
    bush_density: f32,
    /// World viewer: no grass around the camera
    #[arg(long)]
    no_grass: bool,
    /// World viewer: share of the grass's four passes to draw, 0..1
    #[arg(long, value_name = "0..1", default_value_t = 1.0)]
    grass_density: f32,
    /// World viewer: how far from the camera grass is drawn, metres
    #[arg(long, value_name = "METRES", default_value_t = 80.0)]
    grass_radius: f32,
    /// World viewer: no clouds or cirrus (the dome, stars, sun and moon stay)
    #[arg(long)]
    no_clouds: bool,
    /// World viewer: share of the cloud cards to draw, 0..1 (default: the weather file's 0.42)
    #[arg(long, value_name = "0..1")]
    cloud_cover: Option<f32>,
    /// World viewer: no stars at night
    #[arg(long)]
    no_stars: bool,
    /// World viewer: no tree impostors (the models stay in view out to the loaded tiles)
    #[arg(long)]
    no_impostors: bool,
    /// World viewer: tiles around the camera that get distant tree impostors
    #[arg(long, value_name = "TILES", default_value_t = 5)]
    impostor_radius: u32,
    /// World viewer: leave out the soldiers the locations' team spawn sets place
    #[arg(long)]
    no_npcs: bool,
    /// World viewer: don't park the vehicles the locations' spawn sets place
    #[arg(long)]
    no_traffic: bool,
    /// World viewer: people on foot per tile in its civilian roam areas
    #[arg(long, value_name = "N", default_value_t = civilians::DEFAULT_PER_TILE)]
    civilians: usize,
    /// World viewer: no people on foot
    #[arg(long)]
    no_civilians: bool,
    /// World viewer: civilian cars kept driving the roads around Rico (`--no-traffic`
    /// leaves them out too)
    #[arg(long, value_name = "N", default_value_t = traffic::DEFAULT_CARS)]
    traffic: usize,
    /// World viewer: no ambience bed (the climate and game zone's sound set)
    #[arg(long)]
    no_ambience: bool,
    /// World viewer: no music (the adaptive music follows the hour, the chaos and the heat)
    #[arg(long)]
    no_music: bool,
    /// Master volume, 0 … 1 (default 1)
    #[arg(long, value_name = "V", default_value_t = 1.0)]
    volume: f32,
    /// Music volume on top of the master, 0 … 2 (default 1: the game's own mix)
    #[arg(long, value_name = "V", default_value_t = 1.0)]
    music_volume: f32,
    /// World viewer: park a helicopter next to Rico (an `arve.*` container; default:
    /// the transport chopper)
    #[arg(long, value_name = "CONTAINER", num_args = 0..=1, default_missing_value = "arve.v062_chopper.ee")]
    heli: Option<String>,
    /// World viewer: park a plane next to Rico (an `arve.*` plane such as
    /// `arve.v071_jet_fighter.ee`; default: the speed plane)
    #[arg(long, value_name = "CONTAINER", num_args = 0..=1, default_missing_value = "arve.v067_speedplane.ee")]
    plane: Option<String>,
    /// World viewer: moor a boat on the nearest open water (a `seve.*` container;
    /// default: the small motorboat)
    #[arg(long, value_name = "CONTAINER", num_args = 0..=1, default_missing_value = "seve.v083_motorboat_small.ee")]
    boat: Option<String>,
    /// World viewer: drop this vehicle at the start point and drive it
    /// (a vehicle container such as `lave.v014_jeep.ee`; default: the jeep)
    #[arg(long, value_name = "CONTAINER", num_args = 0..=1, default_missing_value = "lave.v014_jeep.ee")]
    drive: Option<String>,
    /// World viewer: the vehicles `--heli`, `--boat`, `--plane` and `--drive` ask for come
    /// with a driver (a soldier in the aircraft and the car, a villager at the helm); a
    /// crewed helicopter circles its start 35 m up and a crewed boat cruises in circles.
    /// `E` from a stunt position (or beside the car or boat) hijacks them
    #[arg(long)]
    crewed: bool,
    /// World viewer: put Rico on the first of those vehicles once both are there: at its
    /// stunt position (`spot`, the default: the roof, the bow, under a helicopter), or on
    /// the wire 5 m under it (`wire`, helicopters)
    #[arg(long, value_name = "spot|wire", num_args = 0..=1, default_missing_value = "spot")]
    ride: Option<String>,
    /// World viewer, for screenshots: aim at the nearest wall and fire the hook so that the
    /// `--screenshot` (at `--exit-after`, default 20 s) catches it in flight (`fly`) or Rico
    /// hanging from it (`hang`)
    #[arg(long, value_name = "fly|hang")]
    grapple_demo: Option<String>,
    /// World viewer: start with this much heat (0..100; 20 a level): from 21 a car of
    /// soldiers comes along the roads, from 41 an attack helicopter hunts Rico
    #[arg(long, value_name = "HEAT")]
    heat: Option<f32>,
    /// World viewer: the time of day to start at, `HH:MM` (default 10:00)
    #[arg(long, value_name = "HH:MM")]
    time: Option<String>,
    /// World viewer: how fast the clock runs relative to the default of one game minute per
    /// real second (60 = a game hour a second, 0 = stopped)
    #[arg(long, value_name = "FACTOR", default_value_t = 1.0)]
    day_speed: f32,
    /// World viewer: cast shadows from the sun (the default; accepted for old command lines)
    #[arg(long, hide = true)]
    sun_shadows: bool,
    /// World viewer: no cascade shadows from the sun. The default draws them: the native pixel
    /// shaders carry the cascade block (research/formats/lighting-gaps-next.md G1); this is
    /// the cheaper mode (jungle tile 40,40 on the reference machine: frame time 16.7 ms without
    /// shadows, 20.1 ms with)
    #[arg(long)]
    no_sun_shadows: bool,
    /// World viewer: open the map (`M`) right away, e.g. for a `--screenshot`
    #[arg(long)]
    open_map: bool,
    /// Open the mission inspector at a stable mission ID.
    #[arg(long, num_args = 0..=1, default_missing_value = "f2t10")]
    open_missions: Option<String>,
    /// Debug only: fake some HUD state (heat 2, notifications, an objective, a settlement
    /// banner, a prompt, two soldiers' markers) a few seconds in, so a `--screenshot`
    /// shows them without playing; `--hud-demo vehicle` shows the HUD of a car instead
    #[arg(long, value_name = "WHAT", num_args = 0..=1, default_missing_value = "foot")]
    hud_demo: Option<String>,
    /// Start with this much chaos (the game's stage, the Black Market's unlocks, the
    /// soldiers' classes and how high the heat may climb follow it)
    #[arg(long, value_name = "POINTS", default_value_t = 0)]
    chaos: u32,
    /// Start with this much money
    #[arg(long, value_name = "DOLLARS", default_value_t = 0)]
    money: u32,
    /// Start this mission (`f2t10`; `jc2tool missions` lists them) at its restart point;
    /// without `--world` the world opens on that tile
    #[arg(long, value_name = "ID")]
    mission: Option<String>,
    /// Open the Black Market (`K`) right away, e.g. for a `--screenshot`
    #[arg(long)]
    black_market: bool,
    /// Debug only: a few seconds in, put Rico in front of the nearest elite soldier,
    /// looking at him (screenshots of the elites' look; try with `--chaos 450000`)
    #[arg(long)]
    face_elite: bool,
    /// Debug only: hold Rico in a state for a screenshot (idle, walk, walk1h, run, sprint,
    /// strafe, jump, fall, dive, land, landheavy, roll, hang, reel, chute, swim, crouch,
    /// seated)
    #[arg(long, value_name = "STATE")]
    pose: Option<String>,
    /// Ground foot placement (JC2-075): `native` (default; only where the original enables
    /// it: on a vehicle's roof), `all` (also while walking on the ground; not native) or `off`
    #[arg(long, value_name = "MODE", default_value = "native")]
    ground_ik: String,
    /// Debug only: the gun in his hand (1-5, as the keys)
    #[arg(long, value_name = "N")]
    weapon: Option<usize>,
    /// Debug only: aim straight ahead
    #[arg(long)]
    aim: bool,
    /// Debug only: keep firing straight ahead
    #[arg(long)]
    firing: bool,
    /// Debug only: hold the camera at this yaw round Rico (0 behind, 90 his right, 180
    /// in front) and pitch (degrees)
    #[arg(long, value_name = "DEG", allow_negative_numbers = true)]
    pose_view: Option<f32>,
    #[arg(long, value_name = "DEG", allow_negative_numbers = true, default_value_t = -10.0)]
    pose_pitch: f32,
    /// Debug only: call this many soldiers in a dozen metres ahead of Rico (who does not
    /// die meanwhile)
    #[arg(long, value_name = "N", default_value_t = 0)]
    pose_soldiers: u32,
    /// Window size in pixels, e.g. 1920x1080 (a size given here is the real frame size whatever
    /// the desktop's DPI scaling; captures use it, e.g. 2561x1429)
    #[arg(long, value_name = "WxH", value_parser = display::parse_size, conflicts_with_all = ["width", "height"])]
    resolution: Option<(u32, u32)>,
    /// Window width in pixels (the height stays at the saved or default value)
    #[arg(long, value_name = "PIXELS", value_parser = display::parse_dim)]
    width: Option<u32>,
    /// Window height in pixels (the width stays at the saved or default value)
    #[arg(long, value_name = "PIXELS", value_parser = display::parse_dim)]
    height: Option<u32>,
    /// Borderless fullscreen on the current monitor (F11 toggles in game)
    #[arg(long, conflicts_with_all = ["windowed", "exclusive"])]
    fullscreen: bool,
    /// Exclusive fullscreen at the monitor's current video mode
    #[arg(long, conflicts_with = "windowed")]
    exclusive: bool,
    /// A window, overriding a saved fullscreen choice
    #[arg(long)]
    windowed: bool,
    /// Quit cleanly after this many seconds (for automated smoke tests)
    #[arg(long, value_name = "SECONDS")]
    exit_after: Option<f32>,
    /// Save a screenshot of the window to this PNG just before exiting (after
    /// `--exit-after`, or after 20 s if that is not given)
    #[arg(long, value_name = "PATH.png")]
    screenshot: Option<PathBuf>,
    /// Parachute capture (JC2-950d): place Rico for a reference scenario, run its scripted
    /// inputs, save a frame at each named instant and quit. One of jungle-deploy-day,
    /// base-night-behind, bank-hard, landing-collapse, canopy-from-above, freefall-poses
    #[arg(long, value_name = "SCENARIO")]
    chute_scenario: Option<String>,
    /// Where `--chute-scenario` writes its frames (default `<game>/jc2-rs/_extract/parachute`)
    #[arg(long, value_name = "DIR", requires = "chute_scenario")]
    chute_out: Option<PathBuf>,
    /// On-foot camera capture (board c0c2758e): save `standing.png` and `running.png` into this
    /// folder and quit (`onfoot_capture.rs`)
    #[arg(long, value_name = "DIR")]
    onfoot_capture: Option<PathBuf>,
    /// Vehicle enter capture (board 27da3660): with `--heli`/`--boat`/`--plane`/`--drive`, save
    /// `outside.png`, enter the vehicle, save `inside.png` from the same fixed camera and write
    /// `slots.txt` (`vehicle_capture.rs`)
    #[arg(long, value_name = "DIR")]
    vehicle_capture: Option<PathBuf>,
    /// Rico model close-ups (board 845e8e4c): save face and chest shots into this folder and
    /// quit (`rico_capture.rs`)
    #[arg(long, value_name = "DIR")]
    rico_capture: Option<PathBuf>,
}

fn find_game_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(d) = std::env::var("JC2_DIR") {
        return PathBuf::from(d);
    }
    let mut candidates = vec![std::env::current_dir().unwrap_or_default()];
    if let Ok(exe) = std::env::current_exe() {
        candidates.push(exe);
    }
    for start in candidates {
        let mut p = start.as_path();
        while let Some(parent) = p.parent() {
            if parent.join("archives_win32").is_dir() {
                return parent.to_path_buf();
            }
            p = parent;
        }
    }
    PathBuf::from(".")
}

fn main() {
    let mut cli = Cli::parse();
    let game_dir = find_game_dir(cli.game_dir);
    let game_dir = std::fs::canonicalize(&game_dir).unwrap_or(game_dir);
    let fs = match GameFs::open(&game_dir) {
        Ok(fs) => Arc::new(fs),
        Err(e) => {
            eprintln!(
                "could not mount Just Cause 2 data at {}: {e}\nusage: jc2 <game_dir> [--model container.ee]   (game_dir = folder containing JustCause2.exe)",
                game_dir.display()
            );
            std::process::exit(2);
        }
    };
    let entries = fs.entries();
    let named = entries
        .iter()
        .filter(|e| fs.names.file(e.name_hash).is_some())
        .count();
    println!(
        "mounted {} ({} archives, {} files, {} named)",
        fs.root.display(),
        fs.archives.len(),
        entries.len(),
        named
    );

    // `--mission`: where it starts (its restart point), and the world there
    let mut world_arg = cli.world.clone();
    let mut start_at = None;
    if let Some(id) = &cli.mission {
        match missions::mission_start_point(&fs, id) {
            Some((tile, xz)) => {
                if world_arg.is_none() {
                    world_arg = Some(format!("{},{}", tile.0, tile.1));
                    start_at = Some(xz);
                }
                println!(
                    "mission {id}: starts at ({:.0}, {:.0}) on tile {},{}",
                    xz[0], xz[1], tile.0, tile.1
                );
            }
            None => eprintln!("mission {id}: no script or no start point found"),
        }
    }
    // `--chute-scenario`: the scenario's tile, start point, clock and clouds; nothing else moves
    let chute = cli.chute_scenario.as_deref().map(|name| {
        chute_capture::Scenario::parse(name).unwrap_or_else(|| {
            eprintln!(
                "--chute-scenario {name}: no such scenario (jungle-deploy-day, base-night-behind, bank-hard, landing-collapse, canopy-from-above, freefall-poses)"
            );
            std::process::exit(2);
        })
    });
    if let Some(s) = chute {
        let site = s.site();
        world_arg = Some(format!("{},{}", site.tile.0, site.tile.1));
        start_at = Some(site.xz);
        cli.time.get_or_insert_with(|| site.clock.to_string());
        cli.day_speed = 0.0;
        cli.cloud_cover = cli.cloud_cover.or(site.cloud_cover);
        cli.no_npcs = true;
        cli.no_civilians = true;
        cli.no_traffic = true;
    }
    let reader_fs = fs.clone();
    let title = match (&cli.model, &world_arg) {
        _ if cli.foliage_check => "jc2-rs — native foliage alpha fixture".to_string(),
        _ if cli.browse => "jc2-rs — asset browser".to_string(),
        _ if cli.particle.is_some() => "jc2-rs — original smoke leaf preview".to_string(),
        (Some(m), _) => format!("jc2-rs — model viewer: {m}"),
        (None, Some(w)) => format!("jc2-rs — world viewer: tile {w}"),
        (None, None) => "jc2-rs — milestone 1: assets from the archives".to_string(),
    };
    let display_path = display::default_path();
    let display_options = display::resolve(
        &display::DisplayFlags {
            resolution: cli.resolution,
            width: cli.width,
            height: cli.height,
            fullscreen: cli.fullscreen,
            exclusive: cli.exclusive,
            windowed: cli.windowed,
        },
        display_path.as_ref(),
    );
    let mut app = App::new();
    app.register_asset_source(
        "jc2",
        AssetSourceBuilder::new(move || Box::new(JcAssetReader::new(reader_fs.clone()))),
    )
    .insert_resource(Game { fs })
    .add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(display::window(&display_options, title)),
                ..default()
            })
            // everything comes through `jc2://`; no `assets/` folder to watch
            .set(AssetPlugin {
                watch_for_changes_override: Some(false),
                ..default()
            }),
    )
    .add_plugins(display::DisplayPlugin {
        options: display_options,
        path: display_path,
    })
    .init_asset::<rbm_loader::RbmAsset>()
    .register_asset_loader(rbm_loader::RbmLoader::default())
    .add_plugins((
        foliage_alpha::FoliageMaterialPlugin,
        general_material::GeneralMaterialPlugin,
        hemisphere_material::HemisphereMaterialPlugin,
        skinned_general_material::SkinnedGeneralMaterialPlugin,
        point_lights::PointLightPlugin,
        effects::EffectsPlugin,
    ));
    if cli.exit_after.is_some() || cli.screenshot.is_some() {
        app.add_plugins(smoke::SmokePlugin {
            exit_after: cli.exit_after,
            screenshot: cli.screenshot,
        });
    }

    if cli.foliage_check {
        app.insert_resource(foliage_fixture::FixtureConfig {
            depth: cli.foliage_check_depth,
        })
        .add_plugins(foliage_fixture::FoliageFixturePlugin);
        app.run();
        return;
    }
    if cli.browse {
        app.add_plugins(browser::BrowserPlugin {
            select: cli.browse_select,
            filter: cli.browse_filter,
        });
        app.run();
        return;
    }

    if let Some(leaf) = cli.particle {
        app.insert_resource(particle_fixture::ParticleViewerConfig {
            leaf,
            frame: cli.particle_frame,
            move_source: cli.particle_move_source,
            seed: cli.particle_seed,
            global_cap: cli.particle_budget,
            owner_cap: cli.particle_owner_budget,
            stop_after: cli.particle_stop_after,
        })
        .add_plugins(particle_fixture::ParticleViewerPlugin);
        app.run();
        return;
    }

    match (cli.model, world_arg) {
        (Some(model), _) => {
            app.insert_resource(viewer::ViewerConfig {
                lock_capture: cli.material_capture,
                model,
                material_check: cli.material_check,
                depth_prepass: cli.depth_prepass,
                normal_maps: !cli.no_normal_maps,
                light_yaw: cli.light_yaw.map(f32::to_radians),
                lod: cli.lod,
                camera_yaw: cli.camera_yaw.map(f32::to_radians),
                camera_pitch: cli.camera_pitch.map(f32::to_radians),
                hide_parts: cli
                    .hide_parts
                    .iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
            })
            .add_plugins(viewer::ViewerPlugin);
        }
        (None, Some(world)) => {
            let tile = parse_tile(&world).unwrap_or_else(|| {
                eprintln!("--world wants X,Z with each in 0..64, e.g. --world 31,14");
                std::process::exit(2);
            });
            let physics = !cli.no_physics;
            let start_hour = match cli.time.as_deref() {
                None => sky::SkyConfig::default().start_hour,
                Some(t) => jc2_formats::weather::parse_clock(t).unwrap_or_else(|| {
                    eprintln!("--time wants HH:MM (0:00 to 24:00), e.g. --time 18:30");
                    std::process::exit(2);
                }),
            };
            app.add_plugins(sky::SkyPlugin {
                config: sky::SkyConfig {
                    start_hour,
                    speed: cli.day_speed.max(0.0),
                    sun_shadows: cli.sun_shadows || !cli.no_sun_shadows,
                },
            })
            // the native final colour chain (clamp, saturation clamp, tint, sqrt encode)
            .add_plugins(post_effects::PostEffectPlugin);
            if let Some(xz) = start_at {
                app.insert_resource(world::StartAt(xz));
            }
            app.insert_resource(world::WorldConfig {
                tile,
                radius: cli.radius.min(8),
                lod: cli.lod.parse().unwrap_or(1),
                object_lod: cli.object_lod,
                lodfactor: cli.lodfactor,
                models: !cli.no_models,
                physics,
                flat_terrain: cli.flat_terrain,
            })
            .insert_resource(world::WcByActivation)
            .add_plugins((
                terrain_material::TerrainMaterialPlugin,
                world::WorldPlugin,
                locations::LocationsPlugin,
                horizon::HorizonPlugin,
                forest_canopy::ForestCanopyPlugin,
                sea::SeaPlugin,
                glow::GlowPlugin,
                veg_fade::VegetationFadePlugin,
                world_particles::WorldParticlesPlugin,
            ))
            .insert_resource(veg_fade::LodFactor::new(cli.lodfactor))
            .insert_resource(scenery::SceneryConfig {
                vegetation: !cli.no_vegetation,
                vegetation_lod: cli.vegetation_lod,
                lodfactor: veg_fade::LodFactor::new(cli.lodfactor).0,
                bush_density: cli.bush_density.clamp(0.0, 1.0),
                far_lod: !cli.no_impostors,
                ..default()
            })
            .add_plugins((
                scenery::SceneryPlugin,
                map::MapPlugin,
                loadscreen::LoadScreenPlugin,
            ));
            app.insert_resource(clouds::CloudConfig {
                clouds: !cli.no_clouds,
                stars: !cli.no_stars,
                cover: cli.cloud_cover.map(|c| c.clamp(0.0, 1.0)),
            })
            .add_plugins((clouds::CloudPlugin, native_sky::NativeSkyPlugin));
            if !cli.no_impostors {
                app.insert_resource(impostors::ImpostorConfig {
                    radius: cli.impostor_radius.clamp(1, 12),
                })
                .add_plugins(impostors::ImpostorPlugin);
            }
            if !cli.no_grass {
                app.insert_resource(grass::GrassConfig {
                    density: cli.grass_density.clamp(0.0, 1.0),
                    radius: cli.grass_radius.clamp(10.0, 400.0),
                })
                .add_plugins(grass::GrassPlugin);
            }
            if cli.open_map {
                app.insert_resource(map::OpenAtStart);
            }
            if let Some(id) = &cli.open_missions {
                app.insert_resource(mission_browser::OpenMissionsAtStart(id.clone()));
            }
            if physics {
                app.add_plugins(gameplay_camera::GameplayCameraPlugin);
                app.add_plugins(physics::JcPhysicsPlugin)
                    .insert_resource(chaos::ChaosStart {
                        chaos: cli.chaos,
                        money: cli.money,
                    })
                    .add_plugins((
                        chaos::ChaosPlugin,
                        missions::MissionsPlugin,
                        mission_browser::MissionBrowserPlugin,
                    ));
                if let Some(id) = &cli.mission {
                    app.insert_resource(missions::MissionStart(id.clone()));
                }
                if cli.no_hero && std::env::var_os("JC2_VEHICLE_FIRE_FIXTURE").is_some() {
                    // capture fixture: the damage model without Rico and the combat plugin
                    app.add_plugins(vehicle_damage::VehicleDamagePlugin);
                    // ... and the death routine (explosion effect, wheels, events), so the burn
                    // timer can end the vehicle (FX-VEH-2 captures)
                    app.add_plugins(explosion::ExplosionPlugin);
                }
                if !cli.no_hero {
                    app.add_plugins((
                        controls::ControlsPlugin::with_user_settings(),
                        character::CharacterPlugin,
                        combat::CombatPlugin,
                        stunts::StuntPlugin,
                        crew::CrewPlugin,
                        audio::JcAudioPlugin,
                        hud::HudPlugin,
                        blackmarket::BlackMarketPlugin,
                    ));
                    if let Some(s) = chute {
                        app.add_plugins(chute_capture::ChuteCapturePlugin(
                            chute_capture::ChuteCapture {
                                scenario: s,
                                out: cli
                                    .chute_out
                                    .clone()
                                    .unwrap_or_else(|| chute_capture::default_out(&game_dir)),
                            },
                        ));
                    }
                    if let Some(dir) = &cli.onfoot_capture {
                        app.add_plugins(onfoot_capture::OnfootCapturePlugin(dir.clone()));
                    }
                    if let Some(dir) = &cli.vehicle_capture {
                        app.add_plugins(vehicle_capture::VehicleCapturePlugin(dir.clone()));
                    }
                    if let Some(dir) = &cli.rico_capture {
                        app.add_plugins(rico_capture::RicoCapturePlugin(dir.clone()));
                    }
                    if let Some(how) = &cli.ride {
                        app.insert_resource(stunts::RideAtStart {
                            wire: how.eq_ignore_ascii_case("wire"),
                            done: false,
                        });
                    }
                    if let Some(how) = &cli.grapple_demo {
                        let shot = cli.exit_after.unwrap_or(20.0);
                        let hang = how.eq_ignore_ascii_case("hang");
                        app.insert_resource(grapple::GrappleDemo {
                            hang,
                            fire_at: shot - if hang { 4.5 } else { 1.0 },
                            aimed: false,
                            fired: false,
                        });
                    }
                    app.insert_resource(match cli.ground_ik.as_str() {
                        "all" => ground_ik::GroundIkMode::AllGrounded,
                        "off" => ground_ik::GroundIkMode::Off,
                        _ => ground_ik::GroundIkMode::Native,
                    });
                    if cli.black_market {
                        app.insert_resource(blackmarket::OpenMarketAtStart);
                    }
                    if cli.face_elite {
                        app.init_resource::<chaos::FaceElite>()
                            .add_systems(Update, chaos::face_elite);
                    }
                    if cli.pose.is_some()
                        || cli.weapon.is_some()
                        || cli.aim
                        || cli.firing
                        || cli.pose_view.is_some()
                        || cli.pose_soldiers > 0
                    {
                        let pose = cli.pose.as_deref().and_then(|p| {
                            let o = poses::PoseOverride::named(p);
                            if o.is_none() {
                                eprintln!("--pose {p}: no such state");
                            }
                            o
                        });
                        app.add_plugins(poses::PoseDemoPlugin(poses::PoseDemo {
                            pose,
                            weapon: cli.weapon,
                            aim: cli.aim,
                            firing: cli.firing,
                            view: cli.pose_view.unwrap_or(0.0),
                            pitch: cli.pose_pitch,
                            soldiers: cli.pose_soldiers,
                        }));
                    }
                    if let Some(what) = &cli.hud_demo {
                        app.insert_resource(hud::HudDemo {
                            vehicle: what == "vehicle",
                        });
                    }
                }
                if !cli.no_npcs {
                    if let Some(v) = cli.heat {
                        app.insert_resource(heat::Heat::starting_at(v));
                    }
                    app.add_plugins(npc::NpcPlugin);
                }
                if !cli.no_civilians && cli.civilians > 0 {
                    app.insert_resource(civilians::CivilianConfig {
                        per_tile: cli.civilians,
                    })
                    .add_plugins(civilians::CivilianPlugin);
                }
                if cli.no_ambience {
                    app.insert_resource(audio::NoAmbience);
                }
                app.insert_resource(audio::AudioLevels {
                    master: cli.volume.clamp(0.0, 4.0),
                    music: cli.music_volume.clamp(0.0, 4.0),
                    music_on: !cli.no_music,
                });
                // vehicles follow `--object-lod` (0 = native slots), else the `--lod` level
                let vehicle_lod = if cli.object_lod == 0 {
                    0
                } else {
                    cli.lod.parse().unwrap_or(1)
                };
                let mut fleet = fleet::Fleet::new(vehicle_lod, !cli.no_traffic);
                fleet.lodfactor = cli.lodfactor;
                // what the command line asks for; without Rico the last one is driven
                let mut asks: Vec<(String, fleet::Place, crew::CrewKind)> = Vec::new();
                use crew::CrewKind::{Civilian, Soldier};
                if let Some(c) = cli.boat {
                    asks.push((c, fleet::Place::OpenWater, Civilian));
                }
                if let Some(c) = cli.heli {
                    asks.push((
                        c,
                        fleet::Place::NearStart(Vec3::new(-16.0, 0.0, 6.0)),
                        Soldier,
                    ));
                }
                if let Some(c) = cli.plane {
                    asks.push((
                        c,
                        fleet::Place::NearStart(Vec3::new(20.0, 0.0, -10.0)),
                        Soldier,
                    ));
                }
                if let Some(c) = cli.drive {
                    asks.push((c, fleet::Place::NearStart(Vec3::ZERO), Soldier));
                }
                let n = asks.len();
                for (i, (container, place, kind)) in asks.into_iter().enumerate() {
                    fleet.requests.push(fleet::Request {
                        container,
                        place,
                        tile: None,
                        drive: i + 1 == n && !cli.crewed,
                        crew: cli.crewed.then_some(kind),
                    });
                }
                app.insert_resource(fleet).add_plugins(fleet::FleetPlugin);
                if !cli.no_traffic && cli.traffic > 0 {
                    app.insert_resource(traffic::TrafficConfig { cars: cli.traffic })
                        .add_plugins(traffic::TrafficPlugin);
                }
            }
        }
        (None, None) => {
            app.add_systems(Startup, milestone1);
        }
    }
    app.run();
}

fn parse_tile(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s
        .split_once([',', 'x', 'X'])
        .or_else(|| s.split_once(' '))?;
    let x: u32 = a.trim().parse().ok()?;
    let z: u32 = b.trim().parse().ok()?;
    (x < 64 && z < 64).then_some((x, z))
}

/// Milestone 1: prove the jc2:// asset source by drawing a texture that lives
/// inside a zlib-compressed SARC in pc4.arc.
fn milestone1(mut commands: Commands, assets: Res<AssetServer>, game: Res<Game>) {
    commands.spawn(Camera2d);
    let candidates = [
        "frontend.blz/fe_bg_rico_dif.dds",
        "fe_bg_rico_alpha_dif.dds",
        "bm_dlctuktuk.dds",
    ];
    let mut spawned = false;
    for c in candidates {
        let (container, _) = c.split_once('/').unwrap_or((c, ""));
        if game.fs.contains(container) {
            let path = format!("jc2://{c}");
            info!("loading {path}");
            commands.spawn(Sprite::from_image(assets.load(path)));
            spawned = true;
            break;
        }
    }
    if !spawned {
        warn!("no demo texture found in the archives");
    }
    let entries = game.fs.entries();
    commands.spawn((
        Text::new(format!(
            "jc2-rs   {} archives, {} files mounted from\n{}\n\nthis texture was read from gui\\frontend.blz (zlib -> SARC -> DDS) by the jc2:// asset source\nrun with --model lave.v046_buscoach.ee for the model viewer",
            game.fs.archives.len(),
            entries.len(),
            game.fs.root.display(),
        )),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
    ));
}

fn parse_particle_stop(value: &str) -> Result<f64, String> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| "expected nonnegative finite seconds".to_string())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("expected nonnegative finite seconds".to_string());
    }
    Ok(seconds)
}
