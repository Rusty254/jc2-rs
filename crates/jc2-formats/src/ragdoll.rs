//! Original reflected Havok BRD ragdoll descriptors. No runtime physics policy.
mod reflection;
use crate::{
    animation::{Bone, QsTransform, Skeleton},
    collision::{self, CollisionBody},
    error::{Error, Result},
    havok::Packfile,
    placement::Transform3,
};
use reflection::Reader;

#[derive(Clone, Debug)]
pub struct Body {
    /// Raw absolute offset in the unwrapped packfile, preserved as identity.
    pub identity: usize,
    pub name: String,
    pub collision: CollisionBody,
    pub inverse_inertia_and_mass: [f32; 4],
    pub center_of_mass_local: [f32; 4],
    pub linear_velocity: [f32; 4],
    pub angular_velocity: [f32; 4],
    pub damping: [f32; 2],
    pub motion_type: u8,
}
#[derive(Clone, Debug)]
pub struct Limit {
    pub atom_class: String,
    pub identity: usize,
    pub enabled: bool,
    /// Reflected axis/measurement bytes by original member name.
    pub axes: Vec<(String, u8)>,
    pub min: f32,
    pub max: f32,
    pub tau: f32,
}
#[derive(Clone, Debug)]
pub struct Joint {
    pub identity: usize,
    pub name: String,
    pub data_identity: usize,
    pub data_class: String,
    pub body_indices: [usize; 2],
    pub frames: Option<[Transform3; 2]>,
    pub limits: Vec<Limit>,
    /// Exact atom identities/classes retained for unsupported motors/friction.
    pub extra_atoms: Vec<(String, usize, String)>,
    pub pivot_error: Option<f32>,
    pub priority: u8,
}
#[derive(Clone, Debug)]
pub struct NamedSkeleton {
    pub identity: usize,
    pub skeleton: Skeleton,
}
#[derive(Clone, Debug)]
pub struct SimpleMapping {
    pub bones: [i16; 2],
    pub a_from_b: QsTransform,
}
#[derive(Clone, Debug)]
pub struct ChainMapping {
    pub bones: [i16; 4],
    pub start_a_from_b: QsTransform,
    pub end_a_from_b: QsTransform,
}
#[derive(Clone, Debug)]
pub struct Mapper {
    pub identity: usize,
    /// Indices into `Ragdoll::skeletons`, in original A/B direction.
    pub skeletons: [usize; 2],
    pub simple: Vec<SimpleMapping>,
    pub chains: Vec<ChainMapping>,
    pub unmapped_bones: Vec<i16>,
    pub keep_unmapped_local: bool,
}
#[derive(Clone, Debug)]
pub struct Ragdoll {
    pub identity: usize,
    pub skeleton_index: usize,
    pub bone_to_body: Vec<i32>,
    /// Original hkaRagdollInstance ordering; no sorting by offsets/names.
    pub bodies: Vec<Body>,
    pub joints: Vec<Joint>,
    pub skeletons: Vec<NamedSkeleton>,
    pub mappers: Vec<Mapper>,
    pub diagnostics: Vec<String>,
}
fn qs(r: &Reader<'_>, o: usize) -> Result<QsTransform> {
    let value = QsTransform {
        translation: r.floats::<4>(o)?[..3].try_into().unwrap(),
        rotation: r.floats(o + 16)?,
        scale: r.floats::<4>(o + 32)?[..3].try_into().unwrap(),
    };
    let length = value.rotation.iter().map(|v| v * v).sum::<f32>();
    if (length - 1.).abs() > 1e-3 || value.scale.iter().any(|&s| s <= 0.) {
        return Err(Error::format("ragdoll: invalid QS rotation/scale"));
    }
    Ok(value)
}
fn transform(r: &Reader<'_>, o: usize) -> Result<Transform3> {
    let mut m = [0.; 12];
    for i in 0..4 {
        m[i * 3..i * 3 + 3].copy_from_slice(&r.floats::<4>(o + i * 16)?[..3]);
    }
    let value = Transform3::from_mat4x3(m);
    let rows = [value.row(0), value.row(1), value.row(2)];
    for i in 0..3 {
        for j in i..3 {
            let dot = (0..3).map(|k| rows[i][k] * rows[j][k]).sum::<f32>();
            if (dot - if i == j { 1. } else { 0. }).abs() > 1e-3 {
                return Err(Error::format("ragdoll: nonrigid transform basis"));
            }
        }
    }
    let determinant = rows[0][0] * (rows[1][1] * rows[2][2] - rows[1][2] * rows[2][1])
        - rows[0][1] * (rows[1][0] * rows[2][2] - rows[1][2] * rows[2][0])
        + rows[0][2] * (rows[1][0] * rows[2][1] - rows[1][1] * rows[2][0]);
    if determinant <= 0. {
        return Err(Error::format("ragdoll: reflected transform basis"));
    }
    Ok(value)
}
fn unsupported_shape(shape: &crate::collision::CollisionShape) -> bool {
    use crate::collision::CollisionShape as S;
    match shape {
        S::Unsupported { .. } => true,
        S::Compound { children } => children.iter().any(unsupported_shape),
        S::Transformed { child, .. } => unsupported_shape(child),
        _ => false,
    }
}
fn string_field(r: &Reader<'_>, o: usize, c: &str, n: &str) -> Result<String> {
    let f = r.field(o, c, n)?;
    r.pointer(f)
        .map(|p| r.string(p))
        .transpose()
        .map(|s| s.unwrap_or_default())
}
fn pointers(r: &Reader<'_>, o: usize, c: &str, n: &str, expected: &str) -> Result<Vec<usize>> {
    let member = r.member(c, n)?;
    if member.subtype != 20 || member.count != 0 {
        return Err(Error::format("ragdoll: expected dynamic pointer array"));
    }
    let (p, count) = r.array(o, c, n, 4)?;
    (0..count)
        .map(|i| {
            let x = r.required_pointer(p + i * 4)?;
            r.object(x, expected)?;
            Ok(x)
        })
        .collect()
}
fn skeleton(r: &Reader<'_>, o: usize) -> Result<NamedSkeleton> {
    r.object(o, "hkaSkeleton")?;
    let name = string_field(r, o, "hkaSkeleton", "name")?;
    let (p, n) = r.array(o, "hkaSkeleton", "parentIndices", 2)?;
    let parents: Vec<_> = (0..n)
        .map(|i| r.u16(p + i * 2).map(|v| v as i16))
        .collect::<Result<_>>()?;
    let refs = pointers(r, o, "hkaSkeleton", "bones", "hkaBone")?;
    if refs.len() != n {
        return Err(Error::format("ragdoll: bone/parent count mismatch"));
    }
    let mut bones = Vec::new();
    for (i, b) in refs.iter().enumerate() {
        let parent = parents[i];
        if parent < -1 || parent >= i as i16 {
            return Err(Error::format("ragdoll: invalid skeleton hierarchy"));
        }
        bones.push(Bone {
            name: string_field(r, *b, "hkaBone", "name")?,
            parent: (parent >= 0).then_some(parent as usize),
        });
    }
    let (p, count) = r.array(o, "hkaSkeleton", "referencePose", 48)?;
    if count != n {
        return Err(Error::format("ragdoll: reference pose count mismatch"));
    }
    let reference_pose = (0..count)
        .map(|i| qs(r, p + i * 48))
        .collect::<Result<_>>()?;
    Ok(NamedSkeleton {
        identity: o,
        skeleton: Skeleton {
            name,
            bones,
            reference_pose,
        },
    })
}
impl Ragdoll {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let r = Reader::parse(bytes)?;
        let instances: Vec<_> = r
            .objects
            .iter()
            .filter(|(_, c)| c.as_str() == "hkaRagdollInstance")
            .map(|(&o, _)| o)
            .collect();
        if instances.len() != 1 {
            return Err(Error::format(
                "ragdoll: expected exactly one hkaRagdollInstance",
            ));
        }
        let identity = instances[0];
        r.object(identity, "hkaRagdollInstance")?;
        let body_ids = pointers(
            &r,
            identity,
            "hkaRagdollInstance",
            "rigidBodies",
            "hkpRigidBody",
        )?;
        if body_ids.is_empty() {
            return Err(Error::format("ragdoll: empty body membership"));
        }
        if body_ids
            .iter()
            .enumerate()
            .any(|(i, b)| body_ids[..i].contains(b))
        {
            return Err(Error::format("ragdoll: duplicate body membership"));
        }
        let pf = Packfile::parse(bytes)?;
        let all_collision = collision::bodies(&pf);
        let all_ids: Vec<_> = pf
            .objects_of("hkpRigidBody")
            .map(|o| o.offset as usize + r.data_start)
            .collect();
        let mut bodies = Vec::new();
        for &b in &body_ids {
            let (motion, mc) = r.nested(b, "hkpRigidBody", "motion")?;
            let (state, sc) = r.nested(motion, &mc, "motionState")?;
            let (swept, swc) = r.nested(state, &sc, "sweptTransform")?;
            let inertia = r.floats::<4>(r.field(motion, &mc, "inertiaAndMassInv")?)?;
            if inertia.iter().any(|&x| x < 0.) {
                return Err(Error::format("ragdoll: negative inverse inertia/mass"));
            }
            let rest = transform(&r, r.field(state, &sc, "transform")?)?;
            let index = all_ids
                .iter()
                .position(|&x| x == b)
                .ok_or_else(|| Error::format("ragdoll: collision body identity"))?;
            let mut collision = all_collision[index].clone();
            collision.transform = rest;
            bodies.push(Body {
                identity: b,
                name: string_field(&r, b, "hkpRigidBody", "name")?,
                collision,
                inverse_inertia_and_mass: inertia,
                center_of_mass_local: r.floats(r.field(swept, &swc, "centerOfMassLocal")?)?,
                linear_velocity: r.floats(r.field(motion, &mc, "linearVelocity")?)?,
                angular_velocity: r.floats(r.field(motion, &mc, "angularVelocity")?)?,
                damping: [
                    r.float(r.field(state, &sc, "linearDamping")?)?,
                    r.float(r.field(state, &sc, "angularDamping")?)?,
                ],
                motion_type: r.read(r.field(motion, &mc, "type")?, 1)?[0],
            });
        }
        let skeletons = r
            .objects
            .iter()
            .filter(|(_, c)| c.as_str() == "hkaSkeleton")
            .map(|(&o, _)| skeleton(&r, o))
            .collect::<Result<Vec<_>>>()?;
        let sk = r.required_pointer(r.field(identity, "hkaRagdollInstance", "skeleton")?)?;
        let skeleton_index = skeletons
            .iter()
            .position(|s| s.identity == sk)
            .ok_or_else(|| Error::format("ragdoll: skeleton reference"))?;
        let (p, n) = r.array(identity, "hkaRagdollInstance", "boneToRigidBodyMap", 4)?;
        let bone_to_body = (0..n)
            .map(|i| r.u32(p + i * 4).map(|v| v as i32))
            .collect::<Result<Vec<_>>>()?;
        if n != skeletons[skeleton_index].skeleton.bones.len()
            || bone_to_body
                .iter()
                .any(|&b| b < -1 || b >= bodies.len() as i32)
        {
            return Err(Error::format("ragdoll: bone-body mapping range/count"));
        }
        for index in 0..bodies.len() {
            if bone_to_body.iter().filter(|&&v| v == index as i32).count() != 1 {
                return Err(Error::format(
                    "ragdoll: body mapping must be one-to-one and complete",
                ));
            }
        }
        let mut diagnostics = Vec::new();
        for b in &bodies {
            if unsupported_shape(&b.collision.shape) {
                diagnostics.push(format!(
                    "unsupported collision shape on body {} @{:x}",
                    b.name, b.identity
                ));
            }
        }
        let mut joints = Vec::new();
        for j in pointers(
            &r,
            identity,
            "hkaRagdollInstance",
            "constraints",
            "hkpConstraintInstance",
        )? {
            if joints.iter().any(|prior: &Joint| prior.identity == j) {
                return Err(Error::format("ragdoll: repeated constraint membership"));
            }
            let d = r.required_pointer(r.field(j, "hkpConstraintInstance", "data")?)?;
            let dc = r
                .objects
                .get(&d)
                .ok_or_else(|| Error::format("ragdoll: constraint data reference"))?
                .clone();
            r.object(d, &dc)?;
            let entity_member = r.member("hkpConstraintInstance", "entities")?;
            if entity_member.kind != 20 || entity_member.count != 2 {
                return Err(Error::format("ragdoll: expected two constraint entities"));
            }
            let e = r.field(j, "hkpConstraintInstance", "entities")?;
            let mut body_indices = [0; 2];
            for (i, x) in body_indices.iter_mut().enumerate() {
                let b = r.required_pointer(e + i * 4)?;
                *x = body_ids
                    .iter()
                    .position(|&x| x == b)
                    .ok_or_else(|| Error::format("ragdoll: constraint body outside membership"))?;
            }
            if body_indices[0] == body_indices[1] {
                return Err(Error::format("ragdoll: self-body constraint"));
            }
            let mut joint = Joint {
                identity: j,
                name: string_field(&r, j, "hkpConstraintInstance", "name")?,
                data_identity: d,
                data_class: dc.clone(),
                body_indices,
                frames: None,
                limits: Vec::new(),
                extra_atoms: Vec::new(),
                pivot_error: None,
                priority: r.read(r.field(j, "hkpConstraintInstance", "priority")?, 1)?[0],
            };
            if matches!(
                dc.as_str(),
                "hkpRagdollConstraintData" | "hkpLimitedHingeConstraintData"
            ) {
                let (a, ac) = r.nested(d, &dc, "atoms")?;
                let (t, tc) = r.nested(a, &ac, "transforms")?;
                let frames = [
                    transform(&r, r.field(t, &tc, "transformA")?)?,
                    transform(&r, r.field(t, &tc, "transformB")?)?,
                ];
                let p0 = bodies[body_indices[0]]
                    .collision
                    .transform
                    .transform_point(frames[0].translation());
                let p1 = bodies[body_indices[1]]
                    .collision
                    .transform
                    .transform_point(frames[1].translation());
                let error = (0..3).map(|i| (p0[i] - p1[i]).powi(2)).sum::<f32>().sqrt();
                joint.frames = Some(frames);
                joint.pivot_error = Some(error);
                if error > 0.01 {
                    diagnostics.push(format!(
                        "joint {} rest pivot mismatch {error:.6}m",
                        joint.name
                    ));
                }
                for (name, m) in &r.classes[&ac].members {
                    if name == "transforms" {
                        continue;
                    }
                    let c = m
                        .class
                        .as_deref()
                        .ok_or_else(|| Error::format("ragdoll: atom reflection class"))?;
                    let o = a + m.offset;
                    if matches!(
                        c,
                        "hkpTwistLimitConstraintAtom"
                            | "hkpConeLimitConstraintAtom"
                            | "hkpAngLimitConstraintAtom"
                    ) {
                        let mut axes = Vec::new();
                        for key in [
                            "twistAxis",
                            "refAxis",
                            "twistAxisInA",
                            "refAxisInB",
                            "angleMeasurementMode",
                            "limitAxis",
                        ] {
                            if r.member(c, key).is_ok() {
                                axes.push((key.to_owned(), r.read(r.field(o, c, key)?, 1)?[0]));
                            }
                        }
                        let min = r.float(r.field(o, c, "minAngle")?)?;
                        let max = r.float(r.field(o, c, "maxAngle")?)?;
                        if min > max {
                            return Err(Error::format("ragdoll: reversed angular limit"));
                        }
                        joint.limits.push(Limit {
                            atom_class: c.into(),
                            identity: o,
                            enabled: r.read(r.field(o, c, "isEnabled")?, 1)?[0] != 0,
                            axes,
                            min,
                            max,
                            tau: r.float(r.field(o, c, "angularLimitsTauFactor")?)?,
                        });
                    } else {
                        joint.extra_atoms.push((name.clone(), o, c.to_owned()));
                    }
                }
            } else {
                diagnostics.push(format!("unsupported joint data {dc} @{d:x}"));
            }
            if r.pointer(r.field(j, "hkpConstraintInstance", "constraintModifiers")?)
                .is_some()
            {
                diagnostics.push(format!("constraint modifiers not decoded @{j:x}"));
            }
            joints.push(joint);
        }
        let mut mappers = Vec::new();
        for (&o, c) in &r.objects {
            if c != "hkaSkeletonMapper" {
                continue;
            }
            r.object(o, c)?;
            let (m, mc) = r.nested(o, c, "mapping")?;
            let mut ids = [0; 2];
            for (i, key) in ["skeletonA", "skeletonB"].iter().enumerate() {
                let sk = r.required_pointer(r.field(m, &mc, key)?)?;
                ids[i] = skeletons
                    .iter()
                    .position(|s| s.identity == sk)
                    .ok_or_else(|| Error::format("ragdoll: mapper skeleton reference"))?;
            }
            let simple_class = "hkaSkeletonMapperDataSimpleMapping";
            let stride = r
                .classes
                .get(simple_class)
                .ok_or_else(|| Error::format("ragdoll: simple mapping reflection"))?
                .size;
            let (p, n) = r.array(m, &mc, "simpleMappings", stride)?;
            let mut simple = Vec::new();
            for i in 0..n {
                let a = p + i * stride;
                let bones = [
                    r.u16(r.field(a, simple_class, "boneA")?)? as i16,
                    r.u16(r.field(a, simple_class, "boneB")?)? as i16,
                ];
                validate_bones(&skeletons, ids, &bones)?;
                simple.push(SimpleMapping {
                    bones,
                    a_from_b: qs(&r, r.field(a, simple_class, "aFromBTransform")?)?,
                });
            }
            let cc = "hkaSkeletonMapperDataChainMapping";
            let stride = r
                .classes
                .get(cc)
                .ok_or_else(|| Error::format("ragdoll: chain reflection"))?
                .size;
            let (p, n) = r.array(m, &mc, "chainMappings", stride)?;
            let mut chains = Vec::new();
            for i in 0..n {
                let a = p + i * stride;
                let mut bones = [0; 4];
                for (k, key) in ["startBoneA", "endBoneA", "startBoneB", "endBoneB"]
                    .iter()
                    .enumerate()
                {
                    bones[k] = r.u16(r.field(a, cc, key)?)? as i16;
                }
                validate_bones(&skeletons, ids, &[bones[0], bones[2]])?;
                validate_bones(&skeletons, ids, &[bones[1], bones[3]])?;
                chains.push(ChainMapping {
                    bones,
                    start_a_from_b: qs(&r, r.field(a, cc, "startAFromBTransform")?)?,
                    end_a_from_b: qs(&r, r.field(a, cc, "endAFromBTransform")?)?,
                });
            }
            let (p, n) = r.array(m, &mc, "unmappedBones", 2)?;
            let unmapped_bones = (0..n)
                .map(|i| r.u16(p + i * 2).map(|x| x as i16))
                .collect::<Result<Vec<_>>>()?;
            if unmapped_bones
                .iter()
                .any(|&x| x < 0 || x as usize >= skeletons[ids[1]].skeleton.bones.len())
            {
                return Err(Error::format("ragdoll: unmapped bone range"));
            }
            mappers.push(Mapper {
                identity: o,
                skeletons: ids,
                simple,
                chains,
                unmapped_bones,
                keep_unmapped_local: r.read(r.field(m, &mc, "keepUnmappedLocal")?, 1)?[0] != 0,
            });
        }
        diagnostics.push(
            "Motors/friction atoms retained by identity/class; powered behavior not decoded".into(),
        );
        Ok(Self {
            identity,
            skeleton_index,
            bone_to_body,
            bodies,
            joints,
            skeletons,
            mappers,
            diagnostics,
        })
    }
}
fn validate_bones(sk: &[NamedSkeleton], ids: [usize; 2], bones: &[i16; 2]) -> Result<()> {
    for i in 0..2 {
        if bones[i] < 0 || bones[i] as usize >= sk[ids[i]].skeleton.bones.len() {
            return Err(Error::format("ragdoll: mapper bone range"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn replace_fixup(bytes: &mut [u8], slot: usize, target: usize) {
        let u = |b: &[u8], o| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        let n = u(bytes, 20);
        for i in 0..n {
            let h = 84 + i * 48;
            let base = u(bytes, h);
            let local = u(bytes, h + 4);
            let global = u(bytes, h + 8);
            let virtuals = u(bytes, h + 12);
            for p in (base + local..base + global).step_by(8) {
                if u(bytes, p) != u32::MAX as usize && base + u(bytes, p) == slot {
                    bytes[p + 4..p + 8].copy_from_slice(&((target - base) as u32).to_le_bytes());
                    return;
                }
            }
            for p in (base + global..base + virtuals).step_by(12) {
                if u(bytes, p) != u32::MAX as usize && base + u(bytes, p) == slot {
                    let section = u(bytes, p + 4);
                    let target_base = u(bytes, 84 + section * 48);
                    bytes[p + 8..p + 12]
                        .copy_from_slice(&((target - target_base) as u32).to_le_bytes());
                    return;
                }
            }
        }
        panic!("fixup not found");
    }
    #[test]
    fn rejects_invalid_headers() {
        for b in [&[][..], &[0; 64][..], &crate::havok::MAGIC[..]] {
            assert!(Ragdoll::parse(b).is_err());
        }
    }
    #[test]
    fn original_ragdolls_and_corruptions() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP original ragdolls: JC2_TEST_DIR absent");
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        for (name, body_count, mapper_count) in [
            ("triplemapped_normal_ragdoll.brd", 14, 6),
            ("stunt_ragdoll_positionmotor.brd", 16, 2),
        ] {
            let bytes = fs.read_any(name).unwrap();
            let d = Ragdoll::parse(&bytes).unwrap();
            assert_eq!(d.bodies.len(), body_count);
            assert_eq!(d.joints.len(), body_count - 1);
            assert_eq!(d.mappers.len(), mapper_count);
            assert!(!d.joints.is_empty());
            assert!(
                d.joints
                    .iter()
                    .all(|j| j.frames.is_some() && j.pivot_error.unwrap() < 1e-5)
            );
            assert!(d.bodies.iter().all(|b| b.name.starts_with("ragdoll_")));
            let r = Reader::parse(&bytes).unwrap();
            let f = r
                .field(d.identity, "hkaRagdollInstance", "rigidBodies")
                .unwrap();
            let mut bad = bytes.clone();
            bad[f + 4..f + 8].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(Ragdoll::parse(&bad).is_err());
            let mut bad = bytes.clone();
            bad.truncate(bytes.len() / 2);
            assert!(Ragdoll::parse(&bad).is_err());
            let mut bad = bytes.clone();
            let (p, _) = r
                .array(d.identity, "hkaRagdollInstance", "boneToRigidBodyMap", 4)
                .unwrap();
            bad[p..p + 4].copy_from_slice(&9999i32.to_le_bytes());
            assert!(Ragdoll::parse(&bad).is_err());
            let mut bad = bytes.clone();
            bad[p + 4..p + 8].copy_from_slice(&0i32.to_le_bytes());
            assert!(
                Ragdoll::parse(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("one-to-one")
            );
            let (constraints, _) = r
                .array(d.identity, "hkaRagdollInstance", "constraints", 4)
                .unwrap();
            let mut bad = bytes.clone();
            replace_fixup(&mut bad, constraints + 4, d.joints[0].identity);
            assert!(
                Ragdoll::parse(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("repeated constraint")
            );
            let entities = r
                .field(d.joints[0].identity, "hkpConstraintInstance", "entities")
                .unwrap();
            let mut bad = bytes.clone();
            replace_fixup(
                &mut bad,
                entities + 4,
                r.required_pointer(entities).unwrap(),
            );
            assert!(
                Ragdoll::parse(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("self-body")
            );
            let (motion, motion_class) = r
                .nested(d.bodies[0].identity, "hkpRigidBody", "motion")
                .unwrap();
            let inertia = r.field(motion, &motion_class, "inertiaAndMassInv").unwrap();
            let mut bad = bytes.clone();
            bad[inertia..inertia + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            assert!(Ragdoll::parse(&bad).is_err());
            let mapper = d.mappers[0].identity;
            let (mapping, class) = r.nested(mapper, "hkaSkeletonMapper", "mapping").unwrap();
            let (simple, count) = r.array(mapping, &class, "simpleMappings", 64).unwrap();
            assert!(count > 0);
            let mut bad = bytes.clone();
            bad[simple..simple + 2].copy_from_slice(&i16::MAX.to_le_bytes());
            assert!(Ragdoll::parse(&bad).is_err());
            let mut bad = bytes.clone();
            bad[simple + 32..simple + 48].fill(0);
            assert!(
                Ragdoll::parse(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("QS rotation")
            );
            let (state, sc) = r.nested(motion, &motion_class, "motionState").unwrap();
            let rest = r.field(state, &sc, "transform").unwrap();
            let mut bad = bytes.clone();
            bad[rest..rest + 4].copy_from_slice(&2f32.to_le_bytes());
            assert!(
                Ragdoll::parse(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("nonrigid")
            );
        }
    }
}
