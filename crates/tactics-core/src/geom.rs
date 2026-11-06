use serde::{Deserialize, Serialize};

/// A grid square. For multi-square creatures, the top-left square of its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32) -> Self {
        Pos { x, y }
    }

    /// Squares between two positions when every diagonal costs one square.
    pub fn chebyshev(self, other: Pos) -> u32 {
        (self.x - other.x).unsigned_abs().max((self.y - other.y).unsigned_abs())
    }

    /// The eight surrounding squares.
    pub fn neighbours(self) -> impl Iterator<Item = Pos> {
        (-1..=1).flat_map(move |dy| (-1..=1).map(move |dx| (dx, dy))).filter(|&(dx, dy)| dx != 0 || dy != 0).map(move |(dx, dy)| Pos::new(self.x + dx, self.y + dy))
    }
}

/// Squares crossed by the straight line between the centres of `a` and `b`,
/// excluding both endpoints (supercover-free sampling, deterministic).
pub fn line_squares(a: Pos, b: Pos) -> Vec<Pos> {
    let (ax, ay) = (a.x as f64 + 0.5, a.y as f64 + 0.5);
    let (bx, by) = (b.x as f64 + 0.5, b.y as f64 + 0.5);
    let steps = (((bx - ax).abs().max((by - ay).abs())) * 4.0).ceil().max(1.0) as usize;
    let mut out: Vec<Pos> = Vec::new();
    for i in 1..steps {
        let t = i as f64 / steps as f64;
        let p = Pos::new((ax + (bx - ax) * t).floor() as i32, (ay + (by - ay) * t).floor() as i32);
        if p != a && p != b && out.last() != Some(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chebyshev_and_lines() {
        assert_eq!(Pos::new(0, 0).chebyshev(Pos::new(3, -2)), 3);
        assert_eq!(line_squares(Pos::new(0, 0), Pos::new(3, 0)), vec![Pos::new(1, 0), Pos::new(2, 0)]);
        assert!(line_squares(Pos::new(0, 0), Pos::new(1, 1)).is_empty());
        assert_eq!(Pos::new(5, 5).neighbours().count(), 8);
    }
}
