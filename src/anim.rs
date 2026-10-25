//! Slide-in timing curve.

/// Apple-style ease-out: CSS `cubic-bezier(0.32, 0.72, 0, 1)` (the curve iOS uses for
/// sliding panels). Fast start, long soft landing.
pub const SLIDE: (f64, f64, f64, f64) = (0.32, 0.72, 0.0, 1.0);

/// CSS cubic-bezier timing function: progress (0..1) at time `t` (0..1).
/// The curve runs from (0,0) to (1,1) with control points (x1,y1) and (x2,y2).
pub fn cubic_bezier((x1, y1, x2, y2): (f64, f64, f64, f64), t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let coord = |a: f64, b: f64, s: f64| {
        let m = 1.0 - s;
        3.0 * m * m * s * a + 3.0 * m * s * s * b + s * s * s
    };
    // Find s with x(s) = t by bisection (x is monotonic for x1, x2 in 0..1), then return y(s).
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if coord(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    coord(y1, y2, (lo + hi) / 2.0)
}

/// Content offset in pixels for elapsed time: `width` at the start, 0 at the end.
pub fn slide_offset(width: i32, elapsed_ms: f64, duration_ms: f64) -> i32 {
    if duration_ms <= 0.0 {
        return 0;
    }
    let p = cubic_bezier(SLIDE, elapsed_ms / duration_ms);
    ((1.0 - p) * width as f64).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_shape() {
        assert_eq!(cubic_bezier(SLIDE, 0.0), 0.0);
        assert_eq!(cubic_bezier(SLIDE, 1.0), 1.0);
        let mut prev = 0.0;
        for i in 1..100 {
            let v = cubic_bezier(SLIDE, i as f64 / 100.0);
            assert!(v >= prev, "monotonic");
            prev = v;
        }
        // ease-out: well ahead of linear early on
        assert!(cubic_bezier(SLIDE, 0.25) > 0.5);
        // linear curve stays linear
        assert!((cubic_bezier((0.0, 0.0, 1.0, 1.0), 0.3) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn offsets() {
        assert_eq!(slide_offset(300, 0.0, 50.0), 300);
        assert_eq!(slide_offset(300, 50.0, 50.0), 0);
        assert_eq!(slide_offset(300, 80.0, 50.0), 0);
        assert_eq!(slide_offset(300, 10.0, 0.0), 0); // animation off
        let mid = slide_offset(300, 16.7, 50.0);
        assert!(mid > 0 && mid < 150, "{}", mid);
    }
}
