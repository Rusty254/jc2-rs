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
//! JC2-141, `research/formats/camera-fov-next.md`), and `f = max(fov^2, 0.38)` has no upper
//! cap. The native on-foot FOV is 40 degrees (`f = 0.487`: foliage fade 112.8..167.8 m); this
//! project's camera still uses Bevy's default 45 degrees (`f = 0.617`: 89..144 m) until the
//! camera-FOV runtime task lands, so the distances here follow the actual `Projection` each
//! frame (non-native until then).
//!
//! Non-native, labelled: the `/lodfactor` graphics setting is not exposed in the project: it is
//! fixed at the native default 0 ([`LODFACTOR`]).

use bevy::prelude::*;
use jc2_formats::veglod;

/// Native default of the `/lodfactor` setting (`DAT_0108b924`, "Geometric detail 0-3,
/// default 0"; JC2-126). Not exposed as a project option.
pub const LODFACTOR: u32 = 0;

/// The fade constants for the current camera: `foliage.xy` the foliage distance fade
/// `1 - sat(dist * y - x)`, `top` / `trunk` the impostor `cb1[1]` vectors. `foliage.z` is 1 when
/// active (the world viewer), 0 for the model viewer and fixtures, which keep full foliage.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct VegetationFade {
    /// Camera vertical FOV in radians used for `f` (the project camera's, see the module docs).
    pub fov: f32,
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
/// `foliage_fade_end_max` = 199.7 m away at any FOV); they are only hidden beyond that plus
/// [`CULL_SLACK`], where the shader would discard every fragment anyway, which keeps the
/// vertex work of far plants down without changing the picture. Other meshes keep the
/// project's `far` range.
pub fn plant_visibility_range(
    foliage: bool,
    far: Option<(f32, f32)>,
) -> Option<bevy::camera::visibility::VisibilityRange> {
    let end = if foliage {
        let at = veglod::foliage_fade_end_max(&veglod::engine_table(LODFACTOR)) + CULL_SLACK;
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

pub struct VegetationFadePlugin;

impl Plugin for VegetationFadePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VegetationFade>()
            .add_systems(First, update_fade_from_camera);
    }
}

fn update_fade_from_camera(
    cams: Query<&Projection, With<Camera3d>>,
    mut fade: ResMut<VegetationFade>,
) {
    let Some(fov) = cams.iter().find_map(|p| match p {
        Projection::Perspective(p) => Some(p.fov),
        _ => None,
    }) else {
        return;
    };
    if fov != fade.fov {
        *fade = VegetationFade::new(LODFACTOR, fov);
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
}
