//! A smooth curve through points, measured by distance along it.
//!
//! The curve is a centripetal Catmull–Rom spline: it passes through every
//! point, and spacing its knots by the square root of the chord length keeps
//! it from looping or overshooting where points bunch up. An open curve gets
//! a mirrored phantom point at each end, so its ends leave and arrive along
//! the first and last chords. Travel is by arc length, so a uniform progress
//! moves at a constant speed.

use bevy_math::Vec3;

/// Samples per segment in the arc-length table.
const STEPS: usize = 24;

#[derive(Clone, Debug)]
pub(crate) struct Spline {
    /// The control points with one phantom at each end (or wrapped).
    controls: Vec<Vec3>,
    /// Segment count; segment `i` runs from `controls[i + 1]` to `controls[i + 2]`.
    segments: usize,
    /// Cumulative length at each table entry, `segments * STEPS + 1` entries.
    table: Vec<f32>,
}

impl Spline {
    /// A curve through `points`; at least one point.
    pub fn new(points: &[Vec3], closed: bool) -> Spline {
        assert!(!points.is_empty(), "a spline needs a point");
        let n = points.len();
        let controls: Vec<Vec3> = if n == 1 {
            vec![points[0]; 4]
        } else if closed {
            let mut c = Vec::with_capacity(n + 3);
            c.push(points[n - 1]);
            c.extend_from_slice(points);
            c.push(points[0]);
            c.push(points[1 % n]);
            c
        } else {
            let mut c = Vec::with_capacity(n + 2);
            c.push(2.0 * points[0] - points[1]);
            c.extend_from_slice(points);
            c.push(2.0 * points[n - 1] - points[n - 2]);
            c
        };
        let segments = controls.len() - 3;
        let mut spline = Spline { controls, segments, table: Vec::new() };
        let mut table = Vec::with_capacity(segments * STEPS + 1);
        table.push(0.0);
        let mut previous = spline.segment(0, 0.0);
        let mut total = 0.0;
        for segment in 0..segments {
            for step in 1..=STEPS {
                let point = spline.segment(segment, step as f32 / STEPS as f32);
                total += point.distance(previous);
                table.push(total);
                previous = point;
            }
        }
        spline.table = table;
        spline
    }

    pub fn length(&self) -> f32 {
        *self.table.last().unwrap_or(&0.0)
    }

    /// The point at a share `u` in `[0, 1]` of the length.
    pub fn at(&self, u: f32) -> Vec3 {
        let (segment, s) = self.locate(u);
        self.segment(segment, s)
    }

    /// The direction of travel at a share `u` of the length; zero for a
    /// curve without length.
    pub fn direction(&self, u: f32) -> Vec3 {
        let length = self.length();
        if length <= f32::EPSILON {
            return Vec3::ZERO;
        }
        let du = (0.01 / length).min(0.01);
        let (a, b) = if u + du <= 1.0 { (u, u + du) } else { (u - du, u) };
        (self.at(b) - self.at(a)).normalize_or_zero()
    }

    /// The segment and its curve parameter at a share of the length.
    fn locate(&self, u: f32) -> (usize, f32) {
        let target = u.clamp(0.0, 1.0) * self.length();
        let index = self.table.partition_point(|&d| d < target).clamp(1, self.table.len() - 1);
        let (low, high) = (self.table[index - 1], self.table[index]);
        let within = if high > low { (target - low) / (high - low) } else { 0.0 };
        let entry = (index - 1) as f32 + within;
        let segment = ((entry / STEPS as f32) as usize).min(self.segments - 1);
        (segment, (entry - (segment * STEPS) as f32) / STEPS as f32)
    }

    /// Segment `index` at curve parameter `s` in `[0, 1]`, by the
    /// Barry–Goldman pyramid over centripetal knots.
    fn segment(&self, index: usize, s: f32) -> Vec3 {
        let [p0, p1, p2, p3] = [
            self.controls[index],
            self.controls[index + 1],
            self.controls[index + 2],
            self.controls[index + 3],
        ];
        if p1.distance_squared(p2) <= f32::EPSILON * f32::EPSILON {
            return p1;
        }
        let knot = |a: Vec3, b: Vec3| a.distance(b).sqrt().max(1e-4);
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        let t = t1 + (t2 - t1) * s;
        let lerp = |a: Vec3, b: Vec3, ta: f32, tb: f32| a + (b - a) * ((t - ta) / (tb - ta));
        let a1 = lerp(p0, p1, t0, t1);
        let a2 = lerp(p1, p2, t1, t2);
        let a3 = lerp(p2, p3, t2, t3);
        let b1 = lerp(a1, a2, t0, t2);
        let b2 = lerp(a2, a3, t1, t3);
        lerp(b1, b2, t1, t2)
    }
}
