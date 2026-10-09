//! Music: the game's adaptive music (`sound\settings_smod_music.bin`, `music.fev`).
//!
//! The data has 14 *track groups* (`Def_Weird`, `Def_Techy`, `Nig_Scary`, `Cha_Prime`,
//! `Cha_Heat` …), each three events that are the same piece in three moods: slot 0
//! `…_Undetected`, slot 1 `…_Combat`, slot 2 `…_Evasion`. Which groups may play comes from
//! two inputs: the hour (`inputs`: from 6 o'clock the day's groups, from 18 the night's,
//! which add `Nig_Scary`) and the chaos (thresholds 50 000, 100 000, 175 000, 290 000
//! and 400 000 add the chaos groups `Cha_Prime`, `Cha_Peak`, `Cha_Heat`, `Cha_Finale`). The
//! mood is the player's alert state; here it follows the heat: no heat is undetected,
//! heat with soldiers in sight is combat, heat after they lost him is evasion (the three
//! events of the group all play, in step, and the mood is a crossfade between them).
//!
//! Each event's sample is a **6-channel MPEG**: three stereo *stems* of one piece (calm,
//! medium, intense). The event's `heat` parameter mixes them through three `Channel Mix`
//! envelopes (calm below a fifth of the heat, medium around 0.4–0.6, intense above 0.8), so
//! the music thickens as the heat rises. The samples are one to two minutes in
//! `Music_Common.fsb` and `Music_stereo.fsb` (over 100 MB each, loose in `sound\fmod\`): the
//! bank header is read and each sample by its offset, never the whole file; the frames of
//! the stems alternate in the data (`fsb::SoundBank::mpeg_pairs`).

use crate::Game;
use crate::audio::{AudioLevels, Sounds};
use crate::audio_dsp::{Shared, Voice};
use bevy::audio::PlaybackSettings;
use bevy::prelude::*;
use jc2_formats::fev::{self, WaveKind};
use jc2_formats::fsb::SoundBank;
use jc2_formats::soundmap::SoundRef;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

/// Seconds after a soldier last saw Rico that the combat music turns to evasion.
pub const EVADE_AFTER: f32 = 6.0;
/// How fast the moods crossfade (time constant, s).
const MOOD_TAU: f32 = 1.2;
/// How fast the stems follow the heat (time constant, s).
const HEAT_TAU: f32 = 1.5;
/// A group plays at least this long before the situation may change it (s).
const GROUP_HOLD: f32 = 15.0;
/// How often the situation is looked at (s).
const LOOK_EVERY: f32 = 2.0;

pub fn add(app: &mut App) {
    app.init_resource::<MusicPlayer>()
        .add_systems(Update, music);
}

/// The mood for the heat: 0 undetected, 1 combat, 2 evasion.
pub fn alert_state(heat: f32, since_contact: f32) -> usize {
    if heat <= 0.0 {
        0
    } else if since_contact < EVADE_AFTER {
        1
    } else {
        2
    }
}

/// The three moods' target levels: the chosen one up, the others down.
pub fn mood_levels(state: usize) -> [f32; 3] {
    let mut l = [0.0; 3];
    l[state.min(2)] = 1.0;
    l
}

/// The level of each of the three stems at `heat` (0..1), from the event's `Channel Mix`
/// envelopes on its `heat` parameter (the first carries the DSP's name): the kind's bit
/// pairs `0x30000`, `0xc0000` and `0x300000` are the stereo pairs 0 (intense), 1 (medium)
/// and 2 (calm).
pub fn stem_levels(e: &fev::Event, heat: f32) -> [f32; 3] {
    let mut out = [1.0; 3];
    let mut seen = [false; 3];
    for l in &e.layers {
        for v in &l.envelopes {
            let pair = match v.kind & !7 {
                0x30000 => 0,
                0xc0000 => 1,
                0x300000 => 2,
                _ => continue,
            };
            let Some(p) = e.params.get(v.param as usize).filter(|p| p.name == "heat") else {
                continue;
            };
            let x = p.normalize(p.min + heat.clamp(0.0, 1.0) * (p.max - p.min));
            out[pair] = v.value_at(x).clamp(0.0, 1.0);
            seen[pair] = true;
        }
    }
    if seen.iter().all(|s| !s) {
        return [1.0; 3];
    }
    out
}

/// The group to play: the current one while the situation still allows it, else a random
/// pick of the groups the hour and chaos allow.
pub fn choose_group(
    music: &jc2_formats::soundmap::Music,
    hour: f32,
    chaos: f32,
    current: Option<u32>,
    rand: f32,
) -> Option<u32> {
    let sel = music.groups_for(hour, chaos)?;
    if let Some(c) = current
        && sel.groups.contains(&c)
    {
        return Some(c);
    }
    let n = sel.groups.len();
    (n > 0).then(|| sel.groups[((rand * n as f32) as usize).min(n - 1)])
}

/// A music bank on disk: where it is and its parsed sample headers.
struct Streamed {
    path: PathBuf,
    bank: SoundBank,
}

impl Streamed {
    fn open(dir: &std::path::Path, name: &str) -> Option<Self> {
        let path = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })?;
        let mut f = std::fs::File::open(&path).ok()?;
        let len = f.metadata().ok()?.len() as usize;
        let mut head = vec![0u8; 48];
        f.read_exact(&mut head).ok()?;
        let n = SoundBank::header_len(&head).ok()?;
        head.resize(n, 0);
        f.seek(SeekFrom::Start(0)).ok()?;
        f.read_exact(&mut head).ok()?;
        Some(Self {
            bank: SoundBank::parse_headers(&head, len).ok()?,
            path,
        })
    }

    /// Sample `i` as plain MP3 streams, one per stereo pair, read from its place in the file.
    fn stems(&self, i: usize) -> Option<Vec<Arc<[u8]>>> {
        let r = self.bank.sample_range(i);
        let mut f = std::fs::File::open(&self.path).ok()?;
        f.seek(SeekFrom::Start(r.start as u64)).ok()?;
        let mut d = vec![0u8; r.len()];
        f.read_exact(&mut d).ok()?;
        let streams = if self.bank.samples[i].channels > 2 {
            self.bank.mpeg_pairs(i, &d)?
        } else {
            vec![self.bank.mpeg_of(i, &d)?]
        };
        Some(streams.into_iter().map(Into::into).collect())
    }
}

struct Stem {
    entity: Entity,
    shared: Arc<Shared>,
}

/// One event of a group: its stems, its mix and the gain it is mixed at.
struct Mood {
    stems: Vec<Stem>,
    event: fev::Event,
    base: f32,
}

struct Active {
    group: u32,
    moods: [Option<Mood>; 3],
    level: [f32; 3],
    /// The heat the stems follow, 0..1.
    heat: f32,
    age: f32,
}

struct Dying {
    stems: Vec<(Stem, f32)>,
    left: f32,
    total: f32,
}

#[derive(Resource, Default)]
pub struct MusicPlayer {
    banks: HashMap<String, Option<Streamed>>,
    active: Option<Active>,
    dying: Vec<Dying>,
    state: usize,
    state_age: f32,
    look: f32,
    rng: u32,
    /// The group playing now and its mood, for the tests and the log.
    pub group: Option<u32>,
    pub mood: usize,
}

impl MusicPlayer {
    fn rand(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x9e37_79b9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The stems of a music event, the event and the gain it is mixed at.
type LoadedMood = (Vec<Arc<[u8]>>, fev::Event, f32);

/// The stems of the event a slot names, the event, and the gain it is mixed at.
fn load_mood(
    player: &mut MusicPlayer,
    sounds: &mut Sounds,
    root: &std::path::Path,
    r: SoundRef,
) -> Option<LoadedMood> {
    let ev = sounds.map.as_ref()?.event(r)?;
    let file = sounds.fev(&ev.file)?;
    let e = file.event_in_group(ev.group, ev.index)?.clone();
    let gain = e.volume * file.category_volume(&e.category);
    let sd = *file.sound_defs_of(&e).first()?;
    let WaveKind::Sample { bank, index, .. } = &file.sound_defs[sd].waveforms.first()?.kind else {
        return None;
    };
    let bank_file = format!("{bank}.fsb");
    let dir = root.join("sound").join("fmod");
    let st = player
        .banks
        .entry(bank_file.clone())
        .or_insert_with(|| Streamed::open(&dir, &bank_file))
        .as_ref()?;
    Some((st.stems(*index as usize)?, e, gain))
}

/// Plays the adaptive music: picks a group for the hour and the chaos, keeps its three
/// moods in step, mixes the stems by the heat, and fades from one group to the next.
#[allow(clippy::too_many_arguments)]
fn music(
    mut commands: Commands,
    time: Res<Time>,
    game: Option<Res<Game>>,
    levels: Res<AudioLevels>,
    mut sounds: ResMut<Sounds>,
    mut player: ResMut<MusicPlayer>,
    mut voices: ResMut<Assets<Voice>>,
    clock: Option<Res<crate::sky::TimeOfDay>>,
    chaos: Option<Res<crate::chaos::Chaos>>,
    heat: Option<Res<crate::heat::Heat>>,
) {
    let dt = time.delta_secs();
    let Some(game) = game else { return };
    // the ones on their way out
    for d in &mut player.dying {
        d.left -= dt;
        for (t, g0) in &d.stems {
            t.shared.set_gain(g0 * (d.left / d.total).clamp(0.0, 1.0));
        }
        if d.left <= 0.0 {
            for (t, _) in &d.stems {
                commands.entity(t.entity).despawn();
            }
        }
    }
    player.dying.retain(|d| d.left > 0.0);
    let off = !levels.music_on || sounds.map.is_none();
    let fade = |a: Active, player: &mut MusicPlayer, secs: f32| {
        let stems: Vec<(Stem, f32)> = a
            .moods
            .into_iter()
            .flatten()
            .flat_map(|m| m.stems)
            .map(|s| {
                let g = s.shared.gain();
                (s, g)
            })
            .collect();
        player.dying.push(Dying {
            stems,
            left: secs,
            total: secs,
        });
    };
    if off {
        if let Some(a) = player.active.take() {
            fade(a, &mut player, 1.0);
        }
        player.group = None;
        return;
    }
    let (heat_value, heat01, since) = heat.map_or((0.0, 0.0, 1e9), |h| {
        (
            h.value,
            (h.value / h.max.max(1.0)).clamp(0.0, 1.0),
            h.since_contact,
        )
    });
    let want = alert_state(heat_value, since);
    let hold = sounds.map.as_ref().map_or(5.0, |m| m.music.state_hold);
    player.state_age += dt;
    if want != player.state && (player.state_age >= hold || want == 1 && player.state == 0) {
        player.state = want;
        player.state_age = 0.0;
    }
    player.look -= dt;
    if player.look <= 0.0 {
        player.look = LOOK_EVERY;
        let hour = clock.map_or(12.0, |c| c.hours);
        let chaos_total = chaos.map_or(0.0, |c| c.total as f32);
        let current = player.active.as_ref().map(|a| a.group);
        let rand = player.rand();
        let pick = sounds
            .map
            .as_ref()
            .and_then(|m| choose_group(&m.music, hour, chaos_total, current, rand));
        let age = player.active.as_ref().map_or(f32::MAX, |a| a.age);
        // the first group, or a change once the situation no longer allows the playing one
        if let Some(g) = pick
            && pick != current
            && (current.is_none() || age >= GROUP_HOLD)
        {
            let slots = sounds
                .map
                .as_ref()
                .and_then(|m| m.music.group(g))
                .map(|g| g.slots);
            if let Some(slots) = slots {
                let root = game.fs.root.clone();
                let mut moods: [Option<Mood>; 3] = [None, None, None];
                for (i, s) in slots.iter().enumerate() {
                    let Some(r) = s else { continue };
                    let Some((streams, event, base)) =
                        load_mood(&mut player, &mut sounds, &root, *r)
                    else {
                        continue;
                    };
                    let stems = streams
                        .into_iter()
                        .map(|bytes| {
                            let shared = Shared::new(0.0, 0.0, 0.0);
                            let h = voices.add(Voice {
                                bytes,
                                shared: shared.clone(),
                                looping: true,
                            });
                            let entity = commands
                                .spawn((AudioPlayer(h), PlaybackSettings::DESPAWN))
                                .id();
                            Stem { entity, shared }
                        })
                        .collect();
                    moods[i] = Some(Mood { stems, event, base });
                }
                if moods.iter().any(Option::is_some) {
                    if let Some(old) = player.active.take() {
                        fade(old, &mut player, 3.8);
                    }
                    info!("music: group {g} at {hour:.1} h, chaos {chaos_total}");
                    player.active = Some(Active {
                        group: g,
                        moods,
                        level: mood_levels(player.state),
                        heat: heat01,
                        age: 0.0,
                    });
                }
            }
        }
    }
    let state = player.state;
    let (master, vol) = (levels.master, levels.music);
    if let Some(a) = &mut player.active {
        a.age += dt;
        let k = 1.0 - (-dt / MOOD_TAU).exp();
        a.heat += (heat01 - a.heat) * (1.0 - (-dt / HEAT_TAU).exp());
        let target = mood_levels(state);
        for (i, goal) in target.iter().enumerate() {
            a.level[i] += (goal - a.level[i]) * k;
            let Some(m) = &a.moods[i] else { continue };
            let mix = if m.stems.len() > 1 {
                stem_levels(&m.event, a.heat)
            } else {
                [1.0; 3]
            };
            for (s, stem) in m.stems.iter().enumerate() {
                let lv = mix.get(s).copied().unwrap_or(1.0);
                stem.shared
                    .set_gain(a.level[i] * m.base * lv * vol * master);
            }
        }
    }
    player.group = player.active.as_ref().map(|a| a.group);
    player.mood = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::soundmap::SoundMap;

    fn test_fs() -> Option<Arc<jc2_formats::GameFs>> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = jc2_formats::GameFs::open(dir).ok()?;
        fs.contains("music.fev").then(|| Arc::new(fs))
    }

    #[test]
    fn the_mood_follows_the_heat() {
        assert_eq!(alert_state(0.0, 0.0), 0);
        assert_eq!(alert_state(0.0, 100.0), 0);
        assert_eq!(alert_state(20.0, 1.0), 1);
        assert_eq!(alert_state(20.0, EVADE_AFTER + 1.0), 2);
        assert_eq!(mood_levels(0), [1.0, 0.0, 0.0]);
        assert_eq!(mood_levels(1), [0.0, 1.0, 0.0]);
        assert_eq!(mood_levels(2), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn groups_by_hour_and_chaos() {
        let Some(fs) = test_fs() else { return };
        let map = SoundMap::load(&fs).unwrap();
        let m = &map.music;
        // by day at no chaos: never the night group (1) nor a chaos group (10..13)
        for r in [0.0, 0.3, 0.6, 0.99] {
            let g = choose_group(m, 12.0, 0.0, None, r).unwrap();
            assert!(![1, 10, 11, 12, 13].contains(&g), "{g}");
        }
        // at night the night group is among the choices
        let night: Vec<u32> = (0..20)
            .map(|i| choose_group(m, 23.0, 0.0, None, i as f32 / 20.0).unwrap())
            .collect();
        assert!(night.contains(&1));
        // a lot of chaos adds the chaos groups
        let wild: Vec<u32> = (0..40)
            .map(|i| choose_group(m, 12.0, 450_000.0, None, i as f32 / 40.0).unwrap())
            .collect();
        assert!(wild.iter().any(|g| (10..=13).contains(g)));
        // the playing group stays while the situation allows it
        assert_eq!(choose_group(m, 12.0, 0.0, Some(4), 0.9), Some(4));
        // and goes when it does not (the night group by day)
        assert_ne!(choose_group(m, 12.0, 0.0, Some(1), 0.0), Some(1));
        assert_eq!(m.state_hold, 5.0);
    }

    /// The samples stream out of the 100 MB banks by offset: three stereo stems each,
    /// every one decoding to the sample's full length.
    #[test]
    fn a_group_streams_from_the_big_banks() {
        let Some(fs) = test_fs() else { return };
        let mut s = Sounds::new(fs.clone());
        let mut player = MusicPlayer::default();
        let g = s.map.as_ref().unwrap().music.group(0).unwrap().slots;
        let root = fs.root.clone();
        let mut moods = 0;
        for r in g.iter().flatten() {
            let Some((stems, event, gain)) = load_mood(&mut player, &mut s, &root, *r) else {
                // the loose banks are not part of every install
                return;
            };
            assert!(gain > 0.0);
            assert_eq!(stems.len(), 3, "three stereo stems");
            for st in &stems {
                use bevy::audio::Decodable;
                let src = bevy::audio::AudioSource { bytes: st.clone() };
                let dec = src.decoder();
                assert_eq!(bevy::audio::Source::channels(&dec).get(), 2);
                assert_eq!(bevy::audio::Source::sample_rate(&dec).get(), 44100);
                let n = dec.count();
                // a minute or more of stereo
                assert!(n > 44100 * 2 * 30, "{n}");
            }
            assert!(event.params.iter().any(|p| p.name == "heat"));
            moods += 1;
        }
        assert_eq!(moods, 3);
    }

    /// The heat mixes the stems: calm below a fifth, intense above 0.8.
    #[test]
    fn heat_mixes_the_stems() {
        let Some(fs) = test_fs() else { return };
        let f = fev::EventFile::parse(&fs.read("music.fev").unwrap()).unwrap();
        let e = f.event("Adaptive Music/Def_Weird_Undetected").unwrap();
        let calm = stem_levels(e, 0.0);
        let hot = stem_levels(e, 1.0);
        let mid = stem_levels(e, 0.5);
        assert!(
            calm[2] > 0.9 && calm[0] < 0.01 && calm[1] < 0.01,
            "{calm:?}"
        );
        assert!(hot[2] < 0.01 && hot[0] > 0.3, "{hot:?}");
        assert!(mid[1] > 0.5 && mid[2] < 0.01, "{mid:?}");
    }

    /// The system: an app with a hero-less world starts a group and the mood crossfades
    /// to combat when the heat rises and back to undetected when it is gone.
    #[test]
    fn the_music_switches_with_the_heat() {
        let Some(fs) = test_fs() else { return };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Voice>()
            .insert_resource(Game { fs: fs.clone() })
            .init_resource::<AudioLevels>()
            .insert_resource(Sounds::new(fs))
            .init_resource::<MusicPlayer>()
            .init_resource::<crate::heat::Heat>()
            .add_systems(Update, music);
        // the first frames pick a group
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        let started = app.world().resource::<MusicPlayer>().group.is_some();
        if !started {
            return; // no loose music banks in this install
        }
        assert_eq!(app.world().resource::<MusicPlayer>().mood, 0);
        // heat with soldiers in sight → combat, then, out of sight, evasion
        {
            let mut h = app.world_mut().resource_mut::<crate::heat::Heat>();
            h.value = 40.0;
            h.since_contact = 0.0;
        }
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            app.update();
        }
        assert_eq!(app.world().resource::<MusicPlayer>().mood, 1);
        // the evasion mood waits for the state hold (5 s of game time); fast-forward it
        app.world_mut()
            .resource_mut::<crate::heat::Heat>()
            .since_contact = EVADE_AFTER + 1.0;
        app.world_mut().resource_mut::<MusicPlayer>().state_age = 10.0;
        std::thread::sleep(std::time::Duration::from_millis(30));
        app.update();
        assert_eq!(app.world().resource::<MusicPlayer>().mood, 2);
        // heat gone → undetected again
        app.world_mut().resource_mut::<crate::heat::Heat>().value = 0.0;
        app.world_mut().resource_mut::<MusicPlayer>().state_age = 10.0;
        std::thread::sleep(std::time::Duration::from_millis(30));
        app.update();
        assert_eq!(app.world().resource::<MusicPlayer>().mood, 0);
        // switched off: it stops
        app.world_mut().resource_mut::<AudioLevels>().music_on = false;
        std::thread::sleep(std::time::Duration::from_millis(30));
        app.update();
        assert_eq!(app.world().resource::<MusicPlayer>().group, None);
    }
}
