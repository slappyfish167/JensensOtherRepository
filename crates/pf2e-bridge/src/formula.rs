//! Parser for PF2e system damage formulas (`DamageRoll` formula strings) into
//! engine damage components.
//!
//! Accepted shapes (as produced by `strike.damage({ getFormula: true })` and
//! spell damage data):
//!
//! ```text
//! 1d8+4                              untyped instance (type from fallback)
//! (1d8 + 4)[slashing]                one typed instance
//! 2d6[fire]                          flavor without parentheses
//! {(1d8 + 4)[slashing],(1d6)[fire]}  several instances (runes, riders)
//! (1d6 + 1d6[precision] + 3)[piercing]   precision part inside an instance
//! (1d6[persistent,fire])             persistent rider (reported, skipped)
//! ```
//!
//! The first component is the weapon's base damage (the engine doubles it on
//! a critical hit and applies deadly/fatal to it); later ones are riders.

use pf2e_core::damage::DamageType;
use pf2e_core::dice::DiceExpr;
use pf2e_core::models::statblock::DamageComponent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("cannot read damage formula '{formula}': {reason}")]
pub struct FormulaError {
    pub formula: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedDamage {
    pub components: Vec<DamageComponent>,
    /// Parts the engine does not model on strikes (persistent riders).
    pub skipped: Vec<String>,
}

pub fn damage_type(name: &str) -> Option<DamageType> {
    use DamageType::*;
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "bludgeoning" => Bludgeoning,
        "piercing" => Piercing,
        "slashing" => Slashing,
        "bleed" => Bleed,
        "acid" => Acid,
        "cold" => Cold,
        "electricity" => Electricity,
        "fire" => Fire,
        "sonic" => Sonic,
        "force" => Force,
        // Remaster names and their legacy equivalents.
        "vitality" | "positive" => Vitality,
        "void" | "negative" => Void,
        "mental" => Mental,
        "poison" => Poison,
        "spirit" | "good" | "evil" | "lawful" | "chaotic" => Spirit,
        "untyped" => Untyped,
        _ => return None,
    })
}

fn err(formula: &str, reason: impl Into<String>) -> FormulaError {
    FormulaError { formula: formula.to_string(), reason: reason.into() }
}

/// Splits on `sep` at nesting depth 0 of (), [], {}.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Index of the bracket that closes the one opening at `open`.
fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `expr[flavor]` or `(expr)[flavor]` → (expr, Some(flavor)); otherwise (s, None).
fn split_flavor(s: &str) -> (&str, Option<&str>) {
    let s = s.trim();
    if !s.ends_with(']') {
        return (s, None);
    }
    // Find the '[' that opens the trailing flavor at depth 0.
    let mut depth = 0i32;
    for (i, c) in s.char_indices().rev() {
        match c {
            ']' | ')' | '}' => depth += 1,
            '[' | '(' | '{' => {
                depth -= 1;
                if depth == 0 {
                    if c != '[' {
                        return (s, None);
                    }
                    let mut expr = s[..i].trim();
                    if expr.starts_with('(') && matching(expr, 0) == Some(expr.len() - 1) {
                        expr = &expr[1..expr.len() - 1];
                    }
                    return (expr, Some(&s[i + 1..s.len() - 1]));
                }
            }
            _ => {}
        }
    }
    (s, None)
}

#[derive(Default)]
struct Flavor {
    damage_type: Option<DamageType>,
    precision: bool,
    persistent: bool,
}

fn read_flavor(f: Option<&str>) -> Flavor {
    let mut out = Flavor::default();
    for tok in f.unwrap_or("").split(',') {
        let tok = tok.trim();
        match tok {
            "precision" => out.precision = true,
            "persistent" => out.persistent = true,
            // Splash damage also hits the target: counted as normal damage.
            "splash" | "" => {}
            t => {
                if let Some(d) = damage_type(t) {
                    out.damage_type = Some(d);
                }
            }
        }
    }
    out
}

/// One additive term: `NdM`, `N`, or a parenthesised sub-expression.
fn add_term(acc: &mut DiceExpr, term: &str, sign: i32, formula: &str) -> Result<(), FormulaError> {
    let t = term.trim();
    if t.is_empty() {
        return Ok(());
    }
    if t.starts_with('(') && matching(t, 0) == Some(t.len() - 1) {
        let mut inner = DiceExpr::default();
        add_expr(&mut inner, &t[1..t.len() - 1], formula)?;
        if sign < 0 && !inner.terms.is_empty() {
            return Err(err(formula, "subtracted dice are not supported"));
        }
        *acc = acc.plus(&if sign < 0 { DiceExpr::flat(-inner.modifier) } else { inner });
        return Ok(());
    }
    if let Ok(n) = t.parse::<i32>() {
        acc.modifier += sign * n;
        return Ok(());
    }
    let (count, sides) = t.split_once(['d', 'D']).ok_or_else(|| err(formula, format!("unexpected term '{t}'")))?;
    let count: u32 = if count.trim().is_empty() { 1 } else { count.trim().parse().map_err(|_| err(formula, format!("bad dice count in '{t}'")))? };
    let sides: u32 = sides.trim().parse().map_err(|_| err(formula, format!("bad die size in '{t}'")))?;
    if sign < 0 {
        return Err(err(formula, "subtracted dice are not supported"));
    }
    if count > 100 || !(1..=100).contains(&sides) {
        return Err(err(formula, format!("implausible dice '{t}'")));
    }
    acc.add_dice(count, sides);
    Ok(())
}

/// Adds a +/- expression (no flavors) into `acc`.
fn add_expr(acc: &mut DiceExpr, expr: &str, formula: &str) -> Result<(), FormulaError> {
    // Normalise binary minus to "+-" at depth 0, then split on '+'.
    let mut norm = String::with_capacity(expr.len() + 4);
    let mut depth = 0i32;
    for c in expr.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == '-' && depth == 0 {
            norm.push_str("+-");
        } else if !c.is_whitespace() {
            norm.push(c);
        }
    }
    for part in split_top(&norm, '+') {
        let (sign, body) = match part.strip_prefix('-') {
            Some(rest) => (-1, rest),
            None => (1, part),
        };
        if body.contains(['*', '/', '@', ']']) || body.chars().any(|c| c.is_ascii_alphabetic() && c != 'd' && c != 'D') {
            return Err(err(formula, format!("unsupported term '{body}'")));
        }
        add_term(acc, body, sign, formula)?;
    }
    Ok(())
}

/// Parses one damage instance into components (main part first, then
/// nested flavored parts such as precision).
fn parse_instance(inst: &str, fallback: Option<DamageType>, formula: &str, out: &mut ParsedDamage) -> Result<(), FormulaError> {
    // Redundant wrapping parentheses: `(1d4[persistent,bleed])`.
    let mut inst = inst.trim();
    while inst.starts_with('(') && matching(inst, 0) == Some(inst.len() - 1) {
        inst = inst[1..inst.len() - 1].trim();
    }
    let (expr, flavor) = split_flavor(inst);
    let flv = read_flavor(flavor);
    if flv.persistent {
        out.skipped.push(format!("persistent {}", inst.trim()));
        return Ok(());
    }
    let damage_type = flv.damage_type.or(fallback).unwrap_or(DamageType::Untyped);
    let mut main = DiceExpr::default();
    let mut nested: Vec<DamageComponent> = Vec::new();
    let mut plain = String::new();
    // Pull out nested flavored terms like `1d6[precision]` or `(1d6)[fire]`.
    let mut norm = String::new();
    let mut depth = 0i32;
    for c in expr.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == '-' && depth == 0 {
            norm.push_str("+-");
        } else if !c.is_whitespace() {
            norm.push(c);
        }
    }
    for part in split_top(&norm, '+') {
        if part.ends_with(']') {
            let (sub, f) = split_flavor(part);
            let sf = read_flavor(f);
            if sf.persistent {
                out.skipped.push(format!("persistent {part}"));
                continue;
            }
            let mut dice = DiceExpr::default();
            add_expr(&mut dice, sub, formula)?;
            nested.push(DamageComponent { dice, damage_type: sf.damage_type.unwrap_or(damage_type), precision: sf.precision || flv.precision });
        } else if !part.is_empty() {
            if !plain.is_empty() {
                plain.push('+');
            }
            plain.push_str(part);
        }
    }
    add_expr(&mut main, &plain.replace("+-", "-"), formula)?;
    if !main.terms.is_empty() || main.modifier != 0 {
        out.components.push(DamageComponent { dice: main, damage_type, precision: flv.precision });
    }
    out.components.extend(nested);
    Ok(())
}

/// Parses a full formula; `fallback` types instances without a flavor.
pub fn parse_damage(formula: &str, fallback: Option<DamageType>) -> Result<ParsedDamage, FormulaError> {
    let f = formula.trim();
    if f.is_empty() {
        return Err(err(formula, "empty formula"));
    }
    if f.len() > 512 {
        return Err(err(formula, "formula too long"));
    }
    let body = if f.starts_with('{') && matching(f, 0) == Some(f.len() - 1) { &f[1..f.len() - 1] } else { f };
    let mut out = ParsedDamage::default();
    for inst in split_top(body, ',') {
        if !inst.trim().is_empty() {
            parse_instance(inst, fallback, formula, &mut out)?;
        }
    }
    if out.components.is_empty() && out.skipped.is_empty() {
        return Err(err(formula, "no damage found"));
    }
    Ok(out)
}

/// A single untyped dice expression such as a persistent-damage formula.
pub fn parse_dice(formula: &str) -> Result<DiceExpr, FormulaError> {
    let mut d = DiceExpr::default();
    let (expr, _) = split_flavor(formula);
    add_expr(&mut d, expr, formula)?;
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DamageType::*;

    fn comps(f: &str) -> Vec<(String, DamageType, bool)> {
        parse_damage(f, Some(Slashing)).unwrap().components.into_iter().map(|c| (c.dice.to_string(), c.damage_type, c.precision)).collect()
    }

    #[test]
    fn plain_and_typed_instances() {
        assert_eq!(comps("1d8+4"), vec![("1d8+4".into(), Slashing, false)]);
        assert_eq!(comps("(1d8 + 4)[piercing]"), vec![("1d8+4".into(), Piercing, false)]);
        assert_eq!(comps("2d6[fire]"), vec![("2d6".into(), Fire, false)]);
        assert_eq!(comps("1d12 - 1"), vec![("1d12-1".into(), Slashing, false)]);
    }

    #[test]
    fn multiple_instances_keep_base_first() {
        assert_eq!(
            comps("{(2d8 + 4)[slashing],(1d6)[fire]}"),
            vec![("2d8+4".into(), Slashing, false), ("1d6".into(), Fire, false)]
        );
    }

    #[test]
    fn nested_precision_and_persistent() {
        let p = parse_damage("{(1d6 + 1d6[precision] + 3)[piercing],(1d4[persistent,bleed])}", None).unwrap();
        let c: Vec<_> = p.components.iter().map(|c| (c.dice.to_string(), c.damage_type, c.precision)).collect();
        assert_eq!(c, vec![("1d6+3".into(), Piercing, false), ("1d6".into(), Piercing, true)]);
        assert_eq!(p.skipped.len(), 1);
    }

    #[test]
    fn legacy_type_names_and_unknowns() {
        assert_eq!(comps("(1d6)[negative]"), vec![("1d6".into(), Void, false)]);
        assert!(parse_damage("@actor.level", None).is_err());
        assert!(parse_damage("2 * (1d6 + 3)", None).is_err(), "crit formulas are not requested");
        assert!(parse_damage("", None).is_err());
    }

    #[test]
    fn persistent_condition_formula() {
        assert_eq!(parse_dice("1d6").unwrap().to_string(), "1d6");
        assert_eq!(parse_dice("2d4 + 2").unwrap().to_string(), "2d4+2");
        assert_eq!(parse_dice("(3)[fire]").unwrap().to_string(), "3");
    }
}
