//! The road network: the AI's road graph `ai\default.rg` (named by `jc2.avproj`:
//! `CAISystemProperties { RoadNetworkFile = "default.rg" }`, loaded by `FUN_00477da0`)
//! and the road splines it was built from, the `CRoadObject`s of the road containers
//! (`locations\roads\roo_*.nlz`, `locations\metropolis\city.district.*.roads.nlz`) and
//! of the tiles' own containers (`31x14.nlz` has two). See `research/formats/civilians.md`.
//!
//! ```text
//! u32  magic 0x?6000df0 — low 24 bits 0x000df0, high byte the version (5 or 6)
//! u32  nodes, u32 edges
//! nodes × { f32 x, y, z (engine axes); u16 flags; u16 extra }              16 bytes
//! edges × { u16 from, u16 to; u32 length (m, along the road);
//!           u32 object_id, u32 instance_id (the CRoadObject's `_object_id`,
//!           `_instance_id`); u8 a (0..2); u8 b (0..2) }                   18 bytes
//! u32  n (64), f32 cell (512 m): n² × { u32 first, u32 count } — the nodes are sorted
//!      by cell, index `z × n + x`, `x = floor((x + 16384) / cell)`
//! version 6: u16 m (128), u16 count; count × { u16 ?, u16 height (m), u16 edge };
//!      m² × u16 running end index per 256 m cell (`x + z × m` order) — a lookup of
//!      the edges near a point (`ClosestRoad`); the first u16 is not decoded
//! ```
//!
//! The graph has no geometry: an edge is a piece of one road spline between two of
//! its nodes (junctions and road ends). [`edge_shape`] finds that piece.

use crate::placement::Transform3;
use crate::property::{Node, PropertyFile, Variant};
use crate::reader::Reader;
use crate::{Error, Result};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadNode {
    /// Engine axes.
    pub pos: [f32; 3],
    /// 0, 1, 3, 5 or 13 (not decoded; 3 marks 43 nodes of degree 3 only).
    pub flags: u16,
    /// 2 on almost every node (not decoded).
    pub extra: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadEdge {
    pub from: u16,
    pub to: u16,
    /// Length along the road (m); equals the spline's arc length between the nodes.
    pub length: u32,
    /// `_object_id` of the road object it lies on.
    pub object_id: u32,
    /// `_instance_id` of the road object.
    pub instance_id: u32,
    /// Two small fields (0..2 each), not decoded.
    pub a: u8,
    pub b: u8,
}

/// The edge-lookup grid of version 6 files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeLookup {
    /// Cells per side (128, of 256 m).
    pub cells: u16,
    /// `(unknown, height in m, edge)`.
    pub records: Vec<(u16, u16, u16)>,
    /// Running end index into `records` per cell.
    pub ends: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadGraph {
    pub version: u8,
    pub nodes: Vec<RoadNode>,
    pub edges: Vec<RoadEdge>,
    /// Node grid: cells per side and cell size (m).
    pub grid: (u32, f32),
    /// `(first, count)` node range per cell.
    pub cells: Vec<(u32, u32)>,
    pub lookup: Option<EdgeLookup>,
    /// Edges touching each node.
    pub adjacency: Vec<Vec<usize>>,
}

const MAGIC_MASK: u32 = 0x00ff_ffff;
const MAGIC: u32 = 0x0000_0df0;
/// The world spans ±16 384 m.
const WORLD_MIN: f32 = -16384.0;

impl RoadGraph {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let magic = r.u32()?;
        let version = (magic >> 24) as u8;
        if magic & MAGIC_MASK != MAGIC || !(5..=6).contains(&version) {
            return Err(Error::format(format!("not a road graph: magic {magic:#x}")));
        }
        let n_nodes = r.u32()? as usize;
        let n_edges = r.u32()? as usize;
        if n_nodes > 65536 || n_edges > 1 << 20 {
            return Err(Error::format("road graph counts out of range"));
        }
        let mut nodes = Vec::with_capacity(n_nodes);
        for _ in 0..n_nodes {
            let pos = [r.f32()?, r.f32()?, r.f32()?];
            nodes.push(RoadNode {
                pos,
                flags: r.u16()?,
                extra: r.u16()?,
            });
        }
        let mut edges = Vec::with_capacity(n_edges);
        for _ in 0..n_edges {
            let e = RoadEdge {
                from: r.u16()?,
                to: r.u16()?,
                length: r.u32()?,
                object_id: r.u32()?,
                instance_id: r.u32()?,
                a: r.u8()?,
                b: r.u8()?,
            };
            if e.from as usize >= n_nodes || e.to as usize >= n_nodes {
                return Err(Error::format("road edge names a node out of range"));
            }
            edges.push(e);
        }
        let n = r.u32()?;
        let cell = r.f32()?;
        if n == 0 || n > 1024 {
            return Err(Error::format("road graph grid size out of range"));
        }
        let mut cells = Vec::with_capacity((n * n) as usize);
        for _ in 0..n * n {
            cells.push((r.u32()?, r.u32()?));
        }
        let lookup = if version == 6 && r.remaining() >= 4 {
            let m = r.u16()?;
            let count = r.u16()? as usize;
            let mut records = Vec::with_capacity(count);
            for _ in 0..count {
                records.push((r.u16()?, r.u16()?, r.u16()?));
            }
            let ends = r.u16_vec(m as usize * m as usize)?;
            Some(EdgeLookup {
                cells: m,
                records,
                ends,
            })
        } else {
            None
        };
        let mut adjacency = vec![Vec::new(); n_nodes];
        for (i, e) in edges.iter().enumerate() {
            adjacency[e.from as usize].push(i);
            if e.to != e.from {
                adjacency[e.to as usize].push(i);
            }
        }
        Ok(Self {
            version,
            nodes,
            edges,
            grid: (n, cell),
            cells,
            lookup,
            adjacency,
        })
    }

    /// Nodes within `radius` of `(x, z)` (engine axes), through the node grid.
    pub fn nodes_near(&self, x: f32, z: f32, radius: f32) -> Vec<usize> {
        let (n, cell) = self.grid;
        let n = n as i32;
        let lo_x = ((x - radius - WORLD_MIN) / cell).floor() as i32;
        let hi_x = ((x + radius - WORLD_MIN) / cell).floor() as i32;
        let lo_z = ((z - radius - WORLD_MIN) / cell).floor() as i32;
        let hi_z = ((z + radius - WORLD_MIN) / cell).floor() as i32;
        let mut out = Vec::new();
        for cz in lo_z.max(0)..=hi_z.min(n - 1) {
            for cx in lo_x.max(0)..=hi_x.min(n - 1) {
                let (first, count) = self.cells[(cz * n + cx) as usize];
                for i in first as usize..(first + count) as usize {
                    let p = self.nodes.get(i).map(|n| n.pos);
                    if let Some(p) = p
                        && (p[0] - x).powi(2) + (p[2] - z).powi(2) <= radius * radius
                    {
                        out.push(i);
                    }
                }
            }
        }
        out
    }

    /// Straight-line distance between an edge's nodes.
    pub fn chord(&self, edge: usize) -> f32 {
        let e = &self.edges[edge];
        dist(
            self.nodes[e.from as usize].pos,
            self.nodes[e.to as usize].pos,
        )
    }

    /// The other end of `edge` seen from `node`.
    pub fn other_end(&self, edge: usize, node: usize) -> usize {
        let e = &self.edges[edge];
        if e.from as usize == node {
            e.to as usize
        } else {
            e.from as usize
        }
    }
}

/// A `CRoadObject`: a road spline, absolute (engine axes).
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSpline {
    pub instance_id: u32,
    pub object_id: u32,
    pub road_type: i32,
    pub size: i32,
    /// Control points; the curve passes through them.
    pub points: Vec<[f32; 3]>,
    /// The fourth value of every control point (0..1, mostly 0.5; not decoded).
    pub w: Vec<f32>,
}

impl RoadSpline {
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|p| dist(p[0], p[1])).sum()
    }
}

fn world_of(n: &Node) -> Option<Transform3> {
    match n.value("world")? {
        Variant::Mat4x3(m) => Some(Transform3::from_mat4x3(*m)),
        Variant::Mat4x4(m) => Some(Transform3::from_mat4x4(*m)),
        _ => None,
    }
}

/// Every `CRoadObject` of a placement file (`.blo`). Its `spline` is `x, y, z, w` per
/// control point relative to its `world`.
pub fn road_splines(file: &PropertyFile) -> Vec<RoadSpline> {
    fn walk(n: &Node, parent: &Transform3, out: &mut Vec<RoadSpline>) {
        let here = world_of(n).map(|w| w.then(parent)).unwrap_or(*parent);
        if n.str("_class") == Some("CRoadObject") {
            if let Some(Variant::Floats(f)) = n.value("spline")
                && f.len() >= 8
            {
                let mut points = Vec::with_capacity(f.len() / 4);
                let mut w = Vec::with_capacity(f.len() / 4);
                for p in f.as_chunks::<4>().0 {
                    points.push(here.transform_point([p[0], p[1], p[2]]));
                    w.push(p[3]);
                }
                out.push(RoadSpline {
                    instance_id: n.int("_instance_id").unwrap_or(0) as u32,
                    object_id: n.int("_object_id").unwrap_or(0) as u32,
                    road_type: n.int("road_type").unwrap_or(0),
                    size: n.int("size").unwrap_or(0),
                    points,
                    w,
                });
            }
            return;
        }
        for (_, c) in &n.children {
            walk(c, &here, out);
        }
    }
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

/// The road containers that are not tiles: the regional road sets and the city's.
pub const ROAD_CONTAINERS: &[&str] = &[
    "roo_arctic01.nlz",
    "roo_arctic02.nlz",
    "roo_city.nlz",
    "roo_coastal.nlz",
    "roo_delta01.nlz",
    "roo_delta02.nlz",
    "roo_desert01.nlz",
    "roo_desert02.nlz",
    "roo_desert03.nlz",
    "roo_etno01.nlz",
    "roo_etno02.nlz",
    "roo_farming01.nlz",
    "roo_farming02.nlz",
    "roo_farming03.nlz",
    "roo_industry.nlz",
    "roo_pirate.nlz",
    "roo_resort01.nlz",
    "roo_resort02.nlz",
    "roo_resort03.nlz",
    "roo_resort04.nlz",
    "roo_resort05.nlz",
    "roo_resort06.nlz",
    "roo_resort07.nlz",
    "roo_threekings.nlz",
    "city.district.a1.roads.nlz",
    "city.district.a2.roads.nlz",
    "city.district.b1.roads.nlz",
    "city.district.b2.roads.nlz",
    "city.district.c1.roads.nlz",
    "city.district.c2.roads.nlz",
    "city.district.d1.roads.nlz",
    "city.district.d2.roads.nlz",
    "city.district.d3.roads.nlz",
];

/// The placement file (`.blo`) of a location container (`roo_city.nlz`, `31x14.nlz`).
pub fn container_placements(fs: &crate::GameFs, container: &str) -> Result<PropertyFile> {
    let data = fs.read_inflated(container)?;
    let sarc = crate::sarc::SmallArchive::parse(&data)?;
    let e = sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".blo"))
        .ok_or_else(|| Error::format(format!("{container}: no .blo")))?;
    let bytes = crate::compress::maybe_inflate(sarc.data(&data, e)?)?;
    PropertyFile::parse_any(&bytes)
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Closest point of a polyline to `p`: (distance, segment, fraction, arc length there).
fn project(points: &[[f32; 3]], p: [f32; 3]) -> (f32, usize, f32, f32) {
    let mut best = (f32::MAX, 0, 0.0, 0.0);
    let mut s = 0.0;
    for (i, w) in points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let l2 = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).max(1e-9);
        let t = ((ab[0] * ap[0] + ab[1] * ap[1] + ab[2] * ap[2]) / l2).clamp(0.0, 1.0);
        let q = [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t];
        let d = dist(p, q);
        let len = l2.sqrt();
        if d < best.0 {
            best = (d, i, t, s + len * t);
        }
        s += len;
    }
    best
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// The piece of `points` between the projections of `p0` and `p1`, from `p0`'s end,
/// starting and ending exactly at `p0` and `p1`.
fn sub_polyline(points: &[[f32; 3]], p0: [f32; 3], p1: [f32; 3]) -> Vec<[f32; 3]> {
    let a = project(points, p0);
    let b = project(points, p1);
    let forward = a.3 <= b.3;
    let (first, last) = if forward { (a, b) } else { (b, a) };
    let mut out = vec![lerp3(points[first.1], points[first.1 + 1], first.2)];
    for p in points.iter().take(last.1 + 1).skip(first.1 + 1) {
        out.push(*p);
    }
    out.push(lerp3(points[last.1], points[last.1 + 1], last.2));
    if !forward {
        out.reverse();
    }
    // the graph's node, not the projection, at the ends
    if let Some(f) = out.first_mut() {
        *f = p0;
    }
    if let Some(l) = out.last_mut() {
        *l = p1;
    }
    out.dedup_by(|a, b| dist(*a, *b) < 0.05);
    out
}

/// How far (m) a node may lie off the spline whose piece the edge is.
pub const NODE_TOLERANCE: f32 = 6.0;
/// How far the piece's length may stray from the edge's `length` (fraction).
pub const LENGTH_TOLERANCE: f32 = 0.15;
/// Edges no longer than this × their chord are taken as straight when no spline has them.
pub const STRAIGHT: f32 = 1.05;

/// The shape of `edge` from its `from` node to its `to` node: the piece of the
/// candidate spline that passes both nodes (within [`NODE_TOLERANCE`]) with an arc
/// length closest to the edge's `length` (within [`LENGTH_TOLERANCE`]). `None` when no
/// candidate fits.
pub fn edge_shape<'a>(
    graph: &RoadGraph,
    edge: usize,
    candidates: impl IntoIterator<Item = &'a RoadSpline>,
) -> Option<Vec<[f32; 3]>> {
    let e = graph.edges.get(edge)?;
    let p0 = graph.nodes[e.from as usize].pos;
    let p1 = graph.nodes[e.to as usize].pos;
    let want = e.length as f32;
    let mut best: Option<(f32, &RoadSpline)> = None;
    for s in candidates {
        if s.points.len() < 2 {
            continue;
        }
        let a = project(&s.points, p0);
        let b = project(&s.points, p1);
        if a.0 > NODE_TOLERANCE || b.0 > NODE_TOLERANCE {
            continue;
        }
        let len = (b.3 - a.3).abs();
        let err = (len - want).abs() / want.max(1.0);
        if err > LENGTH_TOLERANCE {
            continue;
        }
        let score = err + a.0.max(b.0) / 50.0;
        if best.is_none_or(|(sc, _)| score < sc) {
            best = Some((score, s));
        }
    }
    best.map(|(_, s)| sub_polyline(&s.points, p0, p1))
}

/// A polyline moved `metres` to the right of its direction of travel, in engine axes
/// (right-handed: x east, y up, z south; facing +z, south, the right is −x, west — the
/// right of a direction `d` is `d × y`). Traffic keeps to the right-hand lane this way.
pub fn offset_right(points: &[[f32; 3]], metres: f32) -> Vec<[f32; 3]> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    let dir = |a: [f32; 3], b: [f32; 3]| -> [f32; 2] {
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let l = (dx * dx + dz * dz).sqrt().max(1e-6);
        [dx / l, dz / l]
    };
    (0..n)
        .map(|i| {
            let d0 = dir(points[i.saturating_sub(1)], points[i.max(1)]);
            let d1 = dir(points[i.min(n - 2)], points[(i + 1).min(n - 1)]);
            let (mut dx, mut dz) = (d0[0] + d1[0], d0[1] + d1[1]);
            let l = (dx * dx + dz * dz).sqrt();
            if l < 1e-3 {
                (dx, dz) = (d1[0], d1[1]);
            } else {
                (dx, dz) = (dx / l, dz / l);
            }
            // keep the lane's width through bends (a miter), within reason
            let cos_half = (d0[0] * dx + d0[1] * dz).clamp(0.5, 1.0);
            let k = metres / cos_half;
            let p = points[i];
            [p[0] - dz * k, p[1], p[2] + dx * k]
        })
        .collect()
}

/// Road splines by 64 m cell, for finding the candidates of an edge.
#[derive(Clone, Debug, Default)]
pub struct SplineIndex {
    cells: HashMap<(i32, i32), Vec<u32>>,
}

const INDEX_CELL: f32 = 64.0;

impl SplineIndex {
    fn key(x: f32, z: f32) -> (i32, i32) {
        (
            (x / INDEX_CELL).floor() as i32,
            (z / INDEX_CELL).floor() as i32,
        )
    }

    /// Register spline number `id`.
    pub fn insert(&mut self, id: u32, s: &RoadSpline) {
        for w in s.points.windows(2) {
            let steps = (dist(w[0], w[1]) / (INDEX_CELL / 4.0)).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let p = lerp3(w[0], w[1], k as f32 / steps as f32);
                let v = self.cells.entry(Self::key(p[0], p[2])).or_default();
                if v.last() != Some(&id) && !v.contains(&id) {
                    v.push(id);
                }
            }
        }
    }

    /// Splines passing within a cell of `(x, z)`.
    pub fn near(&self, x: f32, z: f32) -> Vec<u32> {
        let (cx, cz) = Self::key(x, z);
        let mut out = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(v) = self.cells.get(&(cx + dx, cz + dz)) {
                    for id in v {
                        if !out.contains(id) {
                            out.push(*id);
                        }
                    }
                }
            }
        }
        out
    }
}

/// The graph with the splines found so far and the edge shapes worked out from them.
#[derive(Clone, Debug, Default)]
pub struct RoadNetwork {
    pub graph: RoadGraph,
    pub splines: Vec<RoadSpline>,
    index: SplineIndex,
    shapes: HashMap<usize, Vec<[f32; 3]>>,
}

impl RoadNetwork {
    pub fn new(graph: RoadGraph) -> Self {
        Self {
            graph,
            ..Default::default()
        }
    }

    /// Add road splines (a road container's, a tile's); duplicates of ones already in
    /// (same ids and first point) are skipped. Returns how many were new.
    pub fn add_splines(&mut self, splines: Vec<RoadSpline>) -> usize {
        let mut added = 0;
        for s in splines {
            let dup = self
                .index
                .near(s.points[0][0], s.points[0][2])
                .iter()
                .any(|&i| {
                    let o = &self.splines[i as usize];
                    o.instance_id == s.instance_id
                        && o.object_id == s.object_id
                        && dist(o.points[0], s.points[0]) < 0.01
                });
            if dup {
                continue;
            }
            let id = self.splines.len() as u32;
            self.index.insert(id, &s);
            self.splines.push(s);
            added += 1;
        }
        added
    }

    /// The edge's shape from its `from` node to its `to` node: the spline piece when one
    /// is known, a straight line when the edge is about as long as its chord, else `None`
    /// (its spline is in a container not read yet). Spline pieces are cached.
    pub fn shape(&mut self, edge: usize) -> Option<Vec<[f32; 3]>> {
        if let Some(s) = self.shapes.get(&edge) {
            return Some(s.clone());
        }
        let e = *self.graph.edges.get(edge)?;
        let p0 = self.graph.nodes[e.from as usize].pos;
        let p1 = self.graph.nodes[e.to as usize].pos;
        let near0 = self.index.near(p0[0], p0[2]);
        let near1 = self.index.near(p1[0], p1[2]);
        let both: Vec<&RoadSpline> = near0
            .iter()
            .filter(|i| near1.contains(i))
            .map(|&i| &self.splines[i as usize])
            .collect();
        if let Some(s) = edge_shape(&self.graph, edge, both) {
            self.shapes.insert(edge, s.clone());
            return Some(s);
        }
        (e.length as f32 <= self.graph.chord(edge) * STRAIGHT + 1.0).then(|| vec![p0, p1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_with(nodes: &[[f32; 3]], edges: &[(u16, u16, u32)]) -> RoadGraph {
        let mut g = RoadGraph {
            version: 6,
            nodes: nodes
                .iter()
                .map(|p| RoadNode {
                    pos: *p,
                    flags: 0,
                    extra: 2,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|&(from, to, length)| RoadEdge {
                    from,
                    to,
                    length,
                    object_id: 0,
                    instance_id: 0,
                    a: 0,
                    b: 0,
                })
                .collect(),
            grid: (1, 32768.0),
            cells: vec![(0, nodes.len() as u32)],
            lookup: None,
            adjacency: vec![Vec::new(); nodes.len()],
        };
        for (i, e) in g.edges.clone().iter().enumerate() {
            g.adjacency[e.from as usize].push(i);
            g.adjacency[e.to as usize].push(i);
        }
        g
    }

    /// A spline bent like an L, two edges along it: each gets its own piece, oriented
    /// from its `from` node, and a third edge on no spline stays straight or unknown.
    #[test]
    fn edges_take_their_piece_of_the_spline() {
        let g = graph_with(
            &[
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 0.0, 100.0],
                [300.0, 0.0, 0.0],
            ],
            &[(1, 0, 100), (1, 2, 100), (1, 3, 200), (0, 2, 200)],
        );
        let s = RoadSpline {
            instance_id: 1,
            object_id: 2,
            road_type: 0,
            size: 1,
            points: vec![
                [0.0, 0.0, 0.0],
                [50.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 0.0, 50.0],
                [100.0, 0.0, 100.0],
            ],
            w: vec![0.5; 5],
        };
        let mut net = RoadNetwork::new(g);
        assert_eq!(net.add_splines(vec![s.clone()]), 1);
        assert_eq!(net.add_splines(vec![s]), 0, "a duplicate");
        let a = net.shape(0).unwrap();
        assert_eq!(
            a,
            vec![[100.0, 0.0, 0.0], [50.0, 0.0, 0.0], [0.0, 0.0, 0.0]]
        );
        let b = net.shape(1).unwrap();
        assert_eq!(b.first(), Some(&[100.0, 0.0, 0.0]));
        assert_eq!(b.last(), Some(&[100.0, 0.0, 100.0]));
        assert_eq!(b.len(), 3);
        // straight, on no spline
        assert_eq!(net.shape(2).unwrap().len(), 2);
        // the corner-cutting edge 0 → 2 is 200 m long but its chord is 141: unknown
        assert!(net.shape(3).is_some(), "the L itself is 200 m from 0 to 2");
    }

    #[test]
    fn the_right_hand_lane() {
        // facing +z (south), right is −x (west); facing +x (east), right is +z (south)
        let lane = offset_right(&[[0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [10.0, 0.0, 10.0]], 2.0);
        assert_eq!(lane[0], [-2.0, 0.0, 0.0]);
        assert_eq!(lane[2], [10.0, 0.0, 12.0]);
        // the corner (a left turn) is pushed out diagonally, 2 m from both legs
        assert!(
            (lane[1][0] + 2.0).abs() < 1e-4 && (lane[1][2] - 12.0).abs() < 1e-4,
            "{:?}",
            lane[1]
        );
    }

    /// `ai\default.rg`: 3653 nodes, 4168 edges over the whole island, sorted into the 64 ×
    /// 64 node grid; edge lengths add up to the road objects' (1145 km); the road
    /// containers' splines give most edges their exact shape. Needs JC2_TEST_DIR.
    #[test]
    fn the_island_road_graph() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("default.rg") else {
            return;
        };
        let g = RoadGraph::parse(&data).unwrap();
        assert_eq!((g.version, g.nodes.len(), g.edges.len()), (6, 3653, 4168));
        assert_eq!(g.grid, (64, 512.0));
        // every node sits in the cell that lists it
        for (k, &(first, count)) in g.cells.iter().enumerate() {
            for i in first..first + count {
                let p = g.nodes[i as usize].pos;
                let cx = ((p[0] + 16384.0) / 512.0).floor() as usize;
                let cz = ((p[2] + 16384.0) / 512.0).floor() as usize;
                assert_eq!(cz * 64 + cx, k);
            }
        }
        let total: u64 = g.edges.iter().map(|e| e.length as u64).sum();
        assert!((1_100_000..1_200_000).contains(&total), "{total} m");
        let lookup = g.lookup.as_ref().unwrap();
        assert_eq!(lookup.cells, 128);
        assert_eq!(*lookup.ends.last().unwrap() as usize, lookup.records.len());
        assert!(
            lookup
                .records
                .iter()
                .all(|r| (r.2 as usize) < g.edges.len())
        );
        // nodes near the start village (31,14)
        assert!(!g.nodes_near(-500.0, -8850.0, 1500.0).is_empty());

        let mut net = RoadNetwork::new(g);
        let mut splines = 0;
        for c in ROAD_CONTAINERS {
            if let Ok(file) = container_placements(&fs, c) {
                splines += net.add_splines(road_splines(&file));
            }
        }
        assert!(splines > 2500, "{splines} road splines");
        let km: f32 = net.splines.iter().map(|s| s.length()).sum::<f32>() / 1000.0;
        assert!((1000.0..1300.0).contains(&km), "{km} km of road");
        let mut matched = 0;
        for i in 0..net.graph.edges.len() {
            if let Some(s) = net.shape(i)
                && s.len() > 2
            {
                matched += 1;
                // the piece is as long as the edge says
                let len: f32 = s.windows(2).map(|w| dist(w[0], w[1])).sum();
                let want = net.graph.edges[i].length as f32;
                assert!((len - want).abs() <= want * 0.2 + 12.0, "{len} vs {want}");
            }
        }
        assert!(matched > 2000, "{matched} edges shaped by a spline");
        eprintln!("default.rg: {splines} splines, {km:.0} km, {matched} edges shaped");
    }
}
