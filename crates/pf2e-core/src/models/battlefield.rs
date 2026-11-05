//! The battle map: a rectangular grid of 5-foot squares.

use crate::grid::Pos;
use serde::{Deserialize, Serialize};

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
}

impl Battlefield {
    pub fn open(width: u32, height: u32) -> Self {
        Battlefield {
            width,
            height,
            map: (0..height).map(|_| ".".repeat(width as usize)).collect(),
            notes: String::new(),
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
        Ok(())
    }

    pub fn in_bounds(&self, p: Pos) -> bool {
        p.x >= 0 && p.y >= 0 && (p.x as u32) < self.width && (p.y as u32) < self.height
    }

    /// Out-of-bounds squares behave as walls.
    pub fn terrain(&self, p: Pos) -> Terrain {
        if !self.in_bounds(p) {
            return Terrain::Wall;
        }
        self.map
            .get(p.y as usize)
            .and_then(|row| row.chars().nth(p.x as usize))
            .and_then(Terrain::from_glyph)
            .unwrap_or(Terrain::Wall)
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
