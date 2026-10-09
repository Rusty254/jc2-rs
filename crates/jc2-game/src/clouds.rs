//! The sky above the camera: a gradient dome, stars, the moon with its halo, the sun's
//! glow, a field of cumulus cards and a high layer of cirrus (`research/formats/weather.md`).
//!
//! * **Dome**: a hemisphere (to a little below the horizon) coloured from the game's sky
//!   table (`skygradient_dif.dds`: zenith to horizon at the hour), brighter around a low
//!   sun. Its horizon is the clear/fog colour `sky.rs` computes, so it meets the haze.
//! * **Stars**: 1 600 sprites of `star_dif.dds` on a sphere that turns with the sky (the
//!   moon's orbit), added to the dome and faded in as the sun sets.
//! * **Moon / sun**: the moon is `moon_full_dif.dds` (the disc with its halo in one
//!   texture), the sun a glow sprite from `sun_dif.dds` behind `sky.rs`'s disc.
//! * **Clouds**: `weathersettings.bin` says where (`cloud_base`, `cloud_height`, the field's
//!   radius 20 km, 555 cards). Cards from `weather_dif.dds` (15 cumulus pictures in a 4 × 4
//!   atlas) at heights between base and base + height, a field that repeats around the
//!   camera (`sky_sprite.wgsl`) and drifts with the wind; tinted by the hour (white by
//!   day, orange at dusk, dark blue by night). The high **cirrus** is `cirrus_clouds_dif.dds`
//!   (three pictures) on a few huge cards at 9 km.
//!
//! `--no-clouds` leaves the clouds and cirrus out, `--cloud-cover 0.8` sets how many of the
//! 555 cards are drawn (the game's default is the file's 0.42), `--no-stars` leaves the stars.

use crate::Game;
use crate::impostors::atlas_image;
use crate::sky::{MoonDisc, SkyData, SkyState, TimeOfDay};
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use jc2_formats::weather::elevation_degrees;
use std::f32::consts::{PI, TAU};

/// Where the sun and moon sprites, the stars and the dome sit (inside the 30 km far plane).
const DOME_RADIUS: f32 = 27_500.0;
const STAR_RADIUS: f32 = 24_500.0;
const DISC_DISTANCE: f32 = 25_000.0;
/// The cirrus layer's height (m above the sea) and its cards' width.
const CIRRUS_HEIGHT: f32 = 9_000.0;
/// The cloud field repeats every this many metres (the file's radius is 20 km).
const FIELD: f32 = 40_000.0;
const STARS: usize = 1600;
/// The celestial axis the sun, moon and stars turn about (`weather.rs`'s `SUN_AXIS`).
const SKY_AXIS: Vec3 = Vec3::new(0.0, -1.0, 0.8);

#[derive(Resource, Clone, Debug)]
pub struct CloudConfig {
    pub clouds: bool,
    pub stars: bool,
    /// Share of the cloud cards drawn, 0..=1; `None` takes the settings file's value.
    pub cover: Option<f32>,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            clouds: true,
            stars: true,
            cover: None,
        }
    }
}

/// A clear-sky override also hides the cirrus. Treat invalid overrides as clear
/// instead of letting NaN reach card counts or future shadow uniforms.
fn effective_cover(cfg: &CloudConfig, default: f32) -> f32 {
    let cover = cfg.cover.unwrap_or(default);
    if cfg.clouds && cover.is_finite() {
        cover.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub type SkySpriteMaterial = ExtendedMaterial<StandardMaterial, SkySpriteExt>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct SkySpriteExt {
    #[uniform(100)]
    pub params: SkySpriteParams,
}

#[derive(ShaderType, Clone, Debug, Default)]
pub struct SkySpriteParams {
    pub range: Vec4,
}

impl MaterialExtension for SkySpriteExt {
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn vertex_shader() -> ShaderRef {
        shader()
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("sky_sprite.wgsl")).with_source("embedded"),
    )
}

#[derive(Resource)]
struct SpriteShader(#[allow(dead_code)] Handle<Shader>);

#[derive(Component)]
struct SkyDome;
#[derive(Component)]
struct StarField;
#[derive(Component)]
struct MoonSprite;
#[derive(Component)]
struct SunGlow;
#[derive(Component)]
struct CloudLayer {
    /// The cirrus layer (the tint is computed from the sky state each frame).
    #[allow(dead_code)]
    cirrus: bool,
}

#[derive(Resource)]
struct SkyAssets {
    dome: Handle<Mesh>,
    star_material: Handle<StandardMaterial>,
    moon_material: Handle<StandardMaterial>,
    sun_material: Handle<StandardMaterial>,
    cloud_material: Option<Handle<SkySpriteMaterial>>,
    cirrus_material: Option<Handle<SkySpriteMaterial>>,
    /// The hour the dome's colours were last computed for.
    dome_hour: f32,
    replaced_moon: bool,
}

pub struct CloudPlugin;

impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<CloudConfig>() {
            app.insert_resource(CloudConfig::default());
        }
        bevy::asset::embedded_asset!(app, "sky_sprite.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "sky_sprite.wgsl");
        app.insert_resource(SpriteShader(shader))
            .add_plugins(MaterialPlugin::<SkySpriteMaterial>::default())
            .add_systems(Startup, setup)
            .add_systems(Update, update_sky);
    }
}

/// A deterministic stream of floats in 0..1.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn texture(game: &Game, images: &mut Assets<Image>, name: &str) -> Option<Handle<Image>> {
    atlas_image(game, name, true).map(|i| images.add(i))
}

/// The upper hemisphere (and a little below the horizon) as a mesh whose vertices carry
/// colours the sky fills in. Rings of constant elevation, from −12° to the zenith.
fn dome_mesh() -> Mesh {
    const RINGS: usize = 28;
    const SEGMENTS: usize = 40;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for r in 0..=RINGS {
        // denser near the horizon
        let t = r as f32 / RINGS as f32;
        let elev = (-12.0 + 102.0 * t.powf(1.6)).to_radians();
        for s in 0..SEGMENTS {
            let az = s as f32 / SEGMENTS as f32 * TAU;
            let d = Vec3::new(
                elev.cos() * az.cos(),
                elev.sin().min(1.0),
                elev.cos() * az.sin(),
            );
            positions.push((d * DOME_RADIUS).to_array());
            normals.push((-d).to_array());
        }
    }
    let mut indices = Vec::new();
    for r in 0..RINGS {
        for s in 0..SEGMENTS {
            let a = (r * SEGMENTS + s) as u32;
            let b = (r * SEGMENTS + (s + 1) % SEGMENTS) as u32;
            let c = ((r + 1) * SEGMENTS + s) as u32;
            let d = ((r + 1) * SEGMENTS + (s + 1) % SEGMENTS) as u32;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let n = positions.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.5f32, 0.6, 0.9, 1.0]; n]);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The dome's vertex colours at an hour: the game's sky table from the zenith to the
/// horizon, taken relative to noon like the clear colour (`sky.rs`), plus a warm glow
/// around a low sun.
pub fn dome_colours(mesh: &Mesh, st: &SkyState, zenith: Vec3) -> Vec<[f32; 4]> {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .unwrap_or(&[]);
    let low_sun = 1.0 - ((st.sun_elevation - 2.0) / 28.0).clamp(0.0, 1.0);
    let sun_set = ((st.sun_elevation + 10.0) / 6.0).clamp(0.0, 1.0);
    positions
        .iter()
        .map(|p| {
            let d = Vec3::from(*p) / DOME_RADIUS;
            let e = d.y.clamp(0.0, 1.0);
            // 0 at the horizon, 1 at the zenith; most of the change is low down
            let k = 1.0 - (1.0 - e).powf(2.2);
            let k = k.powf(0.55);
            let mut c = st.sky.lerp(zenith, k);
            let towards_sun = d.dot(st.sun_dir).max(0.0);
            let glow = towards_sun.powf(6.0) * low_sun * sun_set * (1.0 - 0.6 * e);
            c += Vec3::new(1.0, 0.55, 0.25) * glow * 0.45;
            let lc = Color::srgb(
                c.x.clamp(0.0, 1.0),
                c.y.clamp(0.0, 1.0),
                c.z.clamp(0.0, 1.0),
            )
            .to_linear();
            [lc.red, lc.green, lc.blue, 1.0]
        })
        .collect()
}

/// The zenith colour (sRGB) at an hour: noon's deep blue scaled by how the game's table
/// changes there, faded into the night's.
fn zenith_colour(st: &SkyState, atm: Option<&jc2_formats::weather::Atmosphere>, hour: f32) -> Vec3 {
    const ZENITH_NOON: Vec3 = Vec3::new(0.16, 0.36, 0.78);
    const ZENITH_NIGHT: Vec3 = Vec3::new(0.004, 0.008, 0.022);
    let ratio = atm
        .map(|a| {
            let at = |h: f32| Vec3::from(a.sky_colour(h, 0.02)).max(Vec3::splat(0.03));
            (at(hour) / at(12.0)).clamp(Vec3::splat(0.25), Vec3::splat(1.5))
        })
        .unwrap_or(Vec3::ONE);
    ZENITH_NIGHT.lerp(
        (ZENITH_NOON * ratio).clamp(Vec3::ZERO, Vec3::ONE),
        st.daylight,
    )
}

fn star_mesh() -> Mesh {
    let mut rng = Rng(0x57a4_5eed);
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colours: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for _ in 0..STARS {
        // uniform on the sphere
        let y = rng.next() * 2.0 - 1.0;
        let az = rng.next() * TAU;
        let r = (1.0 - y * y).sqrt();
        let dir = Vec3::new(r * az.cos(), y, r * az.sin());
        let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        let right = dir.cross(up).normalize();
        let top = right.cross(dir);
        // brightness: a few bright ones, many faint
        let m = rng.next();
        let bright = 0.18 + 0.82 * m.powf(5.0);
        let size = 150.0 + 520.0 * m.powf(4.0);
        let tint = match (rng.next() * 4.0) as u32 {
            0 => Vec3::new(1.0, 0.85, 0.7),
            1 => Vec3::new(0.75, 0.85, 1.0),
            _ => Vec3::ONE,
        };
        let centre = dir * STAR_RADIUS;
        let base = positions.len() as u32;
        for (sx, sy, u, v) in [
            (-1.0, -1.0, 0.0, 1.0),
            (1.0, -1.0, 1.0, 1.0),
            (1.0, 1.0, 1.0, 0.0),
            (-1.0, 1.0, 0.0, 0.0),
        ] {
            positions.push((centre + right * sx * size + top * sy * size).to_array());
            uvs.push([u, v]);
            colours.push([tint.x * bright, tint.y * bright, tint.z * bright, 1.0]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let n = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A field of cards: world-space centres, a drift, atlas rectangles and sizes.
#[allow(clippy::too_many_arguments)]
fn card_mesh(
    seed: u32,
    count: usize,
    base: f32,
    height: f32,
    size: (f32, f32),
    squash: f32,
    cells: &[[f32; 4]],
    opacity: (f32, f32),
    wind: Vec2,
    inner: f32,
) -> Mesh {
    let mut rng = Rng(seed);
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut corners: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for _ in 0..count {
        // a ring around the camera from `inner` out to the field's half width
        let a = rng.next() * TAU;
        let r = inner + (FIELD * 0.5 - inner) * rng.next().sqrt();
        let x = a.cos() * r;
        let z = a.sin() * r;
        let y = base + height * rng.next();
        let w = size.0 + (size.1 - size.0) * rng.next();
        let cell = cells[(rng.next() * cells.len() as f32) as usize % cells.len()];
        let flip = rng.next() < 0.5;
        let op = opacity.0 + (opacity.1 - opacity.0) * rng.next();
        let (u0, v0, u1, v1) = (cell[0], cell[1], cell[2], cell[3]);
        let (u0, u1) = if flip { (u1, u0) } else { (u0, u1) };
        let b = positions.len() as u32;
        for (sx, sy, u, v) in [
            (-1.0, -1.0, u0, v1),
            (1.0, -1.0, u1, v1),
            (1.0, 1.0, u1, v0),
            (-1.0, 1.0, u0, v0),
        ] {
            positions.push([x, y, z]);
            normals.push([wind.x, wind.y, op]);
            uvs.push([u, v]);
            corners.push([sx * w * 0.5, sy * w * 0.5 * squash]);
        }
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, corners);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Cells of a `columns × rows` atlas, inset a little so the neighbours do not bleed in.
fn atlas_cells(columns: u32, rows: u32, skip: &[u32]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for i in 0..columns * rows {
        if skip.contains(&i) {
            continue;
        }
        let (c, r) = (i % columns, i / columns);
        let inset = 0.004;
        out.push([
            c as f32 / columns as f32 + inset,
            r as f32 / rows as f32 + inset,
            (c + 1) as f32 / columns as f32 - inset,
            (r + 1) as f32 / rows as f32 - inset,
        ]);
    }
    out
}

fn unlit(texture: Option<Handle<Image>>, alpha: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: texture,
        unlit: true,
        fog_enabled: false,
        alpha_mode: alpha,
        cull_mode: None,
        double_sided: false,
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    game: Res<Game>,
    cfg: Res<CloudConfig>,
    data: Res<SkyData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sprites: ResMut<Assets<SkySpriteMaterial>>,
) {
    let settings = data
        .0
        .as_ref()
        .map(|a| a.settings.clone())
        .unwrap_or_default();
    // the dome
    let dome = meshes.add(dome_mesh());
    let dome_material = standard.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        SkyDome,
        Name::new("sky dome"),
        Mesh3d(dome.clone()),
        MeshMaterial3d(dome_material),
        Transform::default(),
        NotShadowCaster,
        bevy::camera::visibility::NoFrustumCulling,
    ));

    // stars
    let star_tex = texture(&game, &mut images, "textures\\atmosphere\\star_dif.dds");
    let star_material = standard.add(unlit(star_tex, AlphaMode::Add));
    if cfg.stars {
        commands.spawn((
            StarField,
            Name::new("stars"),
            Mesh3d(meshes.add(star_mesh())),
            MeshMaterial3d(star_material.clone()),
            Transform::default(),
            NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }

    // the moon (disc and halo in one texture) and the sun's glow
    let moon_tex = texture(
        &game,
        &mut images,
        "textures\\atmosphere\\moon_full_dif.dds",
    );
    let moon_material = standard.add(unlit(moon_tex, AlphaMode::Blend));
    commands.spawn((
        MoonSprite,
        Name::new("moon"),
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(moon_material.clone()),
        Transform::default(),
        NotShadowCaster,
    ));
    let sun_tex = texture(&game, &mut images, "textures\\atmosphere\\sun_dif.dds");
    let sun_material = standard.add(unlit(sun_tex, AlphaMode::Add));
    commands.spawn((
        SunGlow,
        Name::new("sun glow"),
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(sun_material.clone()),
        Transform::default(),
        NotShadowCaster,
    ));

    // clouds: the cumulus field and the cirrus
    let mut cloud_material = None;
    let mut cirrus_material = None;
    let cover = effective_cover(&cfg, settings.overcast);
    if cover > 0.0 {
        let make = |sprites: &mut Assets<SkySpriteMaterial>,
                    standard_tex: Option<Handle<Image>>,
                    range: Vec4| {
            sprites.add(SkySpriteMaterial {
                base: unlit_fogged(standard_tex),
                extension: SkySpriteExt {
                    params: SkySpriteParams { range },
                },
            })
        };
        if let Some(tex) = texture(&game, &mut images, "textures\\weather\\weather_dif.dds") {
            let count = (settings.k_b8b11972.max(100.0) * cover * 2.4) as usize;
            let mesh = card_mesh(
                0xc10d_5001,
                count,
                settings.cloud_base,
                settings.cloud_height.max(400.0),
                (2200.0, 5200.0),
                0.8,
                &atlas_cells(4, 4, &[15]),
                (0.8, 1.0),
                Vec2::new(6.0, 2.5),
                2500.0,
            );
            let m = make(
                &mut sprites,
                Some(tex),
                Vec4::new(FIELD, 900.0, 2500.0, 14_500.0),
            );
            commands.spawn((
                CloudLayer { cirrus: false },
                Name::new("clouds"),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(m.clone()),
                Transform::default(),
                NotShadowCaster,
                bevy::camera::visibility::NoFrustumCulling,
            ));
            cloud_material = Some(m);
        }
        if let Some(tex) = texture(
            &game,
            &mut images,
            "textures\\weather\\cirrus_clouds_dif.dds",
        ) {
            let mesh = card_mesh(
                0xc10d_5002,
                (24.0 * (0.4 + cover)) as usize,
                CIRRUS_HEIGHT,
                1500.0,
                (14_000.0, 24_000.0),
                0.75,
                &atlas_cells(2, 2, &[1]),
                (0.35, 0.7),
                Vec2::new(2.0, 1.0),
                6000.0,
            );
            let m = make(
                &mut sprites,
                Some(tex),
                Vec4::new(FIELD, 3000.0, 7000.0, 20_000.0),
            );
            commands.spawn((
                CloudLayer { cirrus: true },
                Name::new("cirrus"),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(m.clone()),
                Transform::default(),
                NotShadowCaster,
                bevy::camera::visibility::NoFrustumCulling,
            ));
            cirrus_material = Some(m);
        }
    }
    commands.insert_resource(SkyAssets {
        dome,
        star_material,
        moon_material,
        sun_material,
        cloud_material,
        cirrus_material,
        dome_hour: f32::MAX,
        replaced_moon: false,
    });
}

fn unlit_fogged(texture: Option<Handle<Image>>) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: texture,
        unlit: true,
        // the far cards fade out by themselves (the haze would wash them blue)
        fog_enabled: false,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: false,
        ..default()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Tint of the clouds at an hour (sRGB): white by day, the sun's colour when it is low,
/// a dark blue by night; and how bright the cirrus is. **Non-native** project policy: used
/// for the cirrus layer always, and for the cumulus cards only when `gradients.bmp` could not
/// be read (otherwise their tint is the native lit colour, table `C`, `atmosphere.rs`).
pub fn cloud_tint(st: &SkyState, cirrus: bool) -> Vec3 {
    let day = st.daylight;
    let low = 1.0 - smoothstep(4.0, 26.0, st.sun_elevation);
    // low sun: the cards catch its colour, washed towards white so they stay bright
    let warm = (st.sun_colour * 1.5).lerp(Vec3::ONE * 1.4, 0.35);
    let lit = (Vec3::new(1.0, 1.0, 1.04) * 1.3).lerp(warm, low * 0.8);
    let night = Vec3::new(0.11, 0.14, 0.22) + Vec3::splat(0.06) * st.moon_lux / 2400.0;
    let mut c = night.lerp(lit, day);
    if cirrus {
        c *= 1.05;
    }
    c
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_sky(
    mut commands: Commands,
    time: Res<TimeOfDay>,
    data: Res<SkyData>,
    atmosphere: Option<Res<crate::atmosphere::AtmosphereState>>,
    assets: Option<ResMut<SkyAssets>>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    old_moon: Query<Entity, With<MoonDisc>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sprites: ResMut<Assets<SkySpriteMaterial>>,
    mut dome: Query<&mut Transform, (With<SkyDome>, Without<Camera3d>)>,
    mut stars: Query<
        &mut Transform,
        (
            With<StarField>,
            Without<SkyDome>,
            Without<MoonSprite>,
            Without<SunGlow>,
        ),
    >,
    mut moon: Query<
        (&mut Transform, &mut Visibility),
        (
            With<MoonSprite>,
            Without<SkyDome>,
            Without<StarField>,
            Without<SunGlow>,
        ),
    >,
    mut glow: Query<
        (&mut Transform, &mut Visibility),
        (
            With<SunGlow>,
            Without<SkyDome>,
            Without<StarField>,
            Without<MoonSprite>,
        ),
    >,
) {
    let Some(mut assets) = assets else { return };
    let Some(cam) = cams.iter().next() else {
        return;
    };
    let eye = cam.translation();
    let st = SkyState::at(time.hours, data.0.as_ref());
    // the textured moon replaces the plain disc
    if !assets.replaced_moon {
        for e in &old_moon {
            commands.entity(e).despawn();
            assets.replaced_moon = true;
        }
    }

    // dome: rides with the camera, recoloured when the hour has moved
    for mut tf in &mut dome {
        tf.translation = eye;
    }
    if (time.hours - assets.dome_hour).abs() > 0.015 {
        let zenith = zenith_colour(&st, data.0.as_ref(), time.hours);
        if let Some(mut mesh) = meshes.get_mut(&assets.dome) {
            let colours = dome_colours(&mesh, &st, zenith);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
        }
        assets.dome_hour = time.hours;
    }

    // stars: turn with the moon's orbit, shine when it is dark
    let dark = 1.0 - smoothstep(-16.0, -4.0, st.sun_elevation);
    if let Some(mut m) = standard.get_mut(&assets.star_material) {
        let b = dark * 2.6;
        m.base_color = Color::linear_rgb(b, b, b);
    }
    let axis = SKY_AXIS.normalize();
    let spin = Quat::from_axis_angle(axis, time.hours / 12.0 * PI);
    for mut tf in &mut stars {
        tf.translation = eye;
        tf.rotation = spin;
    }

    // the moon: a card facing the camera, halo and all
    let moon_visible = st.moon_dir.y > -0.05;
    let moon_up = elevation_degrees(st.moon_dir.to_array());
    for (mut tf, mut vis) in &mut moon {
        let pos = eye + st.moon_dir * DISC_DISTANCE;
        let size = 5200.0;
        *tf = Transform::from_translation(pos)
            .looking_at(eye, Vec3::Y)
            .with_scale(Vec3::new(size, size, 1.0));
        *vis = if moon_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if let Some(mut m) = standard.get_mut(&assets.moon_material) {
        // faint in daylight, full at night; reddish when it is low
        let low = 1.0 - smoothstep(0.0, 14.0, moon_up);
        let a = 0.10 + 0.90 * (1.0 - st.daylight);
        m.base_color = Color::srgba(1.0, 1.0 - 0.18 * low, 1.0 - 0.38 * low, a);
    }
    // the sun's glow: additive, strongest when the sun is low
    for (mut tf, mut vis) in &mut glow {
        let pos = eye + st.sun_dir * (DISC_DISTANCE - 200.0);
        let size = 9000.0;
        *tf = Transform::from_translation(pos)
            .looking_at(eye, Vec3::Y)
            .with_scale(Vec3::new(size, size, 1.0));
        *vis = if st.sun_elevation > -8.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if let Some(mut m) = standard.get_mut(&assets.sun_material) {
        let k = smoothstep(-8.0, 3.0, st.sun_elevation);
        let c = st.sun_colour * (0.55 * k);
        m.base_color = Color::linear_rgb(c.x, c.y * 0.95, c.z * 0.85);
    }

    // clouds: the tint follows the hour
    if let Some(h) = &assets.cloud_material
        && let Some(mut m) = sprites.get_mut(h)
    {
        m.base.base_color = match atmosphere.as_deref() {
            // native: the card's lit colour is table C (variant-weighted, linear RGB)
            Some(a) if a.loaded => Color::linear_rgb(a.cloud_lit.x, a.cloud_lit.y, a.cloud_lit.z),
            _ => {
                let c = cloud_tint(&st, false);
                Color::srgba(c.x, c.y, c.z, 1.0)
            }
        };
    }
    if let Some(h) = &assets.cirrus_material
        && let Some(mut m) = sprites.get_mut(h)
    {
        let c = cloud_tint(&st, true);
        m.base.base_color = Color::srgba(c.x, c.y, c.z, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_sky_and_disabled_clouds_have_zero_cover() {
        let mut cfg = CloudConfig::default();
        assert_eq!(effective_cover(&cfg, 0.42), 0.42);
        for cover in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            cfg.cover = Some(cover);
            assert_eq!(effective_cover(&cfg, 1.0), 0.0);
        }
        cfg.cover = Some(2.0);
        assert_eq!(effective_cover(&cfg, 1.0), 1.0);
        cfg.clouds = false;
        assert_eq!(effective_cover(&cfg, 1.0), 0.0);
    }

    #[test]
    fn the_dome_runs_from_the_horizon_colour_to_the_zenith() {
        let st = SkyState::at(12.0, None);
        let zenith = Vec3::new(0.1, 0.3, 0.8);
        let mesh = dome_mesh();
        let colours = dome_colours(&mesh, &st, zenith);
        assert_eq!(colours.len(), mesh.count_vertices());
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        let lin = |v: Vec3| {
            let c = Color::srgb(v.x, v.y, v.z).to_linear();
            Vec3::new(c.red, c.green, c.blue)
        };
        // the top vertex is the zenith colour, one on the horizon the sky colour
        let top = positions
            .iter()
            .enumerate()
            .max_by(|a, b| a.1[1].total_cmp(&b.1[1]))
            .unwrap()
            .0;
        let c = Vec3::new(colours[top][0], colours[top][1], colours[top][2]);
        assert!((c - lin(zenith)).length() < 0.02, "{c:?}");
        let horizon = positions
            .iter()
            .enumerate()
            .min_by(|a, b| a.1[1].abs().total_cmp(&b.1[1].abs()))
            .unwrap()
            .0;
        let h = Vec3::new(
            colours[horizon][0],
            colours[horizon][1],
            colours[horizon][2],
        );
        assert!(
            (h - lin(st.sky)).length() < 0.05,
            "{h:?} vs {:?}",
            lin(st.sky)
        );
    }

    #[test]
    fn clouds_are_white_by_day_orange_at_dusk_and_dark_at_night() {
        let noon = cloud_tint(&SkyState::at(12.0, None), false);
        let dusk = cloud_tint(&SkyState::at(18.3, None), false);
        let night = cloud_tint(&SkyState::at(0.0, None), false);
        assert!(noon.min_element() > 0.9);
        assert!(dusk.x > dusk.z + 0.15, "{dusk:?}");
        assert!(night.max_element() < 0.3, "{night:?}");
    }

    #[test]
    fn card_fields_hold_the_requested_cards() {
        let cells = atlas_cells(4, 4, &[15]);
        assert_eq!(cells.len(), 15);
        let m = card_mesh(
            1,
            100,
            2000.0,
            800.0,
            (1000.0, 2000.0),
            0.8,
            &cells,
            (0.5, 1.0),
            Vec2::new(1.0, 0.0),
            1000.0,
        );
        assert_eq!(m.count_vertices(), 400);
        let p = m
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        for c in p {
            assert!(c[1] >= 2000.0 && c[1] <= 2800.0);
            assert!(c[0].abs() <= FIELD * 0.5 && c[2].abs() <= FIELD * 0.5);
        }
    }

    #[test]
    fn stars_cover_the_sphere() {
        let m = star_mesh();
        assert_eq!(m.count_vertices(), STARS * 4);
        let p = m
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        let up = p.iter().filter(|v| v[1] > 0.0).count();
        assert!(up > STARS * 4 * 4 / 10 && up < STARS * 4 * 6 / 10, "{up}");
    }
}
