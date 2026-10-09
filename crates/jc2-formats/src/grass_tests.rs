use super::*;
use crate::GameFs;
use crate::terrain::TerrainTile;
use crate::vegetation::{Group, TileGround};

fn load() -> Option<(GameFs, Vegetation)> {
    let dir = std::env::var("JC2_TEST_DIR").ok()?;
    let fs = GameFs::open(dir).ok()?;
    if !fs.contains("climate\\zone_0\\grass_types_0.bin") {
        return None;
    }
    let v = Vegetation::load(&fs).ok()?;
    Some((fs, v))
}

#[test]
fn cells_follow_the_engines_pass_pattern() {
    // 4 passes × 8 rows × 2 columns = 64 cells; each cell once
    let mut seen = std::collections::HashSet::new();
    for pass in 0..4i32 {
        for row in 0..8i32 {
            let c0 = (((row & 3) > 1) as i32 + pass + row * 2) & 3;
            for col in [c0, c0 + 4] {
                assert!(seen.insert((row, col)), "cell {row},{col} twice");
            }
        }
    }
    assert_eq!(seen.len(), 64);
}

#[test]
fn grass_types_and_tables_read() {
    let Some((_, v)) = load() else { return };
    let z0 = v.zone(0).unwrap();
    assert_eq!(z0.grass.texture, "global_grass_DIF.dds");
    assert_eq!(z0.grass.types.len(), 28);
    let flowers = &z0.grass.types[z0.grass.index_of("Grass_Flowers").unwrap() as usize];
    assert_eq!(flowers.uv, [0.5, 0.753, 1.0, 1.0]);
    assert_eq!((flowers.width, flowers.height), (1.0, 0.5));
    assert!((flowers.size_min - 0.8).abs() < 1e-6 && (flowers.size_max - 1.4).abs() < 1e-6);
    // the rain forest's grass: 12 + 26 + 26 slots, all of the table's 64 used
    let rain = z0.sets.iter().find(|s| s.name == "Rain_Forest").unwrap();
    let table = rain.table(Group::Grass);
    assert!(table.iter().all(|s| s.is_some()));
    let named = |slot: Option<u16>| z0.grass.types[slot.unwrap() as usize].name.as_str();
    let thin = table
        .iter()
        .filter(|s| named(**s) == "Jungle_Grass_Thin")
        .count();
    let leafs = table
        .iter()
        .filter(|s| named(**s) == "Jungle_Grass_Leafs")
        .count();
    assert_eq!((thin, leafs), (12, 26));
    let thin = &z0.grass.types[z0.grass.index_of("Jungle_Grass_Thin").unwrap() as usize];
    assert!((thin.slope_min - 2f32.to_radians()).abs() < 1e-6);
    assert!((thin.slope_max - 22f32.to_radians()).abs() < 1e-6);
    // the other zones have their own types
    assert!(
        v.zone(1)
            .unwrap()
            .grass
            .index_of("Grass_Arctic_00")
            .is_some()
    );
    assert!(
        v.zone(2)
            .unwrap()
            .grass
            .index_of("Desert_Grass_Dry")
            .is_some()
    );
    // most grass entries of the sets name a type of their zone (the odd leftover, such
    // as `Grass_06`, and sets that list another zone's names, do not)
    let mut resolved = 0;
    for z in v.zones.iter().flatten() {
        for s in &z.sets {
            for e in s.entries.iter().filter(|e| e.group == Group::Grass) {
                if z.grass.index_of(&e.type_name).is_some() {
                    resolved += 1;
                }
            }
        }
    }
    assert!(resolved > 100, "{resolved}");
}

#[test]
fn grass_grows_on_jungle_ground_and_not_on_roads() {
    let Some((fs, v)) = load() else { return };
    let Ok(bytes) = fs.read_inflated("terrain\\terrain_win32_40_40.dat") else {
        return;
    };
    let tile = TerrainTile::parse(&bytes).unwrap();
    let ground = TileGround::new(&tile, 40, 40);
    let (x0, z0) = crate::terrain::tile_origin(40, 40);
    let (mut blades, mut cells, mut on_road) = (0usize, 0usize, 0usize);
    let mut by_type = std::collections::BTreeMap::<String, usize>::new();
    for j in 0..16 {
        for i in 0..16 {
            let (px, pz) = (x0 + i as f32 * 8.0, z0 + j as f32 * 8.0);
            let a = v.generate_grass_patch(px, pz, 4, &ground);
            assert_eq!(
                a,
                v.generate_grass_patch(px, pz, 4, &ground),
                "deterministic"
            );
            cells += 64;
            for b in &a {
                let g = ground.sample(b.position[0], b.position[2]).unwrap();
                // road asphalt (18), dirt road (16), pavement (25): no set, no grass
                if matches!(g.material, 16 | 18 | 25) {
                    on_road += 1;
                }
                let ty = &v.zone(b.zone).unwrap().grass.types[b.type_index as usize];
                let (sx, sz) = (g.normal[0].sin().abs(), g.normal[2].sin().abs());
                assert!(sx.max(sz) <= ty.slope_max + 1e-6 && ty.slope_min <= sx.min(sz));
                assert!(g.height > SEA_LEVEL);
                assert!(b.position[0] >= px - 0.5 && b.position[0] < px + 8.5);
                *by_type.entry(ty.name.clone()).or_default() += 1;
            }
            blades += a.len();
        }
    }
    eprintln!("40,40: {blades} blades in {cells} cells: {by_type:?}");
    assert_eq!(on_road, 0, "grass on roads");
    assert!(blades > cells / 10, "{blades} blades");
    assert!(blades < cells, "never more than a blade per cell");
    // fewer passes draw a subset of the blades of the full ones (the same draws)
    let full = v.generate_grass_patch(x0 + 64.0, z0 + 64.0, 4, &ground);
    let half = v.generate_grass_patch(x0 + 64.0, z0 + 64.0, 2, &ground);
    assert!(half.len() <= full.len());
    assert!(half.iter().all(|b| full.contains(b)));
}

#[test]
fn blade_quads_stand_on_the_ground() {
    let ty = GrassType {
        name: "t".into(),
        width: 1.0,
        height: 0.5,
        uv: [0.0, 0.0, 0.5, 0.25],
        size_min: 1.0,
        size_max: 1.0,
        lean: 0.5,
        sway: 0.05,
        slope_min: 0.0,
        slope_max: 1.0,
        align: 1.0,
    };
    let blade = GrassBlade {
        zone: 0,
        type_index: 0,
        pass: 0,
        position: [10.0, 300.0, 20.0],
        yaw: 0.0,
        size: 0.5,
        normal: [0.0, 1.0, 0.0],
        phase: 210,
    };
    let flat = |_x: f32, _z: f32| 300.0;
    for k in 0..QUADS_PER_BLADE {
        let q = blade_quad(&blade, &ty, k, &flat);
        // bottom vertices on the ground (+ the lift), tops a half metre above
        assert!((q[0].position[1] - 300.05).abs() < 1e-5);
        assert!((q[1].position[1] - 300.05).abs() < 1e-5);
        assert!((q[2].position[1] - 300.55).abs() < 1e-5);
        // two metres wide at the base (width 1 each side), the atlas rectangle's
        // bottom edge at the bottom (v1) and the top at v0
        let dx = q[1].position[0] - q[0].position[0];
        let dz = q[1].position[2] - q[0].position[2];
        assert!(((dx * dx + dz * dz).sqrt() - 2.0).abs() < 1e-4);
        assert_eq!((q[0].uv, q[2].uv), ([0.0, 0.25], [0.5, 0.0]));
        assert_eq!((q[0].along, q[2].along), (0.0, 1.0));
    }
    // the three quads are 120 degrees apart
    let a = blade_quad(&blade, &ty, 0, &flat);
    let b = blade_quad(&blade, &ty, 1, &flat);
    let dir = |q: &[QuadVertex; 4]| {
        let (dx, dz) = (
            q[1].position[0] - q[0].position[0],
            q[1].position[2] - q[0].position[2],
        );
        dz.atan2(dx)
    };
    let diff = (dir(&b) - dir(&a)).rem_euclid(TAU);
    assert!((diff - TAU / 3.0).abs() < 1e-3 || (diff - 2.0 * TAU / 3.0).abs() < 1e-3);
}
