//! Scene geometry → engine battlefield: cropping and wall rasterization.
//!
//! Foundry walls are arbitrary segments; the engine works on 5-foot squares
//! with optional thin walls on square edges (`Battlefield::edges`). An edge
//! between two orthogonally adjacent squares is blocked when the segment
//! joining the two square **centres** crosses a wall. That rule is exact for
//! grid-aligned walls (the common case: corridors keep their width, door
//! gaps stay open) and turns diagonal walls into closed staircases.

use crate::snapshot::{CombatantSnapshot, WallSnapshot};
use pf2e_core::models::battlefield::Edge;
use std::collections::BTreeSet;

/// The scene window analysed by the engine, in scene squares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Crop {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Crop {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width as i32 && y < self.y + self.height as i32
    }
}

/// Largest map the engine accepts per side (`Battlefield::validate`).
pub const MAX_SIDE: u32 = 200;

/// Bounding box of all combatants plus `margin`, clipped to the scene.
/// Errors if the combatants alone need more than [`MAX_SIDE`] squares.
pub fn crop_for(combatants: &[CombatantSnapshot], scene_w: u32, scene_h: u32, margin: u32) -> Result<Crop, String> {
    if combatants.is_empty() {
        return Err("no combatants".into());
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for c in combatants {
        let s = c.size.squares();
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x + s);
        y1 = y1.max(c.y + s);
    }
    let span = ((x1 - x0) as u32).max((y1 - y0) as u32);
    if span > MAX_SIDE {
        return Err(format!("combatants are spread over {span} squares; the engine analyses at most {MAX_SIDE}"));
    }
    let margin = margin.min((MAX_SIDE - span) / 2) as i32;
    let (sw, sh) = (scene_w.max(1) as i32, scene_h.max(1) as i32);
    let cx0 = (x0 - margin).max(0).min(x0);
    let cy0 = (y0 - margin).max(0).min(y0);
    let cx1 = (x1 + margin).min(sw).max(x1);
    let cy1 = (y1 + margin).min(sh).max(y1);
    Ok(Crop { x: cx0, y: cy0, width: (cx1 - cx0) as u32, height: (cy1 - cy0) as u32 })
}

type P = (f64, f64);

fn orient(a: P, b: P, c: P) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn on_segment(a: P, b: P, p: P) -> bool {
    p.0 >= a.0.min(b.0) - 1e-9 && p.0 <= a.0.max(b.0) + 1e-9 && p.1 >= a.1.min(b.1) - 1e-9 && p.1 <= a.1.max(b.1) + 1e-9
}

/// Closed segment intersection (touching counts).
pub fn segments_intersect(p1: P, p2: P, q1: P, q2: P) -> bool {
    let eps = 1e-9;
    let d1 = orient(q1, q2, p1);
    let d2 = orient(q1, q2, p2);
    let d3 = orient(p1, p2, q1);
    let d4 = orient(p1, p2, q2);
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps)) && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps)) {
        return true;
    }
    (d1.abs() <= eps && on_segment(q1, q2, p1))
        || (d2.abs() <= eps && on_segment(q1, q2, p2))
        || (d3.abs() <= eps && on_segment(p1, p2, q1))
        || (d4.abs() <= eps && on_segment(p1, p2, q2))
}

/// Blocked edges inside `crop` (coordinates relative to the crop).
pub fn rasterize(walls: &[WallSnapshot], crop: &Crop) -> BTreeSet<Edge> {
    let mut edges = BTreeSet::new();
    let (w, h) = (crop.width as i32, crop.height as i32);
    for wall in walls.iter().filter(|w| w.blocks_movement && !w.open_door) {
        // Crop-relative wall coordinates.
        let a = (wall.a[0] - crop.x as f64, wall.a[1] - crop.y as f64);
        let b = (wall.b[0] - crop.x as f64, wall.b[1] - crop.y as f64);
        if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
            continue;
        }
        // Squares whose centre-to-neighbour segments could touch the wall.
        let x0 = (a.0.min(b.0).floor() as i32 - 1).max(0);
        let x1 = (a.0.max(b.0).ceil() as i32 + 1).min(w - 1);
        let y0 = (a.1.min(b.1).floor() as i32 - 1).max(0);
        let y1 = (a.1.max(b.1).ceil() as i32 + 1).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = (x as f64 + 0.5, y as f64 + 0.5);
                if x + 1 < w && segments_intersect(c, (c.0 + 1.0, c.1), a, b) {
                    edges.insert(Edge::east(x, y));
                }
                if y + 1 < h && segments_intersect(c, (c.0, c.1 + 1.0), a, b) {
                    edges.insert(Edge::south(x, y));
                }
            }
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use pf2e_core::models::battlefield::Battlefield;

    fn wall(a: [f64; 2], b: [f64; 2]) -> WallSnapshot {
        WallSnapshot { a, b, blocks_movement: true, blocks_sight: true, open_door: false }
    }

    #[test]
    fn grid_aligned_wall_blocks_exactly_its_edges() {
        let crop = Crop { x: 0, y: 0, width: 10, height: 10 };
        // Vertical wall on x = 5 from y = 2 to y = 6.
        let e = rasterize(&[wall([5.0, 2.0], [5.0, 6.0])], &crop);
        let expect: BTreeSet<_> = (2..6).map(|y| Edge::east(4, y)).collect();
        assert_eq!(e, expect);
    }

    #[test]
    fn open_doors_and_sight_only_walls_do_not_block() {
        let crop = Crop { x: 0, y: 0, width: 10, height: 10 };
        let mut door = wall([5.0, 2.0], [5.0, 3.0]);
        door.open_door = true;
        let mut window = wall([1.0, 1.0], [1.0, 4.0]);
        window.blocks_movement = false;
        assert!(rasterize(&[door, window], &crop).is_empty());
    }

    #[test]
    fn diagonal_wall_is_closed() {
        let crop = Crop { x: 0, y: 0, width: 12, height: 12 };
        let mut bf = Battlefield::open(12, 12);
        bf.edges = rasterize(&[wall([1.0, 1.0], [11.0, 11.0])], &crop);
        // No step from strictly above the diagonal to strictly below it.
        for y in 0..12 {
            for x in 0..12 {
                for (dx, dy) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                    let (a, b) = (pf2e_core::grid::Pos::new(x, y), pf2e_core::grid::Pos::new(x + dx, y + dy));
                    let side = |p: pf2e_core::grid::Pos| (p.x as f64 + 0.5) - (p.y as f64 + 0.5);
                    let inside = |p: pf2e_core::grid::Pos| (1..11).contains(&p.x) && (1..11).contains(&p.y);
                    if inside(a) && inside(b) && side(a) * side(b) < 0.0 && b.x < 12 && b.y < 12 && b.x >= 0 {
                        assert!(bf.edge_blocks_step(a, b), "leak from {a:?} to {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn crop_keeps_margin_and_scene_bounds() {
        let c = |x, y| crate::tests::combatant("t", x, y);
        let crop = crop_for(&[c(50, 50), c(60, 52)], 300, 300, 12).unwrap();
        assert_eq!(crop, Crop { x: 38, y: 38, width: 35, height: 27 });
        let edge = crop_for(&[c(1, 1)], 20, 20, 12).unwrap();
        assert_eq!(edge, Crop { x: 0, y: 0, width: 14, height: 14 });
        assert!(crop_for(&[c(0, 0), c(250, 0)], 300, 300, 12).is_err());
    }
}
