//! jc2tool — poke at Just Cause 2 game data from the command line.
//!
//!   jc2tool list    <game_dir> [--unknown] [--kind]
//!   jc2tool info    <game_dir>
//!   jc2tool cat     <game_dir> <name> [--raw] > out
//!   jc2tool extract <game_dir> <out_dir> [--unpack] [--filter substr]
//!   jc2tool prop    <game_dir> <name> [--json]
//!   jc2tool sarc    <game_dir> <name>
//!   jc2tool hash    <string>...
//!   jc2tool harvest <game_dir> [--write names.txt]
//!   jc2tool terrain <game_dir> [X Z] [--pgm out.pgm] [--materials out.ppm] [--map out.ppm] [--seams]
//!   jc2tool placements <game_dir> <container.nlz> [--models]
//!   jc2tool weather <game_dir>
//!   jc2tool vehicle <game_dir> <container.ee>
//!   jc2tool collision <game_dir> <container/file.pfx | container>
//!   jc2tool vegetation <game_dir> [X Z] [--list] [--patch PX PZ]
//!   jc2tool cgd     <game_dir> <container.nlz | file.cgd>
//!   jc2tool icons   <game_dir> [--all]
//!   jc2tool missions <game_dir> [id] [--all] [--strongholds] [--colonels]
//!   jc2tool text    <game_dir> [--lang en] [--key K | --grep substr]
//!   jc2tool font    <game_dir> [name] [--png out.png] [--text STR] [--scale N]
//!   jc2tool fev     <game_dir> [file.fev] [--event path]

mod particle;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use jc2_formats::compress::maybe_inflate;
use jc2_formats::detect::{FileKind, detect};
use jc2_formats::hash::{hash_str, name_hash};
use jc2_formats::property::{Key, Node, PropertyFile, Variant};
use jc2_formats::rbm::RbmModel;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::vfs::{GameFs, Source};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "jc2tool", version, about = "Just Cause 2 data tool")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Mount the game folder and print archive statistics
    Info { game_dir: PathBuf },
    /// List every file the engine can see
    List {
        game_dir: PathBuf,
        /// Only entries whose name is not known
        #[arg(long)]
        unknown: bool,
        /// Detect each entry's content kind (reads every file)
        #[arg(long)]
        kind: bool,
    },
    /// Write one file to stdout (zlib removed unless --raw)
    Cat {
        game_dir: PathBuf,
        name: String,
        #[arg(long)]
        raw: bool,
    },
    /// Extract files to a directory
    Extract {
        game_dir: PathBuf,
        out_dir: PathBuf,
        /// Also unpack SARC containers into sub-directories
        #[arg(long)]
        unpack: bool,
        /// Only names containing this substring
        #[arg(long)]
        filter: Option<String>,
    },
    /// Dump a property container (.bin/.asb/...) as text or JSON
    Prop {
        game_dir: PathBuf,
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// List the entries of a SARC container
    Sarc { game_dir: PathBuf, name: String },
    /// Inspect an RBMDL model: blocks, vertex/triangle counts, textures.
    /// NAME is a file name or `container.ee/model.rbm`; a container alone lists every model in it.
    Rbm { game_dir: PathBuf, name: String },
    /// Export RBMDL model(s) as Wavefront OBJ (+ .mtl naming the diffuse textures)
    Obj {
        game_dir: PathBuf,
        /// `container.ee` (all models, or only --lod N) or `container.ee/model.rbm`
        name: String,
        out: PathBuf,
        /// Only models whose name contains `lodN` (default 1); `--lod all` for everything
        #[arg(long, default_value = "1")]
        lod: String,
    },
    /// Parse every .rbm in every container of the game and report block-type coverage
    ScanRbm {
        game_dir: PathBuf,
        /// Instead of the coverage report: print every container member whose name
        /// contains this (case-insensitive), and every model with a texture that does
        #[arg(long)]
        names: Option<String>,
    },
    /// Hash strings the way the engine does
    Hash { strings: Vec<String> },
    /// Learn file/property names from the data itself
    Harvest {
        game_dir: PathBuf,
        /// Append newly learned file names to this list
        #[arg(long)]
        write: Option<PathBuf>,
    },
    /// Terrain: scan every tile (no args), inspect one tile (X Z), or write the global
    /// height map / one tile as a PGM image
    Terrain {
        game_dir: PathBuf,
        /// Tile column (0..64), with Z
        x: Option<u32>,
        /// Tile row (0..64)
        z: Option<u32>,
        /// Write a 16-bit PGM of the global map (or of the tile when X Z are given)
        #[arg(long)]
        pgm: Option<PathBuf>,
        /// With X Z: write a PPM of the tile's 128×128 render materials — each cell in
        /// the mean colour of the atlas texture its dominant slot draws (slope-aware),
        /// so roads, rock, snow and sand show where they are; prints the slot legend
        #[arg(long)]
        materials: Option<PathBuf>,
        /// With X Z: write the tile's baked PDA map (DXT1 128×128) as a PPM
        #[arg(long)]
        map: Option<PathBuf>,
        /// While scanning, check that every tile's margin matches its neighbours
        #[arg(long)]
        seams: bool,
    },
    /// List the object placements of a location container (`31x14.nlz`) or any `.blo`
    Placements {
        game_dir: PathBuf,
        name: String,
        /// Only objects that draw a model
        #[arg(long)]
        models: bool,
    },
    /// The sky: weather settings, the sun's orbit and the sky colour table by hour
    Weather { game_dir: PathBuf },
    /// Show a vehicle's doll: parts with their placement, wheels, suspension, engine,
    /// transmission (NAME is the container, e.g. `lave.v014_jeep.ee`, or the `.mvdoll`)
    Vehicle { game_dir: PathBuf, name: String },
    /// Decode Havok collision: one `.pfx` (`container/file.pfx`) in full, or every `.pfx`
    /// in a container as a summary
    Collision { game_dir: PathBuf, name: String },
    /// Inspect original reflected BRD ragdoll membership, limits and skeleton mappers.
    Ragdoll { game_dir: PathBuf, name: String },
    /// Inspect the researched MC smoke graph, leaf and raw modifier fields
    Particle {
        game_dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Havok animation: a `.bsk` skeleton (bone tree, reference pose) or a `.ban` clip
    /// (tracks, frames, root motion); `animations.blz/std_idle.ban` addresses a member
    Anim {
        game_dir: PathBuf,
        name: String,
        /// Print the pose at this time (seconds) for every track
        #[arg(long)]
        at: Option<f32>,
    },
    /// Inspect lossless animation event records in stored order.
    EventTrack { game_dir: PathBuf, name: String },
    /// Procedural vegetation: the climate zones, sets and types (no args), or the
    /// trees, bushes and rocks the engine grows on tile X Z
    Vegetation {
        game_dir: PathBuf,
        /// Tile column (0..64), with Z
        x: Option<u32>,
        /// Tile row (0..64)
        z: Option<u32>,
        /// Print every instance, not just the counts per type
        #[arg(long)]
        list: bool,
    },
    /// List the compiled static geometry (`.cgd`) of a location container
    Cgd { game_dir: PathBuf, name: String },
    /// The map's settlements (`settlements.bin` joined with their `icons_global.bin`
    /// icons and `icon_types.bin` textures); `--all` lists every map icon with its kind
    Icons {
        game_dir: PathBuf,
        #[arg(long)]
        all: bool,
    },
    /// The missions (`missions.bmf`, `missiondata.bin`, `missionrules.brf`, the scripts):
    /// every mission with its name, kind, giver, unlock chaos and objectives; with an id
    /// (`f2t10`) that mission in full: objectives, script objects, reward. `--strongholds`
    /// lists the strongholds, `--colonels` scans every location file for colonels
    Missions {
        game_dir: PathBuf,
        id: Option<String>,
        /// Also list the collectables and challenges
        #[arg(long)]
        all: bool,
        #[arg(long)]
        strongholds: bool,
        #[arg(long)]
        colonels: bool,
    },
    /// The road network (`ai\default.rg` + the road splines): the summary, or with `X Z`
    /// the nodes and edges of that tile and the shape of each edge
    Roads {
        game_dir: PathBuf,
        /// Tile column (0..64)
        x: Option<u32>,
        /// Tile row (0..64)
        z: Option<u32>,
    },
    /// The world simulation (`worldsim.bin`: people and cars by zone), or with `X Z` the
    /// civilian roam areas of that tile and who would walk there
    Civilians {
        game_dir: PathBuf,
        x: Option<u32>,
        z: Option<u32>,
    },
    /// The HUD's bitmap fonts (`gui/fonts/fonts.blz`): the metrics of every font (no name)
    /// or of one, and with `--png` a sample string rendered with it (all fonts stacked
    /// when no name is given)
    Font {
        game_dir: PathBuf,
        /// `jc2_smallfont_18` (with or without `.font`)
        name: Option<String>,
        /// Write the sample as a PNG
        #[arg(long)]
        png: Option<PathBuf>,
        /// The sample (default: a sentence, or the font's own characters for the numeral faces)
        #[arg(long)]
        text: Option<String>,
        /// Pixel scale of the PNG
        #[arg(long, default_value = "2")]
        scale: u32,
    },
    /// The localized text database (`text\ms15text.csv`): `--key K` prints one string,
    /// `--grep S` the rows whose key or text contains S, else the counts
    Text {
        game_dir: PathBuf,
        /// Language code or name: en fr it de es ru pl ja
        #[arg(long, default_value = "en")]
        lang: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        grep: Option<String>,
    },
    /// FMOD sound bank (`weapons_common.fsb`): list the samples, or with `--out` write
    /// them (those matching `--only`) as playable .mp3 files
    Fsb {
        game_dir: PathBuf,
        name: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        only: Option<String>,
    },
    /// FMOD event projects (`weapons.fev`): no file lists the projects; a file lists its
    /// events; `--event Single/Pistol_Shot` resolves one event to its layers, sound
    /// definitions, banks and samples (searching every project without a file)
    Fev {
        game_dir: PathBuf,
        file: Option<String>,
        #[arg(long)]
        event: Option<String>,
        /// With a file: print the group tree with each event's index in its group
        #[arg(long)]
        groups: bool,
        /// Count the envelope kinds of the events (kind bits, DSP name, parameter)
        #[arg(long)]
        envelopes: bool,
    },
    /// The game's numeric sound ids: how a (bank, sound) pair names an FMOD event.
    /// No options: the banks. `--id N`: the weapon sound set N (a weapon's
    /// `sound_fire_id`) with its layers resolved; `--bank B --sound S`: one pair;
    /// `--list weapons|characters|vehicles|environment|music|materials|effects` the
    /// tables, every id resolved to its event
    Soundmap {
        game_dir: PathBuf,
        #[arg(long)]
        id: Option<u32>,
        #[arg(long)]
        bank: Option<u32>,
        #[arg(long)]
        sound: Option<u32>,
        #[arg(long)]
        list: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match run(cli) {
        // `jc2tool list ... | head` closes the pipe early; that is not an error.
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        r => r,
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Info { game_dir } => info(&game_dir),
        Cmd::List {
            game_dir,
            unknown,
            kind,
        } => list(&game_dir, unknown, kind),
        Cmd::Cat {
            game_dir,
            name,
            raw,
        } => cat(&game_dir, &name, raw),
        Cmd::Extract {
            game_dir,
            out_dir,
            unpack,
            filter,
        } => extract(&game_dir, &out_dir, unpack, filter.as_deref()),
        Cmd::Prop {
            game_dir,
            name,
            json,
        } => prop(&game_dir, &name, json),
        Cmd::Sarc { game_dir, name } => sarc(&game_dir, &name),
        Cmd::Rbm { game_dir, name } => rbm(&game_dir, &name),
        Cmd::Obj {
            game_dir,
            name,
            out,
            lod,
        } => obj(&game_dir, &name, &out, &lod),
        Cmd::ScanRbm {
            game_dir,
            names: Some(n),
        } => find_members(&game_dir, &n),
        Cmd::ScanRbm {
            game_dir,
            names: None,
        } => scan_rbm(&game_dir),
        Cmd::Hash { strings } => {
            for s in strings {
                println!(
                    "{:08x}  {}  (file-name key {:08x})",
                    hash_str(&s),
                    s,
                    name_hash(&s)
                );
            }
            Ok(())
        }
        Cmd::Harvest { game_dir, write } => harvest(&game_dir, write.as_deref()),
        Cmd::Terrain {
            game_dir,
            x,
            z,
            pgm,
            materials,
            map,
            seams,
        } => terrain(
            &game_dir,
            x.zip(z),
            pgm.as_deref(),
            materials.as_deref(),
            map.as_deref(),
            seams,
        ),
        Cmd::Placements {
            game_dir,
            name,
            models,
        } => placements_cmd(&game_dir, &name, models),
        Cmd::Weather { game_dir } => weather_cmd(&game_dir),
        Cmd::Vehicle { game_dir, name } => vehicle_cmd(&game_dir, &name),
        Cmd::Collision { game_dir, name } => collision_cmd(&game_dir, &name),
        Cmd::Ragdoll { game_dir, name } => ragdoll_cmd(&game_dir, &name),
        Cmd::Particle { game_dir, json } => particle::inspect(&game_dir, json),
        Cmd::Anim { game_dir, name, at } => anim_cmd(&game_dir, &name, at),
        Cmd::EventTrack { game_dir, name } => event_track_cmd(&game_dir, &name),
        Cmd::Vegetation {
            game_dir,
            x,
            z,
            list,
        } => vegetation_cmd(&game_dir, x.zip(z), list),
        Cmd::Cgd { game_dir, name } => cgd_cmd(&game_dir, &name),
        Cmd::Icons { game_dir, all } => icons_cmd(&game_dir, all),
        Cmd::Missions {
            game_dir,
            id,
            all,
            strongholds,
            colonels,
        } => missions_cmd(&game_dir, id.as_deref(), all, strongholds, colonels),
        Cmd::Roads { game_dir, x, z } => roads_cmd(&game_dir, x.zip(z)),
        Cmd::Civilians { game_dir, x, z } => civilians_cmd(&game_dir, x.zip(z)),
        Cmd::Font {
            game_dir,
            name,
            png,
            text,
            scale,
        } => font_cmd(
            &game_dir,
            name.as_deref(),
            png.as_deref(),
            text.as_deref(),
            scale,
        ),
        Cmd::Text {
            game_dir,
            lang,
            key,
            grep,
        } => text_cmd(&game_dir, &lang, key.as_deref(), grep.as_deref()),
        Cmd::Fsb {
            game_dir,
            name,
            out,
            only,
        } => fsb_cmd(&game_dir, &name, out.as_deref(), only.as_deref()),
        Cmd::Fev {
            game_dir,
            file,
            event,
            groups,
            envelopes,
        } => fev_cmd(
            &game_dir,
            file.as_deref(),
            event.as_deref(),
            groups,
            envelopes,
        ),
        Cmd::Soundmap {
            game_dir,
            id,
            bank,
            sound,
            list,
        } => soundmap_cmd(&game_dir, id, bank.zip(sound), list.as_deref()),
    }
}

fn soundmap_cmd(
    game_dir: &Path,
    id: Option<u32>,
    pair: Option<(u32, u32)>,
    list: Option<&str>,
) -> Result<()> {
    use jc2_formats::fev::EventFile;
    use jc2_formats::soundmap::{SoundMap, SoundRef};
    use std::collections::HashMap;
    let fs = open(game_dir)?;
    let map = SoundMap::load(&fs)?;
    let mut files: HashMap<String, EventFile> = HashMap::new();
    let mut name = |r: SoundRef| -> String {
        let Some(e) = map.event(r) else {
            return format!("{}:{} (no such bank)", r.bank, r.sound);
        };
        if !files.contains_key(&e.file)
            && let Some(f) = fs
                .read(&e.file)
                .ok()
                .and_then(|d| EventFile::parse(&d).ok())
        {
            files.insert(e.file.clone(), f);
        }
        match files
            .get(&e.file)
            .and_then(|f| f.event_in_group(e.group, e.index).map(|ev| (f, ev)))
        {
            Some((f, ev)) => format!(
                "{}:{} = {}/{}  (max playbacks {}, steal {})",
                r.bank, r.sound, f.project, ev.path, ev.max_playbacks, ev.steal_priority
            ),
            None => format!(
                "{}:{} = {} group {} #{} (no such event)",
                r.bank, r.sound, e.file, e.group, e.index
            ),
        }
    };
    if let Some((b, s)) = pair {
        println!("{}", name(SoundRef { bank: b, sound: s }));
        return Ok(());
    }
    if let Some(id) = id {
        let set = map
            .weapons
            .sets
            .get(&id)
            .with_context(|| format!("no weapon sound set {id}"))?;
        println!(
            "weapon sound set {id} {}  hold {} ms",
            set.name().unwrap_or(""),
            set.hold_ms
        );
        for l in &set.layers {
            println!(
                "  {:12} {:?}{} setup {} max {}  {}",
                l.name(),
                l.event,
                if l.looping { " loop" } else { "" },
                l.setup,
                l.max_playbacks,
                name(l.sound)
            );
        }
        for w in map
            .weapon_ids
            .iter()
            .filter(|w| w.fire == id as i32 || w.upgrades.iter().any(|u| u.1 == id))
        {
            println!(
                "  used by weapon {} {:?} (upgrades {:?})",
                w.id, w.name, w.upgrades
            );
        }
        return Ok(());
    }
    match list {
        None => {
            println!("projects:");
            for (id, f) in &map.core.projects {
                println!("  {id:2} {f}");
            }
            println!(
                "banks (the id a sound_bank_id names; sound_id counts the events of the group):"
            );
            for b in map.core.banks.values() {
                println!(
                    "  {:2} {:20} {} group {} {}",
                    b.id,
                    b.name,
                    map.core
                        .projects
                        .get(&b.project)
                        .map_or("?", String::as_str),
                    b.group,
                    b.fsb
                );
            }
        }
        Some("materials") => {
            for m in &map.materials.list {
                println!("{:2} {:20} friction {}", m.id, m.name, m.friction);
            }
        }
        Some("weapons") => {
            for w in &map.weapon_ids {
                let set = |s: i32| s as u32;
                let fire = map.weapons.sets.get(&set(w.fire));
                println!(
                    "{:3} {:24} fire {:3} {} upgrades {:?}",
                    w.id,
                    w.name,
                    w.fire,
                    fire.and_then(|s| s.fire())
                        .map_or("-".to_string(), |l| name(l.sound)),
                    w.upgrades
                );
            }
            for s in map.weapons.sets.values() {
                println!(
                    "set {} {} hold {} ms",
                    s.id,
                    s.name().unwrap_or(""),
                    s.hold_ms
                );
                for l in &s.layers {
                    println!(
                        "    {:12} {:?}{}  {}",
                        l.name(),
                        l.event,
                        if l.looping { " loop" } else { "" },
                        name(l.sound)
                    );
                }
            }
        }
        Some("characters") => {
            for e in map.characters.events.values() {
                let set = &map.characters.sets[&e.set];
                println!(
                    "{:34} set {:2}{}  {}",
                    e.name,
                    e.set,
                    e.material
                        .map_or(String::new(), |m| format!(" material {m}")),
                    set.default.map_or("-".to_string(), &mut name)
                );
                for (m, r) in &set.by_material {
                    println!(
                        "      {:2} {:14} {}",
                        m,
                        map.materials.get(*m).map_or("?", |m| m.name.as_str()),
                        name(*r)
                    );
                }
            }
        }
        Some("vehicles") => {
            for m in &map.vehicles.modules {
                println!("{} (module {})", m.name, m.id);
                for p in &m.presets {
                    println!("  {:22} key {:08x}", p.name, p.key);
                    for (role, r) in [
                        ("key_on", p.key_on),
                        ("sustain", p.sustain),
                        ("key_off", p.key_off),
                    ] {
                        if let Some(r) = r {
                            println!("      {role:8} {}", name(r));
                        }
                    }
                    for (n, r) in &p.secondary {
                        println!("      {n:8} {}", name(*r));
                    }
                }
            }
            if let Some(t) = map.vehicles.traffic {
                println!("traffic {}", name(t));
            }
        }
        Some("environment") => {
            for z in &map.environment.zone_sets {
                println!("{:30} bed {}", z.name, name(z.bed));
                if let Some(r) = z.detail_looping {
                    println!("{:30}   loop {}", "", name(r));
                }
                if let Some(r) = z.sprinkle {
                    println!(
                        "{:30}   sprinkle every ~{} s {}",
                        "",
                        z.sprinkle_dt,
                        name(r)
                    );
                }
            }
            for (c, cs) in &map.environment.climates {
                println!(
                    "climate zone {c}: default {} game zones {:?}",
                    cs.default_set, cs.by_game_zone
                );
            }
            for (n, r) in &map.environment.rain {
                println!("{n:20} {}", name(*r));
            }
            if let Some(t) = map.environment.thunder {
                println!("thunder {}", name(t));
            }
        }
        Some("music") => {
            let m = &map.music;
            println!(
                "time rules {:?} (default {})  chaos rules {:?}",
                m.time_rules, m.time_default, m.chaos_rules
            );
            for g in &m.groups {
                println!("group {}", g.id);
                for (i, s) in g.slots.iter().enumerate() {
                    if let Some(r) = s {
                        println!("    slot {i} {}", name(*r));
                    }
                }
            }
            for s in &m.selections {
                println!("time {} chaos {}: groups {:?}", s.time, s.chaos, s.groups);
            }
        }
        Some("effects") => {
            let mut rows = Vec::new();
            for e in fs.entries() {
                let Some(n) = fs.names.file(e.name_hash) else {
                    continue;
                };
                if !n.starts_with("effects\\") || !n.ends_with(".bin") {
                    continue;
                }
                let Ok(d) = fs.read_inflated(n) else { continue };
                let Ok(vs) = jc2_formats::soundmap::effect_sounds(&d) else {
                    continue;
                };
                if vs.is_empty() {
                    continue;
                }
                rows.push((n.to_string(), vs));
            }
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            for (n, vs) in rows {
                println!("{n}");
                for v in vs {
                    let mat = map
                        .materials
                        .by_hash(v.key)
                        .map_or_else(|| "any".to_string(), |m| m.name.clone());
                    println!("    {:14} {}", mat, name(v.sound));
                }
            }
        }
        Some(other) => bail!("unknown --list {other}"),
    }
    Ok(())
}

fn fev_cmd(
    game_dir: &Path,
    file: Option<&str>,
    event: Option<&str>,
    groups: bool,
    envelopes: bool,
) -> Result<()> {
    use jc2_formats::fev::{EventFile, MODE_3D_CUSTOMROLLOFF, RETAIL_FILES, WaveKind};
    let fs = open(game_dir)?;
    let names: Vec<String> = match file {
        Some(f) if f.ends_with(".fev") => vec![f.to_string()],
        Some(f) => vec![format!("{f}.fev")],
        None => RETAIL_FILES.iter().map(|s| s.to_string()).collect(),
    };
    let mut files = Vec::new();
    for n in &names {
        let d = fs.read(n).with_context(|| format!("reading {n}"))?;
        files.push((n.clone(), EventFile::parse(&d).with_context(|| n.clone())?));
    }
    if envelopes {
        let mut counts: std::collections::BTreeMap<(u32, String, String), (usize, String)> =
            Default::default();
        for (_, f) in &files {
            for e in f.events() {
                for l in &e.layers {
                    for v in &l.envelopes {
                        let pname = e
                            .params
                            .get(v.param as usize)
                            .map_or("-".to_string(), |p| p.name.clone());
                        let c = counts
                            .entry((v.kind & !7, v.dsp_name.clone(), pname))
                            .or_insert((0, e.path.clone()));
                        c.0 += 1;
                    }
                }
            }
        }
        for ((kind, dsp, pname), (n, ex)) in counts {
            println!("{kind:#8x} {dsp:24} on {pname:20} x{n:5}  e.g. {ex}");
        }
        return Ok(());
    }
    let Some(path) = event else {
        for (n, f) in &files {
            if groups {
                fn walk(g: &jc2_formats::fev::EventGroup, depth: usize) {
                    for (i, e) in g.events.iter().enumerate() {
                        println!("{:indent$}[{i}] {}", "", e.name, indent = depth * 2 + 4);
                    }
                    for s in &g.groups {
                        println!("{:indent$}group {}", "", s.name, indent = depth * 2 + 4);
                        walk(s, depth + 1);
                    }
                }
                println!("{n} ({})", f.project);
                for (gi, g) in f.groups.iter().enumerate() {
                    println!("  group {gi} {}", g.name);
                    walk(g, 0);
                }
                continue;
            }
            if file.is_none() {
                let banks: Vec<String> = f.banks.iter().map(|b| b.file_name()).collect();
                println!(
                    "{n:16} {:12} {:4} events {:4} sound defs  banks {}",
                    f.project,
                    f.events().len(),
                    f.sound_defs.len(),
                    banks.join(", ")
                );
                continue;
            }
            for e in f.events() {
                let rolloff = if !e.is_3d() {
                    "2D".to_string()
                } else if e.mode & MODE_3D_CUSTOMROLLOFF != 0 {
                    format!("3D custom ..{}m", e.max_distance)
                } else {
                    format!("3D {}..{}m", e.min_distance, e.max_distance)
                };
                let params: Vec<&str> = e.params.iter().map(|p| p.name.as_str()).collect();
                println!(
                    "{}/{:50} vol {:.2} {:18} {} layers  [{}]  {}",
                    f.project,
                    e.path,
                    e.volume,
                    rolloff,
                    e.layers.len(),
                    params.join(", "),
                    e.category
                );
            }
        }
        return Ok(());
    };
    let Some((_, f, e)) = files
        .iter()
        .find_map(|(n, f)| f.event(path).map(|e| (n, f, e)))
    else {
        bail!("no event {path} in {}", names.join(", "));
    };
    println!(
        "{}/{}  kind {:#x}  mode {:#x}  vol {:.3} (category {} ×{:.3})  pitch {:+.4}±{:.4}",
        f.project,
        e.path,
        e.kind,
        e.mode,
        e.volume,
        e.category,
        f.category_volume(&e.category),
        e.pitch,
        e.pitch_randomization
    );
    println!(
        "  3D {}..{} m  max playbacks {} (behaviour {}, steal priority {})  priority {}  fade {}/{} ms",
        e.min_distance,
        e.max_distance,
        e.max_playbacks,
        e.max_playbacks_behavior,
        e.steal_priority,
        e.priority,
        e.fade_in_ms,
        e.fade_out_ms
    );
    for (i, p) in e.params.iter().enumerate() {
        println!(
            "  param {i} {:20} {}..{}  velocity {}  flags {}",
            p.name, p.min, p.max, p.velocity, p.flags
        );
    }
    let mut banks = std::collections::HashMap::new();
    for (li, l) in e.layers.iter().enumerate() {
        let pname = e
            .params
            .get(l.param as usize)
            .map_or("-", |p| p.name.as_str());
        println!(
            "  layer {li} on {pname}: {} sounds, {} envelopes",
            l.sounds.len(),
            l.envelopes.len()
        );
        for s in &l.sounds {
            let sd = &f.sound_defs[s.sound_def as usize];
            let t = f.template_of(sd);
            println!(
                "    {:.3}..{:.3} {}  loop {} vol {:.3}  autopitch {}  | template mode {} vol {:.3} rand {:.3} pitch {:+.4}±{:.4} spawn {}..{} ms",
                s.start,
                s.start + s.length,
                sd.name,
                s.loop_mode,
                s.volume,
                s.autopitch,
                t.play_mode,
                t.volume,
                t.volume_randomization,
                t.pitch,
                t.pitch_randomization,
                t.spawn_min_ms,
                t.spawn_max_ms
            );
            for w in &sd.waveforms {
                match &w.kind {
                    WaveKind::Sample {
                        file,
                        bank,
                        index,
                        length_ms,
                    } => {
                        let key = format!("{}.fsb", bank.to_ascii_lowercase());
                        let b = banks.entry(key.clone()).or_insert_with(|| {
                            fs.read(&key)
                                .ok()
                                .and_then(|d| jc2_formats::fsb::SoundBank::parse(&d).ok())
                        });
                        let resolved = b.as_ref().map(|b| {
                            match jc2_formats::fev::resolve_sample(file, *index, b) {
                                Some(i) => format!("{i:4} {}", b.samples[i].name),
                                None => "   - not in the bank".to_string(),
                            }
                        });
                        println!(
                            "      w{:3} {key} #{index:<4} {length_ms:6} ms  → {}",
                            w.weight,
                            resolved.unwrap_or_else(|| "(bank not readable)".into())
                        );
                    }
                    k => println!("      w{:3} {k:?}", w.weight),
                }
            }
        }
        for v in &l.envelopes {
            let pts: Vec<String> = v
                .points
                .iter()
                .map(|p| format!("{:.2}:{:.2}", p.x, p.y))
                .collect();
            println!(
                "    envelope {:#x}{} on {}: {}",
                v.kind,
                if v.dsp_name.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", v.dsp_name)
                },
                e.params
                    .get(v.param as usize)
                    .map_or("-", |p| p.name.as_str()),
                pts.join(" ")
            );
        }
    }
    Ok(())
}

fn fsb_cmd(game_dir: &Path, name: &str, out: Option<&Path>, only: Option<&str>) -> Result<()> {
    let fs = open(game_dir)?;
    let data = fs.read(name)?;
    let bank = jc2_formats::fsb::SoundBank::parse(&data)?;
    if let Some(dir) = out {
        std::fs::create_dir_all(dir)?;
    }
    let only = only.map(str::to_ascii_lowercase);
    let mut written = 0;
    for (i, s) in bank.samples.iter().enumerate() {
        if only
            .as_ref()
            .is_some_and(|o| !s.name.to_ascii_lowercase().contains(o.as_str()))
        {
            continue;
        }
        let secs = s.length_samples as f32 / s.frequency.max(1) as f32;
        match out {
            None => println!(
                "{i:4} {:6.2}s {:5} Hz {}ch mode {:08x} {:8} B  {}",
                secs, s.frequency, s.channels, s.mode, s.size, s.name
            ),
            Some(dir) => {
                if let Some(mp3) = bank.mpeg(&data, i) {
                    let file = s
                        .name
                        .trim_end_matches(".wav")
                        .replace(['/', '\\', ' '], "_");
                    std::fs::write(dir.join(format!("{i:04}_{file}.mp3")), mp3)?;
                    written += 1;
                }
            }
        }
    }
    if let Some(dir) = out {
        println!("{written} samples written to {}", dir.display());
    }
    Ok(())
}

fn roads_cmd(game_dir: &Path, tile: Option<(u32, u32)>) -> Result<()> {
    use jc2_formats::roads::{
        ROAD_CONTAINERS, RoadGraph, RoadNetwork, container_placements, road_splines,
    };
    let fs = open(game_dir)?;
    let graph = RoadGraph::parse(&fs.read_inflated("default.rg")?)?;
    let total: u64 = graph.edges.iter().map(|e| e.length as u64).sum();
    println!(
        "default.rg v{}: {} nodes, {} edges, {:.0} km of road, node grid {} × {} of {} m",
        graph.version,
        graph.nodes.len(),
        graph.edges.len(),
        total as f64 / 1000.0,
        graph.grid.0,
        graph.grid.0,
        graph.grid.1
    );
    let mut net = RoadNetwork::new(graph);
    let mut splines = 0;
    for c in ROAD_CONTAINERS {
        match container_placements(&fs, c) {
            Ok(f) => splines += net.add_splines(road_splines(&f)),
            Err(e) => eprintln!("{c}: {e}"),
        }
    }
    let Some((tx, tz)) = tile else {
        let mut shaped = 0;
        let mut straight = 0;
        for i in 0..net.graph.edges.len() {
            match net.shape(i) {
                Some(s) if s.len() > 2 => shaped += 1,
                Some(_) => straight += 1,
                None => {}
            }
        }
        println!(
            "{splines} road splines from {} containers: {shaped} edges follow a spline, {straight} \
             are straight, {} wait for a tile's own splines",
            ROAD_CONTAINERS.len(),
            net.graph.edges.len() - shaped - straight
        );
        return Ok(());
    };
    let name = format!("{tx:02}x{tz:02}.nlz");
    if let Ok(f) = container_placements(&fs, &name) {
        let n = net.add_splines(road_splines(&f));
        println!("{name}: {n} road splines of its own");
    }
    let (x0, z0) = jc2_formats::terrain::tile_origin(tx, tz);
    let inside = |p: [f32; 3]| p[0] >= x0 && p[0] < x0 + 512.0 && p[2] >= z0 && p[2] < z0 + 512.0;
    let nodes: Vec<usize> = (0..net.graph.nodes.len())
        .filter(|&i| inside(net.graph.nodes[i].pos))
        .collect();
    println!("tile {tx},{tz}: {} nodes", nodes.len());
    for &i in &nodes {
        let n = net.graph.nodes[i];
        println!(
            "  node {i:4} ({:9.1} {:6.1} {:9.1}) flags {:2} extra {:2} edges {:?}",
            n.pos[0], n.pos[1], n.pos[2], n.flags, n.extra, net.graph.adjacency[i]
        );
    }
    let mut edges: Vec<usize> = nodes
        .iter()
        .flat_map(|&i| net.graph.adjacency[i].clone())
        .collect();
    edges.sort();
    edges.dedup();
    for i in edges {
        let e = net.graph.edges[i];
        let shape = net.shape(i);
        println!(
            "  edge {i:4} {:4} → {:4} {:5} m (chord {:5.0}) road {:08x}/{:08x} a {} b {}: {}",
            e.from,
            e.to,
            e.length,
            net.graph.chord(i),
            e.object_id,
            e.instance_id,
            e.a,
            e.b,
            match &shape {
                Some(s) if s.len() > 2 => format!("{} points along its spline", s.len()),
                Some(_) => "straight".into(),
                None => "spline not read".into(),
            }
        );
    }
    Ok(())
}

fn civilians_cmd(game_dir: &Path, tile: Option<(u32, u32)>) -> Result<()> {
    use jc2_formats::civilians::{GameZones, WorldSim, pick, roam_areas};
    let fs = open(game_dir)?;
    let ws = WorldSim::parse(&fs.read_inflated("worldsim.bin")?)?;
    let zones = GameZones::parse(&fs.read_inflated("gamezone.bin")?)?;
    let Some((tx, tz)) = tile else {
        println!("{} kinds of people:", ws.characters.len());
        for c in &ws.characters {
            println!(
                "  {:36} spawnzones {:?}  gamezones {:?}",
                c.name, c.spawn_zones, c.game_zones
            );
        }
        println!(
            "{} kinds of cars, at most {} at once, ranges {:?}:",
            ws.cars.len(),
            ws.max_cars,
            ws.car_ranges
        );
        for c in &ws.cars {
            println!("  {:36} gamezones {:?}", c.name, c.game_zones);
        }
        return Ok(());
    };
    let name = format!("{tx:02}x{tz:02}.nlz");
    let file = jc2_formats::roads::container_placements(&fs, &name)?;
    let areas = roam_areas(&file);
    let terrain = fs
        .read_inflated(&jc2_formats::terrain::tile_file_name(tx, tz))
        .ok()
        .and_then(|d| jc2_formats::terrain::TerrainTile::parse(&d).ok());
    let (x0, z0) = jc2_formats::terrain::tile_origin(tx, tz);
    println!("{name}: {} civilian roam areas", areas.len());
    for a in &areas {
        let c = a.world.translation();
        let material = terrain
            .as_ref()
            .map(|t| t.material_local(c[0] - x0, c[2] - z0));
        let zone = material.map_or("default", |m| zones.of_material(m));
        let who: Vec<&str> = (0..4)
            .filter_map(|k| pick(&ws.characters, zone, k as f32 / 4.0 + 0.1))
            .map(|t| t.name.as_str())
            .collect();
        println!(
            "  {:24} ({:9.1} {:6.1} {:9.1}) box ±{:.0} × ±{:.0} m, capacity {}, {} spawn points, \
             zone {zone} (material {material:?}): {who:?}",
            a.name,
            c[0],
            c[1],
            c[2],
            a.half_extents[0],
            a.half_extents[2],
            a.capacity,
            a.spawn_points.len()
        );
    }
    Ok(())
}

fn icons_cmd(game_dir: &Path, all: bool) -> Result<()> {
    use jc2_formats::icons::{
        icon_types, kind_name, map_icons, map_pixel, settlement_icons, settlements,
        texture_for_kind,
    };
    use jc2_formats::terrain::tile_index;
    let fs = open(game_dir)?;
    let types = icon_types(&fs.read_inflated("icon_types.bin")?)?;
    let icons = map_icons(&fs.read_inflated("icons_global.bin")?)?;
    let sets = settlements(&fs.read_inflated("settlements.bin")?)?;
    let names = jc2_formats::text::TextDb::load(&fs, "en");
    let tile = |x: f32, z: f32| {
        tile_index(x, z)
            .map(|(a, b)| format!("{a},{b}"))
            .unwrap_or_else(|| "-".into())
    };
    if all {
        // settlement icons carry kind 0; their kind is in settlements.bin
        let set_kind: std::collections::HashMap<String, i32> = sets
            .iter()
            .map(|s| (s.id.to_ascii_lowercase(), s.kind))
            .collect();
        println!(
            "{:<16} {:>4}  {:<18} {:>27}  {:>11}  {:<6} vis  texture",
            "id", "kind", "", "position", "pixel", "tile"
        );
        for i in &icons {
            let kind = match i.kind {
                0 => set_kind
                    .get(&i.id.to_ascii_lowercase())
                    .copied()
                    .unwrap_or(0),
                k => k,
            };
            let (px, pz) = map_pixel(i.position[0], i.position[2]);
            println!(
                "{:<16} {:>4}  {:<18} ({:8.1} {:6.1} {:8.1})  ({:4.0} {:4.0})  {:<6} {}    {}",
                i.id,
                kind,
                kind_name(kind).unwrap_or(""),
                i.position[0],
                i.position[1],
                i.position[2],
                px,
                pz,
                tile(i.position[0], i.position[2]),
                i.visible as u8,
                texture_for_kind(&types, kind).unwrap_or("-"),
            );
        }
        println!("{} icons, {} icon types", icons.len(), types.len());
        return Ok(());
    }
    let joined = settlement_icons(&types, &icons, &sets);
    println!(
        "{:<12} {:>4}  {:<18} {:>27}  {:>11}  {:<6} {:<28} {:<17} texture",
        "id", "kind", "", "position", "pixel", "tile", "name", "name key"
    );
    for s in &joined {
        let (px, pz) = map_pixel(s.position[0], s.position[2]);
        println!(
            "{:<12} {:>4}  {:<18} ({:8.1} {:6.1} {:8.1})  ({:4.0} {:4.0})  {:<6} {:<28} {:<17} {}",
            s.id,
            s.kind,
            kind_name(s.kind).unwrap_or("?"),
            s.position[0],
            s.position[1],
            s.position[2],
            px,
            pz,
            tile(s.position[0], s.position[2]),
            names.as_ref().and_then(|n| n.get(&s.text)).unwrap_or("?"),
            s.text,
            s.texture.as_deref().unwrap_or("-"),
        );
    }
    let mut counts: std::collections::BTreeMap<i32, usize> = Default::default();
    for s in &joined {
        *counts.entry(s.kind).or_default() += 1;
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(k, n)| format!("{n} {}", kind_name(*k).unwrap_or("?")))
        .collect();
    println!(
        "{} settlements on the map ({} in settlements.bin): {}",
        joined.len(),
        sets.len(),
        summary.join(", ")
    );
    Ok(())
}

fn font_cmd(
    game_dir: &Path,
    name: Option<&str>,
    png: Option<&Path>,
    text: Option<&str>,
    scale: u32,
) -> Result<()> {
    use jc2_formats::font::FontSet;
    let fs = open(game_dir)?;
    let set = FontSet::load(&fs)?;
    let fonts: Vec<_> = match name {
        Some(n) => vec![set.get(n).with_context(|| format!("no font {n}"))?],
        None => set.fonts.iter().collect(),
    };
    let mut rows: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for f in &fonts {
        let (cap_top, cap_h) = f.cap_box();
        let chars: Vec<char> = f.chars().map(|(c, _)| c).collect();
        println!(
            "{}: atlas {}x{} {:?}, {} glyphs, {} code points ({}..{})",
            f.name,
            f.atlas.width,
            f.atlas.height,
            f.atlas.format,
            f.glyphs.len(),
            chars.len(),
            chars.first().map(|c| *c as u32).unwrap_or(0),
            chars.last().map(|c| *c as u32).unwrap_or(0),
        );
        println!(
            "  line height {} (+{} gap), letter spacing {} ({} px), caps {} px tall, {} px below the line's top",
            f.line_height,
            f.line_gap,
            f.spacing,
            f.spacing_px(),
            cap_h,
            cap_top
        );
        if name.is_some() {
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            for c in &chars {
                let c = *c as u32;
                match ranges.last_mut() {
                    Some(r) if r.1 + 1 == c => r.1 = c,
                    _ => ranges.push((c, c)),
                }
            }
            let list: Vec<String> = ranges
                .iter()
                .map(|(a, b)| {
                    if a == b {
                        format!("U+{a:04X}")
                    } else {
                        format!("U+{a:04X}..{b:04X}")
                    }
                })
                .collect();
            println!("  code points: {}", list.join(" "));
            let missing: String = (' '..='~').filter(|c| f.glyph(*c).is_none()).collect();
            println!(
                "  printable ASCII missing: {}",
                if missing.is_empty() { "none" } else { &missing }
            );
            for c in "AgO0 .".chars() {
                if let Some(g) = f.glyph(c) {
                    println!(
                        "  {c:?}: {}x{} px, y offset {}, advance {}, atlas rect {:?}",
                        g.width,
                        g.height,
                        g.y_offset,
                        g.width as i32 + f.spacing_px(),
                        f.rect_px(g)
                    );
                }
            }
        }
        if png.is_some() {
            let want = text.unwrap_or("Open parachute SPACE  Heat Level 2  0123456789 $+");
            let have = want.chars().filter(|c| f.glyph(*c).is_some()).count();
            let sample: String = if have * 2 >= want.chars().count() {
                want.to_string()
            } else {
                chars.iter().take(48).collect()
            };
            rows.push(f.render_line(&sample, [255, 255, 255, 255], [40, 40, 90, 255], 4));
        }
    }
    if let Some(path) = png {
        let scale = scale.max(1);
        let w = rows.iter().map(|r| r.0).max().unwrap_or(1) * scale;
        let h: u32 = rows.iter().map(|r| r.1 * scale + 4).sum::<u32>().max(1);
        let mut img = vec![0u8; (w * h * 4) as usize];
        for px in img.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&[0, 0, 0, 255]);
        }
        let mut y0 = 0;
        for (rw, rh, pix) in &rows {
            for y in 0..rh * scale {
                for x in 0..rw * scale {
                    let s = &pix[(((y / scale) * rw + x / scale) * 4) as usize..][..4];
                    img[(((y0 + y) * w + x) * 4) as usize..][..4].copy_from_slice(s);
                }
            }
            y0 += rh * scale + 4;
        }
        write_png_rgba(path, w, h, &img)?;
        println!("wrote {} ({w}x{h})", path.display());
    }
    Ok(())
}

/// A minimal PNG writer (8-bit RGBA, no filtering) for the tool's pictures.
fn write_png_rgba(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity((w * h * 4 + h) as usize);
    for row in rgba.chunks_exact((w * 4) as usize) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&raw)?;
    let idat = z.finish()?;
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

fn text_cmd(game_dir: &Path, lang: &str, key: Option<&str>, grep: Option<&str>) -> Result<()> {
    use jc2_formats::text::{LANGUAGES, TextDb, language_index};
    let fs = open(game_dir)?;
    let Some(idx) = language_index(lang) else {
        bail!("unknown language {lang:?}; one of en fr it de es ru pl ja");
    };
    let db = TextDb::load(&fs, lang).context("text\\ms15text.csv not found")?;
    if let Some(k) = key {
        match db.get(k) {
            Some(t) => println!("{t}"),
            None => bail!("no text for {k:?}"),
        }
        return Ok(());
    }
    if let Some(g) = grep {
        let g = g.to_lowercase();
        let mut rows: Vec<(&str, &str)> = db
            .iter()
            .filter(|(k, t)| k.contains(&g) || t.to_lowercase().contains(&g))
            .collect();
        rows.sort();
        for (k, t) in &rows {
            println!("{k:<28} {t}");
        }
        println!("{} rows", rows.len());
        return Ok(());
    }
    println!(
        "{} strings in {} ({}); --key K or --grep S to read them",
        db.len(),
        LANGUAGES[idx].1,
        LANGUAGES[idx].0
    );
    Ok(())
}

fn open(game_dir: &Path) -> Result<GameFs> {
    GameFs::open(game_dir).with_context(|| format!("mounting {}", game_dir.display()))
}

fn info(game_dir: &Path) -> Result<()> {
    let fs = open(game_dir)?;
    println!("game: {}", fs.root.display());
    let mut total = 0u64;
    for a in &fs.archives {
        println!(
            "  {:<22} {:>6} entries  {:>8.1} MB",
            a.name,
            a.table.len(),
            a.size() as f64 / 1e6
        );
        total += a.size() as u64;
    }
    let entries = fs.entries();
    let named = entries
        .iter()
        .filter(|e| fs.names.file(e.name_hash).is_some())
        .count();
    println!(
        "  dropzone: {} files",
        entries
            .iter()
            .filter(|e| e.source == Source::Dropzone)
            .count()
    );
    println!(
        "visible files: {} ({} named, {:.1}%), {:.2} GB of archives",
        entries.len(),
        named,
        100.0 * named as f64 / entries.len() as f64,
        total as f64 / 1e9
    );
    Ok(())
}

fn list(game_dir: &Path, unknown_only: bool, with_kind: bool) -> Result<()> {
    let fs = open(game_dir)?;
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for e in fs.entries() {
        let name = fs.names.file(e.name_hash);
        if unknown_only && name.is_some() {
            continue;
        }
        let kind = if with_kind {
            let data = fs.read_hash(e.name_hash)?;
            let k = detect(&data);
            if k == FileKind::Zlib {
                let inner = maybe_inflate(&data)?;
                format!("zlib+{:?}", detect(&inner))
            } else {
                format!("{k:?}")
            }
        } else {
            String::new()
        };
        writeln!(
            w,
            "{:08x} {:>10} {:<18} {:<14} {}",
            e.name_hash,
            e.size,
            fs.archive_name(e.source),
            kind,
            name.unwrap_or("?")
        )?;
    }
    Ok(())
}

fn cat(game_dir: &Path, name: &str, raw: bool) -> Result<()> {
    let fs = open(game_dir)?;
    let data = if raw {
        fs.read(name)?
    } else {
        read_any(&fs, name)?
    };
    let mut out = std::io::stdout().lock();
    out.write_all(&data)?;
    out.flush()?;
    Ok(())
}

fn extract(game_dir: &Path, out_dir: &Path, unpack: bool, filter: Option<&str>) -> Result<()> {
    let fs = open(game_dir)?;
    let mut n = 0usize;
    for e in fs.entries() {
        let data = fs.read_hash(e.name_hash)?;
        let kind = detect(&data);
        let rel = fs.names.file_or_placeholder(e.name_hash, kind.extension());
        if let Some(f) = filter
            && !rel.contains(f)
        {
            continue;
        }
        let rel_path: PathBuf = rel.split(['\\', '/']).collect();
        let path = out_dir.join(&rel_path);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, &data).with_context(|| path.display().to_string())?;
        n += 1;
        if unpack {
            let inner = maybe_inflate(&data)?;
            if jc2_formats::sarc::is_sarc(&inner) {
                let s = SmallArchive::parse(&inner)?;
                let dir = path.with_extension(format!(
                    "{}.d",
                    path.extension().and_then(|x| x.to_str()).unwrap_or("sarc")
                ));
                std::fs::create_dir_all(&dir)?;
                for ent in &s.entries {
                    let bytes = s.data(&inner, ent)?;
                    let p = dir.join(ent.name.replace(['\\', '/'], "_"));
                    std::fs::write(&p, bytes)?;
                    n += 1;
                }
            }
        }
    }
    eprintln!("wrote {n} files to {}", out_dir.display());
    Ok(())
}

/// Read `name` or `container/member` (zlib removed).
/// Split `path\to\container.ee/member` into the container (a file the engine knows,
/// directories and all) and the member inside it; a plain file has no member.
fn split_member(fs: &GameFs, name: &str) -> (String, Option<String>) {
    let name = name.replace('/', "\\");
    if fs.contains(&name) {
        return (name, None);
    }
    // the longest prefix that is a known file is the container
    let mut end = name.len();
    while let Some(i) = name[..end].rfind('\\') {
        if fs.contains(&name[..i]) {
            return (name[..i].to_string(), Some(name[i + 1..].to_string()));
        }
        end = i;
    }
    (name, None)
}

fn read_any(fs: &GameFs, name: &str) -> Result<Vec<u8>> {
    match split_member(fs, name) {
        (name, None) => Ok(fs.read_inflated(&name)?),
        (container, Some(member)) => {
            let data = fs.read_inflated(&container)?;
            let s = SmallArchive::parse(&data)?;
            let e = s
                .get(&member)
                .ok_or_else(|| anyhow::anyhow!("{member} not in {container}"))?;
            let bytes = s.data(&data, e)?;
            Ok(maybe_inflate(bytes)?.into_owned())
        }
    }
}

fn load_property(fs: &GameFs, name: &str) -> Result<PropertyFile> {
    let data = read_any(fs, name)?;
    if !jc2_formats::property::looks_like_property(&data)
        && !jc2_formats::property::is_blackboard(&data)
    {
        bail!(
            "{name} does not look like a property container ({:?})",
            detect(&data)
        );
    }
    Ok(PropertyFile::parse_any(&data)?)
}

fn prop(game_dir: &Path, name: &str, json: bool) -> Result<()> {
    let fs = open(game_dir)?;
    let pf = load_property(&fs, name)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pf)?);
        return Ok(());
    }
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for (i, n) in pf.nodes.iter().enumerate() {
        writeln!(w, "node[{i}]")?;
        print_node(&mut w, &fs, n, 1)?;
    }
    Ok(())
}

fn key_name(fs: &GameFs, k: &Key) -> String {
    match k {
        Key::Name(n) => n.clone(),
        Key::Hash(h) => fs
            .names
            .property(*h)
            .map(str::to_string)
            .unwrap_or_else(|| format!("#{h:08x}")),
    }
}

fn print_node(w: &mut impl Write, fs: &GameFs, n: &Node, depth: usize) -> Result<()> {
    let pad = "  ".repeat(depth);
    if let Some(t) = &n.tag {
        writeln!(w, "{pad}@tag \"{t}\"")?;
    }
    for (k, v) in &n.values {
        let val = match v {
            Variant::Int(i) => format!("{i}"),
            Variant::Float(f) => format!("{f}"),
            Variant::Str(s) => format!("\"{s}\""),
            Variant::Vec2(a) => format!("{a:?}"),
            Variant::Vec3(a) => format!("{a:?}"),
            Variant::Vec4(a) => format!("{a:?}"),
            Variant::Mat3x3(a) => format!("mat3x3{a:?}"),
            Variant::Mat4x3(a) => format!("mat4x3{a:?}"),
            Variant::Mat4x4(a) => format!("mat4x4{a:?}"),
            Variant::Ints(a) => format!("ints[{}]{:?}", a.len(), &a[..a.len().min(8)]),
            Variant::Floats(a) => format!("floats[{}]{:?}", a.len(), &a[..a.len().min(8)]),
        };
        writeln!(w, "{pad}{} = {val}", key_name(fs, k))?;
    }
    for (k, c) in &n.children {
        writeln!(w, "{pad}{} {{", key_name(fs, k))?;
        print_node(w, fs, c, depth + 1)?;
        writeln!(w, "{pad}}}")?;
    }
    Ok(())
}

fn sarc(game_dir: &Path, name: &str) -> Result<()> {
    let fs = open(game_dir)?;
    let data = fs.read_inflated(name)?;
    let s = SmallArchive::parse(&data)?;
    println!("SARC v{} {} entries", s.version, s.entries.len());
    for e in &s.entries {
        let bytes = s.data(&data, e)?;
        println!(
            "  {:>9}  {:<12}  {}",
            e.size,
            format!("{:?}", detect(bytes)),
            e.name
        );
    }
    Ok(())
}

fn harvest(game_dir: &Path, write: Option<&Path>) -> Result<()> {
    let mut fs = open(game_dir)?;
    let before = fs.names.file_count();
    let learned = fs.harvest_names();
    let entries = fs.entries();
    let named = entries
        .iter()
        .filter(|e| fs.names.file(e.name_hash).is_some())
        .count();
    println!(
        "learned {learned} names ({} -> {} file names); {}/{} archive entries now named",
        before,
        fs.names.file_count(),
        named,
        entries.len()
    );
    if let Some(p) = write {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)?;
        for e in &entries {
            if let Some(n) = fs.names.file(e.name_hash) {
                writeln!(f, "{n}")?;
            }
        }
        println!("appended resolved names to {}", p.display());
    }
    Ok(())
}

/// Resolve `container.ee/member.rbm` or `file.rbm` to (label, bytes) pairs.
/// A bare container yields every `.rbm` member.
fn resolve_models(fs: &GameFs, name: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let (container, member) = split_member(fs, name);
    let data = fs.read_inflated(&container)?;
    if jc2_formats::sarc::is_sarc(&data) {
        let s = SmallArchive::parse(&data)?;
        let mut out = Vec::new();
        for e in &s.entries {
            let is_rbm = e.name.to_ascii_lowercase().ends_with(".rbm");
            let wanted = match &member {
                Some(m) => e.name.eq_ignore_ascii_case(m),
                None => is_rbm,
            };
            if wanted {
                out.push((e.name.clone(), s.data(&data, e)?.to_vec()));
            }
        }
        if out.is_empty() {
            bail!("no matching .rbm in {container}");
        }
        Ok(out)
    } else if member.is_none() {
        Ok(vec![(container, data)])
    } else {
        bail!("{container} is not a SARC container")
    }
}

fn rbm(game_dir: &Path, name: &str) -> Result<()> {
    let fs = open(game_dir)?;
    for (label, data) in resolve_models(&fs, name)? {
        let m = match RbmModel::parse(&data) {
            Ok(m) => m,
            Err(e) => {
                println!("{label}: {e}");
                continue;
            }
        };
        println!(
            "{label}: RBMDL {}.{}.{}  bbox {:?}..{:?}  {} blocks, {} verts, {} tris{}",
            m.version.0,
            m.version.1,
            m.version.2,
            m.bbox_min,
            m.bbox_max,
            m.blocks.len(),
            m.vertex_count(),
            m.triangle_count(),
            m.truncated_at
                .as_ref()
                .map(|(i, t)| format!("  [stopped at block {i}: unsupported type {t}]"))
                .unwrap_or_default()
        );
        for (i, b) in m.blocks.iter().enumerate() {
            let bounds = b
                .mesh
                .bounds()
                .map(|(lo, hi)| format!("{:.2?}..{:.2?}", lo, hi))
                .unwrap_or_default();
            println!(
                "  [{i}] {} v{}  {} verts {} tris  normals={} uv0={}  {}",
                b.type_name,
                b.version,
                b.mesh.vertex_count(),
                b.mesh.triangle_count(),
                !b.mesh.normals.is_empty(),
                !b.mesh.uv0.is_empty(),
                bounds
            );
            let tex: Vec<&str> = b
                .material
                .textures()
                .into_iter()
                .filter(|t| !t.is_empty())
                .collect();
            println!("      textures: {}", tex.join(", "));
            if let Some(flags) = b.foliage_flags {
                println!(
                    "      foliage flags: {flags:#010x} (ordinary shader alpha enabled={})",
                    flags & 32 == 0
                );
            }
            if !b.params.is_empty() {
                println!(
                    "      params: {:?}",
                    b.params
                        .iter()
                        .map(|p| (p * 1000.0).round() / 1000.0)
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    Ok(())
}

fn obj(game_dir: &Path, name: &str, out: &Path, lod: &str) -> Result<()> {
    let fs = open(game_dir)?;
    let models = resolve_models(&fs, name)?;
    let lod_tag = format!("lod{lod}");
    let mut obj_text = String::new();
    let mut mtl_text = String::new();
    let mtl_name = out
        .with_extension("mtl")
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model.mtl".into());
    obj_text.push_str(&format!(
        "# exported by jc2tool from {name}\nmtllib {mtl_name}\n"
    ));
    let mut base = 1usize;
    let mut materials = std::collections::BTreeSet::new();
    let mut written = 0;
    for (label, data) in &models {
        if lod != "all" && models.len() > 1 && !label.to_ascii_lowercase().contains(&lod_tag) {
            continue;
        }
        let m = match RbmModel::parse(data) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("skipping {label}: {e}");
                continue;
            }
        };
        for (bi, b) in m.blocks.iter().enumerate() {
            let mesh = &b.mesh;
            if mesh.positions.is_empty() {
                continue;
            }
            let mat = if b.material.diffuse.is_empty() {
                "none".to_string()
            } else {
                b.material.diffuse.replace('.', "_")
            };
            materials.insert((mat.clone(), b.material.diffuse.clone()));
            obj_text.push_str(&format!(
                "o {}_{}_{}\nusemtl {}\n",
                label.replace(' ', "_"),
                bi,
                b.type_name,
                mat
            ));
            for p in &mesh.positions {
                obj_text.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
            }
            let has_uv = mesh.uv0.len() == mesh.positions.len();
            let has_n = mesh.normals.len() == mesh.positions.len();
            if has_uv {
                for t in &mesh.uv0 {
                    obj_text.push_str(&format!("vt {} {}\n", t[0], 1.0 - t[1]));
                }
            }
            if has_n {
                for n in &mesh.normals {
                    obj_text.push_str(&format!("vn {} {} {}\n", n[0], n[1], n[2]));
                }
            }
            for tri in mesh.indices.as_chunks::<3>().0 {
                let idx = |k: u16| base + k as usize;
                let f = |k: u16| match (has_uv, has_n) {
                    (true, true) => format!("{0}/{0}/{0}", idx(k)),
                    (true, false) => format!("{0}/{0}", idx(k)),
                    (false, true) => format!("{0}//{0}", idx(k)),
                    (false, false) => format!("{0}", idx(k)),
                };
                obj_text.push_str(&format!("f {} {} {}\n", f(tri[0]), f(tri[1]), f(tri[2])));
            }
            base += mesh.positions.len();
            written += 1;
        }
    }
    for (mat, tex) in &materials {
        mtl_text.push_str(&format!("newmtl {mat}\nKd 0.8 0.8 0.8\n"));
        if !tex.is_empty() {
            mtl_text.push_str(&format!("map_Kd {tex}\n"));
        }
        mtl_text.push('\n');
    }
    std::fs::write(out, obj_text).with_context(|| out.display().to_string())?;
    std::fs::write(out.with_extension("mtl"), mtl_text)?;
    eprintln!(
        "wrote {written} meshes ({} vertices) to {}",
        base - 1,
        out.display()
    );
    Ok(())
}

/// `scan-rbm --names`: every file, container member (nested containers too) and model
/// texture whose name contains `needle`.
fn find_members(game_dir: &Path, needle: &str) -> Result<()> {
    let fs = open(game_dir)?;
    let needle = needle.to_ascii_lowercase();
    fn walk(label: &str, data: &[u8], needle: &str, depth: usize) {
        if jc2_formats::sarc::is_sarc(data) {
            let Ok(s) = SmallArchive::parse(data) else {
                return;
            };
            for ent in &s.entries {
                let Ok(b) = s.data(data, ent) else { continue };
                let path = format!("{label}/{}", ent.name);
                if ent.name.to_ascii_lowercase().contains(needle) {
                    println!("member  {path}  ({} B)", b.len());
                }
                let inflated = maybe_inflate(b).unwrap_or_else(|_| b.to_vec().into());
                if depth < 3 && jc2_formats::sarc::is_sarc(&inflated) {
                    walk(&path, &inflated, needle, depth + 1);
                } else {
                    walk_model(&path, &inflated, needle);
                }
            }
        } else {
            walk_model(label, data, needle);
        }
    }
    fn walk_model(label: &str, data: &[u8], needle: &str) {
        if !jc2_formats::rbm::is_rbm(data) {
            return;
        }
        let Ok(m) = RbmModel::parse(data) else { return };
        let mut hits: Vec<String> = Vec::new();
        for b in &m.blocks {
            for t in b.material.textures() {
                if t.to_ascii_lowercase().contains(needle) && !hits.iter().any(|h| h == t) {
                    hits.push(t.to_string());
                }
            }
        }
        if !hits.is_empty() {
            println!(
                "model   {label}  ({} verts)  textures: {}",
                m.vertex_count(),
                hits.join(", ")
            );
        }
    }
    for e in fs.entries() {
        let name = fs
            .names
            .file_short(e.name_hash)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:08x}", e.name_hash));
        if name.to_ascii_lowercase().contains(&needle) {
            println!("file    {name}  ({} B)", e.size);
        }
        let Ok(raw) = fs.read_hash(e.name_hash) else {
            continue;
        };
        let Ok(data) = maybe_inflate(&raw) else {
            continue;
        };
        walk(&name, &data, &needle, 0);
    }
    Ok(())
}

fn scan_rbm(game_dir: &Path) -> Result<()> {
    use std::collections::BTreeMap;
    let fs = open(game_dir)?;
    let mut models = 0usize;
    let mut ok = 0usize;
    let mut blocks: BTreeMap<String, usize> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    let mut unsupported_examples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    let mut error_examples: BTreeMap<String, String> = BTreeMap::new();
    let mut bbox_bad: BTreeMap<String, usize> = BTreeMap::new();
    let mut bbox_examples: BTreeMap<String, String> = BTreeMap::new();
    let mut tris = 0usize;
    // VegetationBark census (lighting research 646639f2): layout flag, material flags and the occupied
    // texture slots (bit i = `Material::textures()[i]` non-empty) -> block count and one example.
    let mut bark: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for e in fs.entries() {
        let Ok(raw) = fs.read_hash(e.name_hash) else {
            continue;
        };
        let data = match maybe_inflate(&raw) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let container = fs
            .names
            .file_short(e.name_hash)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:08x}", e.name_hash));
        let mut items: Vec<(String, &[u8])> = Vec::new();
        if jc2_formats::sarc::is_sarc(&data) {
            if let Ok(s) = SmallArchive::parse(&data) {
                for ent in &s.entries {
                    if let Ok(b) = s.data(&data, ent)
                        && jc2_formats::rbm::is_rbm(b)
                    {
                        items.push((format!("{container}/{}", ent.name), b));
                    }
                }
            }
        } else if jc2_formats::rbm::is_rbm(&data) {
            items.push((container.clone(), &data));
        }
        for (label, bytes) in items {
            models += 1;
            match RbmModel::parse(bytes) {
                Ok(m) => {
                    ok += 1;
                    tris += m.triangle_count();
                    let ext = (0..3)
                        .map(|i| (m.bbox_max[i] - m.bbox_min[i]).abs())
                        .fold(0.0f32, f32::max);
                    let has_bbox = ext > 0.001; // Merged models carry no bounding box
                    for b in &m.blocks {
                        *blocks
                            .entry(format!("{} v{}", b.type_name, b.version))
                            .or_default() += 1;
                        if b.type_name == "VegetationBark" {
                            let slots = b
                                .material
                                .textures()
                                .iter()
                                .enumerate()
                                .filter(|(_, t)| !t.is_empty())
                                .fold(0u32, |a, (i, _)| a | (1 << i));
                            let key = format!(
                                "layout_flags={:#x} material_flags={:#x} slots={slots:#010b} verts_have_colour={}",
                                b.params.first().copied().unwrap_or(0.0) as u32,
                                b.material.flags,
                                !b.mesh.packed_colours.is_empty()
                            );
                            bark.entry(key).or_insert_with(|| (0, label.clone())).0 += 1;
                        }
                        // vertex bounds must sit inside the file's bounding box: validates position scaling
                        if let (true, Some((lo, hi))) = (has_bbox, b.mesh.bounds()) {
                            let over = (0..3).any(|i| {
                                lo[i] < m.bbox_min[i] - 0.15 * ext
                                    || hi[i] > m.bbox_max[i] + 0.15 * ext
                            });
                            if over {
                                let key = format!("{} v{}", b.type_name, b.version);
                                *bbox_bad.entry(key.clone()).or_default() += 1;
                                bbox_examples.entry(key).or_insert_with(|| {
                                    format!(
                                        "{label}: verts {lo:.2?}..{hi:.2?} vs bbox {:.2?}..{:.2?}",
                                        m.bbox_min, m.bbox_max
                                    )
                                });
                            }
                        }
                    }
                    if let Some((_, t)) = m.truncated_at {
                        *unsupported.entry(t.clone()).or_default() += 1;
                        let ex = unsupported_examples.entry(t).or_default();
                        if ex.len() < 4 && !ex.iter().any(|e: &String| e.starts_with(&container)) {
                            ex.push(format!("{label} ({} B)", bytes.len()));
                        }
                    }
                }
                Err(err) => {
                    let msg = err.to_string();
                    let key: String = msg.chars().take(60).collect();
                    *errors.entry(key.clone()).or_default() += 1;
                    error_examples.entry(key).or_insert(label);
                }
            }
        }
    }
    println!("{models} models, {ok} parsed ({} triangles)", tris);
    println!("block types parsed:");
    for (k, v) in &blocks {
        println!("  {v:>7}  {k}");
    }
    if !bark.is_empty() {
        println!("VegetationBark blocks by layout / material flags / occupied texture slots:");
        for (k, (n, ex)) in &bark {
            println!("  {n:>7}  {k}   e.g. {ex}");
        }
    }
    if !unsupported.is_empty() {
        println!("models cut short by an unsupported block type:");
        for (k, v) in &unsupported {
            println!(
                "  {v:>7}  {k}   e.g. {}",
                unsupported_examples[k].join(" | ")
            );
        }
    }
    if !bbox_bad.is_empty() {
        println!("blocks whose vertices fall outside the model bbox (position scale suspect):");
        for (k, v) in &bbox_bad {
            println!("  {v:>7}  {k}   e.g. {}", bbox_examples[k]);
        }
    }
    if !errors.is_empty() {
        println!("parse errors:");
        for (k, v) in &errors {
            println!("  {v:>7}  {k}   e.g. {}", error_examples[k]);
        }
    }
    Ok(())
}

fn write_pgm(
    path: &Path,
    width: usize,
    height: usize,
    px: impl Iterator<Item = u16>,
) -> Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P5\n{width} {height}\n65535\n")?;
    for v in px {
        out.write_all(&v.to_be_bytes())?;
    }
    Ok(())
}

fn write_ppm(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{width} {height}\n255\n")?;
    out.write_all(&rgb[..width * height * 3])?;
    Ok(())
}

/// Gradient magnitude of a tile cell: from the normal layer when the file has one, else
/// from the heights.
fn cell_slope(t: &jc2_formats::terrain::TerrainTile, cz: usize, cx: usize) -> f32 {
    let (dx, dz) = t
        .layer_gradient(cz, cx)
        .unwrap_or_else(|| t.cell_gradient(cz, cx));
    (dx * dx + dz * dz).sqrt()
}

/// Picture of a tile's render materials: every cell in the mean colour of the two atlas
/// textures the game's shader blends there (`blend_layers`); cells whose slot the
/// tables do not know come out magenta.
fn material_picture(
    t: &jc2_formats::terrain::TerrainTile,
    land: &jc2_formats::landscape::Landscape,
    zone: u8,
) -> Vec<u8> {
    use jc2_formats::terrain::TILE_CELLS;
    let mut rgb = Vec::with_capacity(TILE_CELLS * TILE_CELLS * 3);
    for cz in 0..TILE_CELLS {
        for cx in 0..TILE_CELLS {
            let cell = t.meta_cell(cz, cx);
            match land.cell_colour(zone, &cell, cell_slope(t, cz, cx)) {
                Some(c) => rgb.extend(
                    c.iter()
                        .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
                ),
                None => rgb.extend_from_slice(&[255, 0, 255]),
            }
        }
    }
    rgb
}

/// How well the colours of `material_picture` match the tile's baked PDA map:
/// Pearson correlation of the luminance over the cells, `None` without a map.
fn map_correlation(t: &jc2_formats::terrain::TerrainTile, rgb: &[u8]) -> Option<f64> {
    let map = t.map.as_ref()?.decode_rgba(0)?;
    let lum = |p: &[u8]| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
    let a: Vec<f64> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| lum(p.as_slice()))
        .collect();
    let b: Vec<f64> = map
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| lum(p.as_slice()))
        .collect();
    if a.len() != b.len() {
        return None;
    }
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(&b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    (saa > 0.0 && sbb > 0.0).then(|| sab / (saa * sbb).sqrt())
}

/// Mean absolute difference, in raw height units per metre, between a tile's normal
/// layer and the gradient of its own heights; `None` without the layer.
fn normal_layer_error(t: &jc2_formats::terrain::TerrainTile) -> Option<f32> {
    use jc2_formats::terrain::{HEIGHT_SCALE, TILE_CELLS};
    let mut sum = 0.0f32;
    for cz in 0..TILE_CELLS {
        for cx in 0..TILE_CELLS {
            let (lx, lz) = t.layer_gradient(cz, cx)?;
            let (hx, hz) = t.cell_gradient(cz, cx);
            // the layer clamps at ±127 raw/m
            let clamp = |g: f32| g.clamp(-127.0 * HEIGHT_SCALE, 127.0 * HEIGHT_SCALE);
            sum += (lx - clamp(hx)).abs() + (lz - clamp(hz)).abs();
        }
    }
    Some(sum / (2.0 * (TILE_CELLS * TILE_CELLS) as f32) / HEIGHT_SCALE)
}

fn terrain(
    game_dir: &Path,
    tile: Option<(u32, u32)>,
    pgm: Option<&Path>,
    materials: Option<&Path>,
    map: Option<&Path>,
    seams: bool,
) -> Result<()> {
    use jc2_formats::landscape::{Landscape, META_SLOTS};
    use jc2_formats::terrain::*;
    let fs = open(game_dir)?;
    let land = Landscape::load(&fs)
        .map_err(|e| eprintln!("material tables not loaded: {e}"))
        .ok();
    if let Some((x, z)) = tile {
        let name = tile_file_name(x, z);
        let t = TerrainTile::parse(&fs.read_inflated(&name)?)?;
        let (x0, z0) = tile_origin(x, z);
        let (lo, hi) = t.raw_range();
        println!("{name}: version {} kind {}", t.version, t.kind);
        println!(
            "  covers x {x0}..{} z {z0}..{}",
            x0 + TILE_SIZE,
            z0 + TILE_SIZE
        );
        println!(
            "  heights {:.1}..{:.1} m (raw {lo}..{hi}){}",
            height_to_metres(lo),
            height_to_metres(hi),
            if t.is_empty() {
                "  [empty: open sea]"
            } else {
                ""
            }
        );
        for (i, sec) in t.sections.iter().enumerate() {
            let texs: Vec<String> = sec
                .textures
                .iter()
                .map(|d| format!("{}×{} {:?} mips {}", d.width, d.height, d.format, d.mips))
                .collect();
            println!(
                "  section {i}: version {} [{}]",
                sec.version,
                texs.join(", ")
            );
        }
        let mut counts: std::collections::BTreeMap<u8, usize> = Default::default();
        for cz in 0..TILE_CELLS {
            for cx in 0..TILE_CELLS {
                *counts.entry(t.cell_material(cz, cx)).or_default() += 1;
            }
        }
        let mut by_count: Vec<(u8, usize)> = counts.into_iter().collect();
        by_count.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        println!(
            "  materials: {} distinct; {}",
            by_count.len(),
            by_count
                .iter()
                .take(8)
                .map(|(m, n)| format!("0x{m:02x}×{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!(
            "  mesh data: {} pieces, {} bits in all",
            t.mesh_data.len(),
            t.mesh_data.iter().map(|p| p.bits as usize).sum::<usize>()
        );
        if !t.gamezone.is_empty() {
            let zones = t.gamezone.iter().collect::<std::collections::BTreeSet<_>>();
            println!(
                "  game zones: {}",
                zones
                    .iter()
                    .map(|z| format!("0x{z:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        println!("  tail: {} bytes", t.tail.len());
        println!(
            "  centre height {:.1} m",
            t.sample_local(256.0, 256.0).unwrap()
        );
        let container = tile_container_name(x, z);
        println!(
            "  location container {container}: {}",
            if fs.contains(&container) {
                "present"
            } else {
                "none"
            }
        );
        if let (Ok(bin), Ok(bmp)) = (
            fs.read_inflated("areasets.bin"),
            fs.read_inflated("areasets.bmp"),
        ) && let Ok(sets) = jc2_formats::areaset::AreaSets::parse(&bin, &bmp)
        {
            println!(
                "  area set: {}",
                sets.set_for_tile(x, z)
                    .map(|s| format!("{} ({})", s.name, s.container()))
                    .unwrap_or_else(|| "none".into())
            );
        }
        // the materials: gameplay byte map and the render slots
        let zone = land.as_ref().map(|l| l.texture_zone(x, z)).unwrap_or(0);
        let climate = land
            .as_ref()
            .and_then(|l| l.zones.as_ref())
            .map(|zm| zm.climate_zone(x, z));
        println!(
            "  texture zone (atlas) {zone}{}",
            climate
                .map(|c| format!(", climate zone {c}"))
                .unwrap_or_default()
        );
        let mut game_counts = std::collections::BTreeMap::<u8, usize>::new();
        for cz in 0..TILE_CELLS {
            for cx in 0..TILE_CELLS {
                *game_counts.entry(t.cell_material(cz, cx)).or_default() += 1;
            }
        }
        let mut slot_counts = [0usize; META_SLOTS];
        let mut agree = 0usize;
        let mut weight_sum_ok = 0usize;
        for cz in 0..TILE_CELLS {
            for cx in 0..TILE_CELLS {
                let cell = t.meta_cell(cz, cx);
                for (s, _) in cell.blend() {
                    slot_counts[s as usize] += 1;
                }
                if cell.weights.iter().map(|w| *w as u32).sum::<u32>() == 15 {
                    weight_sum_ok += 1;
                }
                let expect = land.as_ref().and_then(|l| {
                    l.materials
                        .get(t.cell_material(cz, cx))
                        .and_then(|g| l.metas.by_id(g.meta_material))
                        .map(|m| m.slot)
                });
                if expect == Some(cell.dominant()) {
                    agree += 1;
                }
            }
        }
        println!("  gameplay materials (132×132 byte map, own cells):");
        for (idx, n) in &game_counts {
            let m = land.as_ref().and_then(|l| l.materials.get(*idx));
            println!(
                "    {idx:3} ×{n:5}  {}{}",
                m.map(|m| m.name.as_str())
                    .unwrap_or("(not in terrain_materials.bin)"),
                m.filter(|m| !m.vegetation_set.is_empty())
                    .map(|m| format!("  vegetation {}", m.vegetation_set))
                    .unwrap_or_default()
            );
        }
        if t.has_layers() {
            println!(
                "  render slots (A4R4G4B4 layers; {weight_sum_ok}/{} cells with weights summing to 15):",
                TILE_CELLS * TILE_CELLS
            );
            let table = land.as_ref().map(|l| l.metas.slots(zone));
            for (slot, n) in slot_counts.iter().enumerate() {
                if *n == 0 {
                    continue;
                }
                let desc = table
                    .as_ref()
                    .and_then(|t| t[slot])
                    .map(|m| {
                        format!(
                            "meta {} (climate zone {}), textures flat {} … steep {}",
                            m.id, m.climate_zone, m.slopes[0].texture, m.slopes[15].texture
                        )
                    })
                    .unwrap_or_else(|| "unknown".into());
                println!("    slot {slot:2} in {n:5} cells  {desc}");
            }
            if land.is_some() {
                println!(
                    "  byte map → MetaMaterial → slot agrees with the dominant render slot in {agree}/{} cells",
                    TILE_CELLS * TILE_CELLS
                );
            }
            if let Some(err) = normal_layer_error(&t) {
                println!(
                    "  normal layer vs. the heights' gradient: mean |difference| {err:.2} raw/m (A8L8 = 128 + Δraw/m, L = x, A = z)"
                );
            }
            if let Some(l) = land.as_ref().filter(|l| {
                l.atlas_colours
                    .get(zone as usize)
                    .is_some_and(|c| !c.is_empty())
            }) {
                let rgb = material_picture(&t, l, zone);
                if let Some(r) = map_correlation(&t, &rgb) {
                    println!(
                        "  shader colours (mean atlas colours of the blended layers) vs. the PDA map: luminance correlation {r:.3}"
                    );
                }
            }
        } else {
            println!("  no render layers in this file");
        }
        if let Some(p) = pgm {
            write_pgm(p, TILE_SAMPLES, TILE_SAMPLES, t.heights.iter().copied())?;
            println!("wrote {}", p.display());
        }
        if let Some(p) = materials {
            let land = land
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--materials needs the climate tables"))?;
            if land
                .atlas_colours
                .get(zone as usize)
                .is_none_or(|c| c.is_empty())
            {
                bail!(
                    "--materials needs {} for the colours",
                    jc2_formats::landscape::diffuse_atlas_name(zone)
                );
            }
            let rgb = material_picture(&t, land, zone);
            write_ppm(p, TILE_CELLS, TILE_CELLS, &rgb)?;
            println!("wrote {} (row 0 = lowest z)", p.display());
        }
        if let Some(p) = map {
            let img = t
                .map
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no map texture in this tile"))?;
            let rgba = img
                .decode_rgba(0)
                .ok_or_else(|| anyhow::anyhow!("undecodable map format {:?}", img.format))?;
            let rgb: Vec<u8> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            write_ppm(p, img.width as usize, img.height as usize, &rgb)?;
            println!("wrote {} (row 0 = lowest z)", p.display());
        }
        return Ok(());
    }
    let g = GlobalTerrain::parse(&fs.read_inflated("terrain_win32.dat")?)?;
    let (lo, hi) = (
        g.heights.iter().min().unwrap(),
        g.heights.iter().max().unwrap(),
    );
    println!(
        "terrain_win32.dat: version {}, {GLOBAL_SAMPLES}×{GLOBAL_SAMPLES} samples at {GLOBAL_CELL} m, heights {:.1}..{:.1} m, tail {} bytes",
        g.version,
        height_to_metres(*lo),
        height_to_metres(*hi),
        g.tail.len()
    );
    if let Some(p) = pgm {
        write_pgm(p, GLOBAL_SAMPLES, GLOBAL_SAMPLES, g.heights.iter().copied())?;
        println!("wrote {}", p.display());
    }
    let n = TILES_PER_SIDE as usize;
    let mut tiles: Vec<Option<TerrainTile>> = Vec::with_capacity(n * n);
    let (mut missing, mut failed, mut empty, mut containers) = (0usize, 0usize, 0usize, 0usize);
    // material statistics across the game
    let mut with_layers = 0usize;
    let mut layouts = std::collections::BTreeMap::<String, usize>::new();
    let mut slot_cells = [0usize; META_SLOTS];
    let mut unknown_slot_cells = 0usize;
    let mut weight_bad = 0usize;
    let mut game_unknown = std::collections::BTreeMap::<u8, usize>::new();
    let mut agree = 0usize;
    let mut agree_total = 0usize;
    let (mut normal_err, mut normal_n) = (0.0f64, 0usize);
    let (mut corr_sum, mut corr_n, mut corr_low) = (0.0f64, 0usize, 0usize);
    let (mut lo, mut hi) = (u16::MAX, 0u16);
    let mut global_err = 0.0f64;
    let mut global_n = 0usize;
    for z in 0..TILES_PER_SIDE {
        for x in 0..TILES_PER_SIDE {
            let name = tile_file_name(x, z);
            let t = match fs.read_inflated(&name) {
                Err(_) => {
                    missing += 1;
                    tiles.push(None);
                    continue;
                }
                Ok(d) => match TerrainTile::parse(&d) {
                    Ok(t) => t,
                    Err(e) => {
                        failed += 1;
                        println!("{name}: {e}");
                        tiles.push(None);
                        continue;
                    }
                },
            };
            if t.is_empty() {
                empty += 1;
            }
            let (a, b) = t.raw_range();
            lo = lo.min(a);
            hi = hi.max(b);
            if fs.contains(&tile_container_name(x, z)) {
                containers += 1;
            }
            let layout: Vec<String> = t
                .sections
                .iter()
                .map(|s| {
                    s.textures
                        .iter()
                        .map(|d| format!("{}x{}:{:?}/{}", d.width, d.height, d.format, d.mips))
                        .collect::<Vec<_>>()
                        .join("+")
                })
                .collect();
            *layouts.entry(layout.join(" | ")).or_default() += 1;
            if t.has_layers() {
                with_layers += 1;
                let zone = land.as_ref().map(|l| l.texture_zone(x, z)).unwrap_or(0);
                let table = land.as_ref().map(|l| l.metas.slots(zone));
                for cz in 0..TILE_CELLS {
                    for cx in 0..TILE_CELLS {
                        let cell = t.meta_cell(cz, cx);
                        if cell.weights.iter().map(|w| *w as u32).sum::<u32>() != 15 {
                            weight_bad += 1;
                        }
                        for (s, _) in cell.blend() {
                            slot_cells[s as usize] += 1;
                            if table.as_ref().is_some_and(|t| t[s as usize].is_none()) {
                                unknown_slot_cells += 1;
                            }
                        }
                        if let Some(l) = &land {
                            let g = t.cell_material(cz, cx);
                            match l.materials.get(g) {
                                None => *game_unknown.entry(g).or_default() += 1,
                                Some(m) => {
                                    agree_total += 1;
                                    if l.metas.by_id(m.meta_material).map(|m| m.slot)
                                        == Some(cell.dominant())
                                    {
                                        agree += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(e) = normal_layer_error(&t) {
                    normal_err += e as f64;
                    normal_n += 1;
                }
                if let Some(l) = land.as_ref().filter(|l| {
                    l.atlas_colours
                        .get(zone as usize)
                        .is_some_and(|c| !c.is_empty())
                }) && !t.is_empty()
                    && let Some(r) = map_correlation(&t, &material_picture(&t, l, zone))
                {
                    corr_sum += r;
                    corr_n += 1;
                    if r < 0.3 {
                        corr_low += 1;
                    }
                }
            }
            // the global map should agree with the tile at the tile's own 64 m samples
            let (x0, z0) = tile_origin(x, z);
            for k in 0..8 {
                for j in 0..8 {
                    let (lx, lz) = (j as f32 * 64.0, k as f32 * 64.0);
                    let fine = t.sample_local(lx, lz).unwrap();
                    let coarse = g.height_at(x0 + lx, z0 + lz);
                    global_err += (fine - coarse).abs() as f64;
                    global_n += 1;
                }
            }
            tiles.push(Some(t));
        }
    }
    println!(
        "tiles: {} parsed, {missing} missing, {failed} failed, {empty} empty (open sea), {containers} with a location container",
        n * n - missing - failed
    );
    println!(
        "heights {:.1}..{:.1} m; mean |tile − global map| at the tiles' 64 m samples: {:.2} m",
        height_to_metres(lo),
        height_to_metres(hi),
        global_err / global_n.max(1) as f64
    );
    println!("section layouts (textures per section):");
    for (layout, count) in &layouts {
        println!("  {count:5}  {layout}");
    }
    println!(
        "render layers in {with_layers} tiles; {weight_bad} cells whose weights do not sum to 15; {unknown_slot_cells} slot uses the tables do not define"
    );
    println!(
        "  slot use (cells): {}",
        slot_cells
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(s, n)| format!("{s}:{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if normal_n > 0 {
        println!(
            "  normal layer vs. the heights' gradient: mean |difference| {:.2} raw/m over {normal_n} tiles",
            normal_err / normal_n as f64
        );
    }
    if corr_n > 0 {
        println!(
            "  shader colours vs. the PDA map: mean luminance correlation {:.3} over {corr_n} land tiles, {corr_low} below 0.3",
            corr_sum / corr_n as f64
        );
    }
    if land.is_some() {
        println!(
            "  byte map → MetaMaterial → slot agrees with the dominant render slot in {agree}/{agree_total} cells ({:.1}%)",
            100.0 * agree as f64 / agree_total.max(1) as f64
        );
        if !game_unknown.is_empty() {
            println!(
                "  byte map values not in terrain_materials.bin: {}",
                game_unknown
                    .iter()
                    .map(|(k, v)| format!("{k}×{v}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    if seams {
        let mut bad_pairs = 0usize;
        let mut bad_samples = 0usize;
        for z in 0..n {
            for x in 0..n {
                let Some(t) = &tiles[z * n + x] else { continue };
                let east = if x + 1 < n {
                    tiles[z * n + x + 1].as_ref()
                } else {
                    None
                };
                let south = if z + 1 < n {
                    tiles[(z + 1) * n + x].as_ref()
                } else {
                    None
                };
                let m = t.seam_mismatches(east, south);
                if m > 0 {
                    bad_pairs += 1;
                    bad_samples += m;
                    if bad_pairs <= 10 {
                        println!("  seam mismatch at tile {x},{z}: {m} samples");
                    }
                }
            }
        }
        println!("seams: {bad_pairs} tiles with mismatching margins ({bad_samples} samples)");
    }
    Ok(())
}

fn placements_cmd(game_dir: &Path, name: &str, models_only: bool) -> Result<()> {
    use jc2_formats::placement::{bare_name, placements};
    let fs = open(game_dir)?;
    let name = name.replace('\\', "/");
    // a container: find its .blo member
    let file = if name.to_ascii_lowercase().ends_with(".blo") {
        load_property(&fs, &name)?
    } else {
        let data = fs.read_inflated(&name)?;
        let s = SmallArchive::parse(&data)?;
        let blo = s
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".blo"))
            .ok_or_else(|| anyhow::anyhow!("no .blo in {name}"))?;
        PropertyFile::parse_any(&maybe_inflate(s.data(&data, blo)?)?)?
    };
    let all = placements(&file);
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    let mut classes: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut shown = 0usize;
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &all {
        *classes.entry(p.class.as_str()).or_default() += 1;
        let t = p.position();
        if t != [0.0, 0.0, 0.0] {
            for i in 0..3 {
                lo[i] = lo[i].min(t[i]);
                hi[i] = hi[i].max(t[i]);
            }
        }
        if models_only && p.model.is_none() {
            continue;
        }
        shown += 1;
        writeln!(
            w,
            "{:>4} {:<24} {:<28} ({:9.2} {:8.2} {:9.2})  {}{}",
            p.root,
            p.class,
            p.name,
            t[0],
            t[1],
            t[2],
            p.model.as_deref().map(bare_name).unwrap_or(""),
            if p.visible { "" } else { "  [hidden]" }
        )?;
    }
    writeln!(
        w,
        "{shown} shown of {} objects, {} with models; positions x {:.0}..{:.0} y {:.0}..{:.0} z {:.0}..{:.0}",
        all.len(),
        all.iter().filter(|p| p.model.is_some()).count(),
        lo[0],
        hi[0],
        lo[1],
        hi[1],
        lo[2],
        hi[2]
    )?;
    let mut by_count: Vec<_> = classes.into_iter().collect();
    by_count.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    let summary: Vec<String> = by_count
        .iter()
        .take(12)
        .map(|(c, n)| format!("{c}×{n}"))
        .collect();
    writeln!(w, "classes: {}", summary.join(" "))?;
    Ok(())
}

fn weather_cmd(game_dir: &Path) -> Result<()> {
    use jc2_formats::weather::{
        Atmosphere, CloudShadowSettings, elevation_degrees, moon_direction, sun_direction,
    };
    let fs = open(game_dir)?;
    let a = Atmosphere::load(&fs)?;
    println!("{:#?}", a.settings);
    match CloudShadowSettings::load(&fs) {
        Ok(shadows) => {
            println!("\ncloud_shadow_intensity = {}", shadows.intensity);
            println!("Unresolved cloud-shadow properties (original keys and types):");
            for (key, value) in shadows.unknown {
                println!("  #{:08x} = {value:?}", key.hash());
            }
        }
        Err(e) => println!("\ncloud shadow settings unavailable: {e}"),
    }
    println!(
        "
sky gradient {}x{}, fog gradient {}x{} (alpha at rows 0/32/64/96/127: {:.2} {:.2} {:.2} {:.2} {:.2})",
        a.sky.width,
        a.sky.height,
        a.fog.width,
        a.fog.height,
        a.fog.texel(0, 0)[3],
        a.fog.texel(0, 32)[3],
        a.fog.texel(0, 64)[3],
        a.fog.texel(0, 96)[3],
        a.fog.texel(0, 127)[3],
    );
    println!(
        "
hour  sun elev  sun dir (engine x y z)      moon elev   sky zenith   sky horizon (rgb 0..255)"
    );
    for h in 0..24 {
        let hour = h as f32;
        let s = sun_direction(hour);
        let m = moon_direction(hour);
        let z = a.sky_colour(hour, 0.0).map(|c| (c * 255.0).round() as u32);
        let o = a.sky_colour(hour, 0.95).map(|c| (c * 255.0).round() as u32);
        println!(
            "{h:>4}  {:>7.1}  ({:>6.3} {:>6.3} {:>6.3})   {:>8.1}   {:>3} {:>3} {:>3}    {:>3} {:>3} {:>3}",
            elevation_degrees(s),
            s[0],
            s[1],
            s[2],
            elevation_degrees(m),
            z[0],
            z[1],
            z[2],
            o[0],
            o[1],
            o[2],
        );
    }
    Ok(())
}

fn vehicle_cmd(game_dir: &Path, name: &str) -> Result<()> {
    use jc2_formats::vehicle::VehicleDoll;
    let fs = open(game_dir)?;
    let name = name.replace('\\', "/");
    let data = if name.to_ascii_lowercase().ends_with(".mvdoll") {
        read_any(&fs, &name)?
    } else {
        let container = fs.read_inflated(&name)?;
        let s = SmallArchive::parse(&container)?;
        let doll = s
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".mvdoll"))
            .ok_or_else(|| anyhow::anyhow!("no .mvdoll in {name}"))?;
        maybe_inflate(s.data(&container, doll)?)?.into_owned()
    };
    let doll = VehicleDoll::parse(&data)?;
    if let Some(b) = &doll.base {
        println!(
            "base {}: mass {} kg, centre of mass {:?}",
            b.name, b.mass, b.center_of_mass
        );
    }
    if let Some(h) = &doll.heli {
        println!(
            "helicopter: {} kg, top {} km/h, lift {} N up / {} N down ({:.1} / {:.1} m/s²), tilt {:.0}°, yaw {} rad/s, pitch {} roll {}, damping {}/{}, rotor {} rpm",
            h.mass,
            h.top_speed_kmh,
            h.lift_max,
            h.lift_down,
            h.lift_max / h.mass,
            h.lift_down / h.mass,
            h.max_tilt.to_degrees(),
            h.yaw_speed,
            h.pitch_speed,
            h.roll_speed,
            h.linear_damping,
            h.angular_damping,
            h.rotor_max_rpm
        );
    }
    if let Some(e) = &doll.engine {
        println!(
            "engine {}: {}..{} rpm, {} N·m at {} rpm, upshift {:?}, downshift {:?}",
            e.name, e.rpm_min, e.rpm_max, e.torque, e.torque_rpm, e.upshift_rpm, e.downshift_rpm
        );
    }
    if let Some(t) = &doll.transmission {
        println!(
            "transmission {}: gears {:?}, reverse {}, top speed {} km/h",
            t.name, t.gears, t.reverse, t.top_speed_kmh
        );
    }
    if let Some(st) = &doll.steering {
        println!(
            "steering {}: {}° full lock, reduced above {} km/h",
            st.name, st.max_angle, st.speed_kmh
        );
    }
    for a in &doll.axles {
        println!(
            "axle {} (#{}, wheels from {}): radius {} width {} mass {} friction {} grip {}",
            a.name, a.index, a.first_wheel, a.radius, a.width, a.mass, a.friction, a.grip
        );
        if let Some(s) = &a.suspension {
            println!(
                "    suspension: travel {} m, strength {}, damping {}/{}, direction {:?}",
                s.length, s.strength, s.compression, s.relaxation, s.direction
            );
        }
        if let Some(b) = &a.brake {
            println!(
                "    brake: {} N·m{}",
                b.max_torque,
                if b.handbrake { ", handbrake" } else { "" }
            );
        }
    }
    for st in &doll.stunts {
        let t = st.position();
        println!(
            "stunt position {} ({:?}): ({:.3} {:.3} {:.3}){}",
            st.name,
            st.kind,
            t[0],
            t[1],
            t[2],
            if st.zone.is_empty() {
                String::new()
            } else {
                format!(" zone {}", st.zone)
            }
        );
        for (what, tr) in [
            ("grapple jump", &st.grapple_trigger),
            ("parachute landing", &st.parachute_trigger),
        ] {
            if let Some(tr) = tr {
                println!(
                    "    {what} trigger: centre {:?}, half extents {:?}",
                    tr.center, tr.half_extents
                );
            }
        }
    }
    for seat in &doll.seats {
        let t = seat.world.translation();
        println!(
            "seat {}: ({:.3} {:.3} {:.3}){}",
            seat.name,
            t[0],
            t[1],
            t[2],
            if seat.is_driver() { "  driver" } else { "" }
        );
    }
    println!("{} parts:", doll.parts.len());
    for p in &doll.parts {
        let t = p.world.translation();
        println!(
            "  {:indent$}{:<22} {:<40} ({:6.2} {:5.2} {:6.2}){}{}{}",
            "",
            p.name,
            p.model,
            t[0],
            t[1],
            t[2],
            p.wheel.map(|w| format!("  wheel {w}")).unwrap_or_default(),
            p.scale
                .map(|s| format!("  scale {:.3}", s[0]))
                .unwrap_or_default(),
            if p.collision.is_empty() {
                String::new()
            } else {
                format!("  col {}", p.collision)
            },
            indent = p.depth * 2
        );
    }
    Ok(())
}

fn print_shape(shape: &jc2_formats::collision::CollisionShape, depth: usize) {
    use jc2_formats::collision::CollisionShape as S;
    let pad = "  ".repeat(depth);
    match shape {
        S::Box {
            half_extents,
            radius,
        } => {
            println!("{pad}box half {half_extents:?} radius {radius}")
        }
        S::Sphere { radius } => println!("{pad}sphere radius {radius}"),
        S::Capsule { a, b, radius } => println!("{pad}capsule {a:?} .. {b:?} radius {radius}"),
        S::Cylinder { a, b, radius } => println!("{pad}cylinder {a:?} .. {b:?} radius {radius}"),
        S::ConvexHull { points, radius } => {
            println!("{pad}hull {} points radius {radius}", points.len())
        }
        S::TriMesh {
            vertices,
            triangles,
        } => println!(
            "{pad}mesh {} vertices {} triangles",
            vertices.len(),
            triangles.len()
        ),
        S::Transformed { transform, child } => {
            let t = transform.translation();
            println!(
                "{pad}transformed t=({:.3} {:.3} {:.3}) rows {:?}",
                t[0],
                t[1],
                t[2],
                [transform.row(0), transform.row(1), transform.row(2)]
            );
            print_shape(child, depth + 1);
        }
        S::Compound { children } => {
            println!("{pad}compound of {}", children.len());
            for c in children {
                print_shape(c, depth + 1);
            }
        }
        S::Unsupported { class } => println!("{pad}UNSUPPORTED {class}"),
    }
}

fn anim_cmd(game_dir: &Path, name: &str, at: Option<f32>) -> Result<()> {
    use jc2_formats::animation::{InterleavedAnimation, Skeleton, SplineAnimation};
    let fs = open(game_dir)?;
    let data = read_any(&fs, name)?;
    if let Ok(sk) = Skeleton::parse(&data) {
        println!("skeleton {:?}: {} bones", sk.name, sk.bones.len());
        for (i, b) in sk.bones.iter().enumerate() {
            let depth = {
                let mut d = 0;
                let mut p = b.parent;
                while let Some(pi) = p {
                    d += 1;
                    p = sk.bones[pi].parent;
                }
                d
            };
            let r = &sk.reference_pose[i];
            println!(
                "{i:3} {:indent$}{:<20} t=({:7.3} {:7.3} {:7.3})  q=({:6.3} {:6.3} {:6.3} {:6.3})",
                "",
                b.name,
                r.translation[0],
                r.translation[1],
                r.translation[2],
                r.rotation[0],
                r.rotation[1],
                r.rotation[2],
                r.rotation[3],
                indent = depth * 2
            );
        }
        return Ok(());
    }
    if let Ok(clip) = SplineAnimation::parse(&data) {
        println!(
            "spline clip: {} tracks, {} frames, {:.3} s ({:.4} s/frame), {} block(s), root motion {}",
            clip.track_count(),
            clip.num_frames,
            clip.duration,
            clip.frame_duration,
            clip.blocks.len(),
            match clip.root_speed() {
                Some(v) => format!("{v:.2} m/s"),
                None => "none".into(),
            }
        );
        if at.is_some() {
            for (i, r) in clip.root_motion.iter().enumerate() {
                println!("RM {i} {:.6} {:.6} {:.6} {:.6}", r[0], r[1], r[2], r[3]);
            }
        }
        let mut pose = Vec::new();
        clip.sample(at.unwrap_or(0.0), &mut pose);
        for (ti, t) in clip.blocks[0].iter().enumerate() {
            let kind = |knots: usize| if knots == 0 { "static" } else { "spline" };
            print!(
                "{ti:3} {:<20} pos {:<6} rot {:<6} scale {:<6}",
                clip.track_names[ti],
                kind(t.position.knots.len()),
                kind(t.rotation.knots.len()),
                kind(t.scale.knots.len()),
            );
            if at.is_some()
                && let Some(p) = pose.get(ti)
            {
                print!(
                    "  t=({:7.3} {:7.3} {:7.3}) q=({:6.3} {:6.3} {:6.3} {:6.3})",
                    p.translation[0],
                    p.translation[1],
                    p.translation[2],
                    p.rotation[0],
                    p.rotation[1],
                    p.rotation[2],
                    p.rotation[3]
                );
            }
            println!();
        }
        return Ok(());
    }
    let clip = InterleavedAnimation::parse(&data)?;
    println!(
        "interleaved clip: {} tracks, {} frames, {:.3} s",
        clip.track_names.len(),
        clip.frames.len(),
        clip.duration
    );
    for (ti, n) in clip.track_names.iter().enumerate() {
        println!("{ti:3} {n}");
    }
    Ok(())
}

fn ragdoll_cmd(game_dir: &Path, name: &str) -> Result<()> {
    let fs = open(game_dir)?;
    let bytes = read_any(&fs, name)?;
    let d = jc2_formats::ragdoll::Ragdoll::parse(&bytes)?;
    println!(
        "{name}: ragdoll @{:x}, {} bodies, {} joints, {} skeletons, {} mappers",
        d.identity,
        d.bodies.len(),
        d.joints.len(),
        d.skeletons.len(),
        d.mappers.len()
    );
    for (i, s) in d.skeletons.iter().enumerate() {
        println!(
            "skeleton {i}: {:?} @{:x}, {} bones{}",
            s.skeleton.name,
            s.identity,
            s.skeleton.bones.len(),
            if i == d.skeleton_index {
                " (ragdoll)"
            } else {
                ""
            }
        );
    }
    println!("bone→body: {:?}", d.bone_to_body);
    for (i, b) in d.bodies.iter().enumerate() {
        println!(
            "body {i}: {:?} @{:x} pos={:?} inverse inertia/mass={:?} damping={:?}",
            b.name,
            b.identity,
            b.collision.transform.translation(),
            b.inverse_inertia_and_mass,
            b.damping
        );
        print_shape(&b.collision.shape, 1);
    }
    for (i, j) in d.joints.iter().enumerate() {
        println!(
            "joint {i}: {:?} @{:x} {} @{:x} bodies={:?} pivot_error={:?}",
            j.name, j.identity, j.data_class, j.data_identity, j.body_indices, j.pivot_error
        );
        for l in &j.limits {
            println!(
                "  {} enabled={} range={}..{} axes={:?} tau={}",
                l.atom_class, l.enabled, l.min, l.max, l.axes, l.tau
            );
        }
        println!("  retained atoms: {:?}", j.extra_atoms);
    }
    for m in &d.mappers {
        println!(
            "mapper @{:x}: {:?} → {:?}, {} simple, {} chains, {} unmapped, keep_local={}",
            m.identity,
            d.skeletons[m.skeletons[0]].skeleton.name,
            d.skeletons[m.skeletons[1]].skeleton.name,
            m.simple.len(),
            m.chains.len(),
            m.unmapped_bones.len(),
            m.keep_unmapped_local
        );
    }
    for message in &d.diagnostics {
        println!("diagnostic: {message}");
    }
    Ok(())
}

fn collision_cmd(game_dir: &Path, name: &str) -> Result<()> {
    use jc2_formats::collision::{bodies, bounds};
    use jc2_formats::havok::Packfile;
    let fs = open(game_dir)?;
    let name = name.replace('\\', "/");
    if name.to_ascii_lowercase().ends_with(".pfx") || name.to_ascii_lowercase().ends_with(".pfs") {
        let data = read_any(&fs, &name)?;
        let pf = Packfile::parse(&data)?;
        println!(
            "{name}: {} ({} bytes of data, {} objects, {} class names)",
            pf.version,
            pf.data().len(),
            pf.objects.len(),
            pf.classnames.len()
        );
        for (vname, class, off) in pf.root_variants() {
            println!("  root variant {vname:?}: {class} @{off}");
        }
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for o in &pf.objects {
            *counts.entry(o.class.as_str()).or_default() += 1;
        }
        println!(
            "  objects: {}",
            counts
                .iter()
                .map(|(c, n)| format!("{c}×{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        for (i, b) in bodies(&pf).iter().enumerate() {
            let t = b.transform.translation();
            println!("body {i}: at ({:.3} {:.3} {:.3})", t[0], t[1], t[2]);
            print_shape(&b.shape, 1);
            if let Some((lo, hi)) = bounds(b) {
                println!("  bounds {lo:?} .. {hi:?}");
            }
        }
        return Ok(());
    }
    // a container: summarise every .pfx
    let data = fs.read_inflated(&name)?;
    let s = SmallArchive::parse(&data)?;
    let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let (mut files, mut failed) = (0usize, 0usize);
    for e in &s.entries {
        if !e.name.to_ascii_lowercase().ends_with(".pfx") {
            continue;
        }
        files += 1;
        let bytes = maybe_inflate(s.data(&data, e)?)?;
        match Packfile::parse(&bytes) {
            Ok(pf) => {
                let bs = bodies(&pf);
                let mut line = Vec::new();
                for b in &bs {
                    for (_, leaf) in b.shape.flatten() {
                        *kinds.entry(leaf.kind()).or_default() += 1;
                        line.push(leaf.kind());
                    }
                    if b.shape.flatten().is_empty() {
                        line.push("UNSUPPORTED");
                        failed += 1;
                    }
                }
                let summary: std::collections::BTreeMap<&str, usize> =
                    line.iter().fold(Default::default(), |mut m, k| {
                        *m.entry(k).or_default() += 1;
                        m
                    });
                println!(
                    "{:<40} {} bodies: {}",
                    e.name,
                    bs.len(),
                    summary
                        .iter()
                        .map(|(k, n)| format!("{k}×{n}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            Err(err) => {
                failed += 1;
                println!("{:<40} ERROR {err}", e.name);
            }
        }
    }
    println!(
        "{files} collision files, {failed} unsupported; leaves: {}",
        kinds
            .iter()
            .map(|(k, n)| format!("{k}×{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

fn vegetation_cmd(game_dir: &Path, tile: Option<(u32, u32)>, list: bool) -> Result<()> {
    use jc2_formats::terrain::{TerrainTile, tile_file_name};
    use jc2_formats::vegetation::{Fbm, Group, TileGround, Vegetation};
    let fs = open(game_dir)?;
    let veg = Vegetation::load(&fs).context("loading climate\\*")?;
    let Some((tx, tz)) = tile else {
        for (zi, z) in veg.zones.iter().enumerate() {
            let Some(z) = z else {
                println!("zone {zi}: not loaded");
                continue;
            };
            let originals = z.sets.len() / 3;
            println!(
                "zone {zi}: {} types, {} sets (×3 variants), models in {}",
                z.types.len(),
                originals,
                z.container()
            );
            for s in z.sets.iter().take(originals) {
                let per_group = |g: Group| s.table(g).iter().filter(|t| t.is_some()).count();
                println!(
                    "  {:<28} tree {:>2}/64  bush {:>2}/64  grass {:>2}/64  ({} entries)",
                    s.name,
                    per_group(Group::Tree),
                    per_group(Group::Bush),
                    per_group(Group::Grass),
                    s.entries.len()
                );
                for e in &s.entries {
                    if e.group == Group::Grass {
                        let g = z
                            .grass
                            .types
                            .iter()
                            .find(|g| g.name.eq_ignore_ascii_case(&e.type_name));
                        println!(
                            "      {:<30} grass p={:<5} slots {:<3} {}",
                            e.type_name,
                            e.probability,
                            e.count,
                            g.map(|g| format!(
                                "uv {:?} size {}..{} slope {:.0}..{:.0} deg",
                                g.uv,
                                g.size_min,
                                g.size_max,
                                g.slope_min.to_degrees(),
                                g.slope_max.to_degrees()
                            ))
                            .unwrap_or_else(|| "(grass type not in this zone)".into())
                        );
                        continue;
                    }
                    let t = z
                        .types
                        .iter()
                        .find(|t| t.name.eq_ignore_ascii_case(&e.type_name));
                    println!(
                        "      {:<30} {:<5} p={:<5} {}",
                        e.type_name,
                        format!("{:?}", e.group).to_ascii_lowercase(),
                        e.probability,
                        t.map(|t| {
                            format!(
                                "{} slope {:.3}..{:.3} height {}..{} coverage {} freq {} radius {}{}{}",
                                t.model,
                                t.slope_cos_min,
                                t.slope_cos_max,
                                t.height_min,
                                t.height_max,
                                t.coverage,
                                t.noise_frequency,
                                t.radius,
                                if t.has_collision { " collision" } else { "" },
                                if t.breakable { " breakable" } else { "" }
                            )
                        })
                        .unwrap_or_else(|| "(type not in this zone)".into())
                    );
                }
            }
        }
        let used = veg.material_names.iter().flatten().count();
        println!("{used} terrain materials name a set:");
        for (i, n) in veg.material_names.iter().enumerate() {
            let Some(n) = n else { continue };
            println!(
                "  material {i:>3} (0x{i:02x}) {:<24} -> {}",
                format!("{n:?}"),
                veg.materials[i]
                    .map(|m| format!("zone {} set {}", m.zone, m.set))
                    .unwrap_or_else(|| "none".into())
            );
        }
        return Ok(());
    };
    let data = fs.read_inflated(&tile_file_name(tx, tz))?;
    let terrain = TerrainTile::parse(&data)?;
    let ground = TileGround::new(&terrain, tx, tz);
    let fbm = Fbm::vegetation();
    let zone = veg.zone_of_tile(tx, tz);
    let instances = veg.generate_tile(tx, tz, &ground, &fbm);
    let mut per_type: std::collections::BTreeMap<String, usize> = Default::default();
    for i in &instances {
        let name = veg
            .type_of(i)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("zone {} type {}", i.zone, i.type_index));
        *per_type.entry(name).or_default() += 1;
        if list {
            println!(
                "{:<5} {:<28} ({:9.2} {:8.2} {:9.2}) yaw {:6.1}°",
                format!("{:?}", i.group).to_ascii_lowercase(),
                veg.type_of(i).map(|t| t.name.as_str()).unwrap_or("?"),
                i.position[0],
                i.position[1],
                i.position[2],
                i.yaw.to_degrees()
            );
        }
    }
    let trees = instances.iter().filter(|i| i.group == Group::Tree).count();
    println!(
        "tile {tx},{tz}: climate zone {zone}; {} instances ({trees} trees, {} bushes/rocks)",
        instances.len(),
        instances.len() - trees
    );
    for (name, n) in &per_type {
        println!("  {n:>5}  {name}");
    }
    Ok(())
}

fn cgd_cmd(game_dir: &Path, name: &str) -> Result<()> {
    use jc2_formats::cgd::CompiledGeometry;
    let fs = open(game_dir)?;
    let name = name.replace('\\', "/");
    let data = if name.to_ascii_lowercase().ends_with(".cgd") {
        read_any(&fs, &name)?
    } else {
        let blob = fs.read_inflated(&name)?;
        let s = SmallArchive::parse(&blob)?;
        let e = s
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".cgd"))
            .ok_or_else(|| anyhow::anyhow!("no .cgd in {name}"))?;
        maybe_inflate(s.data(&blob, e)?)?.into_owned()
    };
    let g = CompiledGeometry::parse(&data)?;
    println!(
        "{name}: version {}, {} instances of {} models",
        g.version,
        g.instances.len(),
        g.models.len()
    );
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for (i, inst) in g.instances.iter().enumerate() {
        let p = inst.world.translation();
        *counts.entry(g.model_of(inst)).or_default() += 1;
        println!(
            "{i:>4} {:<20} ({:9.2} {:8.2} {:9.2})  box ({:.1} {:.1} {:.1})..({:.1} {:.1} {:.1})",
            g.model_of(inst),
            p[0],
            p[1],
            p[2],
            inst.aabb_min[0],
            inst.aabb_min[1],
            inst.aabb_min[2],
            inst.aabb_max[0],
            inst.aabb_max[1],
            inst.aabb_max[2]
        );
    }
    for (m, n) in &counts {
        println!("  {n:>4}  {m}");
    }
    Ok(())
}

fn missions_cmd(
    game_dir: &Path,
    id: Option<&str>,
    all: bool,
    strongholds: bool,
    colonels: bool,
) -> Result<()> {
    use jc2_formats::missions::{
        MissionData, MissionKind, ScriptKind, colonels as find_colonels, first_property_member,
        giver_position, mission_script, objective_chain, parse_bmf, parse_regions, parse_rules,
        start_point, strongholds as find_strongholds,
    };
    use jc2_formats::terrain::tile_index;
    let fs = open(game_dir)?;
    let text = jc2_formats::text::TextDb::load(&fs, "en");
    let t = |k: &str| -> String {
        text.as_ref()
            .and_then(|d| d.get(k))
            .map(jc2_formats::text::strip_cues)
            .unwrap_or_default()
    };
    let tile = |p: [f32; 3]| {
        tile_index(p[0], p[2])
            .map(|(a, b)| format!("{a},{b}"))
            .unwrap_or_else(|| "-".into())
    };
    let icons = fs.read_inflated("icons_global.bin").ok();
    if strongholds {
        let list = find_strongholds(icons.as_deref().context("icons_global.bin")?)?;
        println!(
            "{:<7} {:<6} {:<10} {:>5}  {:<15} {:<7} {:<18} name",
            "id", "takes", "faction", "count", "count event", "tile", "base container"
        );
        for s in &list {
            println!(
                "{:<7} {:<6} {:<10} {:>5}  {:<15} {:<7} {:<18} {}",
                s.id,
                s.mission,
                s.faction().map_or("?", |f| f.name()),
                s.total,
                s.count_event,
                tile(s.position),
                s.base_container(),
                t(&format!("{}_title_txt", s.mission))
            );
        }
        println!("{} strongholds", list.len());
        return Ok(());
    }
    if colonels {
        let mut n = 0;
        let mut names: Vec<String> = fs
            .entries()
            .iter()
            .filter_map(|e| fs.names.file(e.name_hash).map(str::to_string))
            .filter(|n| {
                let l = n.to_ascii_lowercase();
                l.starts_with("locations\\") && l.ends_with(".nlz")
            })
            .collect();
        names.sort();
        for name in names {
            let Ok(data) = fs.read_inflated(&name) else {
                continue;
            };
            let Some(file) = first_property_member(&data, &[".blo"]) else {
                continue;
            };
            for c in find_colonels(&file) {
                n += 1;
                println!(
                    "{:<6} {:<28} tile {:<6} hp {:>4} guards {} doll {:<24} activity {:?} counts for {:<8} {}",
                    c.id,
                    t(&c.name_key()),
                    tile(c.position),
                    c.colonel.health,
                    c.guards.len(),
                    c.colonel.doll_container().unwrap_or_default(),
                    c.activity,
                    c.counter.clone().unwrap_or_default(),
                    jc2_formats::placement::bare_name(&name)
                );
            }
        }
        println!("{n} colonels");
        return Ok(());
    }
    let table = parse_bmf(&fs.read_inflated("missions.bmf")?)?;
    let rules = parse_rules(&fs.read_inflated("missionrules.brf")?)?;
    let data = fs
        .read_inflated("missiondata.bin")
        .ok()
        .and_then(|d| MissionData::parse(&d).ok());
    let collectable = |k: MissionKind| {
        matches!(
            k,
            MissionKind::Skulls
                | MissionKind::BlackBoxes
                | MissionKind::DrugDrops
                | MissionKind::Challenge
        )
    };
    let giver = |m: &str| {
        icons
            .as_deref()
            .and_then(|i| giver_position(i, m))
            .map(tile)
            .unwrap_or_else(|| "-".into())
    };
    match id {
        None => {
            println!(
                "{:<7} {:<20} {:<10} {:<7} {:>7} {:<7} {:>3}  name",
                "id", "kind", "faction", "after", "chaos", "giver", "obj"
            );
            for m in &table {
                if collectable(m.kind) && !all {
                    continue;
                }
                println!(
                    "{:<7} {:<20} {:<10} {:<7} {:>7} {:<7} {:>3}  {}",
                    m.id,
                    m.kind.name(),
                    m.faction().map_or("?", |f| f.name()),
                    m.prerequisite().unwrap_or("-"),
                    m.chaos,
                    giver(&m.id),
                    objective_chain(&rules, &m.id).len(),
                    t(&format!("{}_name_txt", m.id))
                );
            }
            if let Some(d) = &data {
                println!("\nrewards (missiondata.bin, max_chaos {}):", d.max_chaos);
                for (k, r) in &d.rewards {
                    println!(
                        "  {k:<26} ${:>6}  chaos {:>6}  weapon parts {}  vehicle parts {}  armour parts {}",
                        r.money, r.chaos, r.weapon_parts, r.vehicle_parts, r.armor_parts
                    );
                }
            }
            if let Ok(r) = fs.read_inflated("regions.csv") {
                println!("\nregions (regions.csv):");
                for g in parse_regions(&r) {
                    println!("  {}_r{}  from {} chaos", g.stronghold, g.index, g.chaos);
                }
            }
        }
        Some(id) => {
            let m = table
                .iter()
                .find(|m| m.id.eq_ignore_ascii_case(id))
                .with_context(|| format!("no mission {id:?} in missions.bmf"))?;
            println!(
                "{} \"{}\": {} for the {}, after {}, unlocked at {} chaos, giver on tile {}",
                m.id,
                t(&format!("{}_name_txt", m.id)),
                m.kind.name(),
                m.faction().map_or("?", |f| f.name()),
                m.prerequisite().unwrap_or("-"),
                m.chaos,
                giver(&m.id)
            );
            let desc = t(&format!("{}_intfile_desc_txt", m.id));
            if !desc.is_empty() {
                println!("  {}", desc.replace("\\n", " "));
            }
            if let Some(r) = data.as_ref().and_then(|d| d.reward(m.kind)) {
                println!(
                    "  reward: ${}, {} chaos, {} weapon parts, {} vehicle parts",
                    r.money, r.chaos, r.weapon_parts, r.vehicle_parts
                );
            }
            println!("objectives (missionrules.brf):");
            for (i, o) in objective_chain(&rules, &m.id).iter().enumerate() {
                println!("  {}. {o}", i + 1);
            }
            let objs = mission_script(&fs, &m.id)?;
            if let Some(p) = start_point(&objs) {
                println!(
                    "starts at ({:.0}, {:.0}, {:.0}), tile {}",
                    p[0],
                    p[1],
                    p[2],
                    tile(p)
                );
            }
            println!("script: {} objects", objs.len());
            for o in &objs {
                let p = o.world.translation();
                let what = match &o.kind {
                    ScriptKind::Message { show_on, text } => {
                        format!("objective \"{}\" on {show_on}", t(text))
                    }
                    ScriptKind::TargetIndicator { text } => format!("marker \"{}\"", t(text)),
                    ScriptKind::MapIcon { icon, objective } => {
                        format!("map icon {icon} of {objective}")
                    }
                    ScriptKind::EventTrigger { on, fire } => format!("{on} => {fire}"),
                    ScriptKind::TimeTrigger {
                        start_on,
                        time,
                        fire,
                        ..
                    } => format!("{start_on} +{time}s => {fire}"),
                    ScriptKind::CountTrigger {
                        up_on, max, fire, ..
                    } => format!("{max} x {up_on} => {fire}"),
                    ScriptKind::ShapeTrigger { shape, enter, .. } => {
                        format!("{shape:?} enter => {enter}")
                    }
                    ScriptKind::Dialogue { play_on, cue, done } => {
                        format!("{play_on}: \"{}\" => {done}", t(cue))
                    }
                    ScriptKind::Spawn {
                        entity,
                        spawned,
                        distance,
                        team,
                    } => format!(
                        "spawn {}{} within {distance} m => {spawned}",
                        entity.as_deref().unwrap_or(""),
                        if *team { " (soldiers)" } else { "" }
                    ),
                    ScriptKind::RestartPoint { mission } => {
                        format!("restart point of {mission}")
                    }
                    ScriptKind::Other => String::new(),
                };
                println!(
                    "  {:<22} {:<34} ({:>8.1} {:>6.1} {:>8.1}) {}{}",
                    o.class,
                    o.name,
                    p[0],
                    p[1],
                    p[2],
                    if o.enabled { "" } else { "[off] " },
                    what
                );
            }
        }
    }
    Ok(())
}

fn event_track_cmd(game_dir: &Path, name: &str) -> Result<()> {
    let fs = open(game_dir)?;
    let (resolved, track) = jc2_formats::event_track::EventTrack::resolve(&fs, name)?;
    println!(
        "{resolved}: {} records; stored order, raw clip-clock timestamps",
        track.records.len()
    );
    for event in &track.records {
        println!(
            "ordinal={} key={:08x} time={:?} selector={:?} argument={:?}",
            event.ordinal,
            event.key.hash(),
            event.time,
            event.selector,
            event.argument
        );
        println!("  raw={:?}", event.raw);
    }
    println!(
        "Native intervals are [low,high). Chronological cursor and explicit generation ownership are jc2-rs policies; no gameplay side effects are inferred."
    );
    Ok(())
}
