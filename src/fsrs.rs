//! FSRS-6, the scheduler Anki offers in place of SM-2. A word has a stability, the days until the chance of
//! recalling it falls to 90%, and a difficulty from 1 to 10; every answer updates both, and the next review comes
//! when the chance falls to the retention the learner wants. The parameters are the defaults of py-fsrs 6.

const W: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629, 1.6483,
    0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];
const S_MIN: f64 = 0.001;
const MAX_INTERVAL: f64 = 36_500.0;
pub const DEFAULT_RETENTION: f64 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Memory {
    pub stability: f64,
    pub difficulty: f64,
}

fn decay() -> f64 {
    -W[20]
}

fn factor() -> f64 {
    0.9f64.powf(1.0 / decay()) - 1.0
}

/// The chance of recalling a word `days` whole days after its last review.
pub fn retrievability(days: f64, stability: f64) -> f64 {
    (1.0 + factor() * days / stability).powf(decay())
}

/// The whole days until the chance of recalling falls to `retention`, from one day to a hundred years.
pub fn interval(stability: f64, retention: f64) -> f64 {
    (stability / factor() * (retention.powf(1.0 / decay()) - 1.0)).round().clamp(1.0, MAX_INTERVAL)
}

fn initial_difficulty(rating: u8) -> f64 {
    W[4] - (W[5] * (f64::from(rating) - 1.0)).exp() + 1.0
}

/// The memory after an answer: 1 forgot, 2 hard, 3 good, 4 easy. `days` counts the whole days since the last
/// review; `None` is a word seen for the first time.
pub fn next(memory: Option<Memory>, days: i64, rating: u8) -> Memory {
    let rating = rating.clamp(1, 4);
    let Some(m) = memory else {
        return Memory {
            stability: W[usize::from(rating) - 1].max(S_MIN),
            difficulty: initial_difficulty(rating).clamp(1.0, 10.0),
        };
    };
    let g = f64::from(rating);
    let stability = if days < 1 {
        // Seen again the same day: the short-term formula, which never lowers it for a pass.
        let mut grow = (W[17] * (g - 3.0 + W[18])).exp() * m.stability.powf(-W[19]);
        if rating >= 2 {
            grow = grow.max(1.0);
        }
        m.stability * grow
    } else {
        let r = retrievability(days as f64, m.stability);
        if rating == 1 {
            let long = W[11] * m.difficulty.powf(-W[12]) * ((m.stability + 1.0).powf(W[13]) - 1.0) * ((1.0 - r) * W[14]).exp();
            long.min(m.stability / (W[17] * W[18]).exp())
        } else {
            let hard = if rating == 2 { W[15] } else { 1.0 };
            let easy = if rating == 4 { W[16] } else { 1.0 };
            let grow = W[8].exp() * (11.0 - m.difficulty) * m.stability.powf(-W[9]) * (((1.0 - r) * W[10]).exp() - 1.0);
            m.stability * (1.0 + grow * hard * easy)
        }
    }
    .max(S_MIN);
    // The difficulty moves less the nearer it is to 10, and drifts back toward that of an easy first answer.
    let delta = -W[6] * (g - 3.0);
    let damped = m.difficulty + (10.0 - m.difficulty) * delta / 9.0;
    let difficulty = (W[7] * initial_difficulty(4) + (1.0 - W[7]) * damped).clamp(1.0, 10.0);
    Memory { stability, difficulty }
}

/// A memory for a word scheduled by SM-2 before: the interval it had is taken as its stability (SM-2 aims at
/// about 90% recall on the due day), and the ease, 2.5 for an average word, as the difficulty of a good answer.
pub fn from_sm2(interval: f64, ease: f64) -> Memory {
    Memory {
        stability: interval.max(W[0]),
        difficulty: (initial_difficulty(3) + (2.5 - ease) * 6.5).clamp(1.0, 10.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    /// Reviews from `reference.py` against py-fsrs 6.3.2 (no learning steps, no fuzz): the seconds since the
    /// previous review, the rating, then the stability, difficulty and interval it gives.
    fn check(steps: &[(i64, u8, f64, f64, f64)]) {
        let mut memory = None;
        for (i, &(delay, rating, stability, difficulty, days)) in steps.iter().enumerate() {
            let m = next(memory, delay / DAY, rating);
            assert!((m.stability - stability).abs() < 1e-5, "step {i}: stability {} != {stability}", m.stability);
            assert!((m.difficulty - difficulty).abs() < 1e-5, "step {i}: difficulty {} != {difficulty}", m.difficulty);
            assert_eq!(interval(m.stability, DEFAULT_RETENTION), days, "step {i}: interval");
            memory = Some(m);
        }
    }

    #[test]
    fn matches_py_fsrs() {
        check(&[(0, 3, 2.306500, 2.118104, 2.0), (2 * DAY, 3, 10.964332, 2.111214, 11.0), (11 * DAY, 3, 46.280217, 2.104331, 46.0), (40 * DAY, 3, 151.596349, 2.097455, 152.0)]);
        check(&[
            (0, 3, 2.306500, 2.118104, 2.0),
            (2 * DAY, 3, 10.964332, 2.111214, 11.0),
            (11 * DAY, 1, 1.538337, 7.392238, 2.0),
            (600, 3, 1.571197, 7.380074, 2.0),
            (5 * DAY, 2, 5.056829, 8.246000, 5.0),
            (9 * DAY, 4, 20.802403, 7.645123, 21.0),
        ]);
        check(&[(0, 1, 0.212000, 6.413300, 1.0), (600, 1, 0.083357, 8.806304, 1.0), (600, 3, 0.103141, 8.792727, 1.0), (DAY, 3, 0.695255, 8.779162, 1.0)]);
        check(&[(0, 4, 8.295600, 1.0, 8.0), (8 * DAY, 2, 26.704183, 4.010609, 27.0)]);
        check(&[(0, 2, 1.293100, 5.112171, 1.0), (3600, 3, 1.335900, 5.102287, 1.0)]);
    }

    #[test]
    fn retention_and_the_curve() {
        assert!((retrievability(10.0, 5.0) - 0.845885).abs() < 1e-6);
        assert!((retrievability(7.0, 7.0) - 0.9).abs() < 1e-9);
        assert_eq!([0.8, 0.9, 0.95].map(|r| interval(10.0, r)), [33.0, 10.0, 4.0]);
        assert_eq!(interval(0.01, 0.9), 1.0);
        assert_eq!(interval(1e9, 0.9), MAX_INTERVAL);
    }

    #[test]
    fn sm2_words_keep_their_pace() {
        let average = from_sm2(6.0, 2.5);
        assert_eq!(average.stability, 6.0);
        assert!((average.difficulty - initial_difficulty(3)).abs() < 1e-9);
        assert_eq!(from_sm2(0.0, 1.3).stability, W[0]);
        assert!(from_sm2(10.0, 1.3).difficulty > 9.0 && from_sm2(10.0, 3.0).difficulty == 1.0);
    }
}
