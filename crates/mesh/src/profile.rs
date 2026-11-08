//! Bevel profiles: the shape a bevel's band follows across its width.
//!
//! A profile runs from `(1, 0)` to `(0, 1)` in a square whose corner `(0, 0)` is the
//! original sharp edge, with the shape's inside toward `(1, 1)`. So a straight line
//! between the ends is a flat chamfer, a curve bulging toward `(0, 0)` is a round, and
//! points near `(0, 0)` keep the shape close to its sharp edge. Blender's custom
//! profile widget draws the same square.
//!
//! A custom profile is a list of points. Between two smooth points, or a smooth and a
//! sharp one, the line curves (a Catmull-Rom curve through the points); between two
//! sharp points it is straight. The bevel's segments are then placed along it.

/// The most points a custom profile can have.
pub const MAX_PROFILE_POINTS: usize = 24;

/// One point of a custom profile, in the profile's square.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfilePoint {
    /// Across, from the sharp edge (0) to the far side (1).
    pub x: f32,
    /// Up, from the sharp edge (0) to the far side (1).
    pub y: f32,
    /// A corner rather than a smooth bend.
    pub sharp: bool,
}

impl ProfilePoint {
    const fn new(x: f32, y: f32, sharp: bool) -> Self {
        Self { x, y, sharp }
    }
}

/// Ready-made profiles, as in Blender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ProfilePreset {
    Round,
    Chamfer,
    SupportLoops,
    Steps,
    CorniceMolding,
    CrownMolding,
}

impl ProfilePreset {
    /// Every preset, in menu order.
    pub const ALL: [ProfilePreset; 6] = [
        ProfilePreset::Round,
        ProfilePreset::Chamfer,
        ProfilePreset::SupportLoops,
        ProfilePreset::Steps,
        ProfilePreset::CorniceMolding,
        ProfilePreset::CrownMolding,
    ];

    /// Its name in the menu.
    pub fn name(self) -> &'static str {
        match self {
            ProfilePreset::Round => "Round",
            ProfilePreset::Chamfer => "Chamfer",
            ProfilePreset::SupportLoops => "Support Loops",
            ProfilePreset::Steps => "Steps",
            ProfilePreset::CorniceMolding => "Cornice Molding",
            ProfilePreset::CrownMolding => "Crown Molding",
        }
    }

    /// Each list runs from (1, 0) to (0, 1).
    fn points(self) -> &'static [ProfilePoint] {
        match self {
            ProfilePreset::Round => &ROUND,
            ProfilePreset::Chamfer => &CHAMFER,
            ProfilePreset::SupportLoops => &SUPPORT_LOOPS,
            ProfilePreset::Steps => &STEPS,
            ProfilePreset::CorniceMolding => &CORNICE,
            ProfilePreset::CrownMolding => &CROWN,
        }
    }
}

const ROUND: [ProfilePoint; 5] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.617, 0.076, false),
    ProfilePoint::new(0.293, 0.293, false),
    ProfilePoint::new(0.076, 0.617, false),
    ProfilePoint::new(0.0, 1.0, true),
];
const CHAMFER: [ProfilePoint; 2] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.0, 1.0, true),
];
/// Hugs the sharp edge, leaving tight loops of edges close to it.
const SUPPORT_LOOPS: [ProfilePoint; 4] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.1, 0.0, true),
    ProfilePoint::new(0.0, 0.1, true),
    ProfilePoint::new(0.0, 1.0, true),
];
const STEPS: [ProfilePoint; 9] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.75, 0.0, true),
    ProfilePoint::new(0.75, 0.25, true),
    ProfilePoint::new(0.5, 0.25, true),
    ProfilePoint::new(0.5, 0.5, true),
    ProfilePoint::new(0.25, 0.5, true),
    ProfilePoint::new(0.25, 0.75, true),
    ProfilePoint::new(0.0, 0.75, true),
    ProfilePoint::new(0.0, 1.0, true),
];
/// A fillet, a hollow, then a rounded bead, between flat steps.
const CORNICE: [ProfilePoint; 11] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.85, 0.0, true),
    ProfilePoint::new(0.85, 0.1, true),
    ProfilePoint::new(0.62, 0.16, false),
    ProfilePoint::new(0.5, 0.35, false),
    ProfilePoint::new(0.4, 0.5, true),
    ProfilePoint::new(0.25, 0.52, false),
    ProfilePoint::new(0.13, 0.65, false),
    ProfilePoint::new(0.12, 0.85, true),
    ProfilePoint::new(0.0, 0.85, true),
    ProfilePoint::new(0.0, 1.0, true),
];
/// An S-shaped ogee between two small flats.
const CROWN: [ProfilePoint; 9] = [
    ProfilePoint::new(1.0, 0.0, true),
    ProfilePoint::new(0.9, 0.05, true),
    ProfilePoint::new(0.62, 0.1, false),
    ProfilePoint::new(0.48, 0.3, false),
    ProfilePoint::new(0.36, 0.56, false),
    ProfilePoint::new(0.16, 0.72, false),
    ProfilePoint::new(0.08, 0.8, true),
    ProfilePoint::new(0.0, 0.88, true),
    ProfilePoint::new(0.0, 1.0, true),
];

/// A custom profile: its points, kept in a fixed array so settings stay simple values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CustomProfile {
    points: [ProfilePoint; MAX_PROFILE_POINTS],
    count: usize,
    /// Segments also go on straight parts, not only on curves.
    pub sample_straight: bool,
    /// Segments are spread evenly by length, and the points themselves may be skipped.
    pub even_lengths: bool,
}

impl Default for CustomProfile {
    fn default() -> Self {
        Self::preset(ProfilePreset::SupportLoops)
    }
}

impl CustomProfile {
    /// A copy of a preset.
    pub fn preset(preset: ProfilePreset) -> Self {
        let mut profile = Self {
            points: [ProfilePoint::new(0.0, 0.0, true); MAX_PROFILE_POINTS],
            count: 0,
            sample_straight: false,
            even_lengths: false,
        };
        for &p in preset.points() {
            profile.points[profile.count] = p;
            profile.count += 1;
        }
        profile
    }

    /// Which preset this is, if it is one unchanged.
    pub fn matching_preset(&self) -> Option<ProfilePreset> {
        ProfilePreset::ALL
            .into_iter()
            .find(|&p| self.points() == p.points())
    }

    /// The points, from `(1, 0)` to `(0, 1)`.
    pub fn points(&self) -> &[ProfilePoint] {
        &self.points[..self.count]
    }

    /// Moves point `i`, kept inside the square. The two ends stay where they are.
    pub fn move_point(&mut self, i: usize, x: f32, y: f32) {
        if i == 0 || i + 1 >= self.count {
            return;
        }
        self.points[i].x = x.clamp(0.0, 1.0);
        self.points[i].y = y.clamp(0.0, 1.0);
    }

    /// Adds a point after point `i`. Returns its index, or `None` if the profile is full.
    pub fn insert_after(&mut self, i: usize, x: f32, y: f32) -> Option<usize> {
        if self.count >= MAX_PROFILE_POINTS || i + 1 >= self.count {
            return None;
        }
        let at = i + 1;
        self.points.copy_within(at..self.count, at + 1);
        self.points[at] = ProfilePoint::new(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0), false);
        self.count += 1;
        Some(at)
    }

    /// Removes point `i`, unless it is an end.
    pub fn remove(&mut self, i: usize) {
        if i == 0 || i + 1 >= self.count {
            return;
        }
        self.points.copy_within(i + 1..self.count, i);
        self.count -= 1;
    }

    /// Turns point `i` from smooth to sharp, or back.
    pub fn toggle_sharp(&mut self, i: usize) {
        if i < self.count {
            self.points[i].sharp = !self.points[i].sharp;
        }
    }

    fn valid(&self) -> bool {
        self.count >= 2
            && self
                .points()
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// A dense line through the profile, for drawing and for sampling. Each piece
    /// between two points gets `per_piece` steps. Returns the points and, for each
    /// piece, the index of its first point in the line.
    pub fn line(&self, per_piece: usize) -> (Vec<[f32; 2]>, Vec<usize>) {
        let points = self.points();
        let n = points.len();
        let at = |i: usize| [points[i].x, points[i].y];
        // A smooth point's direction runs from the point before it to the one after;
        // a sharp point, or an end, has none, so the curve leaves it straight.
        let tangent = |i: usize| {
            if points[i].sharp || i == 0 || i + 1 == n {
                [0.0, 0.0]
            } else {
                let (a, b) = (at(i - 1), at(i + 1));
                [(b[0] - a[0]) / 2.0, (b[1] - a[1]) / 2.0]
            }
        };
        let mut line = Vec::with_capacity((n - 1) * per_piece + 1);
        let mut starts = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            starts.push(line.len());
            let (p0, p1, m0, m1) = (at(i), at(i + 1), tangent(i), tangent(i + 1));
            for k in 0..per_piece {
                let t = k as f32 / per_piece as f32;
                let (t2, t3) = (t * t, t * t * t);
                let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                let h10 = t3 - 2.0 * t2 + t;
                let h01 = -2.0 * t3 + 3.0 * t2;
                let h11 = t3 - t2;
                line.push(std::array::from_fn(|c| {
                    h00 * p0[c] + h10 * m0[c] + h01 * p1[c] + h11 * m1[c]
                }));
            }
        }
        line.push(at(n - 1));
        (line, starts)
    }

    /// Points `segments + 1` along the profile, from `(1, 0)` to `(0, 1)`.
    pub fn sample(&self, segments: usize) -> Vec<[f32; 2]> {
        let segments = segments.max(1);
        if !self.valid() {
            return (0..=segments)
                .map(|j| {
                    let t = j as f32 / segments as f32;
                    [1.0 - t, t]
                })
                .collect();
        }
        const PER_PIECE: usize = 48;
        let (line, starts) = self.line(PER_PIECE);
        let pieces = self.count - 1;
        let even = |from: usize, to: usize, parts: usize| -> Vec<[f32; 2]> {
            // `parts` pieces of equal length along line[from..=to], without its end.
            let lengths: Vec<f32> = line[from..=to]
                .windows(2)
                .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
                .collect();
            let total: f32 = lengths.iter().sum();
            let mut out = Vec::with_capacity(parts);
            let mut walked = 0.0;
            let mut k = 0;
            for j in 0..parts {
                let wanted = total * j as f32 / parts as f32;
                while k < lengths.len() && walked + lengths[k] < wanted {
                    walked += lengths[k];
                    k += 1;
                }
                if k >= lengths.len() {
                    out.push(line[to]);
                    continue;
                }
                let t = if lengths[k] > 0.0 {
                    (wanted - walked) / lengths[k]
                } else {
                    0.0
                };
                let (a, b) = (line[from + k], line[from + k + 1]);
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
            out
        };
        let last = line.len() - 1;
        if self.even_lengths || segments < pieces {
            let mut out = even(0, last, segments);
            out.push(line[last]);
            return out;
        }
        // Every point is kept; the extra segments go to the longest pieces that may
        // take them: curved ones, and straight ones too if asked.
        let points = self.points();
        let straight = |i: usize| points[i].sharp && points[i + 1].sharp;
        let piece_length = |i: usize| {
            let to = if i + 1 < pieces { starts[i + 1] } else { last };
            line[starts[i]..=to]
                .windows(2)
                .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
                .sum::<f32>()
        };
        let mut takers: Vec<usize> = (0..pieces)
            .filter(|&i| self.sample_straight || !straight(i))
            .collect();
        if takers.is_empty() {
            takers = (0..pieces).collect();
        }
        let mut parts = vec![1usize; pieces];
        for _ in 0..segments - pieces {
            // The taker whose pieces are longest so far gets the next segment.
            let best = takers
                .iter()
                .copied()
                .max_by(|&a, &b| {
                    (piece_length(a) / parts[a] as f32)
                        .total_cmp(&(piece_length(b) / parts[b] as f32))
                        .then(b.cmp(&a))
                })
                .unwrap_or(0);
            parts[best] += 1;
        }
        let mut out = Vec::with_capacity(segments + 1);
        for i in 0..pieces {
            let to = if i + 1 < pieces { starts[i + 1] } else { last };
            out.extend(even(starts[i], to, parts[i]));
        }
        out.push(line[last]);
        out
    }

    /// Where the profile crosses the square's diagonal, as a share of the way from
    /// `(1, 1)` toward the sharp edge at `(0, 0)`: about 0.29 for a round.
    pub fn diagonal(&self) -> f32 {
        let (line, _) = self.line(32);
        let mut best = (f32::MAX, 0.5);
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (da, db) = (a[0] - a[1], b[0] - b[1]);
            if da == 0.0 || (da > 0.0) != (db > 0.0) {
                let t = if (da - db).abs() > 1e-9 {
                    da / (da - db)
                } else {
                    0.0
                };
                let x = a[0] + (b[0] - a[0]) * t;
                let distance = (x - 0.5).abs();
                if distance < best.0 {
                    best = (distance, x);
                }
            }
        }
        best.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_runs_end_to_end_and_samples_to_its_segments() {
        for preset in ProfilePreset::ALL {
            let profile = CustomProfile::preset(preset);
            assert_eq!(profile.matching_preset(), Some(preset));
            for segments in 1..=16 {
                let samples = profile.sample(segments);
                assert_eq!(samples.len(), segments + 1, "{preset:?}");
                assert_eq!(samples[0], [1.0, 0.0]);
                assert_eq!(samples[segments], [0.0, 1.0]);
                for p in &samples {
                    assert!((-0.01..=1.01).contains(&p[0]) && (-0.01..=1.01).contains(&p[1]));
                }
            }
        }
    }

    #[test]
    fn points_are_kept_unless_lengths_are_even() {
        let steps = CustomProfile::preset(ProfilePreset::Steps);
        // With as many segments as pieces, the samples are the points themselves.
        let exact = steps.sample(8);
        for (sample, point) in exact.iter().zip(steps.points()) {
            assert!((sample[0] - point.x).abs() < 1e-5 && (sample[1] - point.y).abs() < 1e-5);
        }
        // Extra segments on all-straight pieces still keep every point.
        let more = steps.sample(12);
        for point in steps.points() {
            assert!(
                more.iter()
                    .any(|s| (s[0] - point.x).abs() < 1e-5 && (s[1] - point.y).abs() < 1e-5)
            );
        }
        // Even lengths: equal steps along the line.
        let even = CustomProfile {
            even_lengths: true,
            ..steps
        }
        .sample(4);
        let step =
            |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        assert_eq!(even.len(), 5);
        assert!(step(even[1], even[2]) > 0.0);
    }

    #[test]
    fn the_round_preset_is_nearly_a_quarter_circle() {
        let round = CustomProfile::preset(ProfilePreset::Round);
        for p in round.sample(16) {
            // On the circle of radius 1 around (1, 1).
            let r = ((p[0] - 1.0).powi(2) + (p[1] - 1.0).powi(2)).sqrt();
            assert!((r - 1.0).abs() < 0.02, "{p:?} at {r}");
        }
        assert!((round.diagonal() - (1.0 - 0.5f32.sqrt())).abs() < 0.01);
        assert!((CustomProfile::preset(ProfilePreset::Chamfer).diagonal() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn editing_keeps_the_ends_and_the_limit() {
        let mut profile = CustomProfile::preset(ProfilePreset::Chamfer);
        assert_eq!(profile.insert_after(0, 0.3, 0.3), Some(1));
        profile.move_point(1, 2.0, -1.0);
        assert_eq!(profile.points()[1].x, 1.0);
        assert_eq!(profile.points()[1].y, 0.0);
        profile.move_point(0, 0.5, 0.5);
        assert_eq!(profile.points()[0].x, 1.0, "the ends stay");
        profile.remove(0);
        assert_eq!(profile.points().len(), 3);
        profile.remove(1);
        assert_eq!(profile.points().len(), 2);
        while profile.insert_after(0, 0.5, 0.5).is_some() {}
        assert_eq!(profile.points().len(), MAX_PROFILE_POINTS);
        assert_eq!(profile.matching_preset(), None);
    }
}
