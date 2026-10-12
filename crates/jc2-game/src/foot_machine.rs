//! Rico's on-foot animation graph, read from the game's own data and evaluated the way the
//! native evaluator does (`research/formats/animation-statemachines-next.md`,
//! `research/formats/rico-controller-next.md`).
//!
//! * [`Machine`] is a `.afsm` (`basestatemachine.afsm`, `parachute.afsm`): states, their
//!   numbered `Transition0..` lists, the shared global list, and each transition's action,
//!   target and condition list with its polarity.
//! * [`AnimSet`] is the `.asb` join (`kc001_base.asb`): state name -> `.ban` clips, blend
//!   time and playback speed.
//! * [`FootDriver`] feeds the machine the way the character controller does: it turns the
//!   physical events of the capsule (touchdown, take-off, the chute going up or down, input)
//!   into the machine's actions and evaluates conditions through [`Sense`]. The condition
//!   *thresholds* (landing speeds, input magnitude, sea-level offsets, ray lengths) are read
//!   from the graph's own parameters, never typed here.
//!
//! What is native and what is inferred is labelled at each condition class in
//! [`evaluate`]; the dispatcher that decides *which* action the game sends (jump up versus
//! jump at, when the chord is pulled) is not decoded (JC2-170) and is an inference.

use bevy::prelude::Component;
use jc2_formats::GameFs;
use jc2_formats::hash::hash_str;
use jc2_formats::property::{Node, PropertyFile, Variant};
use std::collections::HashMap;

/// Raw key of a transition's action string (spelling unconfirmed; role from content).
const ACTION: u32 = 0x186f_e1a8;
/// Raw key of a transition's target-state string.
const TARGET: u32 = 0x45f9_d071;
/// The collection of global transition references.
const GLOBAL: u32 = 0x60dd_fab2;
/// The per-condition polarity child of a transition (value 0: the condition must be false).
const POLARITY: u32 = 0x664c_2c54;

// condition parameter keys (spellings unknown; roles from the evaluators, see the research doc)
/// Threshold of the speed / input-magnitude conditions (`007f2960`, `00725020`, `007a3230`).
const P_SPEED: u32 = 0x0643_fc50;
const P_INPUT: u32 = 0x5d86_d650;
/// 0: the value must be below the threshold; otherwise above it (strict).
const P_ABOVE: u32 = 0x3c9c_0ff1;
/// `CAtSeaLevelCondition` offset (`0059a1e0`) and joint index.
const P_SEA_OFFSET: u32 = 0x49b4_a2d6;
const P_SEA_JOINT: u32 = 0x77cf_dc9b;
/// Ray conditions: origin, direction, length, "a hit is expected".
const P_RAY_ORIGIN: u32 = 0x4aaa_87db;
const P_RAY_LEN: u32 = 0xc40d_cfd2;
const P_RAY_HIT: u32 = 0xca14_dc32;
/// `CDashEnabledCondition`: 1 negates.
const P_NEGATE: u32 = 0xd853_1e97;
/// `CAttachedToSmartObjectCondition` / `CAttachedCondition`: 1 = "not attached".
const P_NOT_ATTACHED: u32 = 0x8e63_0f66;
/// `CHeightOverGroundCondition`.
const P_HEIGHT: u32 = 0xb915_a785;

// asb keys
const ANIM_ID: u32 = 0x0703_fc2d;
/// The link's state name (spelling unknown; the probe writes it as 3175636446).
const STATE_KEY: u32 = 0xbd48_5dde;
const BAN_LIST: u32 = 0xc516_d565;
const BLEND: u32 = 0x57ab_7197;
const SPEED: u32 = 0xb7a9_d0b3;
const G_DESC: u32 = 0x28fa_9c38;
const G_LINK: u32 = 0xbe52_aa3d;

/// `CFallDamageCondition::evaluate` (`007f84c0`): the character's fall timer (`+0xa1c`) is
/// above this many seconds. The timer (`007a97f0`) adds the frame time while the main layer
/// is in `S_FALL` or `S_JUMP_TO_FALL_TRANSITION` and clears on ground contact.
pub const HEAVY_LANDING_FALL_SECONDS: f32 = 1.5;

#[derive(Clone, Debug, Default)]
pub struct Condition {
    pub class: String,
    pub params: Vec<(u32, Variant)>,
}

impl Condition {
    pub fn float(&self, key: u32) -> Option<f32> {
        self.params.iter().find(|(k, _)| *k == key).and_then(|(_, v)| match v {
            Variant::Float(f) => Some(*f),
            Variant::Int(i) => Some(*i as f32),
            _ => None,
        })
    }
    fn vec3(&self, key: u32) -> Option<[f32; 3]> {
        self.params.iter().find(|(k, _)| *k == key).and_then(|(_, v)| match v {
            Variant::Vec3(a) => Some(*a),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Transition {
    pub action: String,
    pub target: String,
    /// (condition name, required value): every condition must equal its polarity.
    pub conditions: Vec<(String, bool)>,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    /// The native state class (`CFallState`, ...): not needed to evaluate the graph; the
    /// tests pin it and the research documents the classes.
    #[allow(dead_code)]
    pub class: String,
    /// Transition names in ordinal order (`Transition0..`): the precedence.
    pub transitions: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub initial: String,
    pub states: HashMap<String, State>,
    pub transitions: HashMap<String, Transition>,
    pub conditions: HashMap<String, Condition>,
    /// The global transition list, checked after the state's own.
    pub global: Vec<String>,
}

fn sval(n: &Node, key: u32) -> Option<&str> {
    match n.value_hash(key)? {
        Variant::Str(s) => Some(s),
        _ => None,
    }
}

/// `Name0..N`-style numbered references of one node, in ordinal order.
fn numbered<'a>(n: &'a Node, prefix: &str) -> Vec<(&'a str, u32)> {
    let mut out = Vec::new();
    for i in 0..64 {
        let h = hash_str(&format!("{prefix}{i}"));
        if let Some(Variant::Str(s)) = n.value_hash(h) {
            out.push((s.as_str(), h));
        }
    }
    out
}

impl Machine {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let file = PropertyFile::parse(data).map_err(|e| e.to_string())?;
        let top = file.nodes.first().ok_or("no root")?;
        let root = top
            .children
            .iter()
            .find(|(k, _)| k.hash() == hash_str("StateMachine"))
            .map(|(_, n)| n)
            .ok_or("no StateMachine node")?;
        let name_key = hash_str("Name");
        let class_key = hash_str("Class");
        let mut m = Machine {
            initial: sval(root, hash_str("InitialState")).unwrap_or("").to_string(),
            ..Default::default()
        };
        fn child(n: &Node, key: u32) -> Option<&Node> {
            n.children.iter().find(|(k, _)| k.hash() == key).map(|(_, c)| c)
        }
        for (k, c) in &root.children {
            let h = k.hash();
            if h == hash_str("States") {
                for (_, s) in &c.children {
                    let name = sval(s, name_key).ok_or("unnamed state")?.to_string();
                    let trs = child(s, hash_str("Transitions"))
                        .map(|t| numbered(t, "Transition").into_iter().map(|(n, _)| n.to_string()).collect())
                        .unwrap_or_default();
                    m.states.insert(
                        name,
                        State {
                            class: sval(s, class_key).unwrap_or("").to_string(),
                            transitions: trs,
                        },
                    );
                }
            } else if h == hash_str("Transitions") {
                for (_, t) in &c.children {
                    let name = sval(t, name_key).ok_or("unnamed transition")?.to_string();
                    let refs = child(t, hash_str("Conditions"))
                        .map(|cd| {
                            (0..16)
                                .filter_map(|i| {
                                    let key = hash_str(&format!("Condition{i}"));
                                    match cd.value_hash(key) {
                                        Some(Variant::Str(s)) => Some((s.clone(), key)),
                                        _ => None,
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let polarity = child(t, POLARITY);
                    let conditions = refs
                        .into_iter()
                        .map(|(n, key)| {
                            let want = match polarity.and_then(|p| p.value_hash(key)) {
                                Some(Variant::Int(0)) => false,
                                Some(Variant::Float(f)) if *f == 0.0 => false,
                                _ => true,
                            };
                            (n, want)
                        })
                        .collect();
                    m.transitions.insert(
                        name,
                        Transition {
                            action: sval(t, ACTION).unwrap_or("").to_string(),
                            target: sval(t, TARGET).unwrap_or("").to_string(),
                            conditions,
                        },
                    );
                }
            } else if h == hash_str("Conditions") {
                for (_, cn) in &c.children {
                    let name = sval(cn, name_key).ok_or("unnamed condition")?.to_string();
                    let params = cn
                        .values
                        .iter()
                        .filter(|(k, _)| k.hash() != name_key && k.hash() != class_key)
                        .map(|(k, v)| (k.hash(), v.clone()))
                        .collect();
                    m.conditions.insert(
                        name,
                        Condition {
                            class: sval(cn, class_key).unwrap_or("").to_string(),
                            params,
                        },
                    );
                }
            } else if h == GLOBAL
                && let Some(t) = child(c, hash_str("Transitions"))
            {
                m.global = numbered(t, "Transition").into_iter().map(|(n, _)| n.to_string()).collect();
            }
        }
        Ok(m)
    }

    pub fn load(fs: &GameFs, file: &str) -> Option<Self> {
        Self::parse(&fs.read_any(file).ok()?).ok()
    }

    /// Dispatch `action` in `state`: the first transition in the state's own list and then
    /// the global list whose action matches and whose conditions all hold. Conditions the
    /// evaluator cannot decide count as not holding and are returned in `unsupported`.
    pub fn dispatch<'a>(
        &'a self,
        state: &str,
        action: &str,
        eval: &mut dyn FnMut(&Condition) -> Option<bool>,
        unsupported: &mut Vec<String>,
    ) -> Option<(&'a str, &'a str)> {
        let local = self.states.get(state).map(|s| s.transitions.as_slice()).unwrap_or(&[]);
        for tn in local.iter().chain(self.global.iter()) {
            let Some(t) = self.transitions.get(tn) else { continue };
            if t.action != action {
                continue;
            }
            let mut ok = true;
            for (cn, want) in &t.conditions {
                let Some(c) = self.conditions.get(cn) else {
                    ok = false;
                    break;
                };
                match eval(c) {
                    Some(v) if v == *want => {}
                    Some(_) => {
                        ok = false;
                        break;
                    }
                    None => {
                        if !unsupported.contains(cn) {
                            unsupported.push(cn.clone());
                        }
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return Some((tn.as_str(), t.target.as_str()));
            }
        }
        None
    }
}

/// One state's clips and timing from the animation set.
#[derive(Clone, Debug, Default)]
pub struct StateAnim {
    /// Lower-case `.ban` file names (the order of the file is hash order, not meaning).
    pub bans: Vec<String>,
    /// `57ab7197`: the blend-in time (suggestive; the existing runtime treats it as one).
    pub blend: f32,
    /// `playback_speed` (`b7a9d0b3`, spelling confirmed by hashing).
    pub speed: f32,
}

#[derive(Clone, Debug, Default)]
pub struct AnimSet {
    pub states: HashMap<String, StateAnim>,
}

impl AnimSet {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let file = PropertyFile::parse(data).map_err(|e| e.to_string())?;
        let top = file.nodes.first().ok_or("no root")?;
        let mut groups: HashMap<u32, &Node> = HashMap::new();
        for (_, c) in &top.children {
            for (k, g) in &c.children {
                groups.insert(k.hash(), g);
            }
        }
        let num = |n: &Node, key: u32| -> Option<f32> {
            match n.value_hash(key)? {
                Variant::Float(f) => Some(*f),
                Variant::Int(i) => Some(*i as f32),
                _ => None,
            }
        };
        let mut desc: HashMap<String, StateAnim> = HashMap::new();
        if let Some(g) = groups.get(&G_DESC) {
            for (_, d) in &g.children {
                let Some(id) = sval(d, ANIM_ID) else { continue };
                let bans = d
                    .children
                    .iter()
                    .find(|(k, _)| k.hash() == BAN_LIST)
                    .map(|(_, l)| {
                        l.values
                            .iter()
                            .filter_map(|(_, v)| match v {
                                Variant::Str(s) => Some(
                                    s.rsplit(['\\', '/']).next().unwrap_or(s).to_ascii_lowercase(),
                                ),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                desc.insert(
                    id.to_string(),
                    StateAnim {
                        bans,
                        blend: num(d, BLEND).unwrap_or(0.0),
                        speed: num(d, SPEED).unwrap_or(1.0),
                    },
                );
            }
        }
        let mut set = AnimSet::default();
        if let Some(g) = groups.get(&G_LINK) {
            for (_, l) in &g.children {
                let (Some(id), Some(state)) = (sval(l, ANIM_ID), sval(l, STATE_KEY)) else {
                    continue;
                };
                if let Some(d) = desc.get(id) {
                    set.states.insert(state.to_string(), d.clone());
                }
            }
        }
        Ok(set)
    }

    pub fn load(fs: &GameFs, file: &str) -> Option<Self> {
        Self::parse(&fs.read_any(file).ok()?).ok()
    }
}

/// Everything the condition evaluators read from the character, in game units (metres,
/// seconds, m/s). The driver fills it each frame.
#[derive(Clone, Debug)]
pub struct Sense {
    pub ground_contact: bool,
    /// `+0xa1c`: seconds in `S_FALL` / `S_JUMP_TO_FALL_TRANSITION` since the last ground contact.
    pub fall_time: f32,
    /// Length of the movement input vector (`0 ..= 1`).
    pub input_magnitude: f32,
    /// Horizontal speed (m/s) of the character.
    pub horizontal_speed: f32,
    /// Vertical speed (m/s, up positive).
    pub vertical_speed: f32,
    /// The state's clip has finished (`CAnimationFinishCondition`).
    pub anim_finished: bool,
    /// The main layer's crossfade has finished (`CLayerIsBlendFinished`).
    pub main_blend_finished: bool,
    /// Root joint (feet) and Hips heights over the sea surface (m).
    pub feet_above_sea: f32,
    pub hips_above_sea: f32,
    /// In the sea past the point he swims (the controller's own swimming flag).
    pub swimming: bool,
    /// Metres from the feet to the ground below (large when none within the probe).
    pub ground_clearance: f32,
    /// Metres to the first obstacle straight ahead (large when none within the probe).
    pub forward_clearance: f32,
    /// The sprint key is down.
    pub dash_enabled: bool,
}

impl Default for Sense {
    fn default() -> Self {
        Self {
            ground_contact: true,
            fall_time: 0.0,
            input_magnitude: 0.0,
            horizontal_speed: 0.0,
            vertical_speed: 0.0,
            anim_finished: false,
            main_blend_finished: true,
            feet_above_sea: 10.0,
            hips_above_sea: 11.0,
            swimming: false,
            ground_clearance: 0.0,
            forward_clearance: f32::INFINITY,
            dash_enabled: false,
        }
    }
}

fn compare(value: f32, threshold: f32, above: bool) -> bool {
    // native: flag 0 -> `value < threshold`; otherwise `threshold < value` (both strict:
    // `007f2960`, `00725020`, `007a3230`, `0059a1e0`)
    if above { threshold < value } else { value < threshold }
}

/// Evaluate one condition. `None`: the class (or this parameterisation) is not supported
/// and the caller must not treat it as true.
pub fn evaluate(c: &Condition, s: &Sense) -> Option<bool> {
    let above = c.float(P_ABOVE).unwrap_or(0.0) != 0.0;
    Some(match c.class.as_str() {
        // native 0088b250 / its negation
        "CGroundContactCondition" => s.ground_contact,
        "CInAirCondition" => !s.ground_contact,
        // native 007f84c0
        "CFallDamageCondition" => s.fall_time > HEAVY_LANDING_FALL_SECONDS,
        // native 007a3230, 007f2960, 00725020
        "CInputMagnitudeCondition" => compare(s.input_magnitude, c.float(P_INPUT)?, above),
        "CHorizontalSpeedCondition" => compare(s.horizontal_speed, c.float(P_SPEED)?, above),
        "CVerticalSpeedCondition" => compare(s.vertical_speed, c.float(P_SPEED)?, above),
        // native 00817d10 (on foot), 007340c0 (canopy: its own object; the same signal here)
        "CAnimationFinishCondition" | "CParachuteAnimationFinishCondition" => s.anim_finished,
        "CLayerIsBlendFinished" => s.main_blend_finished,
        // native 0059a1e0: joint height over the water surface. The swim-depth family is
        // answered by the controller's own swimming flag (inferred: its threshold sits at
        // the Hips and the hero's capsule centre stands in for it).
        "CAtSeaLevelCondition" => {
            let off = c.float(P_SEA_OFFSET)?;
            let hips = c.float(P_SEA_JOINT).is_some_and(|j| j == 1.0);
            if off < 0.0 && !above {
                s.swimming
            } else if off < 0.0 && above && off > -0.2 {
                !s.swimming
            } else {
                let h = if hips { s.hips_above_sea } else { s.feet_above_sea };
                compare(h, off, above)
            }
        }
        // native 007caa40 casts from `origin` along `dir` for `len`; only the downward
        // ray is supported (the fan geometry is a Gap and is answered by the same downward
        // clearance: inferred). A hit is expected when `ca14dc32` is non-zero.
        "CRayCastCondition" | "CRayCastFanCondition" => {
            let origin = c.vec3(P_RAY_ORIGIN).map_or(0.0, |o| o[1]);
            let len = c.float(P_RAY_LEN)?;
            let hit = s.ground_clearance <= len - origin;
            hit == (c.float(P_RAY_HIT).unwrap_or(0.0) != 0.0)
        }
        // forward rays: the clearance ahead against the length (fan arc: Gap)
        "CForwardRayCastFanCondition" => {
            let hit = s.forward_clearance <= c.float(P_RAY_LEN)?;
            hit == (c.float(P_RAY_HIT).unwrap_or(0.0) != 0.0)
        }
        // native 006cccb0: height over the ground, 0 on the ground; flag 0 -> threshold <= h
        "CHeightOverGroundCondition" => {
            let h = if s.ground_contact { 0.0 } else { s.ground_clearance };
            let t = c.float(P_HEIGHT)?;
            if c.float(P_RAY_HIT).unwrap_or(0.0) == 0.0 { t <= h } else { h <= t }
        }
        "CDashEnabledCondition" => s.dash_enabled != (c.float(P_NEGATE).unwrap_or(0.0) != 0.0),
        // never attached to a smart object, never in ghost mode, alive (the hero only)
        "CAttachedToSmartObjectCondition" | "CAttachedCondition" => {
            c.float(P_NOT_ATTACHED).unwrap_or(0.0) != 0.0
        }
        "CGhostModeCondition" => true,
        "CCharacterIsAliveCondition" => true,
        // native 00583170: the layer's state equals the named one. The upper body machine is
        // not tracked by name here; the wielding states are transient (inferred: not in them).
        "CLayerIsInStateCondition" => false,
        _ => return None,
    })
}

/// The main-layer machine's presentation of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    pub time: f32,
    pub action: &'static str,
    pub from: String,
    pub to: String,
    pub transition: String,
}

/// The physical events of one frame the driver turns into actions.
#[derive(Clone, Debug, Default)]
pub struct FootFrame {
    pub dt: f32,
    /// The ground contact changed from air to ground this frame.
    pub touched_down: bool,
    /// He left the ground by jumping this frame.
    pub jumped: bool,
    /// Movement input present.
    pub moving: bool,
    /// Aiming (legs strafe).
    pub aiming: bool,
    pub sprint: bool,
    /// The canopy is wanted open (the controller's chute flag).
    pub chute_wanted: bool,
    /// The crouch key is down (hold mode: the routine `005d4e60` sends `ACT_CROUCH` every
    /// frame it is) and nothing the controller blocks it for is going on.
    pub crouch: bool,
    /// The evade key is down; the press and the tap-release are read from its level.
    pub evade_down: bool,
    /// The input device is the keyboard/mouse (setting `0x10` is 0): it evades on the press.
    pub keyboard: bool,
}

/// What a step needs besides the physical events: the graph, and the question "has the clip of
/// this state finished playing" (the animator's, asked per state because a state entered
/// earlier in the same step has not played yet).
pub struct Ctx<'a> {
    pub machine: &'a Machine,
    pub finished: &'a dyn Fn(&str) -> bool,
}

/// The state of one character's main-layer machine.
#[derive(Component, Clone, Debug)]
pub struct FootDriver {
    pub state: String,
    /// Seconds since the state was entered.
    pub state_time: f32,
    /// `+0xa1c` (see [`HEAVY_LANDING_FALL_SECONDS`]).
    pub fall_time: f32,
    /// Horizontal speed when he last left the ground or last stepped in the air, for the
    /// landing conditions (the controller zeroes the velocity on contact).
    pub air_velocity: (f32, f32),
    /// Seconds on the ground in a canopy state that no landing edge accepted.
    stuck: f32,
    /// Record every taken transition (tests, traces).
    pub trace: Option<Vec<Trace>>,
    /// The ground contact of the previous frame (take-off and touchdown edges).
    pub was_grounded: bool,
    clock: f32,
    /// Conditions the evaluator could not decide, for diagnostics.
    pub unsupported: Vec<String>,
    /// The evade key's timing (`foot_actions::EvadeKey`).
    pub evade_key: crate::foot_actions::EvadeKey,
    /// `ACT_CROUCH` was accepted this frame: the stand (`ACT_NONE`) is not sent (inferred from
    /// `005d4e60`, which raises the crouch flags `+0x259/+0x265` only when it is accepted).
    crouched_now: bool,
}

impl Default for FootDriver {
    fn default() -> Self {
        Self::new("S_UPRIGHT_IDLE")
    }
}

/// States that are the canopy being pulled, unfolded or flown.
pub fn is_chute_state(s: &str) -> bool {
    matches!(
        s,
        "S_PULL_OPEN_PARACHUTE_VERTICAL"
            | "S_UNFOLD_PARACHUTE_VERTICAL"
            | "S_UNFOLD_PARACHUTE_HORIZONTAL"
            | "S_PARACHUTE"
    )
}

fn is_swim_state(s: &str) -> bool {
    s.starts_with("S_SWIM") || s == "S_BLEND_TO_SWIM_IDLE"
}

impl FootDriver {
    pub fn new(state: &str) -> Self {
        Self {
            state: state.to_string(),
            state_time: 0.0,
            fall_time: 0.0,
            air_velocity: (0.0, 0.0),
            stuck: 0.0,
            trace: None,
            was_grounded: true,
            clock: 0.0,
            unsupported: Vec::new(),
            evade_key: Default::default(),
            crouched_now: false,
        }
    }

    /// The machine has been stepped at least once (the data was there to drive it).
    pub fn stepped(&self) -> bool {
        self.clock > 0.0
    }

    fn enter(&mut self, to: &str) {
        if self.state != to {
            self.state = to.to_string();
            self.state_time = 0.0;
        }
    }

    /// Force a state (leaving the machine's control: grapple, vehicle; or a resync).
    pub fn set_state(&mut self, to: &str) {
        self.enter(to);
        self.stuck = 0.0;
    }

    /// Dispatch one action: `None` when no transition accepts it, otherwise whether the
    /// state changed (a transition to the same state re-enters it).
    fn send(&mut self, ctx: &Ctx, action: &'static str, sense: &Sense) -> Option<bool> {
        // the state's clip finished or not is asked of the caller for the state the
        // dispatch happens in: a state entered a moment ago has not played yet
        let mut s = sense.clone();
        s.anim_finished = (ctx.finished)(&self.state);
        let mut eval = |c: &Condition| evaluate(c, &s);
        let mut unsupported = Vec::new();
        let taken = ctx
            .machine
            .dispatch(&self.state, action, &mut eval, &mut unsupported)
            .map(|(t, to)| (t.to_string(), to.to_string()));
        for u in unsupported {
            if !self.unsupported.contains(&u) {
                self.unsupported.push(u);
            }
        }
        let (transition, to) = taken?;
        if let Some(trace) = &mut self.trace
            && (self.state != to || action != "ACT_UPDATE")
        {
            trace.push(Trace {
                time: self.clock,
                action,
                from: self.state.clone(),
                to: to.clone(),
                transition,
            });
        }
        let changed = self.state != to;
        self.enter(&to);
        Some(changed)
    }

    /// One frame: the physical events become actions in the controller's order, then
    /// `ACT_UPDATE`. The order is inferred (the dispatcher is JC2-170).
    pub fn step(&mut self, ctx: &Ctx, f: &FootFrame, sense: &mut Sense) {
        self.clock += f.dt;
        self.state_time += f.dt;
        // native 007a97f0: the fall timer runs only in the two fall states, and clears on
        // ground contact in any other state
        if matches!(self.state.as_str(), "S_FALL" | "S_JUMP_TO_FALL_TRANSITION") {
            self.fall_time += f.dt;
        } else if sense.ground_contact {
            self.fall_time = 0.0;
        }
        sense.fall_time = self.fall_time;

        let g = sense.ground_contact;
        if f.touched_down {
            // the controller zeroes the velocity on contact: the conditions read the last
            // airborne velocity instead (the chute's bands are on the speed at touchdown)
            sense.horizontal_speed = self.air_velocity.0;
            sense.vertical_speed = self.air_velocity.1;
        }
        if f.jumped {
            // native (`00816700`, the only dispatcher of the jump actions; JC2-170): the
            // player's jump is `ACT_JUMP_AT` from every state, standing or running. The
            // `ACT_JUMP_UP` edges (the `std_still_jump_*` clips) have no caller in the exe.
            if self.send(ctx, "ACT_JUMP_AT", sense).is_none() {
                // non-native resync: the jump left the ground from a state with no jump edge
                self.set_state("S_FALL");
            }
        }
        // native order of the handler (`007b6520`): jump, kick, crouch, evade, then the
        // locomotion intent (the relative order to the canopy and the landing is inferred)
        self.crouched_now = false;
        if f.crouch && !f.jumped {
            // hold mode (setting `0x28` = 0): `005d4e60` sends `ACT_CROUCH`; the graph accepts
            // it from the ground states when the input magnitude is not above 0.7
            self.crouched_now = self.send(ctx, "ACT_CROUCH", sense).is_some();
        }
        let fire = self.evade_key.update(f.dt, f.evade_down, f.keyboard);
        if (fire || (self.state == "S_EVADE" && self.evade_key.queued())) && !f.jumped {
            // `00a3d500` needs `ACT_EVADE` accepted on layer 0; the retry in `S_EVADE` finds
            // no edge there (the state's only edge is `ACT_UPDATE`), so it never lands
            self.send(ctx, "ACT_EVADE", sense);
        }
        if f.touched_down {
            // the canopy's landing edges and the jump-to-fall transition's are `ACT_LAND`
            // edges; the fall states land on `ACT_UPDATE` + ground contact (below)
            self.send(ctx, "ACT_UPDATE", sense);
            self.send(ctx, "ACT_LAND", sense);
        }
        // the chute: level-triggered (open while wanted and not yet open; close while open
        // and no longer wanted). Never on the ground: landing handles that.
        if !g {
            let open = is_chute_state(&self.state);
            if (f.chute_wanted && !open) || (!f.chute_wanted && open) {
                self.send(ctx, "ACT_PULLCHORD", sense);
            }
        }
        // native (`005ff2f0`): no sprint within the tap threshold of an evade press
        let mut f = f.clone();
        if !self.evade_key.sprint_allowed() {
            f.sprint = false;
            sense.dash_enabled = false;
        }
        let f = &f;
        if (g || sense.swimming) && !f.jumped {
            self.ground_input(ctx, f, sense);
        }
        if is_swim_state(&self.state) {
            let action = if f.moving { "ACT_NAVIGATE" } else { "ACT_NONE" };
            self.send(ctx, action, sense);
        }
        self.send(ctx, "ACT_UPDATE", sense);
        // non-native resync: a canopy state that never accepted the landing (the speed bands
        // leave a gap between -8.0 and -8.1 m/s) lands as the nice landing after a quarter second
        if g && is_chute_state(&self.state) {
            self.stuck += f.dt;
            if self.stuck > 0.25 {
                self.set_state("S_PARACHUTE_LANDING");
            }
        } else {
            self.stuck = 0.0;
        }
        // non-native resync: the swim states without the sea under him
        if g && !sense.swimming && is_swim_state(&self.state) {
            self.set_state(if f.moving { "S_UPRIGHT_BASIC_NAVIGATION" } else { "S_UPRIGHT_IDLE" });
        }
    }

    /// The actions of walking, running, strafing and stopping (inferred dispatch).
    fn ground_input(&mut self, ctx: &Ctx, f: &FootFrame, sense: &Sense) {
        if f.moving {
            if sense.swimming {
                let action = if f.aiming { "ACT_STRAFE" } else { "ACT_NAVIGATE" };
                self.send(ctx, action, sense);
            } else if f.sprint {
                // the idle and stop states reach the dash through the navigation first
                if self.send(ctx, "ACT_DASH", sense).is_none() {
                    self.send(ctx, "ACT_NAVIGATE", sense);
                }
            } else if f.aiming {
                self.send(ctx, "ACT_STRAFE", sense);
            } else {
                self.send(ctx, "ACT_NAVIGATE", sense);
            }
            // non-native (the dispatch of the roll's tail is not decoded, Gap 4 of
            // `rico-crouch-evade` in `rico-controller-next.md`): `S_EVADE_INTERRUPT` has no
            // navigate, strafe or dash edge; its way on is `ACT_NONE` (to idle, or to the
            // dash by `ACT_UPDATE` when the sprint is on)
            if self.state == "S_EVADE_INTERRUPT" {
                self.send(ctx, "ACT_NONE", sense);
            }
        } else if matches!(self.state.as_str(), "S_UPRIGHT_BASIC_NAVIGATION" | "S_DASH") {
            self.send(ctx, "ACT_UPRIGHT_STOP", sense);
        } else if !self.crouched_now {
            // (a held crouch that was accepted this frame is not told to stand)
            self.send(ctx, "ACT_NONE", sense);
        }
    }

    /// Would the current state take `action` now (the native gates ask exactly this: the
    /// jump handler needs `ACT_JUMP_AT` accepted, the canopy opens only if `ACT_PULLCHORD` is).
    pub fn accepts(&self, m: &Machine, action: &str, sense: &Sense) -> bool {
        let mut u = Vec::new();
        m.dispatch(&self.state, action, &mut |c| evaluate(c, sense), &mut u).is_some()
    }

    /// The speed and direction memory for the landing conditions: call every airborne frame.
    pub fn note_air(&mut self, horizontal: f32, vertical: f32) {
        self.air_velocity = (horizontal, vertical);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> Option<GameFs> {
        GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()
    }

    fn base(fs: &GameFs) -> Machine {
        Machine::load(fs, "basestatemachine.afsm").expect("basestatemachine.afsm")
    }

    fn go(m: &Machine, from: &str, action: &str, s: &Sense) -> Option<String> {
        let mut u = Vec::new();
        m.dispatch(from, action, &mut |c| evaluate(c, s), &mut u).map(|(_, to)| to.to_string())
    }

    /// The parachute canopy graph, edge by edge (`parachute.afsm`: 5 states, 6 transitions).
    #[test]
    fn canopy_graph_matches_the_decoded_trace() {
        let Some(fs) = fs() else { return };
        let m = Machine::load(&fs, "parachute.afsm").expect("parachute.afsm");
        assert_eq!((m.states.len(), m.transitions.len(), m.conditions.len()), (5, 6, 1));
        assert_eq!(m.initial, "PCS_CLOSED");
        let mut s = Sense { anim_finished: false, ..Sense::default() };
        assert_eq!(go(&m, "PCS_CLOSED", "PCA_OPEN_VERTICAL", &s).as_deref(), Some("PCS_OPENING_VERTICAL"));
        assert_eq!(go(&m, "PCS_CLOSED", "PCA_OPEN_HORIZONTAL", &s).as_deref(), Some("PCS_OPENING_HORIZONTAL"));
        // an unfinished update stays, a finished one opens
        assert_eq!(go(&m, "PCS_OPENING_VERTICAL", "PCA_UPDATE", &s), None);
        s.anim_finished = true;
        assert_eq!(go(&m, "PCS_OPENING_VERTICAL", "PCA_UPDATE", &s).as_deref(), Some("PCS_OPEN"));
        assert_eq!(go(&m, "PCS_OPEN", "PCA_CLOSE", &s).as_deref(), Some("PCS_CLOSING"));
        assert_eq!(go(&m, "PCS_CLOSING", "PCA_UPDATE", &s).as_deref(), Some("PCS_CLOSED"));
        assert_eq!(go(&m, "PCS_CLOSING", "PCA_OPEN_VERTICAL", &s).as_deref(), Some("PCS_OPENING_VERTICAL"));
        assert_eq!(go(&m, "PCS_OPENING_HORIZONTAL", "PCA_CLOSENOW", &s).as_deref(), Some("PCS_CLOSED"));
    }

    /// The polarity child (`664c2c54`) is read: `T_JUMP_AT` needs the upper body NOT to be
    /// in the wielding states; plain transitions need their conditions true.
    #[test]
    fn polarity_is_read() {
        let Some(fs) = fs() else { return };
        let m = base(&fs);
        let t = &m.transitions["T_JUMP_AT"];
        assert_eq!(
            t.conditions,
            vec![
                ("C_WAIT_FOR_WIELDING_UP".to_string(), false),
                ("C_WAIT_FOR_WIELDING_DOWN".to_string(), false)
            ]
        );
        let t = &m.transitions["T_JUMP_LAND_RUN"];
        assert_eq!(t.conditions.iter().filter(|(_, w)| !w).count(), 0);
        assert_eq!(m.states["S_PARACHUTE"].class, "CParachuteState");
        assert_eq!(m.states["S_JUMP_START_STANDING"].class, "CJumpControllerState");
        assert_eq!(m.states.len(), 272);
        assert_eq!(m.transitions.len(), 388);
        assert_eq!(m.global.len(), 4);
    }

    /// The parachute landing bands are read from the graph and evaluated with the native
    /// strict comparisons: nice (horizontal < 8.1 and vertical > -8.0), then roll
    /// (horizontal > 8.0), then hard (vertical < -8.1).
    #[test]
    fn chute_landing_bands() {
        let Some(fs) = fs() else { return };
        let m = base(&fs);
        let land = |h: f32, v: f32| {
            let s = Sense { horizontal_speed: h, vertical_speed: v, ..Default::default() };
            go(&m, "S_PARACHUTE", "ACT_LAND", &s)
        };
        assert_eq!(land(5.0, -5.5).as_deref(), Some("S_PARACHUTE_LANDING"));
        assert_eq!(land(8.05, -5.5).as_deref(), Some("S_PARACHUTE_LANDING")); // nice before roll
        assert_eq!(land(9.0, -5.5).as_deref(), Some("S_LANDING_ROLL"));
        assert_eq!(land(5.0, -9.0).as_deref(), Some("S_LANDING_HARD"));
        assert_eq!(land(9.0, -9.0).as_deref(), Some("S_LANDING_ROLL")); // roll before hard
        assert_eq!(land(5.0, -8.05), None); // the dead band between -8.0 and -8.1
    }

    /// Landing from a fall: heavy (fall timer past 1.5 s), running (input past 0.95), or light.
    #[test]
    fn fall_landings() {
        let Some(fs) = fs() else { return };
        let m = base(&fs);
        let at = |fall: f32, input: f32| {
            let s = Sense {
                ground_contact: true,
                fall_time: fall,
                input_magnitude: input,
                ..Default::default()
            };
            go(&m, "S_FALL", "ACT_UPDATE", &s)
        };
        assert_eq!(at(0.4, 0.0).as_deref(), Some("S_JUMP_LAND"));
        assert_eq!(at(0.4, 1.0).as_deref(), Some("S_JUMP_RUNNING_LAND"));
        assert_eq!(at(0.4, 0.95), Some("S_JUMP_LAND".to_string())); // strictly above 0.95
        assert_eq!(at(2.0, 1.0).as_deref(), Some("S_JUMP_LAND_HEAVY"));
        assert_eq!(at(1.5, 0.0).as_deref(), Some("S_JUMP_LAND")); // strictly above 1.5 s
    }

    /// The crouch (JC2-051), edge by edge from `basestatemachine.afsm`: `ACT_CROUCH` from the
    /// ground states only while the input magnitude is not above 0.7 (polarity), the descent
    /// waits for its clip, `ACT_NONE` stands up, input above 0.7 runs out of the crouch, and the
    /// crouch states take no navigate, strafe or dash.
    #[test]
    fn crouch_edges() {
        let Some(fs) = fs() else { return };
        let m = base(&fs);
        let input = |i: f32| Sense { input_magnitude: i, ..Default::default() };
        let done = Sense { anim_finished: true, ..Default::default() };
        for from in ["S_UPRIGHT_IDLE", "S_UPRIGHT_BASIC_NAVIGATION", "S_UPRIGHT_ROTATE_CW_INTERUPT"] {
            assert_eq!(go(&m, from, "ACT_CROUCH", &input(0.0)).as_deref(), Some("S_DOWN_TO_CROUCH"), "{from}");
            assert_eq!(go(&m, from, "ACT_CROUCH", &input(0.7)).as_deref(), Some("S_DOWN_TO_CROUCH"), "{from}");
            assert_eq!(go(&m, from, "ACT_CROUCH", &input(0.71)), None, "{from}: strictly above 0.7 blocks");
        }
        assert_eq!(go(&m, "S_JUMP_LAND_INTERRUPT", "ACT_CROUCH", &input(1.0)).as_deref(), Some("S_CROUCH"));
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_CROUCH", &done).as_deref(), Some("S_CROUCH"));
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_CROUCH", &Sense::default()), None); // the roll is not over
        // the descent waits for crouch_DN
        assert_eq!(go(&m, "S_DOWN_TO_CROUCH", "ACT_UPDATE", &Sense::default()), None);
        assert_eq!(go(&m, "S_DOWN_TO_CROUCH", "ACT_UPDATE", &done).as_deref(), Some("S_CROUCH"));
        assert_eq!(go(&m, "S_DOWN_TO_CROUCH", "ACT_NONE", &done), None); // no stand during the descent
        // held: the crouch stays; released (nothing sent): up, and out to idle when it ends
        assert_eq!(go(&m, "S_CROUCH", "ACT_CROUCH", &input(0.0)).as_deref(), Some("S_CROUCH"));
        assert_eq!(go(&m, "S_CROUCH", "ACT_NONE", &input(0.0)).as_deref(), Some("S_UP_FROM_CROUCH"));
        assert_eq!(go(&m, "S_UP_FROM_CROUCH", "ACT_NONE", &Sense::default()), None);
        assert_eq!(go(&m, "S_UP_FROM_CROUCH", "ACT_NONE", &done).as_deref(), Some("S_UPRIGHT_IDLE"));
        assert_eq!(
            go(&m, "S_UP_FROM_CROUCH", "ACT_NAVIGATE", &input(1.0)).as_deref(),
            Some("S_UPRIGHT_BASIC_NAVIGATION")
        );
        // input above 0.7 stands him up into the run, in all three crouch states
        for from in ["S_CROUCH", "S_DOWN_TO_CROUCH"] {
            assert_eq!(go(&m, from, "ACT_UPDATE", &input(0.8)).as_deref(), Some("S_UPRIGHT_BASIC_NAVIGATION"), "{from}");
            assert_eq!(go(&m, from, "ACT_UPDATE", &input(0.7)), None, "{from}: not above 0.7");
        }
        // no walking, aiming or sprinting out of the crouch by those actions
        for a in ["ACT_NAVIGATE", "ACT_STRAFE", "ACT_DASH"] {
            assert_eq!(go(&m, "S_CROUCH", a, &input(0.5)), None, "{a}");
        }
        // the jump and the evade are open from every crouch state
        for from in ["S_CROUCH", "S_DOWN_TO_CROUCH", "S_UP_FROM_CROUCH"] {
            assert_eq!(
                go(&m, from, "ACT_JUMP_AT", &Sense::default()).as_deref(),
                Some("S_JUMP_START_SPRINT_FORWARD"),
                "{from}"
            );
            assert_eq!(go(&m, from, "ACT_EVADE", &Sense::default()).as_deref(), Some("S_EVADE"), "{from}");
        }
        // there is no obstructed-stand or low-ceiling predicate anywhere in the graph
        assert!(!m.conditions.values().any(|c| c.class.contains("Ceiling") || c.class.contains("Clearance")));
        // the crouch walk of the rate table (`00572ca0`) is not a state of this graph
        assert!(!m.states.contains_key("S_CROUCH_WALK"));
    }

    /// The evasive roll: from the ground states (and the landing interrupts), no second roll,
    /// jump or crouch during it, then the interrupt state, which rolls again, goes to idle or
    /// the dash, or crouches once the clip is over.
    #[test]
    fn evade_edges() {
        let Some(fs) = fs() else { return };
        let m = base(&fs);
        let s = Sense::default();
        let done = Sense { anim_finished: true, ..Default::default() };
        for from in [
            "S_UPRIGHT_IDLE",
            "S_UPRIGHT_BASIC_NAVIGATION",
            "S_DASH",
            "S_INSTANT_DASH",
            "S_UPRIGHT_ROTATE_CW_INTERUPT",
            "S_JUMP_LAND_INTERRUPT",
            "S_JUMP_RUNNING_LAND_INTERRUPT",
            "S_CROUCH",
        ] {
            assert_eq!(go(&m, from, "ACT_EVADE", &s).as_deref(), Some("S_EVADE"), "{from}");
        }
        // the landing itself, the air and the water take no evade
        for from in ["S_JUMP_LAND", "S_JUMP_LAND_HEAVY", "S_FALL", "S_SKYDIVE", "S_PARACHUTE", "S_SWIM_IDLE"] {
            assert_eq!(go(&m, from, "ACT_EVADE", &done), None, "{from}");
        }
        // during the roll: the only edge is the update to the interrupt state, at the clip's end
        for a in ["ACT_EVADE", "ACT_JUMP_AT", "ACT_CROUCH", "ACT_NAVIGATE", "ACT_NONE", "ACT_DASH"] {
            assert_eq!(go(&m, "S_EVADE", a, &done), None, "{a}");
        }
        assert_eq!(go(&m, "S_EVADE", "ACT_UPDATE", &s), None);
        assert_eq!(go(&m, "S_EVADE", "ACT_UPDATE", &done).as_deref(), Some("S_EVADE_INTERRUPT"));
        let dash = Sense { anim_finished: true, dash_enabled: true, ..Default::default() };
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_EVADE", &done).as_deref(), Some("S_EVADE"));
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_NONE", &done).as_deref(), Some("S_UPRIGHT_IDLE"));
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_NONE", &dash), None); // dash on: not idle
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_UPDATE", &dash).as_deref(), Some("S_DASH"));
        assert_eq!(go(&m, "S_EVADE_INTERRUPT", "ACT_UPDATE", &done), None);
        // the interrupt state has no navigate, strafe, dash or jump edge
        for a in ["ACT_NAVIGATE", "ACT_STRAFE", "ACT_JUMP_AT", "ACT_DASH"] {
            assert_eq!(go(&m, "S_EVADE_INTERRUPT", a, &done), None, "{a}");
        }
        // the animation set's entries for the roll and the crouch
        let Some(set) = AnimSet::load(&fs, "kc001_base.asb") else { return };
        let row = |st: &str| {
            let a = &set.states[st];
            (a.bans.clone(), a.blend, a.speed)
        };
        assert_eq!(row("S_EVADE"), (vec!["evade_fwd.ban".to_string()], 0.1, 1.0));
        assert_eq!(row("S_EVADE_INTERRUPT"), (vec!["evade_fwd.ban".to_string()], 0.1, 1.0));
        assert_eq!(row("S_DOWN_TO_CROUCH"), (vec!["crouch_dn.ban".to_string()], 0.2, 1.0));
        assert_eq!(row("S_CROUCH"), (vec!["cr_idle.ban".to_string()], 0.2, 1.0));
        assert_eq!(row("S_UP_FROM_CROUCH"), (vec!["crouch_up.ban".to_string()], 0.1, 1.0));
    }

    /// Every state the controller can reach plays the clip the animation set binds to it.
    #[test]
    fn state_clips_follow_the_animation_set() {
        let Some(fs) = fs() else { return };
        let Some(set) = AnimSet::load(&fs, "kc001_base.asb") else { return };
        eprintln!("animation set: {} states", set.states.len());
        for (state, clip) in crate::character::NATIVE_STATE_CLIPS {
            let anim = set.states.get(*state).unwrap_or_else(|| panic!("{state} has no animation"));
            let file = clip.file();
            assert!(
                anim.bans.iter().any(|b| b == file),
                "{state}: {file} is not among {:?}",
                anim.bans
            );
        }
        // the approximations really are not the set's clip for those states
        for (state, clip) in crate::character::APPROXIMATE_STATE_CLIPS {
            let anim = &set.states[*state];
            assert!(!anim.bans.iter().any(|b| b == clip.file()), "{state} is now exact");
        }
    }
}
