//! Engine ↔ Bevy coordinates: the one place that says how the game's axes map onto
//! Bevy's (`research/formats/coordinates.md`).
//!
//! **The engine frame is right-handed, like Bevy's, and the conversion is the identity.**
//! x is east and the model's right, y is up, z is south and the model's back: a vehicle's
//! nose, a character's face, look along −z, which is Bevy's own forward
//! (`Transform::forward`). The evidence (the tests below): the jeep's doll puts the front
//! axle at z ≈ −1.7, the steering wheel at x = −0.56 and the door it calls "left"
//! (`Door_CL1`) at −x, which with the nose at −z is the driver's left — and Just Cause 2's
//! cars are left-hand drive; the PDA map (north up, east right) matches the height maps
//! as stored with x east and z south; the game's triangles are counter-clockwise about
//! their own normals (3837 of 3837 on the jeep's body), Bevy's front-face rule.
//!
//! Until this module the game mirrored z everywhere (`(x, y, z) → (x, y, −z)`), which drew
//! Panau as its mirror image, put the wheel on the right and turned every model inside
//! out. The functions are kept although most are now trivial: every place that turns
//! game data into Bevy space goes through one of them, so the convention lives here.

use bevy::prelude::*;
use jc2_formats::animation::QsTransform;
use jc2_formats::placement::Transform3;

/// An engine-space point (or direction, or normal) as a Bevy vector.
#[inline]
pub fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::from(p)
}

/// The inverse of [`to_bevy`]: a Bevy vector in engine axes.
#[inline]
pub fn to_engine(v: Vec3) -> [f32; 3] {
    v.to_array()
}

/// The engine's horizontal coordinates `(x, z)` of a Bevy position (tile indices, the
/// height maps, the map image all take these).
#[inline]
pub fn engine_xz(v: Vec3) -> (f32, f32) {
    let [x, _, z] = to_engine(v);
    (x, z)
}

/// A Bevy position from the engine's `(x, y, z)`.
#[inline]
pub fn bevy_xyz(x: f32, y: f32, z: f32) -> Vec3 {
    to_bevy([x, y, z])
}

/// An engine quaternion `[x, y, z, w]` as a Bevy rotation.
#[inline]
pub fn quat_to_bevy(q: [f32; 4]) -> Quat {
    Quat::from_array(q)
}

/// A placement transform (three basis rows and a translation, row-vector convention:
/// `p' = p·M + t`) as a Bevy `Transform`. The engine's rows are Bevy's columns.
pub fn mat_to_bevy(t: &Transform3) -> Transform {
    let col = |r: usize| to_bevy(t.row(r)).extend(0.0);
    let p = to_bevy(t.row(3)).extend(1.0);
    Transform::from_matrix(Mat4::from_cols(col(0), col(1), col(2), p))
}

/// A bone or root transform (`.bsk` reference pose, `.ban` clip samples) as a Bevy
/// `Transform`.
pub fn qs_to_bevy(t: &QsTransform) -> Transform {
    Transform {
        translation: to_bevy(t.translation),
        rotation: quat_to_bevy(t.rotation).normalize(),
        scale: Vec3::from(t.scale),
    }
}

/// The game's triangle lists as Bevy index lists. The game's front faces are
/// counter-clockwise about their normals, as Bevy's are: nothing to swap. (With a
/// mirroring conversion every triangle would need two of its corners swapped.)
#[inline]
pub fn winding_to_bevy<T>(_triangles: &mut [T]) {}

/// Where an engine model (vehicle, character, prop) faces in its own frame, in Bevy
/// axes: engine −z, which is also Bevy's `Transform::forward`.
pub const MODEL_FORWARD: Vec3 = Vec3::NEG_Z;
/// A model's right in its own frame (the driver's right hand, the starboard side).
pub const MODEL_RIGHT: Vec3 = Vec3::X;

/// The way a model with rotation `rot` faces.
#[inline]
pub fn forward_of(rot: Quat) -> Vec3 {
    rot * MODEL_FORWARD
}

/// The yaw (`Quat::from_rotation_y`) that turns [`MODEL_FORWARD`] onto the horizontal
/// part of `dir`. The same angle as a Bevy camera's yaw (`EulerRot::YXZ`) looking along
/// `dir`: positive turns left (anticlockwise seen from above).
#[inline]
pub fn yaw_toward(dir: Vec3) -> f32 {
    (-dir.x).atan2(-dir.z)
}

/// The yaw of a model's rotation, as [`yaw_toward`] of the way it faces.
#[inline]
pub fn yaw_of(rot: Quat) -> f32 {
    yaw_toward(forward_of(rot))
}

/// The horizontal direction a model faces at `yaw` (the inverse of [`yaw_toward`]).
#[inline]
pub fn facing_dir(yaw: f32) -> Vec3 {
    Quat::from_rotation_y(yaw) * MODEL_FORWARD
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;
    use jc2_formats::lod::LodDescriptor;
    use jc2_formats::rbm::RbmModel;
    use jc2_formats::sarc::SmallArchive;
    use jc2_formats::vehicle::VehicleDoll;

    /// The helpers agree with each other and with Bevy's own conventions.
    #[test]
    fn conventions() {
        use std::f32::consts::FRAC_PI_2;
        // a placement transform does what the engine's row-vector maths does
        let t = Transform3::from_mat4x3([
            0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 10.0, 20.0, 30.0,
        ]);
        let p = [1.0, 2.0, 3.0];
        assert!(
            (mat_to_bevy(&t).transform_point(to_bevy(p)) - to_bevy(t.transform_point(p))).length()
                < 1e-5
        );
        assert_eq!(to_engine(to_bevy(p)), p);
        assert_eq!(engine_xz(bevy_xyz(4.0, 5.0, 6.0)), (4.0, 6.0));
        // a model faces Bevy's forward; its right is the camera's right behind it
        assert_eq!(MODEL_FORWARD, Transform::IDENTITY.forward().as_vec3());
        assert_eq!(MODEL_RIGHT, MODEL_FORWARD.cross(Vec3::Y));
        // yaw: 0 faces −z, a quarter turn left (positive) faces −x; the inverse pair
        assert!(yaw_toward(Vec3::NEG_Z).abs() < 1e-6);
        assert!((yaw_toward(Vec3::NEG_X) - FRAC_PI_2).abs() < 1e-6);
        for yaw in [-2.5f32, -0.3, 0.0, 1.0, 3.0] {
            assert!((yaw_toward(facing_dir(yaw)) - yaw).abs() < 1e-5);
            let rot = Quat::from_rotation_y(yaw);
            assert!((yaw_of(rot) - yaw).abs() < 1e-5);
            // the same yaw as a Bevy camera looking that way
            let cam = Quat::from_euler(EulerRot::YXZ, yaw, 0.0, 0.0);
            assert!((cam * Vec3::NEG_Z - forward_of(rot)).length() < 1e-5);
        }
        // a bone transform keeps its rotation
        let q = Quat::from_euler(EulerRot::YXZ, 0.4, -0.2, 0.1);
        let qs = QsTransform {
            translation: [1.0, 2.0, 3.0],
            rotation: q.to_array(),
            scale: [1.0; 3],
        };
        let tf = qs_to_bevy(&qs);
        assert!(tf.rotation.dot(q).abs() > 0.99999);
        assert_eq!(tf.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    /// A container's members and its vehicle doll; `None` without JC2_TEST_DIR.
    fn jeep() -> Option<(Vec<u8>, SmallArchive, VehicleDoll)> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = GameFs::open(dir).ok()?;
        let data = fs.read_inflated("lave.v014_jeep.ee").ok()?;
        let sarc = SmallArchive::parse(&data).ok()?;
        let e = sarc
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with("vdoll"))?;
        let doll = VehicleDoll::parse(sarc.data(&data, e).ok()?).ok()?;
        Some((data.clone(), sarc, doll))
    }

    /// The LOD-1 model of a doll part, parsed.
    fn part_model(data: &[u8], sarc: &SmallArchive, lod_file: &str) -> Option<RbmModel> {
        let text = sarc.data(data, sarc.get(lod_file)?).ok()?;
        let d = LodDescriptor::parse(&String::from_utf8_lossy(text));
        let model = d.model_for_level(1)?;
        RbmModel::parse(sarc.data(data, sarc.get(model)?).ok()?).ok()
    }

    /// Seen from behind with Bevy's camera conventions (a camera looking along `forward`
    /// with +y up has `forward × y` on its right), the jeep's steering wheel and the door
    /// the doll calls "L" must be on the driver's LEFT: Just Cause 2's cars are left-hand
    /// drive (`reference/vehicles/car-driver-seat-LEFT-hand-drive_b05.jpg`). Fails if the
    /// conversion mirrors the world. Needs JC2_TEST_DIR.
    #[test]
    fn jeep_is_left_hand_drive_in_bevy() {
        let Some((data, sarc, doll)) = jeep() else {
            return;
        };
        // forward: from the rear axle to the front one (axle 0 is the front)
        let (mut front, mut rear) = (Vec::new(), Vec::new());
        for p in doll.wheels() {
            let hub = mat_to_bevy(&p.world).translation;
            match doll.axle_for_wheel(p.wheel.unwrap_or(0)).map(|a| a.index) {
                Some(0) => front.push(hub),
                _ => rear.push(hub),
            }
        }
        assert!(!front.is_empty() && !rear.is_empty());
        let mean = |v: &[Vec3]| v.iter().copied().sum::<Vec3>() / v.len() as f32;
        let (front, rear) = (mean(&front), mean(&rear));
        let centre = (front + rear) * 0.5;
        let forward = Vec3::new(front.x - rear.x, 0.0, front.z - rear.z).normalize();
        let right = forward.cross(Vec3::Y);

        let wheel = doll
            .parts
            .iter()
            .find(|p| p.model.to_ascii_lowercase().contains("steering"))
            .map(|p| mat_to_bevy(&p.world).translation)
            .expect("no steering wheel part");
        let wheel_side = (wheel - centre).dot(right);
        eprintln!(
            "forward {forward:?}, right {right:?}, steering wheel {wheel:?}: {wheel_side:+.2} m to the right"
        );

        // the doors: their geometry sits in place in the model (the parts are at 0,0,0)
        let side_of = |name: &str| -> f32 {
            let part = doll
                .parts
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .unwrap_or_else(|| panic!("no {name}"));
            let model = part_model(&data, &sarc, part.model_file()).expect("door model");
            let tf = mat_to_bevy(&part.world);
            let mut sum = Vec3::ZERO;
            let mut n = 0.0;
            for b in &model.blocks {
                let mesh = crate::rbm_loader::block_to_mesh(b);
                if let Some(ps) = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|a| a.as_float3())
                {
                    for p in ps {
                        sum += tf.transform_point(Vec3::from(*p));
                        n += 1.0;
                    }
                }
            }
            (sum / n - centre).dot(right)
        };
        let (left_door, right_door) = (side_of("V014-Door_CL1"), side_of("V014-Door_CR1"));
        eprintln!("Door_CL1 {left_door:+.2} m to the right, Door_CR1 {right_door:+.2} m");

        assert!(
            wheel_side < -0.3,
            "the steering wheel is on the driver's right ({wheel_side:+.2} m): the world is mirrored"
        );
        assert!(left_door < -0.5, "Door_CL1 on the right: {left_door:+.2}");
        assert!(right_door > 0.5, "Door_CR1 on the left: {right_door:+.2}");
    }

    /// Triangle winding: what the game stores, and what reaches Bevy. Bevy's front faces
    /// are counter-clockwise; a triangle's right-handed normal `(b − a) × (c − a)` must
    /// agree with the vertex normals. Needs JC2_TEST_DIR.
    #[test]
    fn front_faces_are_counter_clockwise_in_bevy() {
        let Some((data, sarc, _)) = jeep() else {
            return;
        };
        let model = RbmModel::parse(
            sarc.data(&data, sarc.get("v014-body_m_lod1.rbm").unwrap())
                .unwrap(),
        )
        .unwrap();
        let agree = |pos: &[[f32; 3]], nrm: &[[f32; 3]], idx: &[u32]| -> (usize, usize) {
            let (mut yes, mut all) = (0, 0);
            for &[ia, ib, ic] in idx.as_chunks::<3>().0 {
                let [a, b, c] = [ia, ib, ic].map(|i| Vec3::from(pos[i as usize]));
                let n: Vec3 = [ia, ib, ic]
                    .iter()
                    .map(|&i| Vec3::from(nrm[i as usize]))
                    .sum();
                let g = (b - a).cross(c - a);
                if g.length() < 1e-8 || n.length() < 1e-3 {
                    continue;
                }
                all += 1;
                if g.dot(n) > 0.0 {
                    yes += 1;
                }
            }
            (yes, all)
        };
        let (mut eng, mut bev) = ((0, 0), (0, 0));
        for b in &model.blocks {
            let md = &b.mesh;
            if md.normals.len() != md.positions.len() || md.positions.is_empty() {
                continue;
            }
            let idx: Vec<u32> = md.indices.iter().map(|&i| i as u32).collect();
            let (y, a) = agree(&md.positions, &md.normals, &idx);
            eng = (eng.0 + y, eng.1 + a);
            let mesh = crate::rbm_loader::block_to_mesh(b);
            let pos = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
                .unwrap();
            let nrm = mesh
                .attribute(Mesh::ATTRIBUTE_NORMAL)
                .and_then(|a| a.as_float3())
                .unwrap();
            let idx: Vec<u32> = mesh.indices().unwrap().iter().map(|i| i as u32).collect();
            let (y, a) = agree(pos, nrm, &idx);
            bev = (bev.0 + y, bev.1 + a);
        }
        eprintln!(
            "engine space: {}/{} triangles counter-clockwise about their normals; Bevy: {}/{}",
            eng.0, eng.1, bev.0, bev.1
        );
        assert!(bev.1 > 1000);
        assert!(
            bev.0 as f32 > 0.9 * bev.1 as f32,
            "Bevy front faces are not counter-clockwise: {}/{}",
            bev.0,
            bev.1
        );
    }
}
