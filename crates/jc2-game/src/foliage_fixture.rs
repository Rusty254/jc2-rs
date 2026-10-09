//! Diagnostic exact-half alpha panels. No original distance scheduler is synthesized.
use crate::foliage_alpha::{FoliageInstance, FoliageMaterial, source_material};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use jc2_formats::foliage::FoliageAlpha;

#[derive(Resource)]
pub struct FixtureConfig {
    pub depth: bool,
}
pub struct FoliageFixturePlugin;
impl Plugin for FoliageFixturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, reset_and_reuse);
    }
}
#[derive(Component)]
struct ReusePanel {
    phase: u8,
}

fn exact_image(alpha: u16) -> Image {
    // .25 is exactly half 0x3400. Adjacent halves deliberately straddle equality;
    // converting these to 8-bit UNORM would invalidate this fixture.
    let pixel = [0x3800u16, 0x3800, 0x3800, alpha]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut image = Image::new(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixel.repeat(16),
        TextureFormat::Rgba16Float,
        RenderAssetUsages::all(),
    );
    image.sampler = ImageSampler::nearest();
    image
}
fn setup(
    mut commands: Commands,
    cfg: Res<FixtureConfig>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.08, 0.13, 0.2)));
    let camera = commands
        .spawn((
            Camera3d::default(),
            Transform::from_xyz(0., 4., 10.).looking_at(Vec3::new(0., 1., 0.), Vec3::Y),
            AmbientLight {
                brightness: 80.,
                ..default()
            },
        ))
        .id();
    if cfg.depth {
        commands
            .entity(camera)
            .insert(bevy::core_pipeline::prepass::DepthPrepass);
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(18., 12.))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.3, 0.3),
            ..default()
        })),
    ));
    let quad = meshes.add(Rectangle::new(0.8, 1.8));
    let source_state = FoliageAlpha::initialized(64);
    let mut shared_zero = None;
    for (i, (name, half, flags, byte)) in [
        ("below", 0x33ff, 64, 255),
        ("equal", 0x3400, 64, 255),
        ("above", 0x3401, 64, 255),
        ("disabled-zero", 0x0000, 96, 255),
        ("byte-zero", 0x0000, 64, 0),
        ("quantized-125", 0x3400, 64, 125),
        ("quantized-1", 0x3400, 64, 1),
        ("shared-default-zero", 0x0000, 64, 255),
        ("shared-override-zero", 0x0000, 64, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let source = if i == 8 {
            shared_zero.clone().unwrap()
        } else {
            let image = images.add(exact_image(half));
            let handle = materials.add(source_material(
                StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(image),
                    unlit: true,
                    cull_mode: None,
                    double_sided: true,
                    ..default()
                },
                FoliageAlpha::initialized(flags),
            ));
            if i == 7 {
                shared_zero = Some(handle.clone());
            }
            handle
        };
        let state = FoliageAlpha {
            raw_flags: flags,
            instance_byte: byte,
        };
        // Half values used here: zero, .25 and its nearest half neighbors.
        let alpha = match half {
            0 => 0.,
            0x33ff => 0.24987793,
            0x3400 => 0.25,
            0x3401 => 0.25024414,
            _ => unreachable!(),
        };
        info!(
            "foliage fixture panel={i} name={name} half={half:#06x} flags={flags:#x} byte={byte} reference={} expected_discard={} distance_factor=1",
            state.reference(),
            state.discards(alpha)
        );
        let panel = commands
            .spawn((
                Mesh3d(quad.clone()),
                MeshMaterial3d(source.clone()),
                FoliageInstance { source, state },
                Transform::from_xyz(i as f32 - 4., 1.3, 0.),
            ))
            .id();
        if i == 8 {
            commands.entity(panel).insert(ReusePanel { phase: 0 });
        }
    }
    info!(
        "foliage fixture main/depth={} shared zero panels7/8; resetting/reusing panel8 before capture; initialized source byte={}",
        cfg.depth, source_state.instance_byte
    );
}
fn reset_and_reuse(time: Res<Time>, mut panels: Query<(&mut ReusePanel, &mut FoliageInstance)>) {
    for (mut panel, mut instance) in &mut panels {
        if panel.phase == 0 && time.elapsed_secs() > 1. {
            instance.state.instance_byte = 255;
            panel.phase = 1;
            info!("foliage fixture reset shared panel8 to source byte255");
        } else if panel.phase == 1 && time.elapsed_secs() > 2. {
            instance.state.instance_byte = 0;
            panel.phase = 2;
            info!("foliage fixture reused shared panel8 private override byte0; panel7 unchanged");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_half_texels_preserve_exact_equality_and_neighbor_inputs() {
        for alpha in [0x33ff, 0x3400, 0x3401, 0] {
            let image = exact_image(alpha);
            assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
            let bytes = image.data.unwrap();
            for pixel in bytes.as_chunks::<8>().0 {
                assert_eq!(&pixel[6..8], &alpha.to_le_bytes());
            }
        }
    }
}
