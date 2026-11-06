//! Grid geometry: distance (5-10-5 diagonals), reach, movement, line of
//! effect, cover, flanking and areas.
//!
//! Coordinates: `Pos { x, y }` is the top-left square of a creature's space.
//! Continuous geometry uses square units where square (x, y) spans
//! `[x, x+1] × [y, y+1]`.

use crate::models::battlefield::{Battlefield, Terrain};
use crate::stats::Size;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative hasher for small integer keys (the pathfinder's tables).
/// Deterministic and not DoS-resistant — fine for keys that are grid
/// coordinates bounded by the map size.
#[derive(Default, Clone, Copy)]
pub struct GridHasher(u64);

impl GridHasher {
    #[inline]
    fn add(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for GridHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.add(*b as u64);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    fn write_i32(&mut self, i: i32) {
        self.add(i as u32 as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type GridMap<K, V> = HashMap<K, V, BuildHasherDefault<GridHasher>>;

pub use tactics_core::Pos;

/// PF2e distance in feet for a displacement of (dx, dy) squares:
/// every second diagonal costs 10 feet.
pub fn distance_ft(dx: i32, dy: i32) -> u32 {
    let (dx, dy) = (dx.unsigned_abs(), dy.unsigned_abs());
    let (long, short) = (dx.max(dy), dx.min(dy));
    5 * long + 5 * (short / 2)
}

pub fn footprint(pos: Pos, size: Size) -> Vec<Pos> {
    let s = size.squares();
    let mut v = Vec::with_capacity((s * s) as usize);
    for dy in 0..s {
        for dx in 0..s {
            v.push(Pos::new(pos.x + dx, pos.y + dy));
        }
    }
    v
}

/// Gap in squares between two spaces on each axis (1 = adjacent, 0 = overlap).
fn gaps(a: Pos, asz: Size, b: Pos, bsz: Size) -> (i32, i32) {
    let (a0x, a1x) = (a.x, a.x + asz.squares() - 1);
    let (a0y, a1y) = (a.y, a.y + asz.squares() - 1);
    let (b0x, b1x) = (b.x, b.x + bsz.squares() - 1);
    let (b0y, b1y) = (b.y, b.y + bsz.squares() - 1);
    let gx = (b0x - a1x).max(a0x - b1x).max(0);
    let gy = (b0y - a1y).max(a0y - b1y).max(0);
    (gx, gy)
}

/// Distance between two creatures' spaces in feet.
pub fn creature_distance(a: Pos, asz: Size, b: Pos, bsz: Size) -> u32 {
    let (gx, gy) = gaps(a, asz, b, bsz);
    distance_ft(gx, gy)
}

/// Reach check including the 10-foot-reach diagonal exception.
pub fn within_reach(a: Pos, asz: Size, b: Pos, bsz: Size, reach_ft: u32) -> bool {
    let (gx, gy) = gaps(a, asz, b, bsz);
    distance_ft(gx, gy) <= reach_ft || (reach_ft == 10 && gx <= 2 && gy <= 2)
}

pub fn center(pos: Pos, size: Size) -> (f64, f64) {
    let s = size.squares() as f64;
    (pos.x as f64 + s / 2.0, pos.y as f64 + s / 2.0)
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MoveRules {
    pub speed_ft: u32,
    pub size: Size,
    /// Squares the mover cannot pass through (spaces of hostile creatures).
    pub impassable: HashSet<Pos>,
    /// Squares the mover cannot end in (any other creature's space).
    pub occupied: HashSet<Pos>,
    pub ignore_difficult_terrain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reach {
    pub cost_ft: u32,
    state: (Pos, u8),
}

/// Result of a movement search; supports path reconstruction.
#[derive(Debug, Clone, Default)]
pub struct MoveMap {
    pub start: Pos,
    /// Ordered: `destinations()` iterates it, and its order is observable.
    best: BTreeMap<Pos, Reach>,
    /// Lookup-only (path reconstruction), so a hash map is safe.
    prev: GridMap<(Pos, u8), (Pos, u8)>,
}

impl MoveMap {
    /// All squares the mover may end its movement in, with their cost.
    pub fn destinations(&self) -> impl Iterator<Item = (Pos, u32)> + '_ {
        self.best.iter().map(|(p, r)| (*p, r.cost_ft))
    }

    pub fn cost_to(&self, p: Pos) -> Option<u32> {
        self.best.get(&p).map(|r| r.cost_ft)
    }

    /// Squares visited from (excluding) start to (including) `dest`.
    pub fn path_to(&self, dest: Pos) -> Option<Vec<Pos>> {
        let mut state = self.best.get(&dest)?.state;
        let mut path = vec![state.0];
        while let Some(&p) = self.prev.get(&state) {
            if p.0 == self.start && p.1 == 0 {
                break;
            }
            path.push(p.0);
            state = p;
        }
        path.reverse();
        if path.first() == Some(&self.start) {
            path.remove(0);
        }
        Some(path)
    }
}

/// The squares of a space without allocating (hot path of the pathfinder).
fn space(pos: Pos, size: Size) -> impl Iterator<Item = Pos> {
    let s = size.squares();
    (0..s).flat_map(move |dy| (0..s).map(move |dx| Pos::new(pos.x + dx, pos.y + dy)))
}

fn step_extra_cost(bf: &Battlefield, pos: Pos, size: Size, ignore_difficult: bool) -> Option<u32> {
    let mut extra = 0;
    for sq in space(pos, size) {
        let e = bf.terrain(sq).extra_cost()?;
        extra = extra.max(e);
    }
    Some(if ignore_difficult { 0 } else { extra })
}

/// Dijkstra over (square, diagonal parity). Deterministic: ties break on
/// coordinates via the ordered heap; the hash maps are lookup-only.
pub fn movement_map(bf: &Battlefield, start: Pos, rules: &MoveRules) -> MoveMap {
    let mut map = MoveMap { start, ..Default::default() };
    let mut dist: GridMap<(Pos, u8), u32> = GridMap::default();
    let mut heap = BinaryHeap::new();
    dist.insert((start, 0), 0);
    heap.push(Reverse((0u32, start.y, start.x, 0u8)));

    while let Some(Reverse((cost, y, x, parity))) = heap.pop() {
        let pos = Pos::new(x, y);
        if dist.get(&(pos, parity)).copied() != Some(cost) {
            continue;
        }
        if pos != start && space(pos, rules.size).all(|sq| !rules.occupied.contains(&sq)) {
            let better = map.best.get(&pos).map(|r| cost < r.cost_ft).unwrap_or(true);
            if better {
                map.best.insert(pos, Reach { cost_ft: cost, state: (pos, parity) });
            }
        }
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let next = Pos::new(x + dx, y + dy);
            if space(next, rules.size).any(|sq| !bf.in_bounds(sq) || rules.impassable.contains(&sq)) {
                continue;
            }
            let Some(extra) = step_extra_cost(bf, next, rules.size, rules.ignore_difficult_terrain) else {
                continue;
            };
            // Thin walls: every square of the mover's space must cross freely.
            if !bf.edges.is_empty() && space(pos, rules.size).any(|sq| bf.edge_blocks_step(sq, Pos::new(sq.x + dx, sq.y + dy))) {
                continue;
            }
            let diagonal = dx != 0 && dy != 0;
            // Moving diagonally past a wall corner is not allowed.
            if diagonal
                && (bf.terrain(Pos::new(x + dx, y)) == Terrain::Wall || bf.terrain(Pos::new(x, y + dy)) == Terrain::Wall)
                && rules.size.squares() == 1
            {
                continue;
            }
            let (base, np) = if diagonal {
                if parity == 0 {
                    (5, 1)
                } else {
                    (10, 0)
                }
            } else {
                (5, parity)
            };
            let nc = cost + base + extra;
            if nc > rules.speed_ft {
                continue;
            }
            let key = (next, np);
            if dist.get(&key).map(|&d| nc < d).unwrap_or(true) {
                dist.insert(key, nc);
                map.prev.insert(key, (pos, parity));
                heap.push(Reverse((nc, next.y, next.x, np)));
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Line of effect & cover
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverLevel {
    #[default]
    None,
    Lesser,
    Standard,
    Greater,
}

impl CoverLevel {
    pub fn ac_bonus(self) -> i32 {
        match self {
            CoverLevel::None => 0,
            CoverLevel::Lesser => 1,
            CoverLevel::Standard => 2,
            CoverLevel::Greater => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverInfo {
    pub line_of_effect: bool,
    pub cover: CoverLevel,
}

/// Squares crossed by a segment (sampled; deterministic).
pub fn segment_cells(a: (f64, f64), b: (f64, f64)) -> Vec<Pos> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let steps = ((len / 0.05).ceil() as usize).max(1);
    let mut out: Vec<Pos> = Vec::new();
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let p = Pos::new((a.0 + dx * t).floor() as i32, (a.1 + dy * t).floor() as i32);
        if out.last() != Some(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Whether consecutive squares of a sampled segment cross a thin wall.
/// A diagonal hop (the segment passing a square corner) uses the same
/// conservative corner rule as movement.
pub fn cells_cross_edge(bf: &Battlefield, cells: &[Pos]) -> bool {
    !bf.edges.is_empty() && cells.windows(2).any(|w| bf.edge_blocks_step(w[0], w[1]))
}

/// Cover per the core rule: draw a line between the centers of the two
/// spaces. Walls on it → standard cover; other creatures → lesser cover.
/// No line of effect only if the center line and all four corner lines are
/// blocked by walls.
pub fn compute_cover(
    bf: &Battlefield,
    attacker: Pos,
    attacker_size: Size,
    target: Pos,
    target_size: Size,
    other_creature_squares: &HashSet<Pos>,
) -> CoverInfo {
    let own: HashSet<Pos> = footprint(attacker, attacker_size).into_iter().chain(footprint(target, target_size)).collect();
    let from = center(attacker, attacker_size);
    let to = center(target, target_size);
    let s = target_size.squares() as f64;
    let inset = 0.1;
    let corners = [
        (target.x as f64 + inset, target.y as f64 + inset),
        (target.x as f64 + s - inset, target.y as f64 + inset),
        (target.x as f64 + inset, target.y as f64 + s - inset),
        (target.x as f64 + s - inset, target.y as f64 + s - inset),
    ];
    let wall_blocked = |end: (f64, f64)| {
        let cells = segment_cells(from, end);
        cells.iter().any(|c| !own.contains(c) && bf.terrain(*c) == Terrain::Wall) || cells_cross_edge(bf, &cells)
    };
    let center_blocked = wall_blocked(to);
    let all_blocked = center_blocked && corners.iter().all(|c| wall_blocked(*c));
    if all_blocked {
        return CoverInfo { line_of_effect: false, cover: CoverLevel::Greater };
    }
    let cover = if center_blocked {
        CoverLevel::Standard
    } else if segment_cells(from, to).iter().any(|c| !own.contains(c) && other_creature_squares.contains(c)) {
        CoverLevel::Lesser
    } else {
        CoverLevel::None
    };
    CoverInfo { line_of_effect: true, cover }
}

/// Flanking: the line between the centers of attacker and ally passes
/// through opposite sides or corners of the target's space.
pub fn flanks(attacker: Pos, asz: Size, ally: Pos, bsz: Size, target: Pos, tsz: Size) -> bool {
    let a = center(attacker, asz);
    let b = center(ally, bsz);
    let s = tsz.squares() as f64;
    let (x0, y0, x1, y1) = (target.x as f64, target.y as f64, target.x as f64 + s, target.y as f64 + s);
    let inside = |p: (f64, f64)| p.0 > x0 && p.0 < x1 && p.1 > y0 && p.1 < y1;
    if inside(a) || inside(b) {
        return false;
    }
    // Liang–Barsky: does segment a→b pass through the open rectangle?
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let eps = 1e-6;
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [(-dx, a.0 - x0 - eps), (dx, x1 - a.0 - eps), (-dy, a.1 - y0 - eps), (dy, y1 - a.1 - eps)] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    t0 < t1
}

// ---------------------------------------------------------------------------
// Areas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "shape", content = "size_ft")]
pub enum AreaShape {
    /// Centered on a grid intersection (the top-left corner of the chosen square).
    Burst(u32),
    /// Radiates from the caster's space.
    Emanation(u32),
    /// From the caster toward the chosen square.
    Cone(u32),
    Line(u32),
}

pub fn area_squares(bf: &Battlefield, shape: AreaShape, origin: Pos, origin_size: Size, aim: Pos) -> Vec<Pos> {
    let mut out = Vec::new();
    let in_area = |p: Pos| -> bool {
        match shape {
            AreaShape::Burst(r) => {
                let dxs = if p.x >= aim.x { p.x - aim.x + 1 } else { aim.x - p.x };
                let dys = if p.y >= aim.y { p.y - aim.y + 1 } else { aim.y - p.y };
                distance_ft(dxs, dys) <= r
            }
            AreaShape::Emanation(r) => creature_distance(origin, origin_size, p, Size::Medium) <= r,
            AreaShape::Cone(len) => {
                let o = center(origin, origin_size);
                let t = (aim.x as f64 + 0.5, aim.y as f64 + 0.5);
                let c = (p.x as f64 + 0.5, p.y as f64 + 0.5);
                let (vx, vy) = (t.0 - o.0, t.1 - o.1);
                let (wx, wy) = (c.0 - o.0, c.1 - o.1);
                let (vl, wl) = ((vx * vx + vy * vy).sqrt(), (wx * wx + wy * wy).sqrt());
                if vl < 1e-9 || wl < 1e-9 {
                    return false;
                }
                let cos = (vx * wx + vy * wy) / (vl * wl);
                cos >= (std::f64::consts::FRAC_PI_4).cos() - 1e-9
                    && creature_distance(origin, origin_size, p, Size::Medium) <= len
                    && !footprint(origin, origin_size).contains(&p)
            }
            AreaShape::Line(len) => {
                let o = center(origin, origin_size);
                let t = (aim.x as f64 + 0.5, aim.y as f64 + 0.5);
                let (vx, vy) = (t.0 - o.0, t.1 - o.1);
                let vl = (vx * vx + vy * vy).sqrt().max(1e-9);
                let squares = len as f64 / 5.0;
                let end = (o.0 + vx / vl * squares, o.1 + vy / vl * squares);
                segment_cells(o, end).contains(&p) && !footprint(origin, origin_size).contains(&p)
            }
        }
    };
    for y in 0..bf.height as i32 {
        for x in 0..bf.width as i32 {
            let p = Pos::new(x, y);
            if bf.terrain(p) != Terrain::Wall && in_area(p) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_distance() {
        assert_eq!(distance_ft(1, 1), 5);
        assert_eq!(distance_ft(2, 2), 15);
        assert_eq!(distance_ft(3, 3), 20);
        assert_eq!(distance_ft(6, 0), 30);
    }

    #[test]
    fn reach_ten_diagonal_exception() {
        let a = Pos::new(0, 0);
        let b = Pos::new(2, 2);
        assert!(!within_reach(a, Size::Medium, b, Size::Medium, 5));
        assert!(within_reach(a, Size::Medium, b, Size::Medium, 10));
        assert!(within_reach(a, Size::Medium, Pos::new(1, 1), Size::Medium, 5));
        // Large creature adjacency.
        assert!(within_reach(Pos::new(0, 0), Size::Large, Pos::new(2, 1), Size::Medium, 5));
    }

    #[test]
    fn movement_respects_terrain_and_diagonals() {
        let mut bf = Battlefield::open(10, 10);
        let rules = MoveRules { speed_ft: 25, size: Size::Medium, ..Default::default() };
        let m = movement_map(&bf, Pos::new(0, 0), &rules);
        assert_eq!(m.cost_to(Pos::new(5, 0)), Some(25));
        assert_eq!(m.cost_to(Pos::new(2, 2)), Some(15));
        assert_eq!(m.cost_to(Pos::new(6, 0)), None);
        bf.set_terrain(Pos::new(1, 0), Terrain::Difficult);
        let m = movement_map(&bf, Pos::new(0, 0), &rules);
        assert_eq!(m.cost_to(Pos::new(1, 0)), Some(10));
        let path = m.path_to(Pos::new(3, 0)).unwrap();
        assert_eq!(*path.last().unwrap(), Pos::new(3, 0));
    }

    #[test]
    fn walls_block_and_give_cover() {
        let mut bf = Battlefield::open(10, 10);
        for y in 0..10 {
            bf.set_terrain(Pos::new(5, y), Terrain::Wall);
        }
        let none = HashSet::new();
        let info = compute_cover(&bf, Pos::new(2, 5), Size::Medium, Pos::new(8, 5), Size::Medium, &none);
        assert!(!info.line_of_effect);
        let open = Battlefield::open(10, 10);
        let info = compute_cover(&open, Pos::new(2, 5), Size::Medium, Pos::new(8, 5), Size::Medium, &none);
        assert_eq!(info.cover, CoverLevel::None);
        let blockers: HashSet<Pos> = [Pos::new(5, 5)].into_iter().collect();
        let info = compute_cover(&open, Pos::new(2, 5), Size::Medium, Pos::new(8, 5), Size::Medium, &blockers);
        assert_eq!(info.cover, CoverLevel::Lesser);
    }

    #[test]
    fn thin_walls_block_movement_and_line_of_effect() {
        use crate::models::battlefield::Edge;
        // A thin wall along x = 5 (east borders of column 4), rows 0..10, with
        // a one-square door gap at y = 9.
        let mut bf = Battlefield::open(10, 10);
        for y in 0..9 {
            bf.edges.insert(Edge::east(4, y));
        }
        bf.validate().unwrap();
        let rules = MoveRules { speed_ft: 120, size: Size::Medium, ..Default::default() };
        let m = movement_map(&bf, Pos::new(4, 0), &rules);
        // Must walk down to the gap (y = 9) and back up: 45 + 5 + 45 ft.
        assert_eq!(m.cost_to(Pos::new(5, 0)), Some(95));
        // Squares beside the wall stay usable: no corridor is lost.
        assert_eq!(m.cost_to(Pos::new(4, 1)), Some(5));
        let none = HashSet::new();
        let info = compute_cover(&bf, Pos::new(4, 3), Size::Medium, Pos::new(5, 3), Size::Medium, &none);
        assert!(!info.line_of_effect, "adjacent creatures across a wall have no line of effect");
        // Through the gap there is a clear line.
        let info = compute_cover(&bf, Pos::new(4, 9), Size::Medium, Pos::new(6, 9), Size::Medium, &none);
        assert!(info.line_of_effect);
        assert_eq!(info.cover, CoverLevel::None);
    }

    #[test]
    fn thin_wall_corners_block_diagonals() {
        use crate::models::battlefield::Edge;
        let mut bf = Battlefield::open(4, 4);
        bf.edges.insert(Edge::east(0, 0));
        assert!(bf.edge_blocks_step(Pos::new(0, 1), Pos::new(1, 0)));
        assert!(bf.edge_blocks_step(Pos::new(0, 0), Pos::new(1, 1)));
        assert!(!bf.edge_blocks_step(Pos::new(0, 2), Pos::new(1, 3)));
    }

    #[test]
    fn empty_edges_keep_serialized_form() {
        let bf = Battlefield::open(2, 2);
        let json = serde_json::to_string(&bf).unwrap();
        assert!(!json.contains("edges"), "{json}");
        let back: Battlefield = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bf);
    }

    #[test]
    fn flanking_geometry() {
        let t = Pos::new(5, 5);
        assert!(flanks(Pos::new(4, 5), Size::Medium, Pos::new(6, 5), Size::Medium, t, Size::Medium));
        assert!(flanks(Pos::new(4, 4), Size::Medium, Pos::new(6, 6), Size::Medium, t, Size::Medium));
        assert!(!flanks(Pos::new(4, 5), Size::Medium, Pos::new(5, 4), Size::Medium, t, Size::Medium));
    }

    #[test]
    fn burst_shapes() {
        let bf = Battlefield::open(20, 20);
        let five = area_squares(&bf, AreaShape::Burst(5), Pos::new(0, 0), Size::Medium, Pos::new(10, 10));
        assert_eq!(five.len(), 4);
        let ten = area_squares(&bf, AreaShape::Burst(10), Pos::new(0, 0), Size::Medium, Pos::new(10, 10));
        // 4x4 minus the four corners (which are 15 ft away).
        assert_eq!(ten.len(), 12);
    }
}
