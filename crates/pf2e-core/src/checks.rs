//! Checks, DCs and the four degrees of success.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegreeOfSuccess {
    CriticalFailure = 0,
    Failure = 1,
    Success = 2,
    CriticalSuccess = 3,
}

impl DegreeOfSuccess {
    pub const ALL: [DegreeOfSuccess; 4] = [
        DegreeOfSuccess::CriticalFailure,
        DegreeOfSuccess::Failure,
        DegreeOfSuccess::Success,
        DegreeOfSuccess::CriticalSuccess,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: i32) -> Self {
        match i.clamp(0, 3) {
            0 => DegreeOfSuccess::CriticalFailure,
            1 => DegreeOfSuccess::Failure,
            2 => DegreeOfSuccess::Success,
            _ => DegreeOfSuccess::CriticalSuccess,
        }
    }

    pub fn step_up(self) -> Self {
        Self::from_index(self as i32 + 1)
    }

    pub fn step_down(self) -> Self {
        Self::from_index(self as i32 - 1)
    }

    pub fn is_success(self) -> bool {
        self >= DegreeOfSuccess::Success
    }

    pub fn label(self) -> &'static str {
        match self {
            DegreeOfSuccess::CriticalFailure => "critical failure",
            DegreeOfSuccess::Failure => "failure",
            DegreeOfSuccess::Success => "success",
            DegreeOfSuccess::CriticalSuccess => "critical success",
        }
    }
}

/// Core PF2e degree-of-success rule:
/// * total ≥ DC + 10 → critical success; total ≥ DC → success;
///   total ≤ DC − 10 → critical failure; otherwise failure.
/// * a natural 20 improves the result one step, a natural 1 worsens it one step.
pub fn degree_of_success(natural: u8, total: i32, dc: i32) -> DegreeOfSuccess {
    let base = if total >= dc + 10 {
        DegreeOfSuccess::CriticalSuccess
    } else if total >= dc {
        DegreeOfSuccess::Success
    } else if total <= dc - 10 {
        DegreeOfSuccess::CriticalFailure
    } else {
        DegreeOfSuccess::Failure
    };
    match natural {
        20 => base.step_up(),
        1 => base.step_down(),
        _ => base,
    }
}

/// A flat check succeeds when the natural die meets the DC. No modifiers apply.
pub fn flat_check_succeeds(natural: u8, dc: i32) -> bool {
    natural as i32 >= dc
}

pub fn flat_check_success_chance(dc: i32) -> f64 {
    ((21 - dc).clamp(0, 20)) as f64 / 20.0
}

/// Probability of each degree (indexed by `DegreeOfSuccess::index`) for a d20
/// check with `modifier` against `dc`. Exact: enumerates all 20 faces.
pub fn degree_distribution(modifier: i32, dc: i32) -> [f64; 4] {
    let mut out = [0.0; 4];
    for face in 1..=20u8 {
        out[degree_of_success(face, face as i32 + modifier, dc).index()] += 0.05;
    }
    out
}

/// One representative natural roll per achievable degree, with its probability.
/// Used by the search engine to branch chance nodes over the real rules.
pub fn representative_faces(modifier: i32, dc: i32) -> Vec<(u8, DegreeOfSuccess, f64)> {
    let mut groups: Vec<(u8, DegreeOfSuccess, f64)> = Vec::new();
    for face in 1..=20u8 {
        let d = degree_of_success(face, face as i32 + modifier, dc);
        if let Some(g) = groups.iter_mut().find(|g| g.1 == d) {
            g.2 += 0.05;
        } else {
            groups.push((face, d, 0.05));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use DegreeOfSuccess::*;

    #[test]
    fn degrees_follow_the_ten_rule() {
        assert_eq!(degree_of_success(10, 28, 18), CriticalSuccess);
        assert_eq!(degree_of_success(10, 18, 18), Success);
        assert_eq!(degree_of_success(10, 17, 18), Failure);
        assert_eq!(degree_of_success(10, 8, 18), CriticalFailure);
    }

    #[test]
    fn naturals_shift_one_step() {
        assert_eq!(degree_of_success(20, 17, 18), Success);
        assert_eq!(degree_of_success(20, 18, 18), CriticalSuccess);
        assert_eq!(degree_of_success(1, 30, 18), Success);
        assert_eq!(degree_of_success(1, 18, 18), Failure);
        // Natural 20 that is still a crit failure becomes a failure.
        assert_eq!(degree_of_success(20, 5, 30), Failure);
    }

    #[test]
    fn distribution_sums_to_one() {
        for m in -5..30 {
            let d = degree_distribution(m, 20);
            assert!((d.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        }
        // +10 vs DC 20: face 20 crits (30), 10..19 succeed, 2..9 fail,
        // face 1 (11, a failure) is stepped down to a critical failure.
        let d = degree_distribution(10, 20);
        assert!((d[CriticalSuccess.index()] - 0.05).abs() < 1e-9);
        assert!((d[Success.index()] - 0.50).abs() < 1e-9);
        assert!((d[Failure.index()] - 0.40).abs() < 1e-9);
        assert!((d[CriticalFailure.index()] - 0.05).abs() < 1e-9);
    }

    #[test]
    fn representatives_cover_distribution() {
        let reps = representative_faces(7, 18);
        let total: f64 = reps.iter().map(|r| r.2).sum();
        assert!((total - 1.0).abs() < 1e-9);
        for (face, deg, _) in reps {
            assert_eq!(degree_of_success(face, face as i32 + 7, 18), deg);
        }
    }
}
