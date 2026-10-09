//! Vehicle "dolls" (`.mvdoll`): the part tree and the physics modules of a vehicle,
//! found inside its container (`lave.v014_jeep.ee` → `v014_jeep.mvdoll`).
//!
//! An ordinary property container. Under the root sit a `CFamilyProp` (a tree of
//! `CReferenceProp` name stubs), the modules (`CCarSteeringModule`, `CCarGlobalModule`,
//! one `CWheelModule` per axle with a `CSuspensionModule` and a `CBrakeModule` inside,
//! `CCarEngineModule`, `CCarRaycastModule`, `CCarPhysicalBaseModule`,
//! `CCarTransmissionModule`, AI/damage/buoyancy modules) and a tree of `CPartProp`
//! parts rooted at the body. Every part has `model` (full `.lod` path, `#376e117f` its
//! bare name), `pfx` (collision), `world` (its transform **in the vehicle's frame**: the
//! `local` matrices are identity and nested parts repeat their parent's `world`, so no
//! composition is needed), `mass`, and for tyres `is_wheel = 1`, `#44da2bab` = wheel
//! number and `#42131844` = scale. Right-hand tyres carry a 180° turn about y so the
//! same tyre model fits both sides.
//!
//! Axes are the engine's (x right, y up, z forward); the driver's seat, the steering
//! wheel and wheel 0 all sit at **negative z**, so a vehicle's nose points down −z in
//! its own frame.
//!
//! Key names below marked "by value" are hashes whose name is still unknown; the
//! meaning comes from the values across the land vehicles (sedan 1300 kg / jeep 4000 kg /
//! bus 20000 kg for the mass, 400 / 1000 / 3500 for the torque…).

use crate::placement::Transform3;
use crate::property::{Key, Node, PropertyFile, Variant};

#[derive(Clone, Debug, PartialEq)]
pub struct VehiclePart {
    pub name: String,
    /// Full engine path of the `.lod` (empty for collision-only parts).
    pub model_path: String,
    /// Bare `.lod` name as the container keys it; instanced parts carry an `N_`
    /// prefix (`0_v014-seats01-seat01.lod`) that the container does not.
    pub model: String,
    /// Bare `.pfx` collision name, if any.
    pub collision: String,
    /// Transform in the vehicle's frame.
    pub world: Transform3,
    pub scale: Option<[f32; 3]>,
    pub mass: f32,
    /// Wheel number when this part is a tyre.
    pub wheel: Option<u32>,
    /// Nesting depth below the body.
    pub depth: usize,
}

impl VehiclePart {
    /// The `.lod` name without an instance prefix (`0_x.lod` → `x.lod`).
    pub fn model_file(&self) -> &str {
        strip_instance_prefix(&self.model)
    }
}

pub fn strip_instance_prefix(name: &str) -> &str {
    match name.split_once('_') {
        Some((n, rest)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => name,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suspension {
    /// Direction the wheel travels on compression, in the vehicle frame (≈ −y).
    pub direction: [f32; 3],
    /// Travel in metres.
    pub length: f32,
    pub strength: f32,
    pub compression: f32,
    pub relaxation: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brake {
    pub max_torque: f32,
    /// `#d201f1d5` by value: 1 on the rear axle, 0 on the front.
    pub handbrake: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Axle {
    pub name: String,
    /// `#3dd9d99c` by value: 0 for the front axle, 1 for the rear.
    pub index: u32,
    /// `wheel_nr`: number of the first wheel on this axle.
    pub first_wheel: u32,
    pub radius: f32,
    pub width: f32,
    pub mass: f32,
    pub friction: f32,
    /// `#c06c5451` by value (0.75–0.9).
    pub grip: f32,
    pub slip_angle: f32,
    pub suspension: Option<Suspension>,
    pub brake: Option<Brake>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engine {
    pub name: String,
    pub rpm_min: f32,
    pub rpm_max: f32,
    /// `#c78badab` by value: peak torque in N·m (400 sports car … 3500 bus).
    pub torque: f32,
    /// `#c5156b97` by value: rpm of the torque peak.
    pub torque_rpm: f32,
    /// Per gear, rpm at which the transmission shifts up / down.
    pub upshift_rpm: Vec<f32>,
    pub downshift_rpm: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transmission {
    pub name: String,
    /// `#c9d728c7` by value: forward gear ratios.
    pub gears: Vec<f32>,
    /// `#0739740b` by value: reverse ratio (3–3.25).
    pub reverse: f32,
    /// `#dd70f454` by value: speed cap in km/h.
    pub top_speed_kmh: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Steering {
    pub name: String,
    /// Full lock in degrees.
    pub max_angle: f32,
    /// `#c71a85bb` by value: km/h from which steering is reduced.
    pub speed_kmh: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalBase {
    pub name: String,
    /// `#a2b2218e` by value: chassis mass in kg.
    pub mass: f32,
    /// `#3e5f5109` by value: centre of mass in the vehicle frame.
    pub center_of_mass: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleDoll {
    pub parts: Vec<VehiclePart>,
    pub axles: Vec<Axle>,
    pub engine: Option<Engine>,
    pub transmission: Option<Transmission>,
    pub steering: Option<Steering>,
    pub base: Option<PhysicalBase>,
    /// Air vehicles: the helicopter modules, when the doll has them.
    pub heli: Option<Helicopter>,
    /// Sea vehicles: the boat modules, when the doll has them.
    pub boat: Option<Boat>,
    /// A fixed-wing aircraft: the doll has a `CAirPlaneController`. Planes carry the
    /// helicopter modules too (`heli`: mass, thrust, turn rates).
    pub plane: bool,
    /// The `CHelicopterAerodynamicModule`s (a plane's wings and tailplane) and
    /// `CHelicopterRudderModule`s; helicopters have some of both.
    pub wings: Vec<Airfoil>,
    pub rudders: Vec<Airfoil>,
    /// The places Rico can stand on the outside (`CSmartObject`s carrying a
    /// `CRoofTopStuntController`): a car has one on the hardtop and one on the hood, a
    /// boat one on the deck, a plane one on the fuselage; a helicopter has a
    /// `CBellyStuntController` under the hull instead, and the chopper and the military
    /// helicopter a `CThreeNodeClingController` at the nose as well.
    pub stunts: Vec<StuntPosition>,
    /// The seats: the other smart objects named `SO_*` (`SO_Driver`, `SO_Shotgun`,
    /// `SO_L1`, …) in the order of the file, each name once.
    pub seats: Vec<Seat>,
}

/// A seat: a `CSmartObject` that is not a stunt position. `SO_Driver` carries the
/// vehicle's controller (`CHelicopterController`, …), the others a
/// `CPassengerSeatController`.
#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub name: String,
    /// The smart object's transform in the vehicle frame (engine axes): where the seated
    /// character's hips go.
    pub world: Transform3,
    /// The seat controller's `camera_id` (key `0x742cdd23`, camera setting id): the
    /// camera a seated player sees through (`camera-fov-next.md`), when the seat has a
    /// controller carrying one.
    pub camera_id: Option<i32>,
    /// The controller's `camera_id_reverse` (`0xb4ad6a59`).
    pub camera_id_reverse: Option<i32>,
}

impl Seat {
    pub fn is_driver(&self) -> bool {
        self.name.eq_ignore_ascii_case("SO_Driver")
    }
}

/// Which stunt controller a stunt position carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StuntKind {
    /// `CRoofTopStuntController`: standing on top (cars, boats, planes).
    #[default]
    Roof,
    /// `CBellyStuntController`: hanging under a helicopter's hull.
    Belly,
    /// `CThreeNodeClingController`: clinging to a helicopter's nose.
    Cling,
}

impl StuntKind {
    pub fn of_class(class: &str) -> Option<Self> {
        match class {
            "CRoofTopStuntController" => Some(Self::Roof),
            "CBellyStuntController" => Some(Self::Belly),
            "CThreeNodeClingController" | "CClingStuntController" => Some(Self::Cling),
            _ => None,
        }
    }
}

/// An authored smart-object action. Unknown properties, transforms, event/state
/// strings and child identifiers are retained verbatim in `raw`.
#[derive(Clone, Debug, PartialEq)]
pub struct StuntAction {
    pub key: Key,
    pub raw: Node,
}

impl StuntAction {
    /// Cross-doll evidence: #63f9ea61 matches the destination SO's _object_id.
    /// A missing, zero or dangling identifier never becomes a guessed destination.
    pub fn destination_id(&self) -> Option<i32> {
        match self.raw.value_hash(0x63f9_ea61)? {
            Variant::Int(id) if *id != 0 => Some(*id),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        self.raw.str("name").unwrap_or("")
    }

    /// Only the observed jeep roof/front-cling actions have runtime support.
    pub fn transfer_kind(&self) -> Option<StuntKind> {
        match (self.raw.str("_class")?, self.raw.value_hash(0x3775_7a6d)?) {
            ("CStuntAction", Variant::Str(clip)) if clip == "jump_to_front" => {
                Some(StuntKind::Cling)
            }
            ("CClingToRoofAction", Variant::Str(clip)) if clip == "cling_to_roof" => {
                Some(StuntKind::Roof)
            }
            _ => None,
        }
    }
}

/// A stunt position: a `CSmartObject` that carries a stunt controller (and the
/// `CStuntAction`s that move Rico on and off it). `world` is in the vehicle frame (engine
/// axes: the nose points down −z, so the hood has the smaller z).
#[derive(Clone, Debug, PartialEq)]
pub struct StuntPosition {
    /// Stable authored identity; never a position in the decoded vector.
    pub object_id: Option<i32>,
    /// Entire source object, including controller and unknown fields.
    pub raw: Node,
    pub actions: Vec<StuntAction>,
    /// The smart object's name (`SO_Hardtop_RD`, `SO_Hood_RD`).
    pub name: String,
    /// The controller: on top, under the belly or at the nose.
    pub kind: StuntKind,
    /// The smart object's transform in the vehicle frame: where his feet go.
    pub world: Transform3,
    /// `#cceaa327` of the controller: `CAB` on the jeep's hood, empty on its hardtop.
    pub zone: String,
    /// `GroundToRoofTrigger`: the box (centre, half extents — its floor sits on the
    /// ground, so they are half sizes) inside which the grappling jump up here is offered.
    pub grapple_trigger: Option<StuntTrigger>,
    /// `ParachuteToRoofTrigger`: the box above the vehicle in which the canopy lands on it.
    pub parachute_trigger: Option<StuntTrigger>,
}

impl StuntPosition {
    pub fn position(&self) -> [f32; 3] {
        self.world.translation()
    }
}

/// A `CSmartObjectTrigger` volume in the vehicle frame (`#2be10cce` = the extents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StuntTrigger {
    pub center: [f32; 3],
    pub half_extents: [f32; 3],
}

/// A lifting surface of an air vehicle: a `CHelicopterAerodynamicModule` (the wings and
/// the tailplane) or a `CHelicopterRudderModule`. Names by hashing where marked.
#[derive(Clone, Debug, PartialEq)]
pub struct Airfoil {
    /// `#57582d19`: where it acts (engine axes; the wings at x = ±5…10 m).
    pub position: [f32; 3],
    /// `normal`: the way it lifts.
    pub normal: [f32; 3],
    /// `#cd9322f1` on rudders: the axis it turns the plane about.
    pub axis: Option<[f32; 3]>,
    /// `#8d840498`: 1 (speed plane) … 35 (cargo plane) — the area, m² (assumed).
    pub area: f32,
    /// `#abd2b2ba`: 1.25–2.5 — the lift coefficient (assumed).
    pub lift: f32,
    /// `#ba5d47fc`: 0–1 — the drag coefficient (assumed).
    pub drag: f32,
    /// `#f1637111`: 0.47/0.53 on a pair of wings, 0 on the tailplane, 1–10 on rudders.
    pub share: f32,
    /// `#6f8dc0f5`: 1.3 everywhere — the air's density, kg/m³.
    pub density: f32,
}

/// The boat modules of a `seve.*` doll (`CBoatGeneralModule`, `CBoatEngineModule`,
/// `CBoatSteeringModule`, `CBoatRudderModule`, the `CBoyancyModule` points).
#[derive(Clone, Debug, PartialEq)]
pub struct Boat {
    /// `#4365a91d` (the same key as the helicopter's): 2500 … 10000 kg.
    pub mass: f32,
    pub center_of_mass: [f32; 3],
    /// `boyancy`: 1.9–2.4, how many times its weight the hull floats.
    pub buoyancy: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// `#7b40a502`: 29000–50000 — the forward thrust, N (assumed).
    pub thrust: f32,
    /// `#ed1dff74`: 25000–40000 — the reverse thrust, N (assumed).
    pub reverse_thrust: f32,
    /// `#6fd2bd7f`: 15–35 — the rudder's lock, degrees.
    pub max_steer_deg: f32,
    /// `#11d7375d` of the rudder module: where the propeller pushes (engine axes).
    pub rudder: [f32; 3],
    /// The `CBoyancyModule` points: `position` and `radius` (0 for most).
    pub floats: Vec<([f32; 3], f32)>,
}

impl Default for Boat {
    fn default() -> Self {
        Self {
            mass: 2500.0,
            center_of_mass: [0.0, 0.0, 0.0],
            buoyancy: 2.2,
            linear_damping: 0.0,
            angular_damping: 1.0,
            thrust: 40000.0,
            reverse_thrust: 25000.0,
            max_steer_deg: 30.0,
            rudder: [0.0, -0.5, 4.0],
            floats: Vec::new(),
        }
    }
}

/// The helicopter modules of an `arve.*` doll (`CHelicopterPhysicalBaseModule`,
/// `CHelicopterEngineModule`, `CHelicopterSteeringModule`). Names found by hashing
/// candidates are marked; the rest are read by hash with their meaning inferred from
/// how they scale across the five helicopters (see `research/formats/vehicle.md`).
#[derive(Clone, Debug, PartialEq)]
pub struct Helicopter {
    /// `#4365a91d`: 1000 (v059) … 10000 (v062) — the mass, kg.
    pub mass: f32,
    /// `center_of_mass`.
    pub center_of_mass: [f32; 3],
    /// `#b92afbce`: 150–155 — taken as the top speed, km/h.
    pub top_speed_kmh: f32,
    /// `linear_damping`, `angular_damping`.
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// `inertia_pitch`, `inertia_roll`, `inertia_yaw`.
    pub inertia: [f32; 3],
    /// `#ec5a5e47`: 16–20 × the mass — the full climbing lift, N.
    pub lift_max: f32,
    /// `#2936b4b4`: 2.5–4.8 × the mass — the push down when descending, N.
    pub lift_down: f32,
    /// `angular_pitch_speed`, `angular_roll_speed`, `angular_yaw_speed` (rad/s).
    pub pitch_speed: f32,
    pub roll_speed: f32,
    pub yaw_speed: f32,
    /// `max_steer_angle` (rad): how far the body tilts.
    pub max_tilt: f32,
    /// `rotor_max_rpm`.
    pub rotor_max_rpm: f32,
}

impl Default for Helicopter {
    fn default() -> Self {
        Self {
            mass: 4000.0,
            center_of_mass: [0.0, -0.5, 0.0],
            top_speed_kmh: 155.0,
            linear_damping: 0.15,
            angular_damping: 0.75,
            inertia: [4.0, 2.0, 6.0],
            lift_max: 70_000.0,
            lift_down: 10_000.0,
            pitch_speed: 0.23,
            roll_speed: 0.2,
            yaw_speed: 1.25,
            max_tilt: 0.3,
            rotor_max_rpm: 160.0,
        }
    }
}

const HELI_MASS: u32 = 0x4365_a91d;
const HELI_TOP_SPEED: u32 = 0xb92a_fbce;
const HELI_LIFT_MAX: u32 = 0xec5a_5e47;
const HELI_LIFT_DOWN: u32 = 0x2936_b4b4;
const AIR_POSITION: u32 = 0x5758_2d19;
const AIR_AXIS: u32 = 0xcd93_22f1;
const AIR_AREA: u32 = 0x8d84_0498;
const AIR_LIFT: u32 = 0xabd2_b2ba;
const AIR_DRAG: u32 = 0xba5d_47fc;
const AIR_SHARE: u32 = 0xf163_7111;
const AIR_DENSITY: u32 = 0x6f8d_c0f5;
const BOAT_THRUST: u32 = 0x7b40_a502;
const BOAT_REVERSE: u32 = 0xed1d_ff74;
const BOAT_STEER: u32 = 0x6fd2_bd7f;
const BOAT_RUDDER: u32 = 0x11d7_375d;

fn f(n: &Node, key: u32) -> Option<f32> {
    match n.value_hash(key)? {
        Variant::Float(v) => Some(*v),
        Variant::Int(i) => Some(*i as f32),
        _ => None,
    }
}

fn i(n: &Node, key: u32) -> Option<i32> {
    match n.value_hash(key)? {
        Variant::Int(v) => Some(*v),
        Variant::Float(v) => Some(*v as i32),
        _ => None,
    }
}

fn s(n: &Node, key: u32) -> &str {
    match n.value_hash(key) {
        Some(Variant::Str(v)) => v,
        _ => "",
    }
}

fn v3(n: &Node, key: u32) -> Option<[f32; 3]> {
    match n.value_hash(key)? {
        Variant::Vec3(v) => Some(*v),
        _ => None,
    }
}

/// "3, 1.85, 1.4, 1,  0.85" → numbers.
fn list(text: &str) -> Vec<f32> {
    text.split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect()
}

// Hashes of the property names that are known (see data/property_names.txt) …
const NAME: u32 = 0x3142b133; // module / part name, by value
const MODEL: u32 = 0x0f94740b; // "model"
const MODEL_BARE: u32 = 0x376e117f; // by value
const PFX_BARE: u32 = 0x6511a361; // by value
const SCALE: u32 = 0x42131844; // by value
const IS_WHEEL: u32 = 0xd0f0b6e1; // "is_wheel"
const WHEEL_NUMBER: u32 = 0x44da2bab; // by value
const AXLE_INDEX: u32 = 0x3dd9d99c; // by value
const WHEEL_NR: u32 = 0x425574b7; // "wheel_nr"
const WIDTH: u32 = 0x11b73cc0; // "width"
const FRICTION: u32 = 0xd178147a; // "friction"
const GRIP: u32 = 0xc06c5451; // by value
const SLIP_ANGLE: u32 = 0xa005c0e6; // "slip_angle"
const DIRECTION: u32 = 0x08946d97; // "direction"
const LENGTH: u32 = 0xdd7d25aa; // "length"
const STRENGTH: u32 = 0xd84a0e4d; // "strength"
const COMPRESSION: u32 = 0x87dde03e; // "compression"
const RELAXATION: u32 = 0x3b4617cc; // "relaxation"
const MAX_BRAKE_TORQUE: u32 = 0x5235c48e; // "max_brake_torque"
const HANDBRAKE: u32 = 0xd201f1d5; // by value
const RPM_MIN: u32 = 0xd4ee0178; // "rpm_min"
const RPM_MAX: u32 = 0x48290e54; // "rpm_max"
const TORQUE: u32 = 0xc78badab; // by value
const TORQUE_RPM: u32 = 0xc5156b97; // by value
const UPSHIFT_RPM: u32 = 0xd18407e9; // "upshift_rpm"
const DOWNSHIFT_RPM: u32 = 0xc5f45610; // "downshift_rpm"
const GEARS: u32 = 0xc9d728c7; // by value
const REVERSE: u32 = 0x0739740b; // by value
const TOP_SPEED: u32 = 0xdd70f454; // by value
const MAX_ANGLE: u32 = 0xa80ef9c6; // "max_angle"
const STEER_SPEED: u32 = 0xc71a85bb; // by value
const CHASSIS_MASS: u32 = 0xa2b2218e; // by value
const CENTER_OF_MASS: u32 = 0x3e5f5109; // by value
const TRIGGER_EXTENTS: u32 = 0x2be1_0cce; // by value: a smart-object trigger's box
const SEAT_CAMERA_ID: u32 = 0x742c_dd23;
const SEAT_CAMERA_REVERSE: u32 = 0xb4ad_6a59;
const STUNT_ZONE: u32 = 0xcceaa327; // by value: a stunt controller's area tag ("CAB")

fn collect(node: &Node, depth: usize, doll: &mut VehicleDoll) {
    let class = node.str("_class").unwrap_or("");
    match class {
        "CPartProp" => {
            let world = match node.value("world") {
                Some(Variant::Mat4x3(m)) => Transform3::from_mat4x3(*m),
                _ => Transform3::IDENTITY,
            };
            let is_wheel = i(node, IS_WHEEL).unwrap_or(0) != 0;
            doll.parts.push(VehiclePart {
                name: s(node, NAME).to_string(),
                model_path: s(node, MODEL).to_string(),
                model: s(node, MODEL_BARE).to_string(),
                collision: s(node, PFX_BARE).to_string(),
                world,
                scale: v3(node, SCALE),
                mass: node.float("mass").unwrap_or(0.0),
                wheel: is_wheel.then(|| i(node, WHEEL_NUMBER).unwrap_or(0) as u32),
                depth,
            });
            for (_, c) in &node.children {
                collect(c, depth + 1, doll);
            }
            return;
        }
        "CWheelModule" => {
            let mut axle = Axle {
                name: s(node, NAME).to_string(),
                index: i(node, AXLE_INDEX).unwrap_or(0) as u32,
                first_wheel: i(node, WHEEL_NR).unwrap_or(0) as u32,
                radius: node.float("radius").unwrap_or(0.35),
                width: f(node, WIDTH).unwrap_or(0.25),
                mass: node.float("mass").unwrap_or(30.0),
                friction: f(node, FRICTION).unwrap_or(1.0),
                grip: f(node, GRIP).unwrap_or(0.8),
                slip_angle: f(node, SLIP_ANGLE).unwrap_or(0.1),
                suspension: None,
                brake: None,
            };
            for (_, c) in &node.children {
                match c.str("_class").unwrap_or("") {
                    "CSuspensionModule" => {
                        axle.suspension = Some(Suspension {
                            direction: v3(c, DIRECTION).unwrap_or([0.0, -1.0, 0.0]),
                            length: f(c, LENGTH).unwrap_or(0.2),
                            strength: f(c, STRENGTH).unwrap_or(30.0),
                            compression: f(c, COMPRESSION).unwrap_or(2.0),
                            relaxation: f(c, RELAXATION).unwrap_or(2.5),
                        });
                    }
                    "CBrakeModule" => {
                        axle.brake = Some(Brake {
                            max_torque: f(c, MAX_BRAKE_TORQUE).unwrap_or(2000.0),
                            handbrake: i(c, HANDBRAKE).unwrap_or(0) != 0,
                        });
                    }
                    _ => {}
                }
            }
            doll.axles.push(axle);
            return;
        }
        "CCarEngineModule" => {
            doll.engine = Some(Engine {
                name: s(node, NAME).to_string(),
                rpm_min: f(node, RPM_MIN).unwrap_or(800.0),
                rpm_max: f(node, RPM_MAX).unwrap_or(6000.0),
                torque: f(node, TORQUE).unwrap_or(300.0),
                torque_rpm: f(node, TORQUE_RPM).unwrap_or(3000.0),
                upshift_rpm: list(s(node, UPSHIFT_RPM)),
                downshift_rpm: list(s(node, DOWNSHIFT_RPM)),
            });
        }
        "CCarTransmissionModule" => {
            doll.transmission = Some(Transmission {
                name: s(node, NAME).to_string(),
                gears: list(s(node, GEARS)),
                reverse: f(node, REVERSE).unwrap_or(3.0),
                top_speed_kmh: f(node, TOP_SPEED).unwrap_or(120.0),
            });
        }
        "CCarSteeringModule" => {
            doll.steering = Some(Steering {
                name: s(node, NAME).to_string(),
                max_angle: f(node, MAX_ANGLE).unwrap_or(35.0),
                speed_kmh: f(node, STEER_SPEED).unwrap_or(50.0),
            });
        }
        "CBoatGeneralModule" => {
            let b = doll.boat.get_or_insert_with(Boat::default);
            b.mass = f(node, HELI_MASS).unwrap_or(b.mass);
            if let Some(Variant::Vec3(c)) = node.value("center_of_mass") {
                b.center_of_mass = *c;
            }
            b.buoyancy = node.float("boyancy").unwrap_or(b.buoyancy);
            b.linear_damping = node.float("linear_damping").unwrap_or(b.linear_damping);
            b.angular_damping = node.float("angular_damping").unwrap_or(b.angular_damping);
        }
        "CBoatEngineModule" => {
            let b = doll.boat.get_or_insert_with(Boat::default);
            b.thrust = f(node, BOAT_THRUST).unwrap_or(b.thrust);
            b.reverse_thrust = f(node, BOAT_REVERSE).unwrap_or(b.reverse_thrust);
        }
        "CBoatSteeringModule" => {
            let b = doll.boat.get_or_insert_with(Boat::default);
            b.max_steer_deg = f(node, BOAT_STEER).unwrap_or(b.max_steer_deg);
        }
        "CBoatRudderModule" => {
            let b = doll.boat.get_or_insert_with(Boat::default);
            b.rudder = v3(node, BOAT_RUDDER).unwrap_or(b.rudder);
        }
        "CBoyancyModule" => {
            if let Some(Variant::Vec3(p)) = node.value("position") {
                let b = doll.boat.get_or_insert_with(Boat::default);
                b.floats.push((*p, node.float("radius").unwrap_or(0.0)));
            }
        }
        "CAirPlaneController" => {
            doll.plane = true;
        }
        "CHelicopterAerodynamicModule" | "CHelicopterRudderModule" => {
            let foil = Airfoil {
                position: v3(node, AIR_POSITION).unwrap_or([0.0; 3]),
                normal: match node.value("normal") {
                    Some(Variant::Vec3(n)) => *n,
                    _ => [0.0, 1.0, 0.0],
                },
                axis: v3(node, AIR_AXIS),
                area: f(node, AIR_AREA).unwrap_or(1.0),
                lift: f(node, AIR_LIFT).unwrap_or(1.0),
                drag: f(node, AIR_DRAG).unwrap_or(0.0),
                share: f(node, AIR_SHARE).unwrap_or(0.0),
                density: f(node, AIR_DENSITY).unwrap_or(1.3),
            };
            if class == "CHelicopterRudderModule" {
                if !doll.rudders.contains(&foil) {
                    doll.rudders.push(foil);
                }
            } else if !doll.wings.contains(&foil) {
                doll.wings.push(foil);
            }
        }
        "CHelicopterPhysicalBaseModule" => {
            let h = doll.heli.get_or_insert_with(Helicopter::default);
            h.mass = f(node, HELI_MASS).unwrap_or(h.mass);
            if let Some(Variant::Vec3(c)) = node.value("center_of_mass") {
                h.center_of_mass = *c;
            }
            h.top_speed_kmh = f(node, HELI_TOP_SPEED).unwrap_or(h.top_speed_kmh);
            h.linear_damping = node.float("linear_damping").unwrap_or(h.linear_damping);
            h.angular_damping = node.float("angular_damping").unwrap_or(h.angular_damping);
            h.inertia = [
                node.float("inertia_pitch").unwrap_or(h.inertia[0]),
                node.float("inertia_roll").unwrap_or(h.inertia[1]),
                node.float("inertia_yaw").unwrap_or(h.inertia[2]),
            ];
            h.rotor_max_rpm = node.float("rotor_max_rpm").unwrap_or(h.rotor_max_rpm);
            doll.base = Some(PhysicalBase {
                name: s(node, NAME).to_string(),
                mass: h.mass,
                center_of_mass: h.center_of_mass,
            });
        }
        "CHelicopterEngineModule" => {
            let h = doll.heli.get_or_insert_with(Helicopter::default);
            // the first engine module counts (v059 and v061 carry two)
            if h.lift_max == Helicopter::default().lift_max {
                h.lift_max = f(node, HELI_LIFT_MAX).unwrap_or(h.lift_max);
                h.lift_down = f(node, HELI_LIFT_DOWN).unwrap_or(h.lift_down);
            }
        }
        "CHelicopterSteeringModule" => {
            let h = doll.heli.get_or_insert_with(Helicopter::default);
            h.pitch_speed = node.float("angular_pitch_speed").unwrap_or(h.pitch_speed);
            h.roll_speed = node.float("angular_roll_speed").unwrap_or(h.roll_speed);
            h.yaw_speed = node.float("angular_yaw_speed").unwrap_or(h.yaw_speed);
            h.max_tilt = node.float("max_steer_angle").unwrap_or(h.max_tilt);
        }
        "CCarPhysicalBaseModule" => {
            doll.base = Some(PhysicalBase {
                name: s(node, NAME).to_string(),
                mass: f(node, CHASSIS_MASS).unwrap_or(1500.0),
                center_of_mass: v3(node, CENTER_OF_MASS).unwrap_or([0.0, 0.5, 0.0]),
            });
        }
        "CSmartObject" => {
            if let Some(stunt) = stunt_position(node) {
                doll.stunts.push(stunt);
            } else if let Some(name) = node.str("name").filter(|n| n.starts_with("SO_"))
                && !doll.seats.iter().any(|s| s.name == name)
            {
                doll.seats.push(Seat {
                    name: name.to_string(),
                    world: mat(node, "world").unwrap_or(Transform3::IDENTITY),
                    camera_id: seat_camera(node, SEAT_CAMERA_ID),
                    camera_id_reverse: seat_camera(node, SEAT_CAMERA_REVERSE),
                });
            }
        }
        _ => {}
    }
    for (_, c) in &node.children {
        collect(c, depth, doll);
    }
}

/// A key of the seat's controller child (`CDriverSeatController`, `CPassengerSeatController`,
/// the helicopter and plane controllers): the first direct child whose `_class` ends in
/// `Controller` and carries the key.
fn seat_camera(seat: &Node, key: u32) -> Option<i32> {
    seat.children
        .iter()
        .filter(|(_, c)| c.str("_class").is_some_and(|c| c.ends_with("Controller")))
        .find_map(|(_, c)| i(c, key))
}

fn mat(n: &Node, name: &str) -> Option<Transform3> {
    match n.value(name)? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        _ => None,
    }
}

/// A smart object with a stunt controller among its children (`CRoofTopStuntController`,
/// `CBellyStuntController`, `CThreeNodeClingController`) is a stunt position.
fn stunt_position(so: &Node) -> Option<StuntPosition> {
    let (controller, kind) = so
        .children
        .iter()
        .map(|(_, c)| c)
        .find_map(|c| Some((c, StuntKind::of_class(c.str("_class")?)?)))?;
    let trigger = |name: &str| {
        so.children
            .iter()
            .map(|(_, c)| c)
            .find(|c| c.str("_class") == Some("CSmartObjectTrigger") && c.str("name") == Some(name))
            .and_then(|c| {
                Some(StuntTrigger {
                    center: mat(c, "world")?.translation(),
                    half_extents: v3(c, TRIGGER_EXTENTS)?,
                })
            })
    };
    let mut actions = Vec::new();
    fn gather(n: &Node, actions: &mut Vec<StuntAction>) {
        for (key, child) in &n.children {
            let class = child.str("_class").unwrap_or("");
            if class.ends_with("Action") {
                actions.push(StuntAction {
                    key: key.clone(),
                    raw: child.clone(),
                });
            }
            if class != "CSmartObject" {
                gather(child, actions);
            }
        }
    }
    gather(so, &mut actions);
    Some(StuntPosition {
        object_id: so.int("_object_id"),
        raw: so.clone(),
        actions,
        name: so.str("name").unwrap_or("").to_string(),
        kind,
        world: mat(so, "world").unwrap_or(Transform3::IDENTITY),
        zone: s(controller, STUNT_ZONE).to_string(),
        grapple_trigger: trigger("GroundToRoofTrigger"),
        parachute_trigger: trigger("ParachuteToRoofTrigger"),
    })
}

impl VehicleDoll {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        Ok(Self::from_property(&file))
    }

    pub fn from_property(file: &PropertyFile) -> Self {
        let mut doll = VehicleDoll::default();
        for root in &file.nodes {
            collect(root, 0, &mut doll);
        }
        doll.axles.sort_by_key(|a| a.first_wheel);
        doll
    }

    /// Tyre parts in wheel-number order.
    pub fn wheels(&self) -> Vec<&VehiclePart> {
        let mut w: Vec<&VehiclePart> = self.parts.iter().filter(|p| p.wheel.is_some()).collect();
        w.sort_by_key(|p| p.wheel);
        w
    }

    /// The axle a wheel number belongs to (the one with the largest `first_wheel` ≤ n).
    pub fn axle_for_wheel(&self, wheel: u32) -> Option<&Axle> {
        self.axles
            .iter()
            .filter(|a| a.first_wheel <= wheel)
            .max_by_key(|a| a.first_wheel)
    }

    /// Total mass: chassis plus parts.
    pub fn mass(&self) -> f32 {
        self.base.as_ref().map(|b| b.mass).unwrap_or(1500.0)
    }

    /// The camera setting id of the driver's seat controller (`camera_id`).
    pub fn driver_camera_id(&self) -> Option<i32> {
        self.driver_seat().and_then(|s| s.camera_id)
    }

    /// The driver's seat (`SO_Driver`), when the doll has one.
    pub fn driver_seat(&self) -> Option<&Seat> {
        self.seats.iter().find(|s| s.is_driver())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Key;

    #[test]
    fn instance_prefix() {
        assert_eq!(
            strip_instance_prefix("0_v014-seats01-seat01.lod"),
            "v014-seats01-seat01.lod"
        );
        assert_eq!(
            strip_instance_prefix("v014-seats01-seat01.lod"),
            "v014-seats01-seat01.lod"
        );
        assert_eq!(strip_instance_prefix("12_x.lod"), "x.lod");
        assert_eq!(strip_instance_prefix("v046-body_m.lod"), "v046-body_m.lod");
    }

    #[test]
    fn parses_modules_and_parts() {
        let mut body = Node::default();
        body.values
            .push((Key::Name("_class".into()), Variant::Str("CPartProp".into())));
        body.values.push((
            Key::Hash(MODEL_BARE),
            Variant::Str("v014-body_m.lod".into()),
        ));
        let mut tyre = Node::default();
        tyre.values
            .push((Key::Name("_class".into()), Variant::Str("CPartProp".into())));
        tyre.values.push((
            Key::Hash(MODEL_BARE),
            Variant::Str("v014-tyres01-truck1.lod".into()),
        ));
        tyre.values.push((Key::Hash(IS_WHEEL), Variant::Int(1)));
        tyre.values.push((Key::Hash(WHEEL_NUMBER), Variant::Int(2)));
        tyre.values
            .push((Key::Hash(SCALE), Variant::Vec3([0.98, 0.98, 0.98])));
        tyre.values.push((
            Key::Name("world".into()),
            Variant::Mat4x3([
                1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -0.99, 0.62, 1.74,
            ]),
        ));
        body.children.push((Key::Hash(1), tyre));
        let mut axle = Node::default();
        axle.values.push((
            Key::Name("_class".into()),
            Variant::Str("CWheelModule".into()),
        ));
        axle.values.push((Key::Hash(WHEEL_NR), Variant::Int(2)));
        axle.values.push((Key::Hash(AXLE_INDEX), Variant::Int(1)));
        axle.values
            .push((Key::Name("radius".into()), Variant::Float(0.46)));
        let mut susp = Node::default();
        susp.values.push((
            Key::Name("_class".into()),
            Variant::Str("CSuspensionModule".into()),
        ));
        susp.values.push((Key::Hash(LENGTH), Variant::Float(0.35)));
        axle.children.push((Key::Hash(2), susp));
        let mut trm = Node::default();
        trm.values.push((
            Key::Name("_class".into()),
            Variant::Str("CCarTransmissionModule".into()),
        ));
        trm.values.push((
            Key::Hash(GEARS),
            Variant::Str("3.25, 2.0, 1.4, 1,0.8".into()),
        ));
        let mut root = Node::default();
        root.children.push((Key::Hash(3), body));
        root.children.push((Key::Hash(4), axle));
        root.children.push((Key::Hash(5), trm));
        let doll = VehicleDoll::from_property(&PropertyFile { nodes: vec![root] });
        assert_eq!(doll.parts.len(), 2);
        let w = doll.wheels();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].wheel, Some(2));
        assert_eq!(w[0].world.translation(), [-0.99, 0.62, 1.74]);
        assert_eq!(w[0].depth, 1);
        assert_eq!(doll.axle_for_wheel(3).unwrap().radius, 0.46);
        assert_eq!(
            doll.axle_for_wheel(3)
                .unwrap()
                .suspension
                .as_ref()
                .unwrap()
                .length,
            0.35
        );
        assert_eq!(
            doll.transmission.unwrap().gears,
            vec![3.25, 2.0, 1.4, 1.0, 0.8]
        );
    }

    /// Needs JC2_TEST_DIR with `arve.v062_chopper.ee` in the dropzone.
    #[test]
    fn chopper_modules() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("arve.v062_chopper.ee") else {
            return;
        };
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.ends_with(".mvdoll"))
            .unwrap();
        let doll = VehicleDoll::parse(sarc.data(&data, e).unwrap()).unwrap();
        let h = doll.heli.expect("helicopter modules");
        assert_eq!(h.mass, 10000.0);
        assert_eq!(h.lift_max, 200000.0);
        assert_eq!(h.lift_down, 25000.0);
        assert_eq!(h.yaw_speed, 1.25);
        assert!((h.max_tilt - 0.3).abs() < 1e-6);
        assert_eq!(h.center_of_mass, [0.0, -0.5, 0.0]);
        assert!(doll.parts.iter().any(|p| p.name == "RotorStill4"));
    }

    /// Needs JC2_TEST_DIR with `arve.v071_jet_fighter.ee` and `arve.v067_speedplane.ee`.
    #[test]
    fn plane_modules() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let doll = |name: &str| {
            let data = fs.read_inflated(name).ok()?;
            let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
            let e = sarc.entries.iter().find(|e| e.name.ends_with("vdoll"))?;
            Some(VehicleDoll::parse(sarc.data(&data, e).unwrap()).unwrap())
        };
        let Some(jet) = doll("arve.v071_jet_fighter.ee") else {
            return;
        };
        assert!(jet.plane);
        assert_eq!(jet.wings.len(), 3);
        let left = &jet.wings[0];
        assert_eq!(left.position, [-7.0, 0.0, 0.0]);
        assert_eq!((left.area, left.lift, left.density), (1.6, 1.5, 1.3));
        assert_eq!(jet.rudders.len(), 4);
        let h = jet.heli.as_ref().unwrap();
        assert_eq!((h.mass, h.lift_max), (5000.0, 52500.0));
        assert_eq!(jet.wheels().len(), 5);
        // the chopper is no plane
        if let Some(chopper) = doll("arve.v062_chopper.ee") {
            assert!(!chopper.plane);
            assert!(!chopper.rudders.is_empty());
        }
        if let Some(sp) = doll("arve.v067_speedplane.ee") {
            assert!(sp.plane);
            assert_eq!(sp.wings.len(), 3);
            assert_eq!(sp.heli.as_ref().unwrap().mass, 1500.0);
        }
    }

    /// A smart object with a `CRoofTopStuntController` is a stunt position; one without
    /// is not.
    #[test]
    fn stunt_positions_from_smart_objects() {
        let class = |n: &mut Node, c: &str| {
            n.values
                .push((Key::Name("_class".into()), Variant::Str(c.into())));
        };
        let at = |x: f32, y: f32, z: f32| {
            Variant::Mat4x3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, y, z])
        };
        let mut so = Node::default();
        class(&mut so, "CSmartObject");
        so.values
            .push((Key::Name("name".into()), Variant::Str("SO_Roof".into())));
        so.values
            .push((Key::Name("world".into()), at(0.0, 1.8, 0.3)));
        let mut ctl = Node::default();
        class(&mut ctl, "CRoofTopStuntController");
        ctl.values
            .push((Key::Hash(STUNT_ZONE), Variant::Str("CAB".into())));
        so.children.push((Key::Hash(1), ctl));
        let mut trig = Node::default();
        class(&mut trig, "CSmartObjectTrigger");
        trig.values.push((
            Key::Name("name".into()),
            Variant::Str("GroundToRoofTrigger".into()),
        ));
        trig.values
            .push((Key::Name("world".into()), at(0.0, 2.6, 0.1)));
        trig.values
            .push((Key::Hash(TRIGGER_EXTENTS), Variant::Vec3([9.0, 2.5, 12.0])));
        so.children.push((Key::Hash(2), trig));
        let mut seat = Node::default();
        class(&mut seat, "CSmartObject");
        let mut root = Node::default();
        root.children.push((Key::Hash(3), so));
        root.children.push((Key::Hash(4), seat));
        let doll = VehicleDoll::from_property(&PropertyFile { nodes: vec![root] });
        assert_eq!(doll.stunts.len(), 1);
        let st = &doll.stunts[0];
        assert_eq!(st.name, "SO_Roof");
        assert_eq!(st.kind, StuntKind::Roof);
        assert_eq!(st.position(), [0.0, 1.8, 0.3]);
        // the nameless smart object is no seat either
        assert!(doll.seats.is_empty());
        assert_eq!(st.zone, "CAB");
        let g = st.grapple_trigger.unwrap();
        assert_eq!(g.center, [0.0, 2.6, 0.1]);
        assert_eq!(g.half_extents, [9.0, 2.5, 12.0]);
        assert!(st.parachute_trigger.is_none());
    }

    /// The real jeep and bus: a stunt position on the hardtop and one on the hood.
    /// Needs JC2_TEST_DIR with `lave.v014_jeep.ee` reachable.
    /// The driver's seat controller names the camera setting (`camera_id` /
    /// `camera_id_reverse`, JC2-141 census): the jeep is 62 / 74.
    #[test]
    fn driver_camera_ids() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let ids = |name: &str| {
            let data = fs.read_inflated(name).ok()?;
            let sarc = crate::sarc::SmallArchive::parse(&data).ok()?;
            let e = sarc.entries.iter().find(|e| e.name.ends_with(".mvdoll"))?;
            let doll = VehicleDoll::parse(sarc.data(&data, e).ok()?).ok()?;
            let s = doll.driver_seat()?;
            Some((s.camera_id, s.camera_id_reverse))
        };
        if let Some(j) = ids("lave.v014_jeep.ee") {
            assert_eq!(j, (Some(62), Some(74)));
        }
    }

    #[test]
    fn jeep_stunt_positions() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("lave.v014_jeep.ee") else {
            return;
        };
        let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.ends_with(".mvdoll"))
            .unwrap();
        let doll = VehicleDoll::parse(sarc.data(&data, e).unwrap()).unwrap();
        assert_eq!(doll.stunts.len(), 3, "{:?}", doll.stunts);
        let roof = doll
            .stunts
            .iter()
            .find(|s| s.name == "SO_Hardtop_RD")
            .expect("hardtop");
        let p = roof.position();
        // above the seats, a little behind the middle (the nose is −z)
        assert!((p[0] - 0.005).abs() < 0.01, "{p:?}");
        assert!((p[1] - 1.864).abs() < 0.01, "{p:?}");
        assert!((p[2] - 0.284).abs() < 0.01, "{p:?}");
        let g = roof.grapple_trigger.expect("grapple trigger");
        assert!((g.half_extents[1] - 2.58).abs() < 0.01, "{g:?}");
        // the box stands on the ground: centre height = half height
        assert!((g.center[1] - g.half_extents[1]).abs() < 0.1, "{g:?}");
        assert!(roof.parachute_trigger.is_some());
        let hood = doll
            .stunts
            .iter()
            .find(|s| s.name == "SO_Hood_RD")
            .expect("hood");
        assert!(hood.position()[2] < -1.0, "hood is at the nose (−z)");
        assert_eq!(hood.zone, "CAB");
        // every land vehicle that has a doll with a roof controller reads one
        for name in ["lave.v046_buscoach.ee", "lave.v002_sport.ee"] {
            let Ok(data) = fs.read_inflated(name) else {
                continue;
            };
            let sarc = crate::sarc::SmallArchive::parse(&data).unwrap();
            let Some(e) = sarc.entries.iter().find(|e| e.name.ends_with(".mvdoll")) else {
                continue;
            };
            let doll = VehicleDoll::parse(sarc.data(&data, e).unwrap()).unwrap();
            eprintln!("{name}: {:?}", doll.stunts);
        }
        // the jeep's driver sits on the left (−x), ahead of the middle
        let d = doll.driver_seat().expect("SO_Driver").world.translation();
        assert!(d[0] < 0.0 && d[2] < 0.5, "{d:?}");
    }

    /// Helicopters hang Rico under the belly (and the chopper at the nose), boats and
    /// planes stand him on top; each has a driver's seat. Needs JC2_TEST_DIR.
    #[test]
    fn air_and_sea_stunt_positions() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let doll = |name: &str| -> Option<VehicleDoll> {
            let data = fs.read_inflated(name).ok()?;
            let sarc = crate::sarc::SmallArchive::parse(&data).ok()?;
            let e = sarc
                .entries
                .iter()
                .find(|e| e.name.to_ascii_lowercase().ends_with("doll"))?;
            VehicleDoll::parse(sarc.data(&data, e).ok()?).ok()
        };
        let kinds = |d: &VehicleDoll| d.stunts.iter().map(|s| s.kind).collect::<Vec<_>>();
        if let Some(d) = doll("arve.v062_chopper.ee") {
            let belly = d.stunts.iter().find(|s| s.name == "SO_Body_RD").unwrap();
            assert_eq!(belly.kind, StuntKind::Belly);
            let p = belly.position();
            assert!(
                (p[1] + 1.40).abs() < 0.01 && (p[2] + 2.694).abs() < 0.01,
                "{p:?}"
            );
            assert!(kinds(&d).contains(&StuntKind::Cling), "{:?}", d.stunts);
            assert!(d.driver_seat().is_some(), "{:?}", d.seats);
        }
        if let Some(d) = doll("arve.v060_attackheli.ee") {
            assert_eq!(kinds(&d), vec![StuntKind::Belly]);
        }
        for name in [
            "seve.v083_motorboat_small.ee",
            "seve.v011_charter.ee",
            "arve.v071_jet_fighter.ee",
        ] {
            if let Some(d) = doll(name) {
                assert_eq!(kinds(&d), vec![StuntKind::Roof], "{name}");
                assert!(d.driver_seat().is_some(), "{name}: {:?}", d.seats);
            }
        }
    }
}
