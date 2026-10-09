//! Native vegetation fade constants (JC2-140), shared by the foliage materials and the tree
//! impostor cards.
//!
//! The original writes eight distances into the render engine every frame (`00a92ca0`) and its
//! impostor/foliage pass setup turns them and the camera FOV into vertex-shader constants
//! (`00a83740`, `00a843a0`); `jc2_formats::veglod` holds those pure functions and
//! `research/formats/tree-impostors-next.md` the evidence. Here one resource holds the three
//! constant vectors and the materials that consume them are updated when they change (the
//! constants depend only on the graphics setting and the camera FOV, so that is rarely).
//!
//! The FOV is the camera's realised vertical FOV (`camera+0x618`, vertical axis proven by
//! JC2-141, `research/formats/camera-fov-next.md`): [`crate::gameplay_camera::CameraFov`]
//! when the gameplay camera drives it (40 degrees on foot, 20/15 aim, 50 vehicles, ...),
//! else the camera's `Projection`; `f = max(fov^2, 0.38)` has no upper cap. The constants
//! are recomputed after the camera system each frame whenever the FOV or the lodfactor
//! changed (JC2-148).
//!
//! The graphics setting `/lodfactor` (0x17, "Geometric detail 0-3", native default 0) is
//! the [`LodFactor`] resource (`--lodfactor`): it selects the `d2c`/`d34` distances
//! (`veglod::engine_table`) and, in `scenery.rs`, the model-LOD distance unit 1 / 1.5 / 2 / 3
//! (`veglod::distance_unit`). The patch radius 7 / 11 / 13 (`DAT_0108b8fb`) that the same
//! setting changes is not understood and not applied (Gap).

use bevy::prelude::*;
use jc2_formats::veglod;

/// Native default of the `/lodfactor` setting (`DAT_0108b924`, "Geometric detail 0-3,
/// default 0"; JC2-126).
pub const LODFACTOR: u32 = 0;

/// The graphics setting `/lodfactor` 0..=3 (native id 0x17, default 0). Read by the vegetation
/// fade constants and the plant model-LOD selection; set once at start from `--lodfactor`.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodFactor(pub u32);

impl LodFactor {
    /// Values above the documented 0-3 range are clamped to 3.
    pub fn new(level: u32) -> Self {
        Self(level.min(3))
    }
}

impl Default for LodFactor {
    fn default() -> Self {
        Self(LODFACTOR)
    }
}

/// The fade constants for the current camera: `foliage.xy` the foliage distance fade
/// `1 - sat(dist * y - x)`, `top` / `trunk` the impostor `cb1[1]` vectors. `foliage.z` is 1 when
/// active (the world viewer), 0 for the model viewer and fixtures, which keep full foliage.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct VegetationFade {
    /// Camera vertical FOV in radians used for `f` (the project camera's, see the module docs).
    pub fov: f32,
    /// The `/lodfactor` the constants were built for.
    pub lodfactor: u32,
    pub foliage: Vec4,
    pub top: Vec4,
    pub trunk: Vec4,
}

impl VegetationFade {
    /// The native constants for `lodfactor` at camera FOV `fov` (radians).
    pub fn new(lodfactor: u32, fov: f32) -> Self {
        let table = veglod::engine_table(lodfactor);
        let f = veglod::fov_factor(fov);
        let [fx, fy] = veglod::foliage_constants(&table, f);
        Self {
            fov,
            lodfactor,
            foliage: Vec4::new(fx, fy, 1.0, 0.0),
            top: Vec4::from(veglod::top_constants(&table, f)),
            trunk: Vec4::from(veglod::trunk_constants(&table, f)),
        }
    }
}

impl Default for VegetationFade {
    fn default() -> Self {
        Self::new(LODFACTOR, PerspectiveProjection::default().fov)
    }
}

/// Slack (m) added to [`veglod::foliage_fade_end_max`] for the extent of a plant model: the
/// native fade measures the vertex height, the entity origin is on the ground (project bound).
const CULL_SLACK: f32 = 50.0;

/// CPU-side visibility of a mesh of a streamed plant (project, not native). Foliage-material
/// meshes get no `TREE_RANGE` / `BUSH_RANGE` hand-over (their native fade ends at most
/// `foliage_fade_end_max` = 199.7 m away at any FOV for lodfactor 0); they are only hidden beyond that plus
/// [`CULL_SLACK`], where the shader would discard every fragment anyway, which keeps the
/// vertex work of far plants down without changing the picture. Other meshes keep the
/// project's `far` range.
pub fn plant_visibility_range(
    foliage: bool,
    far: Option<(f32, f32)>,
    lodfactor: u32,
) -> Option<bevy::camera::visibility::VisibilityRange> {
    let end = if foliage {
        let at = veglod::foliage_fade_end_max(&veglod::engine_table(lodfactor)) + CULL_SLACK;
        (at, at + 1.0)
    } else {
        far?
    };
    Some(bevy::camera::visibility::VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: end.0..end.1,
        use_aabb: false,
    })
}

/// The vegetation fade constants are updated in this set, after the gameplay camera has
/// realised this frame's FOV; the impostor card system runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct VegetationFadeSet;

pub struct VegetationFadePlugin;

impl Plugin for VegetationFadePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VegetationFade>()
            .init_resource::<LodFactor>()
            .add_systems(
                Update,
                update_fade_from_camera
                    .in_set(VegetationFadeSet)
                    .after(crate::gameplay_camera::drive_fov),
            );
    }
}

fn update_fade_from_camera(
    cams: Query<&Projection, With<Camera3d>>,
    camera_fov: Option<Res<crate::gameplay_camera::CameraFov>>,
    lodfactor: Res<LodFactor>,
    mut fade: ResMut<VegetationFade>,
) {
    let Some(fov) = crate::gameplay_camera::realised_fov(
        camera_fov.as_deref(),
        cams.iter().next(),
    ) else {
        return;
    };
    if fov != fade.fov || lodfactor.0 != fade.lodfactor {
        *fade = VegetationFade::new(lodfactor.0, fov);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_follow_the_pure_functions() {
        // f = 1 at a 1 rad FOV: foliage full to 55 m, gone at 110 m
        let f = VegetationFade::new(0, 1.0);
        assert_eq!(f.foliage.truncate().truncate(), Vec2::new(1.0, 1.0 / 55.0));
        assert_eq!(f.foliage.z, 1.0);
        assert_eq!(f.top.w, 1.0 / 350.0);
        // the project camera's 45 degrees: f = fov^2 = 0.617, ranges stretch by 1/f
        let f = VegetationFade::new(0, std::f32::consts::FRAC_PI_4);
        let want = 55.0 / (std::f32::consts::FRAC_PI_4 * std::f32::consts::FRAC_PI_4);
        assert!((f.foliage.x / f.foliage.y - want).abs() < 0.01);
    }

    #[test]
    fn native_on_foot_fov_gives_the_jc2_141_ranges() {
        let f = VegetationFade::new(0, 40.0_f32 * 0.017_453_292);
        let start = f.foliage.x / f.foliage.y;
        assert!((start - 112.84).abs() < 0.05, "{start}");
        assert!((start + 55.0 - 167.84).abs() < 0.05);
    }

    #[test]
    fn the_camera_fov_drives_the_resource() {
        let mut app = App::new();
        app.add_plugins(VegetationFadePlugin);
        app.world_mut().spawn((
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: 1.2,
                ..default()
            }),
        ));
        app.update();
        assert_eq!(*app.world().resource::<VegetationFade>(), VegetationFade::new(0, 1.2));
    }

    /// JC2-148: the fade start (`d2c / f`) at each mode's realised FOV, with no cap on `f`;
    /// `CameraFov` wins over the projection, and the lodfactor setting is picked up.
    #[test]
    fn realised_fov_and_lodfactor_drive_the_fade() {
        use crate::gameplay_camera::CameraFov;
        let rad = |deg: f32| deg * 0.017_453_292;
        let start = |f: &VegetationFade| f.foliage.x / f.foliage.y;
        let mut app = App::new();
        app.add_plugins(VegetationFadePlugin);
        app.world_mut().spawn((Camera3d::default(), Projection::Perspective(default())));
        // (degrees, start of the foliage fade at lodfactor 0): on foot, aim 20/15 (floor
        // 0.38), vehicle, free camera, and a 90 degree FOV beyond f = 1
        for (deg, want) in [
            (40.0, 112.84),
            (20.0, 144.74),
            (15.0, 144.74),
            (50.0, 72.22),
            (45.0, 89.16),
            (90.0, 22.29),
        ] {
            app.insert_resource(CameraFov { fov_y: rad(deg) });
            app.update();
            let f = *app.world().resource::<VegetationFade>();
            assert!((start(&f) - want).abs() < 0.06, "{deg}: {}", start(&f));
            assert!((1.0 / f.foliage.y - 55.0).abs() < 1e-3);
        }
        // lodfactor 2 and 3 start at 119 / f and 183 / f
        app.insert_resource(CameraFov { fov_y: rad(40.0) });
        for (lod, want) in [(1, 112.84), (2, 119.0 / 0.487_387_84), (3, 183.0 / 0.487_387_84)] {
            app.insert_resource(LodFactor::new(lod));
            app.update();
            let f = *app.world().resource::<VegetationFade>();
            assert_eq!(f.lodfactor, lod);
            assert!((start(&f) - want).abs() < 0.06, "{lod}: {}", start(&f));
        }
        assert_eq!(LodFactor::new(7), LodFactor(3));
        assert_eq!(LodFactor::default(), LodFactor(0));
    }
}
