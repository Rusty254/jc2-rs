//! The distant world: the rest of Panau out to the fog, from the global height map
//! (`terrain_win32.dat`, 512 × 512 samples at 64 m — `jc2_formats::terrain::GlobalTerrain`).
//!
//! The map is cut into 1 km chunks (16 × 16 cells). Chunks within [`FAR`] of the focus
//! (Rico, else the car, else the camera) stream in a few per frame and drop out
//! behind; chunks beyond [`FINE`] use every other sample. A chunk whose four tiles are
//! all loaded in detail is hidden; the others sit [`SINK`] metres low so a detailed tile
//! next to them draws over the seam instead of fighting it. Chunks that are all sea are
//! not built (the sea plane covers them). No colliders: nothing reaches them.

use crate::Game;
use crate::coords::{bevy_xyz, engine_xz, to_bevy};
use crate::world::{FlyCamera, SEA_LEVEL, Streamer, terrain_colour};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use jc2_formats::terrain::{GLOBAL_CELL, GlobalTerrain, TILE_SIZE, WORLD_MIN};
use std::collections::{HashMap, HashSet};

/// Global cells per chunk edge.
pub const CHUNK_CELLS: usize = 16;
/// Chunk edge in metres (1024).
pub const CHUNK_SIZE: f32 = CHUNK_CELLS as f32 * GLOBAL_CELL;
/// Chunks per world edge (32).
pub const CHUNKS: i32 = (32768.0 / CHUNK_SIZE) as i32;
/// Draw distance (m): the extent of the project's far-terrain mesh (**non-native**; the exe's
/// far plane is unresolved, `haze-fog-next.md` Gap 2). The native fog end `p1` is 12 km by day
/// (9 km at night, `haze.rs`), but the fog never reaches the full colour on its own: the horizon
/// band that hides the edge (Gap 1) is not drawn.
pub const FAR: f32 = 14_000.0;
/// Beyond this, chunks use every other sample (128 m).
pub const FINE: f32 = 5_000.0;
/// How far the coarse ground sits below the true height.
pub const SINK: f32 = 18.0;
const CHUNKS_PER_FRAME: usize = 6;

#[derive(Resource)]
pub struct Horizon {
    pub map: GlobalTerrain,
    /// Chunk → (entity, sample step).
    pub chunks: HashMap<(i32, i32), (Entity, usize)>,
    /// Chunks known to be all sea.
    pub sea_only: HashSet<(i32, i32)>,
    material: Handle<StandardMaterial>,
    pub built: usize,
}

#[derive(Component)]
pub struct HorizonChunk(pub (i32, i32));

pub struct HorizonPlugin;

impl Plugin for HorizonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_horizon)
            .add_systems(Update, (stream_horizon, hide_under_tiles).chain());
    }
}

fn setup_horizon(
    mut commands: Commands,
    game: Res<Game>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let map = match game
        .fs
        .read_inflated("terrain_win32.dat")
        .map_err(|e| e.to_string())
        .and_then(|d| GlobalTerrain::parse(&d).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => {
            warn!("terrain_win32.dat: {e}; no distant terrain");
            return;
        }
    };
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        reflectance: 0.08,
        ..default()
    });
    commands.insert_resource(Horizon {
        map,
        chunks: HashMap::new(),
        sea_only: HashSet::new(),
        material,
        built: 0,
    });
}

/// How far a vertex at height `y` is lowered: the full [`SINK`] inland, fading out
/// towards the shore so the coastline stays where it is.
pub fn sink(y: f32) -> f32 {
    SINK * ((y - SEA_LEVEL) / 40.0).clamp(0.0, 1.0)
}

/// The engine-space origin (minimum x and z) of a chunk.
pub fn chunk_origin(c: (i32, i32)) -> (f32, f32) {
    (
        WORLD_MIN + c.0 as f32 * CHUNK_SIZE,
        WORLD_MIN + c.1 as f32 * CHUNK_SIZE,
    )
}

/// The chunk mesh at `step` samples per vertex, or `None` when it is all sea.
pub fn chunk_mesh(map: &GlobalTerrain, c: (i32, i32), step: usize) -> Option<Mesh> {
    let (x0, z0) = chunk_origin(c);
    let cell = GLOBAL_CELL * step as f32;
    let n = CHUNK_CELLS / step + 1;
    let h = |x: f32, z: f32| map.height_at(x, z);
    let mut highest = f32::MIN;
    let mut positions = Vec::with_capacity(n * n);
    let mut normals = Vec::with_capacity(n * n);
    let mut colours = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (x0 + i as f32 * cell, z0 + j as f32 * cell);
            let y = h(x, z);
            highest = highest.max(y);
            let dx = (h(x + GLOBAL_CELL, z) - h(x - GLOBAL_CELL, z)) / (2.0 * GLOBAL_CELL);
            let dz = (h(x, z + GLOBAL_CELL) - h(x, z - GLOBAL_CELL)) / (2.0 * GLOBAL_CELL);
            let normal = to_bevy([-dx, 1.0, -dz]).normalize();
            positions.push(bevy_xyz(x, y - sink(y), z).to_array());
            normals.push(normal.to_array());
            colours.push(terrain_colour(y, normal.y));
        }
    }
    if highest < SEA_LEVEL - 4.0 {
        return None;
    }
    let mut indices: Vec<u32> = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = (j * n + i) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // counter-clockwise seen from above (`world::terrain_mesh`)
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// Distance from a point (engine x, z) to the nearest point of a chunk.
fn chunk_distance(c: (i32, i32), x: f32, z: f32) -> f32 {
    let (x0, z0) = chunk_origin(c);
    let dx = (x0 - x).max(0.0).max(x - (x0 + CHUNK_SIZE));
    let dz = (z0 - z).max(0.0).max(z - (z0 + CHUNK_SIZE));
    (dx * dx + dz * dz).sqrt()
}

fn stream_horizon(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    horizon: Option<ResMut<Horizon>>,
    heroes: Query<&Transform, (With<crate::character::Hero>, Without<HorizonChunk>)>,
    players: Query<&Transform, (With<crate::vehicle::Occupied>, Without<HorizonChunk>)>,
    cams: Query<&Transform, (With<FlyCamera>, Without<HorizonChunk>)>,
) {
    let Some(mut hz) = horizon else { return };
    let Some(p) = heroes
        .single()
        .ok()
        .or_else(|| players.single().ok())
        .or_else(|| cams.single().ok())
        .map(|t| t.translation)
    else {
        return;
    };
    let (x, z) = engine_xz(p);
    let step_for = |c: (i32, i32)| {
        if chunk_distance(c, x, z) > FINE { 2 } else { 1 }
    };
    // drop what fell out of range (with a little slack)
    let gone: Vec<(i32, i32)> = hz
        .chunks
        .keys()
        .copied()
        .filter(|&c| chunk_distance(c, x, z) > FAR + CHUNK_SIZE)
        .collect();
    for c in gone {
        if let Some((e, _)) = hz.chunks.remove(&c) {
            commands.entity(e).despawn();
        }
    }
    // missing or at the wrong detail, nearest first
    let (cx, cz) = (
        ((x - WORLD_MIN) / CHUNK_SIZE).floor() as i32,
        ((z - WORLD_MIN) / CHUNK_SIZE).floor() as i32,
    );
    let r = (FAR / CHUNK_SIZE).ceil() as i32 + 1;
    let mut wanted: Vec<((i32, i32), usize, f32)> = Vec::new();
    for j in (cz - r).max(0)..=(cz + r).min(CHUNKS - 1) {
        for i in (cx - r).max(0)..=(cx + r).min(CHUNKS - 1) {
            let c = (i, j);
            let d = chunk_distance(c, x, z);
            if d > FAR || hz.sea_only.contains(&c) {
                continue;
            }
            let step = step_for(c);
            match hz.chunks.get(&c) {
                Some(&(_, s)) if s == step => {}
                _ => wanted.push((c, step, d)),
            }
        }
    }
    wanted.sort_by(|a, b| a.2.total_cmp(&b.2));
    for (c, step, _) in wanted.into_iter().take(CHUNKS_PER_FRAME) {
        let Some(mesh) = chunk_mesh(&hz.map, c, step) else {
            hz.sea_only.insert(c);
            continue;
        };
        let e = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(hz.material.clone()),
                Transform::IDENTITY,
                HorizonChunk(c),
                Name::new(format!("horizon {},{}", c.0, c.1)),
            ))
            .id();
        if let Some((old, _)) = hz.chunks.insert(c, (e, step)) {
            commands.entity(old).despawn();
        }
        hz.built += 1;
    }
}

/// Hide a chunk once all four of its tiles are drawn in detail.
fn hide_under_tiles(
    streamer: Option<Res<Streamer>>,
    mut chunks: Query<(&HorizonChunk, &mut Visibility)>,
) {
    let Some(st) = streamer else { return };
    let per = (CHUNK_SIZE / TILE_SIZE) as i32;
    for (chunk, mut vis) in &mut chunks {
        let (i, j) = chunk.0;
        let covered = (0..per).all(|a| {
            (0..per).all(|b| {
                let t = ((i * per + a) as u32, (j * per + b) as u32);
                st.loaded.contains_key(&t)
            })
        });
        let want = if covered {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        vis.set_if_neq(want);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::GameFs;

    /// Needs JC2_TEST_DIR with `terrain_win32.dat` in the dropzone.
    #[test]
    fn chunks_follow_the_global_map() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("terrain_win32.dat") else {
            return;
        };
        let map = GlobalTerrain::parse(&data).unwrap();
        // the chunk under the village of tile 31,14 (x -512.., z -9216..)
        let c = (
            ((-256.0 - WORLD_MIN) / CHUNK_SIZE).floor() as i32,
            ((-8960.0 - WORLD_MIN) / CHUNK_SIZE).floor() as i32,
        );
        let mesh = chunk_mesh(&map, c, 1).expect("land");
        let pos = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .unwrap();
        assert_eq!(pos.len(), (CHUNK_CELLS + 1) * (CHUNK_CELLS + 1));
        for p in pos.iter().step_by(37) {
            let [x, _, z] = crate::coords::to_engine(Vec3::from(*p));
            let h = map.height_at(x, z);

            let want = h - sink(h);
            assert!((p[1] - want).abs() < 1e-3, "{p:?} vs {want}");
        }
        let coarse = chunk_mesh(&map, c, 2).unwrap();
        assert_eq!(coarse.count_vertices(), 9 * 9);
        // the far corner of the world is open sea
        assert!(chunk_mesh(&map, (0, 0), 2).is_none());
        // how much land there is: chunks with something above the sea
        let land = (0..CHUNKS)
            .flat_map(|j| (0..CHUNKS).map(move |i| (i, j)))
            .filter(|&c| chunk_mesh(&map, c, 2).is_some())
            .count();
        eprintln!("{land} of {} chunks reach above the sea", CHUNKS * CHUNKS);
        assert!(land > 100 && land < 900, "{land}");
    }
}
