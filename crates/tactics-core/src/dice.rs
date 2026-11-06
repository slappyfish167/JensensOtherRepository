//! Dice expressions such as `1d20+14` or `2d6+1d4+3`.
//!
//! The engine never rolls these. It only *describes* rolls (so a human can roll
//! them) and computes statistics (average, min, max) that the AI layer uses for
//! expected-value reasoning.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiceTerm {
    pub count: u32,
    pub sides: u32,
}

/// A sum of dice terms plus a flat modifier. Serialized as a compact string
/// (`"2d8+4"`) so content files stay human-editable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DiceExpr {
    pub terms: Vec<DiceTerm>,
    pub modifier: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid dice expression '{input}': {reason}")]
pub struct DiceParseError {
    pub input: String,
    pub reason: String,
}

impl DiceExpr {
    pub fn new(count: u32, sides: u32, modifier: i32) -> Self {
        let mut e = DiceExpr { terms: Vec::new(), modifier };
        if count > 0 {
            e.terms.push(DiceTerm { count, sides });
        }
        e
    }

    pub fn flat(modifier: i32) -> Self {
        DiceExpr { terms: Vec::new(), modifier }
    }

    pub fn d20(modifier: i32) -> Self {
        Self::new(1, 20, modifier)
    }

    pub fn is_flat(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn dice_count(&self) -> u32 {
        self.terms.iter().map(|t| t.count).sum()
    }

    pub fn average(&self) -> f64 {
        self.terms
            .iter()
            .map(|t| t.count as f64 * (t.sides as f64 + 1.0) / 2.0)
            .sum::<f64>()
            + self.modifier as f64
    }

    pub fn min(&self) -> i32 {
        self.terms.iter().map(|t| t.count as i32).sum::<i32>() + self.modifier
    }

    pub fn max(&self) -> i32 {
        self.terms.iter().map(|t| (t.count * t.sides) as i32).sum::<i32>() + self.modifier
    }

    /// A human-entered total is plausible if it lies in `[min, max]`.
    pub fn accepts_total(&self, total: i32) -> bool {
        total >= self.min() && total <= self.max()
    }

    /// Adds dice of the same size together (`1d6 + 1d6 = 2d6`).
    pub fn plus(&self, other: &DiceExpr) -> DiceExpr {
        let mut out = self.clone();
        for t in &other.terms {
            out.add_dice(t.count, t.sides);
        }
        out.modifier += other.modifier;
        out
    }

    pub fn add_dice(&mut self, count: u32, sides: u32) {
        if count == 0 {
            return;
        }
        if let Some(existing) = self.terms.iter_mut().find(|t| t.sides == sides) {
            existing.count += count;
        } else {
            self.terms.push(DiceTerm { count, sides });
        }
    }

    pub fn with_modifier(&self, delta: i32) -> DiceExpr {
        let mut out = self.clone();
        out.modifier += delta;
        out
    }

    /// Multiplies the dice count (used for heightening: `2d6` × 3 = `6d6`).
    pub fn times(&self, n: u32) -> DiceExpr {
        DiceExpr {
            terms: self
                .terms
                .iter()
                .map(|t| DiceTerm { count: t.count * n, sides: t.sides })
                .collect(),
            modifier: self.modifier * n as i32,
        }
    }

    /// Replaces every die size with `sides` (the fatal weapon trait).
    pub fn with_die_size(&self, sides: u32) -> DiceExpr {
        let mut out = DiceExpr::flat(self.modifier);
        for t in &self.terms {
            out.add_dice(t.count, sides);
        }
        out
    }
}

impl fmt::Display for DiceExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for t in &self.terms {
            if !first {
                write!(f, "+")?;
            }
            write!(f, "{}d{}", t.count, t.sides)?;
            first = false;
        }
        if self.modifier != 0 || first {
            if first {
                write!(f, "{}", self.modifier)?;
            } else if self.modifier > 0 {
                write!(f, "+{}", self.modifier)?;
            } else {
                write!(f, "{}", self.modifier)?;
            }
        }
        Ok(())
    }
}

impl FromStr for DiceExpr {
    type Err = DiceParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let err = |reason: &str| DiceParseError { input: input.to_string(), reason: reason.to_string() };
        let s: String = input.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
        if s.is_empty() {
            return Err(err("empty expression"));
        }
        let mut expr = DiceExpr::default();
        // Split into signed tokens.
        let mut tokens: Vec<(i32, String)> = Vec::new();
        let mut sign = 1;
        let mut cur = String::new();
        for c in s.chars() {
            match c {
                '+' | '-' => {
                    if !cur.is_empty() {
                        tokens.push((sign, std::mem::take(&mut cur)));
                    } else if (!tokens.is_empty() || c == '+')
                        && !tokens.is_empty() {
                            return Err(err("consecutive operators"));
                        }
                    sign = if c == '-' { -1 } else { 1 };
                }
                _ => cur.push(c),
            }
        }
        if cur.is_empty() {
            return Err(err("trailing operator"));
        }
        tokens.push((sign, cur));

        for (sign, tok) in tokens {
            if let Some((count, sides)) = tok.split_once('d') {
                if sign < 0 {
                    return Err(err("negative dice are not supported"));
                }
                let count: u32 = if count.is_empty() { 1 } else { count.parse().map_err(|_| err("bad dice count"))? };
                let sides: u32 = sides.parse().map_err(|_| err("bad die size"))?;
                if sides == 0 || count > 1000 || sides > 1000 {
                    return Err(err("die size/count out of range"));
                }
                expr.add_dice(count, sides);
            } else {
                let v: i32 = tok.parse().map_err(|_| err("bad number"))?;
                expr.modifier += sign * v;
            }
        }
        Ok(expr)
    }
}

impl TryFrom<String> for DiceExpr {
    type Error = DiceParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<DiceExpr> for String {
    fn from(value: DiceExpr) -> Self {
        value.to_string()
    }
}

/// Parses a die size such as `"d10"` or `"10"`.
pub fn parse_die_size(s: &str) -> Option<u32> {
    s.trim_start_matches('d').parse().ok().filter(|n: &u32| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let e: DiceExpr = "2d8+1d6+4".parse().unwrap();
        assert_eq!(e.to_string(), "2d8+1d6+4");
        assert_eq!(e.min(), 7);
        assert_eq!(e.max(), 26);
        assert!((e.average() - 16.5).abs() < 1e-9);
        let n: DiceExpr = "1d20-2".parse().unwrap();
        assert_eq!(n.to_string(), "1d20-2");
        let f: DiceExpr = "5".parse().unwrap();
        assert_eq!(f.to_string(), "5");
        assert_eq!("-3".parse::<DiceExpr>().unwrap().modifier, -3);
    }

    #[test]
    fn rejects_garbage() {
        assert!("".parse::<DiceExpr>().is_err());
        assert!("2d".parse::<DiceExpr>().is_err());
        assert!("1d6+".parse::<DiceExpr>().is_err());
        assert!("-1d6".parse::<DiceExpr>().is_err());
    }

    #[test]
    fn combines_dice() {
        let a: DiceExpr = "1d6+2".parse().unwrap();
        let b: DiceExpr = "2d6+1".parse().unwrap();
        assert_eq!(a.plus(&b).to_string(), "3d6+3");
        assert_eq!(a.times(3).to_string(), "3d6+6");
        assert_eq!(b.with_die_size(10).to_string(), "2d10+1");
    }

    #[test]
    fn serde_roundtrip() {
        let e: DiceExpr = "3d6+2".parse().unwrap();
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "\"3d6+2\"");
        let back: DiceExpr = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }
}
