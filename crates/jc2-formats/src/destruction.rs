//! Destructible objects in the placement files. A `CDamageControl` node carries a
//! `health` (and `max_health`) and fires `on_destroy` — an event string like
//! `t{gb010.10x10({GUID})}::destroyed` or `e{cov.redbarrel.b({GUID})}::fire`, a trigger
//! (`t{…}`) or emitter (`e{…}`) followed by the event name. Objects listen with their own
//! event strings: the intact parts carry `#c7ca479b` (hide on: `…::switch` on the
//! buildings, `…::destroyed` on the red barrels), the `_dst` debris placed with
//! `visible = 0` carries `#f68f92d4` (show on) or sits under the damage control, with
//! `dynamic = 1` and an `SPhysicsParams` child giving its `mass`. Damage controls nest
//! for staged damage: the barrel's first (16 health) sets it alight (`::fire`), the
//! second (70) blows it up (`::destroyed`, with an `epe.explosion` effect emitter
//! enabled by that event).
//!
//! `destructibles` groups this into units: one per outermost damage control, with the
//! health of its whole chain, the placements to hide (indices into `placements()`) and
//! the debris to throw.

use crate::placement::{Transform3, placements};
use crate::property::{Node, PropertyFile, Variant};

const HIDE_ON: u32 = 0xc7ca_479b;
const SHOW_ON: u32 = 0xf68f_92d4;
const TARGET: u32 = 0x1b9f_7a6c;

#[derive(Clone, Debug, PartialEq)]
pub struct Destructible {
    /// The damage control's `name` (`dac.tower`, `dac.barrel.1`).
    pub name: String,
    pub root: usize,
    /// The health of the damage control and the ones nested in it.
    pub health: f32,
    /// Intact objects that go when it is destroyed (indices into `placements()`).
    pub intact: Vec<usize>,
    /// Debris that appears (`visible = 0` placements, indices into `placements()`),
    /// with each piece's mass when it has physics params.
    pub debris: Vec<(usize, Option<f32>)>,
    /// One of its effect emitters is an explosion.
    pub explodes: bool,
    /// Where it is (the middle of its intact parts).
    pub center: [f32; 3],
    /// The activity its destruction counts as (`gsy.activity.typeNN` among the events
    /// its damage controls fire, or that an event trigger listening to them fires): the
    /// chaos and money of `achievements.bin` (`crate::chaos`).
    pub activity: Option<u32>,
    /// The settlement whose completion it counts towards (`31x14.count.up` → `31x14`).
    pub counter: Option<String>,
    /// What its damage controls fire themselves (`on_destroy`, `#138b2a05`), as written:
    /// the events a mission script hears (`e{gb017b.10x10({…})}::destroyed`).
    pub fires: Vec<String>,
}

const TRIGGER_IN: u32 = 0x63f8_bec3;
const ON_DESTROY: u32 = 0x138b_2a05;
const TRIGGER_OUT: u32 = 0xbd81_1f1d;

/// The events the event triggers of a file pass on: (what they listen to, what they fire).
fn event_triggers(file: &PropertyFile) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            if n.str("_class") == Some("CEventTrigger")
                && let (Some(i), Some(o)) = (str_hash(n, TRIGGER_IN), str_hash(n, TRIGGER_OUT))
            {
                out.push((
                    i.split_whitespace().map(str::to_string).collect(),
                    o.to_string(),
                ));
            }
        });
    }
    out
}

/// How many things count towards each settlement's completion in a file: every string
/// value that fires `<settlement>.count.up` (the sabotage targets and the resource
/// pickups of the location; 15 in `31x14`).
pub fn settlement_totals(file: &PropertyFile) -> std::collections::BTreeMap<String, u32> {
    let mut out = std::collections::BTreeMap::new();
    for root in &file.nodes {
        root.walk(&mut |n| {
            for s in strings(n) {
                if let Some(id) = crate::chaos::counter_in(s) {
                    *out.entry(id.to_string()).or_insert(0) += 1;
                }
            }
        });
    }
    out
}

struct Flat<'a> {
    node: &'a Node,
    /// Index into `placements()`, for nodes with a `_class`.
    index: Option<usize>,
    /// Indices into the flat list.
    parent: Option<usize>,
    root: usize,
    world: Transform3,
}

fn flatten<'a>(file: &'a PropertyFile) -> Vec<Flat<'a>> {
    fn walk<'a>(
        node: &'a Node,
        parent: Option<usize>,
        world: Transform3,
        root: usize,
        next: &mut usize,
        out: &mut Vec<Flat<'a>>,
    ) {
        let mut here = world;
        let mut me = parent;
        if node.str("_class").is_some() {
            if let Some(Variant::Mat4x3(m)) = node.value("world") {
                here = Transform3::from_mat4x3(*m).then(&world);
            } else if let Some(Variant::Mat4x4(m)) = node.value("world") {
                here = Transform3::from_mat4x4(*m).then(&world);
            }
            out.push(Flat {
                node,
                index: Some(*next),
                parent,
                root,
                world: here,
            });
            *next += 1;
            me = Some(out.len() - 1);
        }
        for (_, c) in &node.children {
            walk(c, me, here, root, next, out);
        }
    }
    let mut out = Vec::new();
    let mut next = 0;
    for (i, root) in file.nodes.iter().enumerate() {
        walk(root, None, Transform3::IDENTITY, i, &mut next, &mut out);
    }
    out
}

/// `t{name({GUID})}` of `t{name({GUID})}::event extra`.
fn source(event: &str) -> Option<&str> {
    let e = event.split_whitespace().next()?;
    let (src, _) = e.split_once("::")?;
    (src.starts_with("t{") || src.starts_with("e{")).then_some(src)
}

fn strings(n: &Node) -> impl Iterator<Item = &str> {
    n.values.iter().filter_map(|(_, v)| match v {
        Variant::Str(s) => Some(s.as_str()),
        _ => None,
    })
}

fn str_hash(n: &Node, h: u32) -> Option<&str> {
    match n.value_hash(h)? {
        Variant::Str(s) => Some(s),
        _ => None,
    }
}

fn is_dc(n: &Node) -> bool {
    n.str("_class") == Some("CDamageControl")
}

pub fn destructibles(file: &PropertyFile) -> Vec<Destructible> {
    let flat = flatten(file);
    debug_assert_eq!(
        flat.iter().filter(|f| f.index.is_some()).count(),
        placements(file).len()
    );
    let within = |i: usize, ancestor: usize| {
        let mut p = flat[i].parent;
        while let Some(j) = p {
            if j == ancestor {
                return true;
            }
            p = flat[j].parent;
        }
        false
    };
    let has_dc_ancestor = |i: usize| {
        let mut p = flat[i].parent;
        while let Some(j) = p {
            if is_dc(flat[j].node) {
                return true;
            }
            p = flat[j].parent;
        }
        false
    };
    let visible = |n: &Node| n.int("visible").is_none_or(|v| v != 0);
    let triggers = event_triggers(file);
    let mut out = Vec::new();
    for (i, f) in flat.iter().enumerate() {
        if !is_dc(f.node) || has_dc_ancestor(i) {
            continue;
        }
        let chain: Vec<usize> = (0..flat.len())
            .filter(|&j| j == i || (is_dc(flat[j].node) && within(j, i)))
            .collect();
        let health: f32 = chain
            .iter()
            .map(|&j| flat[j].node.float("health").unwrap_or(0.0))
            .sum();
        let mut sources: Vec<&str> = chain
            .iter()
            .flat_map(|&j| strings(flat[j].node).filter_map(source))
            .collect();
        sources.sort_unstable();
        sources.dedup();
        let targets: Vec<&str> = chain
            .iter()
            .filter_map(|&j| str_hash(flat[j].node, TARGET))
            .filter_map(|t| t.split_once('{').map(|(n, _)| n))
            // the names end in a control character (`sro.go043h\x07{…}`)
            .map(|n| n.trim_matches(|c: char| c.is_control()))
            .filter(|n| !n.is_empty())
            .collect();
        let listens = |n: &Node, key: u32| {
            str_hash(n, key)
                .and_then(source)
                .is_some_and(|s| sources.contains(&s))
        };
        let mut intact = Vec::new();
        let mut debris = Vec::new();
        let mut explodes = false;
        for (j, g) in flat.iter().enumerate() {
            if g.root != f.root {
                continue;
            }
            let n = g.node;
            if n.str("_class") == Some("CEffectPointEmitter")
                && n.str("name").is_some_and(|s| s.contains("explosion"))
                && (within(j, i) || strings(n).filter_map(source).any(|s| sources.contains(&s)))
            {
                explodes = true;
            }
            if n.str("filename").is_none() {
                continue;
            }
            let Some(index) = g.index else { continue };
            if visible(n) {
                // the target names are lower-cased (`sro.go043h{…}` for `sro.GO043h`)
                let named = n
                    .str("name")
                    .is_some_and(|s| targets.iter().any(|t| t.eq_ignore_ascii_case(s)));
                if named || listens(n, HIDE_ON) {
                    intact.push(index);
                }
            } else if within(j, i) || listens(n, SHOW_ON) {
                let mass = n
                    .children
                    .iter()
                    .find(|(_, c)| c.str("_class") == Some("SPhysicsParams"))
                    .and_then(|(_, c)| c.float("mass"));
                debris.push((index, mass));
            }
        }
        if intact.is_empty() {
            // no target and no listener (the water tower): what is drawn under the damage
            // control is what goes
            intact = flat
                .iter()
                .enumerate()
                .filter(|(j, g)| {
                    g.root == f.root
                        && within(*j, i)
                        && visible(g.node)
                        && g.node.str("filename").is_some()
                        && !is_dc(g.node)
                })
                .filter_map(|(_, g)| g.index)
                .collect();
        }
        if intact.is_empty()
            && let Some(parent) = f.parent
        {
            // …or beside it, under the same geometry object (the water tower's `sro.GO098b`)
            intact = flat
                .iter()
                .enumerate()
                .filter(|(j, g)| {
                    g.root == f.root
                        && within(*j, parent)
                        && !within(*j, i)
                        && !has_dc_ancestor(*j)
                        && visible(g.node)
                        && g.node.str("filename").is_some()
                        && !is_dc(g.node)
                })
                .filter_map(|(_, g)| g.index)
                .collect();
        }
        if intact.is_empty() {
            continue;
        }
        // what its destruction fires: the damage controls' own event strings, then what
        // the event triggers listening to those fire in turn
        let mut fired: Vec<String> = chain
            .iter()
            .flat_map(|&j| {
                flat[j]
                    .node
                    .values
                    .iter()
                    .filter(|(k, _)| k.hash() != TARGET)
                    .filter_map(|(_, v)| match v {
                        Variant::Str(s) => Some(s.clone()),
                        _ => None,
                    })
            })
            .collect();
        for _ in 0..3 {
            let tokens: Vec<&str> = fired
                .iter()
                .flat_map(|s| s.split_whitespace())
                .filter(|t| t.contains("::"))
                .collect();
            let more: Vec<String> = triggers
                .iter()
                .filter(|(i, o)| {
                    !fired.contains(o) && i.iter().any(|t| tokens.contains(&t.as_str()))
                })
                .map(|(_, o)| o.clone())
                .collect();
            if more.is_empty() {
                break;
            }
            fired.extend(more);
        }
        let activity = fired.iter().find_map(|s| crate::chaos::activity_in(s));
        let counter = fired
            .iter()
            .find_map(|s| crate::chaos::counter_in(s))
            .map(str::to_string);
        let fires: Vec<String> = chain
            .iter()
            .filter_map(|&j| str_hash(flat[j].node, ON_DESTROY))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let mut center = [0.0f32; 3];
        for &k in &intact {
            let t = flat
                .iter()
                .find(|g| g.index == Some(k))
                .unwrap()
                .world
                .translation();
            for a in 0..3 {
                center[a] += t[a] / intact.len() as f32;
            }
        }
        out.push(Destructible {
            name: f.node.str("name").unwrap_or("").to_string(),
            root: f.root,
            health,
            intact,
            debris,
            explodes,
            center,
            activity,
            counter,
            fires,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `31x14.nlz`.
    #[test]
    fn village_destructibles() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("31x14.nlz") else {
            return;
        };
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.ends_with(".blo"))
            .unwrap();
        let bytes = crate::compress::maybe_inflate(sarc.data(&data, e).unwrap()).unwrap();
        let file = PropertyFile::parse_any(&bytes).unwrap();
        let d = destructibles(&file);
        let p = placements(&file);
        let barrel = d
            .iter()
            .find(|d| d.name == "dac.barrel.1")
            .expect("a red barrel");
        assert_eq!(barrel.health, 86.0);
        assert!(barrel.explodes);
        assert_eq!(p[barrel.intact[0]].name, "sro.barrel");
        assert_eq!(barrel.debris.len(), 2);
        assert_eq!(barrel.debris[0].1, Some(50.0));
        let tower = d.iter().find(|d| d.name == "dac.tower").unwrap();
        assert_eq!(tower.health, 450.0);
        assert!(!tower.intact.is_empty() && !tower.debris.is_empty());
        assert!(tower.debris.iter().all(|(i, _)| !p[*i].visible));
        assert!(d.len() >= 10, "{}", d.len());
        // chaos: the gas tubes are small fuel containers (13), the water tower (14) and
        // the broadcast towers (19, fired by an event trigger listening to the tower's
        // damage control) count towards the base
        let tube = d.iter().find(|d| d.name == "dac.tube.1").unwrap();
        assert_eq!((tube.activity, tube.counter.as_deref()), (Some(13), None));
        let acts: Vec<u32> = d.iter().filter_map(|d| d.activity).collect();
        assert!(
            acts.contains(&14) && acts.contains(&19) && acts.contains(&17),
            "{acts:?}"
        );
        let tower = d.iter().find(|d| d.activity == Some(19)).unwrap();
        assert_eq!(tower.counter.as_deref(), Some("31x14"));
        // a red barrel is a small fuel container (+10)
        assert_eq!(barrel.activity, Some(13));
        let totals = settlement_totals(&file);
        assert_eq!(totals.get("31x14"), Some(&15));
    }
}
