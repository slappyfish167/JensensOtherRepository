//! The battle map: a rectangular grid of 5-foot squares, plus optional
//! *thin walls* on the edges between squares (as drawn in a VTT).

use crate::grid::Pos;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which border of square `(x, y)` an [`Edge`] lies on. Only east and south
/// are needed: the west border of `(x, y)` is the east border of `(x-1, y)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeSide {
    East,
    South,
}

/// A blocked border between two orthogonally adjacent squares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Edge {
    pub x: i32,
    pub y: i32,
    pub side: EdgeSide,
}

impl Edge {
    pub const fn east(x: i32, y: i32) -> Self {
        Edge { x, y, side: EdgeSide::East }
    }
    pub const fn south(x: i32, y: i32) -> Self {
        Edge { x, y, side: EdgeSide::South }
    }

    /// The edge shared by two orthogonally adjacent squares.
    pub fn between(a: Pos, b: Pos) -> Option<Edge> {
        match (b.x - a.x, b.y - a.y) {
            (1, 0) => Some(Edge::east(a.x, a.y)),
            (-1, 0) => Some(Edge::east(b.x, b.y)),
            (0, 1) => Some(Edge::south(a.x, a.y)),
            (0, -1) => Some(Edge::south(b.x, b.y)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    Open,
    Difficult,
    GreaterDifficult,
    /// Blocks movement and line of effect; grants standard cover.
    Wall,
    /// Fog, foliage, smoke: creatures standing inside are concealed.
    Obscured,
}

impl Terrain {
    pub fn from_glyph(c: char) -> Option<Terrain> {
        Some(match c {
            '.' => Terrain::Open,
            '~' => Terrain::Difficult,
            '^' => Terrain::GreaterDifficult,
            '#' => Terrain::Wall,
            '%' => Terrain::Obscured,
            _ => return None,
        })
    }

    pub fn glyph(self) -> char {
        match self {
            Terrain::Open => '.',
            Terrain::Difficult => '~',
            Terrain::GreaterDifficult => '^',
            Terrain::Wall => '#',
            Terrain::Obscured => '%',
        }
    }

    /// Extra movement cost in feet for entering a square of this terrain.
    pub fn extra_cost(self) -> Option<u32> {
        match self {
            Terrain::Open | Terrain::Obscured => Some(0),
            Terrain::Difficult => Some(5),
            Terrain::GreaterDifficult => Some(10),
            Terrain::Wall => None,
        }
    }
}

/// Map rows use one glyph per square: `.` open, `~` difficult, `^` greater
/// difficult, `#` wall, `%` obscured. Row 0 is the top (y = 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battlefield {
    pub width: u32,
    pub height: u32,
    pub map: Vec<String>,
    #[serde(default)]
    pub notes: String,
    /// Thin walls between squares. They block movement and line of effect
    /// exactly like a wall square, but occupy no space. Omitted from the
    /// serialized form when empty, so maps without them (and the
    /// fingerprints of stored combats) are unchanged.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub edges: BTreeSet<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BattlefieldError {
    #[error("map has {rows} rows but height is {height}")]
    RowCount { rows: usize, height: u32 },
    #[error("row {row} has {len} squares but width is {width}")]
    RowWidth { row: usize, len: usize, width: u32 },
    #[error("unknown terrain glyph '{glyph}' at ({x},{y})")]
    Glyph { glyph: char, x: usize, y: usize },
    #[error("battlefield must be between 1x1 and 200x200")]
    Size,
    #[error("wall edge at ({x},{y}) lies outside the map")]
    EdgeOutOfBounds { x: i32, y: i32 },
}

impl Battlefield {
    pub fn open(width: u32, height: u32) -> Self {
        Battlefield {
            width,
            height,
            map: (0..height).map(|_| ".".repeat(width as usize)).collect(),
            notes: String::new(),
            edges: BTreeSet::new(),
        }
    }

    pub fn validate(&self) -> Result<(), BattlefieldError> {
        if self.width == 0 || self.height == 0 || self.width > 200 || self.height > 200 {
            return Err(BattlefieldError::Size);
        }
        if self.map.len() != self.height as usize {
            return Err(BattlefieldError::RowCount { rows: self.map.len(), height: self.height });
        }
        for (y, row) in self.map.iter().enumerate() {
            let len = row.chars().count();
            if len != self.width as usize {
                return Err(BattlefieldError::RowWidth { row: y, len, width: self.width });
            }
            for (x, c) in row.chars().enumerate() {
                if Terrain::from_glyph(c).is_none() {
                    return Err(BattlefieldError::Glyph { glyph: c, x, y });
                }
            }
        }
        if let Some(e) = self.edges.iter().find(|e| !self.in_bounds(Pos::new(e.x, e.y))) {
            return Err(BattlefieldError::EdgeOutOfBounds { x: e.x, y: e.y });
        }
        Ok(())
    }

    pub fn has_edge(&self, e: Edge) -> bool {
        !self.edges.is_empty() && self.edges.contains(&e)
    }

    /// Whether a single-square step from `from` to the neighbouring square
    /// `to` crosses a thin wall. Diagonal steps are blocked by any wall edge
    /// touching the shared corner (no squeezing through wall corners, the
    /// same rule the engine applies to wall squares).
    pub fn edge_blocks_step(&self, from: Pos, to: Pos) -> bool {
        if self.edges.is_empty() {
            return false;
        }
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        if dx == 0 || dy == 0 {
            return Edge::between(from, to).map(|e| self.edges.contains(&e)).unwrap_or(false);
        }
        let side_x = Pos::new(to.x, from.y);
        let side_y = Pos::new(from.x, to.y);
        [(from, side_x), (from, side_y), (side_x, to), (side_y, to)]
            .into_iter()
            .filter_map(|(a, b)| Edge::between(a, b))
            .any(|e| self.edges.contains(&e))
    }

    pub fn in_bounds(&self, p: Pos) -> bool {
        p.x >= 0 && p.y >= 0 && (p.x as u32) < self.width && (p.y as u32) < self.height
    }

    /// Out-of-bounds squares behave as walls.
    pub fn terrain(&self, p: Pos) -> Terrain {
        if !self.in_bounds(p) {
            return Terrain::Wall;
        }
        let Some(row) = self.map.get(p.y as usize) else { return Terrain::Wall };
        // Every terrain glyph is ASCII, so a byte index is the square
        // (O(1); this is the pathfinder's and cover sampler's hot path).
        // Non-ASCII rows only exist in unvalidated maps: fall back to chars.
        let glyph = if row.is_ascii() { row.as_bytes().get(p.x as usize).map(|b| *b as char) } else { row.chars().nth(p.x as usize) };
        glyph.and_then(Terrain::from_glyph).unwrap_or(Terrain::Wall)
    }

    pub fn set_terrain(&mut self, p: Pos, t: Terrain) {
        if !self.in_bounds(p) {
            return;
        }
        let row = &mut self.map[p.y as usize];
        let mut chars: Vec<char> = row.chars().collect();
        chars[p.x as usize] = t.glyph();
        *row = chars.into_iter().collect();
    }
}
