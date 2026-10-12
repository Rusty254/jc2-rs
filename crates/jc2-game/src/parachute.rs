//! The canopy: the retail skinned model `general.blz/gae05_lod1-gae05.rbm` (named by
//! `settings\parachuteparams.bin` entry `parachute00`, exe `0x00e42bc0`; textures `gea05_*`,
//! sic). ONE `SkinnedGeneral` mesh holds the ram-air canopy, its 16 suspension line tubes,
//! the harness straps and the two brake handles, skinned to the 17 bones of `parachute.bsk`
//! with bind pose = the skeleton's reference pose. The joints here are driven by
//! `chute_open_chute.ban` and then the five-clip blend. Evidence and gaps (shader state,
//! `mpm` channels, cloth): `research/formats/parachute-canopy-model.md`.

use crate::coords::qs_to_bevy;
use crate::rbm_loader::RbmAsset;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use jc2_formats::GameFs;
use jc2_formats::animation::{QsTransform, Skeleton, SplineAnimation};

/// The retail canopy model (parachuteparams.bin `parachute00`, `model` key).
const CANOPY_MODEL: &str = "jc2://general.blz/gae05_lod1-gae05.rbm";

/// The canopy blend inputs of the frame (`D+4`, `D+8`, ...), written by the hero update.
#[derive(Resource, Default)]
pub struct CanopyPoseFeed(pub crate::parachute_flight::CanopyPose);

/// The five blend clips of `PC_ANIM_OPEN` (`parachute.asb`), in slot order (landing doc 10.1).
const BLEND_CLIPS: [&str; 5] = [
    "chute_turn_TB_UP.ban",
    "chute_turn_TB_DN.ban",
    "chute_speed_TB_UP.ban",
    "chute_speed_TB_DN.ban",
    "chute_add_idle.ban",
];

/// Sampler weight threshold (`0x00b077c0`): a control takes part above 1.19e-7.
const MIN_CLIP_WEIGHT: f32 = 1.19e-7;

/// The animation step `0x00b0e980` (landing doc 11.2/11.3) applied after the blend and before
/// sampling: time := max(time, 0) + dt * rate, wrapped by the clip length keeping the remainder.
/// The four TB clips run at rate +1.0 (asb `playback_speed` 1), the idle clip at the blend's
/// rate. Cropping and ease are no-ops here (no crop; hard-cut state, blend time 0).
pub fn step_blend(clips: &mut [crate::parachute_flight::ClipBlend; 5], durations: [f32; 5], dt: f32) {
    for (i, c) in clips.iter_mut().enumerate() {
        let rate = if i < 4 { 1.0 } else { c.rate.unwrap_or(1.0) };
        let len = durations[i];
        let mut t = c.time.max(0.0) + dt * rate;
        if len > 0.0 && t > len {
            t -= (t / len).floor() * len;
        }
        c.time = t.max(0.0);
    }
}

/// Additive combine of one bone (`0x00b01870`, landing doc 11.5): the sample is a delta on the
/// LEFT; the pose becomes the short-arc lerp toward the composed transform by `w`.
pub fn additive_blend(pose: &Transform, sample: &Transform, w: f32) -> Transform {
    let comp_t = sample.rotation * pose.translation + sample.translation;
    let comp_q = sample.rotation * pose.rotation;
    let comp_s = sample.scale * pose.scale;
    let comp_q = if pose.rotation.dot(comp_q) < 0.0 { -comp_q } else { comp_q };
    Transform {
        translation: pose.translation.lerp(comp_t, w),
        rotation: (pose.rotation * (1.0 - w) + comp_q * w).normalize(),
        scale: pose.scale.lerp(comp_s, w),
    }
}

/// The parachute skeleton and its opening clip.
#[derive(Resource)]
pub struct ChuteRig {
    pub skeleton: Skeleton,
    pub open: Option<SplineAnimation>,
    /// Track → bone of the opening clip.
    open_bones: Vec<Option<usize>>,
    /// `chute_reel_open.ban` (`PC_ANIM_OPENING_HORIZONTAL`) and `parachute_release.ban`
    /// (`PC_ANIM_CLOSING`) with their track-to-bone maps (`parachute-deploy-collapse.md` 3).
    horizontal: Option<(SplineAnimation, Vec<Option<usize>>)>,
    release: Option<(SplineAnimation, Vec<Option<usize>>)>,
    /// The five blend clips (`BLEND_CLIPS`) and their track-to-bone maps; `None` when absent.
    blend: [Option<(SplineAnimation, Vec<Option<usize>>)>; 5],
}

impl ChuteRig {
    pub fn load(fs: &GameFs) -> Option<Self> {
        let bytes = fs.read_inflated("parachute.bsk").ok()?;
        let skeleton = Skeleton::parse(&bytes).ok()?;
        let bone = |n: &str| skeleton.bone_index(n);
        let open = crate::rig::read_animation_file(fs, "chute_open_chute.ban")
            .and_then(|b| SplineAnimation::parse(&b).ok());
        let open_bones = open
            .as_ref()
            .map(|a| a.track_names.iter().map(|n| bone(n)).collect())
            .unwrap_or_default();
        let clip_with_bones = |name: &str| {
            let anim = crate::rig::read_animation_file(fs, name)
                .and_then(|b| SplineAnimation::parse(&b).ok())?;
            let bones = anim.track_names.iter().map(|n| bone(n)).collect();
            Some((anim, bones))
        };
        let horizontal = clip_with_bones("chute_reel_open.ban");
        let release = clip_with_bones("parachute_release.ban");
        let blend = BLEND_CLIPS.map(|name| {
            let anim = crate::rig::read_animation_file(fs, name)
                .and_then(|b| SplineAnimation::parse(&b).ok())?;
            let bones = anim.track_names.iter().map(|n| bone(n)).collect();
            Some((anim, bones))
        });
        info!(
            "parachute: {} bones, opening clip {}",
            skeleton.bones.len(),
            if open.is_some() { "yes" } else { "no" }
        );
        Some(Self {
            skeleton,
            open,
            open_bones,
            horizontal,
            release,
            blend,
        })
    }

    /// Durations of the five blend clips (0 when a clip is missing).
    fn blend_durations(&self) -> [f32; 5] {
        std::array::from_fn(|i| self.blend[i].as_ref().map_or(0.0, |(a, _)| a.duration))
    }

    /// Bone local transforms under the canopy blend (landing doc 10.2 and 11): `clips` must
    /// already be stepped (`step_blend`, 11.2). The four NORMAL clips are a weighted sum divided
    /// by the weight sum (sampler pass 0, `0x00b01560` + `0x00b261d0`; weights below 1.19e-7
    /// skipped), then `chute_add_idle` is applied ADDITIVE (`additive_blend`, 11.5). `None`
    /// when a clip is missing.
    fn blended_locals(
        &self,
        clips: &[crate::parachute_flight::ClipBlend; 5],
        scratch: &mut Vec<QsTransform>,
    ) -> Option<Vec<Transform>> {
        if self.blend.iter().any(Option::is_none) {
            return None;
        }
        let mut local: Vec<Transform> =
            self.skeleton.reference_pose.iter().map(qs_to_bevy).collect();
        let total: f32 = clips[..4]
            .iter()
            .map(|c| c.weight)
            .filter(|w| *w > MIN_CLIP_WEIGHT)
            .sum();
        let mut acc: Vec<Option<(Vec3, Quat)>> = vec![None; local.len()];
        for (i, c) in clips[..4].iter().enumerate() {
            if c.weight <= MIN_CLIP_WEIGHT || total <= 0.0 {
                continue;
            }
            let w = c.weight / total;
            let (anim, bones) = self.blend[i].as_ref()?;
            anim.sample(c.time.clamp(0.0, anim.duration), scratch);
            for (ti, qs) in scratch.iter().enumerate() {
                let Some(Some(b)) = bones.get(ti) else { continue };
                let t = qs_to_bevy(qs);
                let (p, q) = acc[*b].unwrap_or((Vec3::ZERO, Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)));
                // keep the quaternions in one hemisphere before summing
                let r = if q.dot(t.rotation) < 0.0 { -t.rotation } else { t.rotation };
                acc[*b] = Some((p + t.translation * w, q + r * w));
            }
        }
        for (b, a) in acc.iter().enumerate() {
            if let Some((p, q)) = a {
                local[b].translation = *p;
                local[b].rotation = q.normalize();
            }
        }
        // ADDITIVE pass (`chute_add_idle`, blendHint 1; sampler pass 1 `0x00b01870`): the
        // clip is a delta applied on top of the normal pass, bones it does not key untouched.
        let (idle, bones) = self.blend[4].as_ref()?;
        idle.sample(clips[4].time.clamp(0.0, idle.duration), scratch);
        let w = clips[4].weight;
        for (ti, qs) in scratch.iter().enumerate() {
            let Some(Some(b)) = bones.get(ti) else { continue };
            local[*b] = additive_blend(&local[*b], &qs_to_bevy(qs), w);
        }
        Some(local)
    }

    /// Bone local transforms (Bevy axes) for the pose at `time` into the opening clip
    /// (`None` = the reference pose).
    fn pose_locals(&self, time: Option<f32>, scratch: &mut Vec<QsTransform>) -> Vec<Transform> {
        let clip = self.open.as_ref().map(|a| (a, self.open_bones.as_slice()));
        self.clip_locals(clip, time, scratch)
    }

    /// Bone local transforms for `clip` sampled at `time`; bones the clip does not key keep
    /// the reference pose.
    fn clip_locals(
        &self,
        clip: Option<(&SplineAnimation, &[Option<usize>])>,
        time: Option<f32>,
        scratch: &mut Vec<QsTransform>,
    ) -> Vec<Transform> {
        let mut local: Vec<Transform> = self
            .skeleton
            .reference_pose
            .iter()
            .map(qs_to_bevy)
            .collect();
        if let (Some(t), Some((anim, bones))) = (time, clip) {
            anim.sample(t.clamp(0.0, anim.duration), scratch);
            for (ti, qs) in scratch.iter().enumerate() {
                if let Some(Some(b)) = bones.get(ti) {
                    local[*b] = qs_to_bevy(qs);
                }
            }
        }
        local
    }

    /// Clip lengths for the canopy state machine; the probe's lengths when a clip is absent.
    fn clip_lengths(&self) -> ClipLengths {
        let d = ClipLengths::NATIVE;
        ClipLengths {
            vertical: self.open.as_ref().map_or(d.vertical, |a| a.duration),
            horizontal: self.horizontal.as_ref().map_or(d.horizontal, |(a, _)| a.duration),
            release: self.release.as_ref().map_or(d.release, |(a, _)| a.duration),
        }
    }

    /// Bone positions in the canopy's frame from the local transforms.
    #[cfg(test)]
    fn globals(&self, local: &[Transform]) -> Vec<Vec3> {
        let n = self.skeleton.bones.len();
        let mut global: Vec<Transform> = Vec::with_capacity(n);
        for (i, bone) in self.skeleton.bones.iter().enumerate() {
            let g = match bone.parent {
                Some(p) if p < i => global[p] * local[i],
                _ => local[i],
            };
            global.push(g);
        }
        global.iter().map(|g| g.translation).collect()
    }
}

/// The canopy's own state machine `animations/statemachine/parachute.afsm`
/// (`parachute-deploy-collapse.md` 2): the `PCS_*` states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CanopyState {
    /// `PCS_CLOSED`: model hidden.
    #[default]
    Closed,
    /// `PCS_OPENING_VERTICAL`: `chute_open_chute.ban`, the free-fall chord.
    OpeningVertical,
    /// `PCS_OPENING_HORIZONTAL`: `chute_reel_open.ban`, reel/roof/vehicle entries.
    OpeningHorizontal,
    /// `PCS_OPEN`: the five-clip blend.
    Open,
    /// `PCS_CLOSING`: `parachute_release.ban`.
    Closing,
}

/// Clip lengths in seconds.
#[derive(Clone, Copy, Debug)]
pub struct ClipLengths {
    pub vertical: f32,
    pub horizontal: f32,
    pub release: f32,
}

impl ClipLengths {
    /// Lengths of `chute_open_chute.ban`, `chute_reel_open.ban`, `parachute_release.ban`
    /// (native, probe `parachute_deploy_collapse_probe.py`).
    pub const NATIVE: Self = Self {
        vertical: 2.433,
        horizontal: 2.0,
        release: 1.4,
    };
}

/// `show_parachute` event time of `chute_open_chute.ban` (native, raw clip time).
pub const SHOW_VERTICAL: f32 = 0.33;
/// `show_parachute` event time of `chute_reel_open.ban` (native).
pub const SHOW_HORIZONTAL: f32 = 0.03;
/// `hide_parachute` event time of the character clip `detatch_chute.ban` (native); the
/// cut-away removes the model then.
pub const CUT_AWAY_HIDE: f32 = 0.5;

/// What the hero asks of the canopy this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanopyCommand {
    /// The canopy is wanted (the foot machine is in a canopy state).
    pub up: bool,
    /// The open came from a reel/roof/vehicle entry (`S_UNFOLD_PARACHUTE_HORIZONTAL`).
    pub horizontal: bool,
    /// The canopy is being let go in the air (`S_PARACHUTE_DETACH_TO_FREEFALL`).
    pub cut_away: bool,
}

/// The `PCS_*` machine plus the model's visibility, stepped per frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanopyMachine {
    pub state: CanopyState,
    /// Seconds into the current opening or closing clip.
    pub t: f32,
    /// The model is drawn (`show_parachute` .. `hide_parachute` / `PCS_CLOSED`).
    pub shown: bool,
    /// Seconds since a cut-away close began.
    cut_t: Option<f32>,
}

impl CanopyMachine {
    /// Advance by `dt`. The open command is taken only from `PCS_CLOSED` (native driver
    /// `0x0074edd0`); a close starts the frame the command drops (rules (a)/(b) of the
    /// landing doc) and plays out, hiding at the clip end or, on a cut-away, at 0.5 s.
    pub fn step(&mut self, dt: f32, cmd: CanopyCommand, len: ClipLengths) {
        match self.state {
            CanopyState::Closed => {
                self.shown = false;
                if cmd.up {
                    self.state = if cmd.horizontal {
                        CanopyState::OpeningHorizontal
                    } else {
                        CanopyState::OpeningVertical
                    };
                    self.t = 0.0;
                    self.cut_t = None;
                }
            }
            CanopyState::OpeningVertical | CanopyState::OpeningHorizontal => {
                let (show, dur) = if self.state == CanopyState::OpeningVertical {
                    (SHOW_VERTICAL, len.vertical)
                } else {
                    (SHOW_HORIZONTAL, len.horizontal)
                };
                if !cmd.up {
                    self.begin_close(cmd);
                    self.step_closing(dt, len);
                    return;
                }
                self.t += dt;
                if self.t >= show {
                    self.shown = true;
                }
                if self.t >= dur {
                    self.state = CanopyState::Open;
                }
            }
            CanopyState::Open => {
                self.shown = true;
                if !cmd.up {
                    self.begin_close(cmd);
                    self.step_closing(dt, len);
                }
            }
            CanopyState::Closing => self.step_closing(dt, len),
        }
    }

    fn begin_close(&mut self, cmd: CanopyCommand) {
        self.state = CanopyState::Closing;
        self.t = 0.0;
        self.cut_t = cmd.cut_away.then_some(0.0);
    }

    fn step_closing(&mut self, dt: f32, len: ClipLengths) {
        self.t += dt;
        if let Some(c) = &mut self.cut_t {
            *c += dt;
            if *c >= CUT_AWAY_HIDE {
                self.shown = false;
            }
        }
        if self.t >= len.release {
            self.state = CanopyState::Closed;
            self.shown = false;
        }
    }
}

/// The point of the canopy entity the flight basis turns about, in the entity's own frame.
/// NATIVE (`parachute-attach-pivot.md` 1-2): the canopy update `0x006ccd40` pushes the
/// character's world matrix (`character + 0x15c`) to the model-matrix setter `0x0084cd10`
/// unchanged, and the flight model (`0x005ab3f2..0x005ab52d`) writes its basis into that
/// same entity matrix keeping the position, so the canopy turns about the character
/// origin, Rico's feet. The canopy entity sits at the feet, so its origin is the pivot
/// (`base` and `HarnessExt` rest 1.227 m above it).
pub const CANOPY_PIVOT: Vec3 = Vec3::ZERO;

/// The canopy entity's rotation from the flight basis `M = Rz(a38) * Rx(a30) * Ry(a34)`
/// (`0x005ab3f2-0x005ab421`, `CanopyFlight::matrix`). The rows of `M` are the images of the
/// local X, Y, Z axes (row-vector convention), i.e. the columns of the rotation. Returns the
/// rotation and the translation that keeps [`CANOPY_PIVOT`] fixed, given the entity's rest
/// `origin`.
pub fn canopy_orientation(rows: &crate::parachute_flight::Rows, origin: Vec3) -> Transform {
    let rotation = Quat::from_mat3(&Mat3::from_cols(rows[0], rows[1], rows[2]));
    Transform {
        translation: origin + CANOPY_PIVOT - rotation * CANOPY_PIVOT,
        rotation,
        scale: Vec3::ONE,
    }
}

/// The canopy entity's working state.
#[derive(Component, Default)]
pub struct CanopyRig {
    /// The retail model, requested on the first frame the rig exists.
    model: Option<Handle<RbmAsset>>,
    /// The skeleton's joint entities (bone order) once the model is attached.
    joints: Vec<Entity>,
    attached: bool,
    /// The `PCS_*` state machine and visibility.
    pub machine: CanopyMachine,
    /// This frame's request from the hero (written by `character::hero_visuals`).
    pub command: CanopyCommand,
    scratch: Vec<QsTransform>,
    /// Time of the idle blend clip.
    idle_time: f32,
    /// Rico's hand and `Spine1` poses in the canopy's frame for this frame (written by
    /// `character::hero_visuals`); `None` leaves the clip pose alone.
    pub attach: Option<AttachPoses>,
}

/// Rico's `AttachHandRight` (bone `0x19`), `AttachHandLeft` (`0x18`) and `Spine1` (bone 3)
/// poses in the canopy model's frame (both skeletons share the character's world matrix,
/// `parachute-attach-pivot.md` 3), and which hands hold their handle.
#[derive(Clone, Copy, Debug)]
pub struct AttachPoses {
    pub right_hand: Transform,
    pub left_hand: Transform,
    pub spine1: Transform,
    /// `[left, right]`: which hands keep their handle, from [`hold_flags`].
    pub holding: [bool; 2],
}

/// The inputs of the native hold predicates (`0x00657010`, `parachute-attach-pivot.md` 6).
#[derive(Clone, Copy, Debug, Default)]
pub struct HoldInputs {
    /// Weapon holder draw state `H + 0x64 != 0` (`Arsenal::drawn`).
    pub drawn: bool,
    /// A change is pending (`0x00848690`: `+0x64`, `+0x70`, `+0x74` not all equal). The
    /// parachute flow is believed never to leave one pending (gap 2), the runtime passes false.
    pub pending: bool,
    /// The drawn weapon is holder state 3, the two-handed class (`0x0073bcf0`). INFERRED
    /// mapping: every class that is not one-handed (rifle, shotgun, rocket launcher).
    pub two_handed: bool,
    /// A reload runs (`0x00635840` / `0x00618690`).
    pub reloading: bool,
}

/// `[left, right]` hold flags, the truth table of section 6. The upper-body layer is "calm"
/// (right hand of a one-hand weapon) when no reload runs: INFERRED, the other non-calm `UB_*`
/// states (firing) are not mapped yet (gap 1 of that document). Layer 2 (`LA_*`, left arm)
/// is only entered by dual wield, which the runtime does not have.
pub fn hold_flags(i: HoldInputs) -> [bool; 2] {
    if i.pending {
        return [false, false];
    }
    if !i.drawn {
        return [true, true];
    }
    if i.two_handed {
        return [!i.reloading, false];
    }
    [!i.reloading, !i.reloading]
}

/// Stowed handle offsets from `rgt_1` / `lft_1` (floats at `0x00e4abac`, `0x00e50cc0`,
/// `0x00e3fde0`; `parachute-attach-pivot.md` 3).
pub const STOWED_RIGHT_OFFSET: Vec3 = Vec3::new(-0.05, -0.4, -0.05);
pub const STOWED_LEFT_OFFSET: Vec3 = Vec3::new(0.05, -0.4, -0.05);
/// Stowed handle rotation quaternions (`0x00e3ac3c` / `0x00e3b2fc`, `0x00e4a498`).
pub const STOWED_RIGHT_ROT: Quat = Quat::from_xyzw(0.0, -0.24254, 0.0, 0.97014);
pub const STOWED_LEFT_ROT: Quat = Quat::from_xyzw(0.0, 0.24254, 0.0, 0.97014);

/// Rico's attach poses in the canopy frame. `rico_local` are his bones' local transforms,
/// `body` the transform of his model and `canopy` that of the canopy, both relative to the
/// hero. Natively the two models share one world matrix, so the pose passes as is; the
/// runtime turns its body and canopy separately, hence the change of frame.
pub fn attach_from_rico(
    skeleton: &Skeleton,
    rico_local: &[Transform],
    body: &Transform,
    canopy: &Transform,
    holding: [bool; 2],
) -> Option<AttachPoses> {
    let global = crate::rig::model_space(skeleton, rico_local);
    let to_canopy = canopy.to_matrix().inverse() * body.to_matrix();
    let pose = |name: &str| -> Option<Transform> {
        let g = global.get(skeleton.bone_index(name)?)?;
        Some(Transform::from_matrix(to_canopy * g.to_matrix()))
    };
    Some(AttachPoses {
        right_hand: pose("AttachHandRight")?,
        left_hand: pose("AttachHandLeft")?,
        spine1: pose("Spine1")?,
        holding,
    })
}

/// Set bone `b`'s model-space pose to `target`, keeping its parent chain.
fn set_global(skeleton: &Skeleton, local: &mut [Transform], b: usize, target: Transform) {
    let global = crate::rig::model_space(skeleton, local);
    let parent = match skeleton.bones[b].parent {
        Some(p) if p < b => global[p].to_matrix(),
        _ => Mat4::IDENTITY,
    };
    local[b] = Transform::from_matrix(parent.inverse() * target.to_matrix());
}

/// The per-frame attach `0x00657010` on the canopy's bone locals: `Para_RightHandle` and
/// `Para_LeftHandle` take Rico's hand poses (or, when the hand does not hold, the `rgt_1` /
/// `lft_1` pose moved by the stowed offset and turned by the stowed quaternion);
/// `HarnessExt` takes `Spine1`'s translation and rotation and keeps the canopy bone's own
/// scale. Poses are model space (INFERRED, consistent with every number; whether the native
/// store is an absolute override is unproven, gap 2). Call only while the canopy is open
/// or opening.
pub fn apply_attach(skeleton: &Skeleton, local: &mut [Transform], poses: &AttachPoses) {
    let bone = |n: &str| skeleton.bone_index(n);
    let (Some(rh), Some(lh), Some(r1), Some(l1), Some(harness)) = (
        bone("Para_RightHandle"),
        bone("Para_LeftHandle"),
        bone("rgt_1"),
        bone("lft_1"),
        bone("HarnessExt"),
    ) else {
        return;
    };
    let [hold_left, hold_right] = poses.holding;
    // the stowed spot reads the canopy bones before anything is moved
    let global = crate::rig::model_space(skeleton, local);
    let stowed = |b: usize, offset: Vec3, rotation: Quat| Transform {
        translation: global[b].translation + offset,
        rotation,
        scale: global[b].scale,
    };
    let right = if hold_right {
        poses.right_hand
    } else {
        stowed(r1, STOWED_RIGHT_OFFSET, STOWED_RIGHT_ROT)
    };
    let left = if hold_left {
        poses.left_hand
    } else {
        stowed(l1, STOWED_LEFT_OFFSET, STOWED_LEFT_ROT)
    };
    let own_scale = global[harness].scale;
    set_global(skeleton, local, rh, right);
    set_global(skeleton, local, lh, left);
    set_global(
        skeleton,
        local,
        harness,
        Transform {
            scale: own_scale,
            ..poses.spine1
        },
    );
}

/// Spawn the canopy (hidden) under `parent`, at the model's origin. The skinned model and
/// its joints are attached by `attach_canopy_model` once the asset has loaded.
pub fn spawn_canopy(
    commands: &mut Commands,
    parent: Entity,
    _rig: &ChuteRig,
    _meshes: &mut Assets<Mesh>,
    _materials: &mut Assets<StandardMaterial>,
    origin: Vec3,
) -> Entity {
    let canopy = commands
        .spawn((
            Transform::from_translation(origin),
            Visibility::Hidden,
            crate::character::Canopy,
            CanopyRig::default(),
            Name::new("parachute"),
        ))
        .id();
    commands.entity(parent).add_child(canopy);
    canopy
}

/// Load the retail canopy model and, when it is ready, hang the 17 bone joints (reference
/// pose, inverse bind = its inverse) and the skinned blocks under each canopy.
pub fn attach_canopy_model(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    rig: Option<Res<ChuteRig>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut canopies: Query<(Entity, &mut CanopyRig)>,
) {
    let Some(rig) = rig else { return };
    for (entity, mut canopy) in &mut canopies {
        if canopy.attached {
            continue;
        }
        let handle = canopy
            .model
            .get_or_insert_with(|| assets.load(CANOPY_MODEL))
            .clone();
        let Some(rbm) = rbms.get(&handle) else {
            if matches!(
                assets.get_load_state(handle.id()),
                Some(bevy::asset::LoadState::Failed(_))
            ) {
                warn!("parachute: {CANOPY_MODEL} failed to load; no canopy is drawn");
                canopy.attached = true;
            }
            continue;
        };
        let (joints, bind) =
            crate::rig::spawn_joints(&mut commands, entity, &rig.skeleton, &mut bindposes);
        for b in &rbm.blocks {
            let mut mesh = commands.spawn((
                Mesh3d(b.mesh.clone()),
                b.material_component(),
                Transform::IDENTITY,
                Name::new("canopy"),
                ChildOf(entity),
            ));
            if b.skinned {
                mesh.insert(crate::rig::skin(&joints, &bind));
            }
            // The canopy casts into the sun shadow pass like any animated model (INFERRED from
            // the shared loader, parachute-canopy-shading.md "Shadow submission"; gap 1 of that
            // doc is open), so it is on by default behind this switch.
            if std::env::var_os("JC2_CANOPY_NO_SHADOW").is_some() {
                mesh.insert(bevy::light::NotShadowCaster);
            }
        }
        canopy.joints = joints;
        canopy.attached = true;
    }
}

/// Step the canopy state machine, set the model's visibility from it and keep the joints on
/// the clip of the current state (opening, the five-clip blend while open, release).
pub fn animate_canopy(
    time: Res<Time>,
    rig: Option<Res<ChuteRig>>,
    mut canopies: Query<(&mut CanopyRig, &mut Visibility)>,
    mut joints: Query<&mut Transform>,
    feed: Option<Res<CanopyPoseFeed>>,
) {
    let Some(rig) = rig else { return };
    let dt = time.delta_secs();
    let lengths = rig.clip_lengths();
    for (mut canopy, mut vis) in &mut canopies {
        let cmd = canopy.command;
        canopy.machine.step(dt, cmd, lengths);
        let want = if canopy.machine.shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if !canopy.machine.shown {
            continue;
        }
        let t = canopy.machine.t;
        // once open, the five-clip blend (native weights and times of `CParachuteBlendState`,
        // driven by the flight model's pose inputs)
        let blended = match feed.as_ref() {
            Some(feed) if canopy.machine.state == CanopyState::Open => {
                let durations = rig.blend_durations();
                // blend (weights, times) -> step (+dt * rate) -> sample (landing doc 11.2)
                let mut clips = crate::parachute_flight::blend_update(
                    &feed.0,
                    durations,
                    canopy.idle_time,
                    dt,
                );
                step_blend(&mut clips, durations, dt);
                // the idle clip's post-step time is what the next blend reads
                canopy.idle_time = clips[4].time;
                Some(clips)
            }
            _ => None,
        };
        let CanopyRig {
            joints: joint_list,
            scratch,
            machine,
            attach,
            ..
        } = &mut *canopy;
        if joint_list.is_empty() {
            continue;
        }
        let local = blended
            .and_then(|c| rig.blended_locals(&c, scratch))
            .unwrap_or_else(|| match machine.state {
                CanopyState::OpeningHorizontal => {
                    let clip = rig.horizontal.as_ref().map(|(a, b)| (a, b.as_slice()));
                    rig.clip_locals(clip, Some(t), scratch)
                }
                CanopyState::Closing => {
                    let clip = rig.release.as_ref().map(|(a, b)| (a, b.as_slice()));
                    rig.clip_locals(clip, Some(t), scratch)
                }
                // vertical opening, and the open pose when no blend is available (the clip's
                // last frame)
                _ => {
                    let at = if machine.state == CanopyState::Open {
                        f32::MAX
                    } else {
                        t
                    };
                    rig.pose_locals(rig.open.as_ref().map(|_| at), scratch)
                }
            });
        let mut local = local;
        if let Some(poses) = attach
            && matches!(
                machine.state,
                CanopyState::OpeningVertical | CanopyState::OpeningHorizontal | CanopyState::Open
            )
        {
            apply_attach(&rig.skeleton, &mut local, poses);
        }
        for (&e, tf) in joint_list.iter().zip(&local) {
            if let Ok(mut j) = joints.get_mut(e) {
                *j = *tf;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canopy entity's forward axis (-Z) is the native `A = -R2` of the basis, its up
    /// is `R1`, for the yaw-only, braking, diving and banking poses.
    #[test]
    fn canopy_orientation_follows_flight_basis() {
        let origin = Vec3::new(0.0, -0.9, 0.0);
        for (a30, a38, a34) in [
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 1.2),
            (0.65, 0.0, 0.4),
            (-0.65, 0.0, -2.0),
            (0.2, -0.65, 0.7),
            (-0.3, 0.65, 3.0),
        ] {
            let mut f = crate::parachute_flight::CanopyFlight::default();
            (f.a30, f.a38, f.a34) = (a30, a38, a34);
            let m = f.matrix();
            let t = canopy_orientation(&m, origin);
            assert!((t.rotation * Vec3::NEG_Z - (-m[2])).length() < 1e-5);
            assert!((t.rotation * Vec3::Y - m[1]).length() < 1e-5);
            assert!((t.rotation * Vec3::X - m[0]).length() < 1e-5);
            assert!((t.translation - origin).length() < 1e-6, "pivot at the origin");
        }
        // yaw only equals the old behaviour
        let mut f = crate::parachute_flight::CanopyFlight::default();
        f.a34 = 0.8;
        let t = canopy_orientation(&f.matrix(), Vec3::ZERO);
        assert!((t.rotation * Vec3::X - Quat::from_rotation_y(0.8) * Vec3::X).length() < 1e-5);
    }

    fn skel(names: &[(&str, Option<usize>)]) -> Skeleton {
        Skeleton {
            name: "t".into(),
            bones: names
                .iter()
                .map(|(n, p)| jc2_formats::animation::Bone {
                    name: (*n).into(),
                    parent: *p,
                })
                .collect(),
            reference_pose: vec![QsTransform::IDENTITY; names.len()],
        }
    }

    fn at(x: f32, y: f32, z: f32) -> Transform {
        Transform::from_xyz(x, y, z)
    }

    fn canopy_skel() -> Skeleton {
        skel(&[
            ("Reference", None),
            ("base", Some(0)),
            ("HarnessExt", Some(1)),
            ("rgt_1", Some(1)),
            ("lft_1", Some(1)),
            ("Para_RightHandle", Some(3)),
            ("Para_LeftHandle", Some(4)),
        ])
    }

    fn global_of(sk: &Skeleton, local: &[Transform], name: &str) -> Transform {
        crate::rig::model_space(sk, local)[sk.bone_index(name).unwrap()]
    }

    /// Holding: handles equal the hand poses, `HarnessExt` equals `Spine1` with its own scale.
    #[test]
    fn attach_sets_handles_and_harness() {
        let sk = canopy_skel();
        let mut local = vec![Transform::IDENTITY; sk.bones.len()];
        local[1] = at(0.0, 1.227, 0.0);
        local[2].scale = Vec3::splat(2.0);
        local[3] = at(0.2, 0.3, 0.0);
        let poses = AttachPoses {
            right_hand: Transform {
                translation: Vec3::new(0.3, 1.1, -0.2),
                rotation: Quat::from_rotation_x(0.7),
                scale: Vec3::ONE,
            },
            left_hand: at(-0.3, 1.1, -0.2),
            spine1: Transform {
                translation: Vec3::new(0.0, 1.228, 0.05),
                rotation: Quat::from_rotation_y(0.4),
                scale: Vec3::ONE,
            },
            holding: [true, true],
        };
        apply_attach(&sk, &mut local, &poses);
        let h = global_of(&sk, &local, "HarnessExt");
        assert!((h.translation - poses.spine1.translation).length() < 1e-5);
        assert!(h.rotation.angle_between(poses.spine1.rotation) < 2e-3);
        assert!((h.scale - Vec3::splat(2.0)).length() < 1e-4, "own scale kept");
        let r = global_of(&sk, &local, "Para_RightHandle");
        assert!((r.translation - poses.right_hand.translation).length() < 1e-5);
        assert!(r.rotation.angle_between(poses.right_hand.rotation) < 2e-3);
        let l = global_of(&sk, &local, "Para_LeftHandle");
        assert!((l.translation - poses.left_hand.translation).length() < 1e-5);
    }

    /// Not holding: `rgt_1` / `lft_1` pose plus the stowed offset and quaternion.
    #[test]
    fn attach_stowed_fallback() {
        let sk = canopy_skel();
        let mut local = vec![Transform::IDENTITY; sk.bones.len()];
        local[1] = at(0.0, 1.227, 0.0);
        local[3] = at(0.2, 0.3, 0.0);
        local[4] = at(-0.2, 0.3, 0.0);
        let poses = AttachPoses {
            right_hand: at(9.0, 9.0, 9.0),
            left_hand: at(9.0, 9.0, 9.0),
            spine1: at(0.0, 1.2, 0.0),
            holding: [false, false],
        };
        apply_attach(&sk, &mut local, &poses);
        let r = global_of(&sk, &local, "Para_RightHandle");
        assert!((r.translation - Vec3::new(0.15, 1.127, -0.05)).length() < 1e-5, "{r:?}");
        assert!(r.rotation.dot(STOWED_RIGHT_ROT.normalize()).abs() > 0.99999);
        let l = global_of(&sk, &local, "Para_LeftHandle");
        assert!((l.translation - Vec3::new(-0.15, 1.127, -0.05)).length() < 1e-5, "{l:?}");
        assert!(l.rotation.dot(STOWED_LEFT_ROT.normalize()).abs() > 0.99999);
    }

    /// The truth table of `parachute-attach-pivot.md` 6.
    #[test]
    fn hold_flags_follow_table() {
        let f = |drawn, pending, two_handed, reloading| {
            hold_flags(HoldInputs { drawn, pending, two_handed, reloading })
        };
        assert_eq!(f(false, false, false, false), [true, true], "holstered");
        assert_eq!(f(false, false, true, true), [true, true], "holstered ignores the rest");
        assert_eq!(f(true, true, false, false), [false, false], "change pending");
        assert_eq!(f(true, false, false, false), [true, true], "steady pistol");
        assert_eq!(f(true, false, false, true), [false, false], "pistol reload");
        assert_eq!(f(true, false, true, false), [true, false], "two-handed: left only");
        assert_eq!(f(true, false, true, true), [false, false], "two-handed reload");
    }

    /// Rico's poses pass through the change of frame between his body and the canopy.
    #[test]
    fn attach_from_rico_changes_frame() {
        let rico = skel(&[
            ("Reference", None),
            ("Spine1", Some(0)),
            ("AttachHandRight", Some(1)),
            ("AttachHandLeft", Some(1)),
        ]);
        let mut local = vec![Transform::IDENTITY; 4];
        local[1] = at(0.0, 1.2, 0.0);
        local[2] = at(0.4, 0.0, 0.0);
        local[3] = at(-0.4, 0.0, 0.0);
        // identical frames: the model-space pose as is
        let same = attach_from_rico(&rico, &local, &Transform::IDENTITY, &Transform::IDENTITY, [true; 2])
            .unwrap();
        assert!((same.spine1.translation - Vec3::new(0.0, 1.2, 0.0)).length() < 1e-5);
        assert!((same.right_hand.translation - Vec3::new(0.4, 1.2, 0.0)).length() < 1e-5);
        // body yawed a quarter turn, canopy not: the canopy sees the hand turned
        let body = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let turned = attach_from_rico(&rico, &local, &body, &Transform::IDENTITY, [true; 2]).unwrap();
        assert!((turned.right_hand.translation - Vec3::new(0.0, 1.2, -0.4)).length() < 1e-5);
        assert!(attach_from_rico(&canopy_skel(), &local, &body, &body, [true; 2]).is_none());
    }

    /// Needs JC2_TEST_DIR with `parachute.bsk` and `chute_open_chute.ban` in the dropzone.
    #[test]
    fn canopy_unfolds() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        if !fs.contains("parachute.bsk") {
            return;
        }
        let rig = ChuteRig::load(&fs).expect("parachute skeleton");
        assert_eq!(rig.skeleton.bones.len(), 17);
        let mut scratch = Vec::new();
        let tip = |p: &str| rig.skeleton.bone_index(&format!("{p}_4")).unwrap();
        let chains = [tip("lft"), tip("mid"), tip("rgt")];
        let positions = |t: Option<f32>, scratch: &mut Vec<QsTransform>| {
            rig.globals(&rig.pose_locals(t, scratch))
        };
        let reference = positions(None, &mut scratch);
        let (l, m, r) = (reference[chains[0]], reference[chains[1]], reference[chains[2]]);
        assert!(
            (l.x + 1.92).abs() < 0.05 && (l.y - 4.47).abs() < 0.05,
            "{l:?}"
        );
        assert!(m.x.abs() < 0.05 && (m.y - 4.9).abs() < 0.05, "{m:?}");
        assert!((r.x - 1.93).abs() < 0.05, "{r:?}");
        // the five blend clips load and scrub to a finite pose (lengths: 1.0 s x4, idle 1.333 s)
        if rig.blend.iter().all(Option::is_some) {
            let d = rig.blend_durations();
            assert!((d[0] - 1.0).abs() < 0.05 && (d[4] - 1.333).abs() < 0.05, "{d:?}");
            let clips = crate::parachute_flight::blend_update(
                &crate::parachute_flight::CanopyPose::default(),
                d,
                0.0,
                1.0 / 60.0,
            );
            let p = rig.globals(&rig.blended_locals(&clips, &mut scratch).expect("blend"));
            assert!(p.iter().all(|v| v.is_finite()));
        }
        if let Some(open) = &rig.open {
            let start = positions(Some(0.0), &mut scratch);
            let end = positions(Some(open.duration), &mut scratch);
            let span = |p: &[Vec3]| (p[chains[2]] - p[chains[0]]).length();
            eprintln!(
                "canopy span: folded {:.2} m, open {:.2} m over {:.2} s",
                span(&start),
                span(&end),
                open.duration
            );
            assert!(span(&end) > 3.0, "open span {}", span(&end));
            assert!(span(&start) < span(&end), "the clip should unfold it");
        }
    }

    #[test]
    fn step_matches_probe_fixtures() {
        use crate::parachute_flight::ClipBlend;
        let dt = 1.0 / 60.0;
        let mk = |t: f32, rate: Option<f32>| ClipBlend { weight: 1.0, time: t, rate };
        let mut clips = [mk(0.5, None), mk(1.0, None), mk(0.0, None), mk(0.25, None), mk(-0.003333, Some(1.85))];
        step_blend(&mut clips, [1.0, 1.0, 1.0, 1.0, 1.3333], dt);
        assert!((clips[0].time - 0.516667).abs() < 1e-5);
        assert!((clips[1].time - 0.016667).abs() < 1e-5);
        assert!((clips[4].time - 0.030833).abs() < 1e-5, "{}", clips[4].time);
    }

    #[test]
    fn additive_matches_probe_fixtures() {
        let base = Transform {
            translation: Vec3::X,
            rotation: Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let delta = Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let full = additive_blend(&base, &delta, 1.0);
        assert!((full.translation - Vec3::Y).length() < 1e-5, "{:?}", full.translation);
        assert!(full.rotation.abs_diff_eq(Quat::from_xyzw(0.5, 0.5, 0.5, 0.5), 1e-5), "{:?}", full.rotation);
        let half = additive_blend(&base, &delta, 0.5);
        assert!((half.translation - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-5);
        assert!(half.rotation.abs_diff_eq(Quat::from_xyzw(0.65328, 0.27060, 0.27060, 0.65328), 1e-4), "{:?}", half.rotation);
        assert!((half.scale - Vec3::splat(1.5)).length() < 1e-5);
    }

    fn run(m: &mut CanopyMachine, secs: f32, cmd: CanopyCommand) {
        let dt = 1.0 / 120.0;
        for _ in 0..(secs / dt).round() as usize {
            m.step(dt, cmd, ClipLengths::NATIVE);
        }
    }

    const UP: CanopyCommand = CanopyCommand {
        up: true,
        horizontal: false,
        cut_away: false,
    };

    /// Probe model (`parachute-deploy-collapse.md` 4): vertical deploy shows at 0.33 s.
    #[test]
    fn vertical_deploy_shows_at_033() {
        let mut m = CanopyMachine::default();
        run(&mut m, 0.01, UP);
        assert_eq!(m.state, CanopyState::OpeningVertical);
        run(&mut m, 0.30, UP);
        assert!(!m.shown, "hidden before 0.33 s");
        run(&mut m, 0.05, UP);
        assert!(m.shown, "shown after 0.33 s");
        run(&mut m, 2.2, UP);
        assert_eq!(m.state, CanopyState::Open);
        assert!(m.shown);
    }

    #[test]
    fn horizontal_deploy_shows_at_003() {
        let mut m = CanopyMachine::default();
        let cmd = CanopyCommand {
            horizontal: true,
            ..UP
        };
        run(&mut m, 0.01, cmd);
        assert_eq!(m.state, CanopyState::OpeningHorizontal);
        assert!(!m.shown);
        run(&mut m, 0.04, cmd);
        assert!(m.shown);
        run(&mut m, 2.0, cmd);
        assert_eq!(m.state, CanopyState::Open);
    }

    /// Landing close: visible while `parachute_release` plays (1.4 s), then hidden.
    #[test]
    fn landing_close_is_visible_for_the_release_clip() {
        let mut m = CanopyMachine::default();
        run(&mut m, 3.0, UP);
        assert_eq!(m.state, CanopyState::Open);
        let down = CanopyCommand::default();
        run(&mut m, 1.3, down);
        assert_eq!(m.state, CanopyState::Closing);
        assert!(m.shown, "still drawn 1.3 s after touchdown");
        run(&mut m, 0.15, down);
        assert_eq!(m.state, CanopyState::Closed);
        assert!(!m.shown);
    }

    /// Cut-away: hidden at 0.5 s (`hide_parachute`), before the clip ends.
    #[test]
    fn cut_away_hides_at_half_a_second() {
        let mut m = CanopyMachine::default();
        run(&mut m, 3.0, UP);
        let cut = CanopyCommand {
            cut_away: true,
            ..Default::default()
        };
        run(&mut m, 0.45, cut);
        assert!(m.shown);
        run(&mut m, 0.1, cut);
        assert!(!m.shown);
        assert_eq!(m.state, CanopyState::Closing);
    }

    /// An open command during the close is ignored until `PCS_CLOSED`.
    #[test]
    fn reopen_waits_for_closed() {
        let mut m = CanopyMachine::default();
        run(&mut m, 3.0, UP);
        run(&mut m, 0.2, CanopyCommand::default());
        run(&mut m, 0.2, UP);
        assert_eq!(m.state, CanopyState::Closing);
    }
}
