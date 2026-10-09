//! Lossless typed animation event tracks and a pure project playback cursor.
//! Native dispatch is stored-order [low, high); the chronological cursor below
//! is a jc2-rs policy, not a reproduction of the original single-wrap driver.
use crate::property::{Key, Node, PropertyFile, Variant};
use crate::{Error, GameFs, Result};

const EVENTS: u32 = 0x49f74dfd;
const TIME: u32 = 0x420ff89e;
const SELECTOR: u32 = 0xcb0cc3f4;
const ARGUMENT: u32 = 0x065b3289;

#[derive(Clone, Debug)]
pub struct EventRecord {
    pub ordinal: usize,
    pub key: Key,
    pub time: Option<f32>,
    pub selector: Option<String>,
    pub argument: Option<String>,
    /// Includes unknown fields, duplicate keys, tags and nested children.
    pub raw: Node,
}

#[derive(Clone, Debug)]
pub struct EventTrack {
    pub records: Vec<EventRecord>,
    /// Complete property tree, including additional roots/Events collections.
    pub raw: PropertyFile,
}

impl EventTrack {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw = PropertyFile::parse(bytes)?;
        let mut records = Vec::new();
        for root in &raw.nodes {
            for (_, events) in root.children.iter().filter(|(key, _)| key.hash() == EVENTS) {
                for (key, node) in &events.children {
                    let value = |hash| {
                        node.values
                            .iter()
                            .find(|(key, _)| key.hash() == hash)
                            .map(|(_, v)| v)
                    };
                    let time = match value(TIME) {
                        Some(Variant::Float(v)) => Some(*v),
                        _ => None,
                    };
                    let string = |hash| match value(hash) {
                        Some(Variant::Str(v)) => Some(v.clone()),
                        _ => None,
                    };
                    records.push(EventRecord {
                        ordinal: records.len(),
                        key: key.clone(),
                        time,
                        selector: string(SELECTOR),
                        argument: string(ARGUMENT),
                        raw: node.clone(),
                    });
                }
            }
        }
        if !raw
            .nodes
            .iter()
            .any(|root| root.children.iter().any(|(key, _)| key.hash() == EVENTS))
        {
            return Err(Error::format("event track has no Events collection"));
        }
        Ok(Self { records, raw })
    }

    /// Source override first, then compiled suffix. This is project resolution
    /// policy; the recovered native loader only establishes suffix conversion.
    pub fn load(fs: &GameFs, name: &str) -> Result<(String, Self)> {
        // A present direct override must fail visibly if unreadable/corrupt.
        let data = if fs.contains(name) {
            fs.read_inflated(name)?
        } else {
            read_track_bytes(fs, name)?
        };
        Ok((name.to_owned(), Self::parse(&data)?))
    }

    pub fn resolve(fs: &GameFs, name: &str) -> Result<(String, Self)> {
        match Self::load(fs, name) {
            Err(Error::NotFound(_)) if name.ends_with(".event_track") => {
                Self::load(fs, &format!("{name}b"))
            }
            result => result,
        }
    }

    /// Native interval helper: preserves stored order and both equal-time keys.
    pub fn native_interval(&self, low: f64, high: f64) -> Vec<usize> {
        self.records
            .iter()
            .filter(|r| r.time.is_some_and(|t| low <= t as f64 && (t as f64) < high))
            .map(|r| r.ordinal)
            .collect()
    }

    /// Diagnostic reproduction of 0090ba50 only; does not advance any clock.
    pub fn native_step(
        &self,
        old: f64,
        dt: f64,
        rate: f64,
        duration: f64,
        mode: i32,
    ) -> Vec<usize> {
        let delta = dt * rate;
        let next = old + delta;
        let mut result = Vec::new();
        if next > duration && mode == 0 {
            result.extend(self.native_interval(0.0, next - duration));
        }
        if delta >= 0.0 {
            result.extend(self.native_interval(old, next));
        } else {
            result.extend(self.native_interval(next, old));
        }
        result
    }
}

// Support explicit container/member references without changing global VFS policy.
fn read_track_bytes(fs: &GameFs, name: &str) -> Result<Vec<u8>> {
    let normalized = name.replace('/', "\\");
    let mut end = normalized.len();
    while let Some(index) = normalized[..end].rfind('\\') {
        let container = &normalized[..index];
        if fs.contains(container) {
            let blob = fs.read_inflated(container)?;
            let archive = crate::sarc::SmallArchive::parse(&blob)?;
            let member = &normalized[index + 1..];
            let entry = archive
                .get(member)
                .ok_or_else(|| Error::NotFound(name.to_owned()))?;
            return Ok(crate::compress::maybe_inflate(archive.data(&blob, entry)?)?.into_owned());
        }
        end = index;
    }
    fs.read_any(name)
}

/// Generation is supplied by the consumer; a replaced clip receives a new token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventOwner {
    pub clip: u64,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Once,
    Loop,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    pub ordinal: usize,
    /// Unwrapped occurrence time: repeated cycles remain distinguishable.
    pub time: f64,
    pub owner: EventOwner,
}

#[derive(Clone, Debug)]
pub struct EventCursor {
    duration: f64,
    position: f64,
    playback: Playback,
    owner: EventOwner,
    cancelled: bool,
}

impl EventCursor {
    pub fn new(duration: f64, playback: Playback, owner: EventOwner) -> Result<Self> {
        if !duration.is_finite() || duration <= 0.0 {
            return Err(Error::format(
                "event cursor requires positive finite duration",
            ));
        }
        Ok(Self {
            duration,
            position: 0.0,
            playback,
            owner,
            cancelled: false,
        })
    }
    pub fn position(&self) -> f64 {
        self.position
    }
    /// Seek emits nothing. Seeking backwards intentionally permits replay.
    pub fn seek(&mut self, time: f64) -> Result<()> {
        if !time.is_finite() {
            return Err(Error::format("nonfinite event seek"));
        }
        self.position = match self.playback {
            Playback::Once => time.clamp(0.0, self.duration),
            Playback::Loop => time,
        };
        Ok(())
    }
    /// Cancellation is terminal; replace the cursor with a new owner generation.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Only the explicitly selected event owner advances. No blend-weight gate
    /// or automatic outgoing/incoming dominance is inferred. Returned tokens
    /// must also be checked by consumers before applying deferred effects.
    /// Both directions select [min(old,next), max(old,next)); reverse delivery
    /// is chronological backwards, ties retain source ordinal. Loop tracks use
    /// [0,duration), so exact-duration records are retained but never scheduled.
    /// Once playback clamps at duration, leaving exact-duration events deferred.
    /// At most 100,000 occurrences/step; errors leave the cursor untouched.
    pub fn advance(
        &mut self,
        track: &EventTrack,
        dt: f64,
        rate: f64,
        active_owner: EventOwner,
    ) -> Result<Vec<Crossing>> {
        if !dt.is_finite() || dt < 0.0 || !rate.is_finite() {
            return Err(Error::format("invalid event dt/rate"));
        }
        if self.cancelled || self.owner != active_owner || dt == 0.0 || rate == 0.0 {
            return Ok(Vec::new());
        }
        let proposed = self.position + dt * rate;
        if !proposed.is_finite() {
            return Err(Error::format("event clock overflow"));
        }
        let next = match self.playback {
            Playback::Once => proposed.clamp(0.0, self.duration),
            Playback::Loop => proposed,
        };
        let low = self.position.min(next);
        let high = self.position.max(next);
        let mut crossings = Vec::new();
        for record in &track.records {
            let Some(time) = record
                .time
                .map(f64::from)
                .filter(|t| t.is_finite() && *t >= 0.0 && *t < self.duration)
            else {
                continue;
            };
            match self.playback {
                Playback::Once => {
                    if low <= time && time < high {
                        crossings.push(Crossing {
                            ordinal: record.ordinal,
                            time,
                            owner: self.owner,
                        });
                    }
                }
                Playback::Loop => {
                    let first = ((low - time) / self.duration).ceil();
                    let last = ((high - time) / self.duration).ceil();
                    if !first.is_finite()
                        || !last.is_finite()
                        || first.abs() > (1_u64 << 52) as f64
                        || last.abs() > (1_u64 << 52) as f64
                    {
                        return Err(Error::format("event cycle exceeds precise integer range"));
                    }
                    if last - first > (100_000 - crossings.len()) as f64 {
                        return Err(Error::format("event step exceeds occurrence budget"));
                    }
                    for cycle in first as i64..last as i64 {
                        let occurrence = time + cycle as f64 * self.duration;
                        if low <= occurrence && occurrence < high {
                            crossings.push(Crossing {
                                ordinal: record.ordinal,
                                time: occurrence,
                                owner: self.owner,
                            });
                        }
                    }
                }
            }
            if crossings.len() > 100_000 {
                return Err(Error::format("event step exceeds occurrence budget"));
            }
        }
        crossings.sort_by(|a, b| {
            let order = if next >= self.position {
                a.time.total_cmp(&b.time)
            } else {
                b.time.total_cmp(&a.time)
            };
            order.then(a.ordinal.cmp(&b.ordinal))
        });
        self.position = next;
        Ok(crossings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: EventOwner = EventOwner {
        clip: 1,
        generation: 2,
    };
    fn synthetic(times: &[f32]) -> EventTrack {
        let events = Node {
            children: times
                .iter()
                .enumerate()
                .map(|(i, time)| {
                    (
                        Key::Hash(i as u32),
                        Node {
                            values: vec![
                                (Key::Hash(TIME), Variant::Float(*time)),
                                (Key::Hash(SELECTOR), Variant::Str(format!("event{i}"))),
                            ],
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        };
        EventTrack::parse(
            &PropertyFile {
                nodes: vec![Node {
                    children: vec![(Key::Hash(EVENTS), events)],
                    ..Default::default()
                }],
            }
            .to_bytes(),
        )
        .unwrap()
    }
    fn ids(events: &[Crossing]) -> Vec<usize> {
        events.iter().map(|e| e.ordinal).collect()
    }
    #[test]
    fn decoder_preserves_unknowns_duplicates_and_equal_times() {
        let mut raw = synthetic(&[0.0, 0.0]).raw;
        let record = &mut raw.nodes[0].children[0].1.children[0].1;
        record.values.push((Key::Hash(TIME), Variant::Float(0.9)));
        record
            .values
            .push((Key::Hash(123), Variant::Vec3([1.0, 2.0, 3.0])));
        record.tag = Some("unknown".into());
        let track = EventTrack::parse(&raw.to_bytes()).unwrap();
        assert_eq!(track.raw, raw);
        assert_eq!(
            track.records[0].raw,
            raw.nodes[0].children[0].1.children[0].1
        );
        assert_eq!(track.native_interval(0.0, 0.1), [0, 1]);
        assert!(
            EventTrack::parse(
                &PropertyFile {
                    nodes: vec![Node::default()]
                }
                .to_bytes()
            )
            .is_err()
        );
    }
    #[test]
    fn native_boundaries_and_stored_order() {
        let track = synthetic(&[0.75, 0.0, 0.25, 1.0]);
        assert_eq!(track.native_interval(0.0, 0.25), [1]);
        assert_eq!(track.native_interval(0.25, 0.75), [2]);
        assert!(track.native_interval(0.25, 0.25).is_empty());
        assert_eq!(track.native_step(0.75, 0.5, 1.0, 1.0, 0), [1, 0, 3]);
        assert_eq!(
            track.native_step(0.75, 2.5, 1.0, 1.0, 0),
            [0, 1, 2, 3, 0, 3]
        );
        assert_eq!(track.native_step(0.75, 0.5, -1.0, 1.0, 0), [2]);
        assert!(track.native_step(0.0, 1.0, f64::NAN, 1.0, 0).is_empty());
    }
    #[test]
    fn chronological_multiwrap_keeps_equal_time_keys() {
        let track = synthetic(&[0.75, 0.0, 0.25, 0.25, 1.0]);
        let mut cursor = EventCursor::new(1.0, Playback::Loop, OWNER).unwrap();
        let hits = cursor.advance(&track, 2.5, 1.0, OWNER).unwrap();
        assert_eq!(ids(&hits), [1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(hits.last().unwrap().time, 2.25);
        assert_eq!(cursor.position(), 2.5);
    }
    #[test]
    fn exact_boundary_is_deferred_and_rate_zero_does_not_fire() {
        let track = synthetic(&[0.0, 0.25, 1.0]);
        let mut cursor = EventCursor::new(1.0, Playback::Once, OWNER).unwrap();
        assert!(cursor.advance(&track, 1.0, 0.0, OWNER).unwrap().is_empty());
        assert_eq!(ids(&cursor.advance(&track, 0.25, 1.0, OWNER).unwrap()), [0]);
        assert_eq!(ids(&cursor.advance(&track, 10.0, 1.0, OWNER).unwrap()), [1]);
        assert!(cursor.advance(&track, 10.0, 1.0, OWNER).unwrap().is_empty());
    }
    #[test]
    fn reverse_wrap_seek_and_rate_scaling() {
        let track = synthetic(&[0.75, 0.0, 0.25, 0.25]);
        let mut cursor = EventCursor::new(1.0, Playback::Loop, OWNER).unwrap();
        cursor.seek(0.75).unwrap();
        assert_eq!(
            ids(&cursor.advance(&track, 1.0, -1.0, OWNER).unwrap()),
            [2, 3, 1, 0]
        );
        assert_eq!(cursor.position(), -0.25);
        cursor.seek(0.0).unwrap();
        assert_eq!(
            ids(&cursor.advance(&track, 0.25, 2.0, OWNER).unwrap()),
            [1, 2, 3]
        );
    }
    #[test]
    fn interruption_and_crossfade_require_explicit_generation_owner() {
        let track = synthetic(&[0.0, 0.25]);
        let replacement = EventOwner {
            generation: 3,
            ..OWNER
        };
        let mut outgoing = EventCursor::new(1.0, Playback::Once, OWNER).unwrap();
        let mut incoming = EventCursor::new(1.0, Playback::Once, replacement).unwrap();
        assert!(
            outgoing
                .advance(&track, 0.5, 1.0, replacement)
                .unwrap()
                .is_empty()
        );
        assert_eq!(outgoing.position(), 0.0);
        assert_eq!(
            ids(&incoming.advance(&track, 0.5, 1.0, replacement).unwrap()),
            [0, 1]
        );
        outgoing.cancel();
        assert!(
            outgoing
                .advance(&track, 1.0, 1.0, OWNER)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn invalid_clock_and_excessive_step_are_transactional() {
        let track = synthetic(&[0.0, f32::NAN, -1.0]);
        let mut cursor = EventCursor::new(1.0, Playback::Loop, OWNER).unwrap();
        for (dt, rate) in [
            (f64::NAN, 1.0),
            (1.0, f64::NAN),
            (-1.0, 1.0),
            (100_001.0, 1.0),
            (f64::MAX, f64::MAX),
        ] {
            assert!(cursor.advance(&track, dt, rate, OWNER).is_err());
            assert_eq!(cursor.position(), 0.0);
        }
        assert!(EventCursor::new(0.0, Playback::Loop, OWNER).is_err());
        assert!(cursor.seek(f64::INFINITY).is_err());
        assert_eq!(ids(&cursor.advance(&track, 0.5, 1.0, OWNER).unwrap()), [0]);
    }
    #[test]
    fn original_footsteps_vehicle_entry_reload_and_simultaneous_landing() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP original event tracks: JC2_TEST_DIR is unset");
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        for (name, expected) in [
            (
                "upright_basic",
                vec![
                    (0.05, "footstep"),
                    (0.47, "fx_right_footstep"),
                    (0.65, "footstep"),
                    (1.08, "fx_left_footstep"),
                ],
            ),
            (
                "hjk_getin",
                vec![
                    (0.0, "open_door"),
                    (0.44, "foottouch"),
                    (0.66, "close_door"),
                    (0.9, "footstep_car"),
                ],
            ),
            ("ra_reload_event", vec![(0.29, "reload_rh")]),
        ] {
            let (resolved, track) =
                EventTrack::resolve(&fs, &format!("{name}.event_track")).unwrap();
            assert_eq!(resolved, format!("{name}.event_trackb"));
            let mut cursor = EventCursor::new(2.0, Playback::Once, OWNER).unwrap();
            let hits = cursor.advance(&track, 2.0, 1.0, OWNER).unwrap();
            assert_eq!(hits.len(), expected.len());
            for (hit, (time, selector)) in hits.iter().zip(expected) {
                assert!((hit.time - time).abs() < 1e-6);
                assert_eq!(
                    track.records[hit.ordinal].selector.as_deref(),
                    Some(selector)
                );
                assert_eq!(track.records[hit.ordinal].argument.as_deref(), Some(""));
            }
        }
        let (member_path, member) =
            EventTrack::resolve(&fs, "animations.blz/hjk_getin.event_track").unwrap();
        assert_eq!(member_path, "animations.blz/hjk_getin.event_trackb");
        assert_eq!(member.records.len(), 4);
        let (_, landing) = EventTrack::resolve(&fs, "jump_land.event_track").unwrap();
        let ids = landing.native_interval(0.0, 0.01);
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|i| landing.records[*i].time == Some(0.0)));
    }
}
