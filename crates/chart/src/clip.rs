//! Clipping to a tile rectangle and ring winding, in tile pixel coordinates.

pub type P = [f64; 2];

#[derive(Clone, Copy)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Sutherland–Hodgman clip of a closed ring (first point may or may not repeat).
pub fn clip_ring(ring: &[P], r: Rect) -> Vec<P> {
    let mut pts: Vec<P> = ring.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    type Inside = fn(&P, &Rect) -> bool;
    type Cross = fn(&P, &P, &Rect) -> P;
    let edges: [(Inside, Cross); 4] = [
        (|p, r| p[0] >= r.x0, |a, b, r| ix(a, b, r.x0)),
        (|p, r| p[0] <= r.x1, |a, b, r| ix(a, b, r.x1)),
        (|p, r| p[1] >= r.y0, |a, b, r| iy(a, b, r.y0)),
        (|p, r| p[1] <= r.y1, |a, b, r| iy(a, b, r.y1)),
    ];
    for (inside, cross) in edges {
        if pts.is_empty() {
            break;
        }
        let input = std::mem::take(&mut pts);
        let mut prev = *input.last().unwrap();
        for cur in input {
            let (ci, pi) = (inside(&cur, &r), inside(&prev, &r));
            if ci {
                if !pi {
                    pts.push(cross(&prev, &cur, &r));
                }
                pts.push(cur);
            } else if pi {
                pts.push(cross(&prev, &cur, &r));
            }
            prev = cur;
        }
    }
    pts
}

fn ix(a: &P, b: &P, x: f64) -> P {
    let t = (x - a[0]) / (b[0] - a[0]);
    [x, a[1] + t * (b[1] - a[1])]
}

fn iy(a: &P, b: &P, y: f64) -> P {
    let t = (y - a[1]) / (b[1] - a[1]);
    [a[0] + t * (b[0] - a[0]), y]
}

/// Clip a polyline; returns the pieces inside the rectangle (Liang–Barsky per segment).
pub fn clip_line(line: &[P], r: Rect) -> Vec<Vec<P>> {
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    for w in line.windows(2) {
        match clip_seg(w[0], w[1], r) {
            Some((a, b, a_moved, b_moved)) => {
                if cur.is_empty() || a_moved {
                    if cur.len() > 1 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    cur.push(a);
                }
                cur.push(b);
                if b_moved {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

fn clip_seg(a: P, b: P, r: Rect) -> Option<(P, P, bool, bool)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a[0] - r.x0), (dx, r.x1 - a[0]), (-dy, a[1] - r.y0), (dy, r.y1 - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return None;
                }
                t1 = t1.min(t);
            }
        }
    }
    let pa = [a[0] + t0 * dx, a[1] + t0 * dy];
    let pb = [a[0] + t1 * dx, a[1] + t1 * dy];
    Some((pa, pb, t0 > 0.0, t1 < 1.0))
}

/// Signed area in tile coordinates (y down); MVT exterior rings are positive.
pub fn signed_area(r: &[[i32; 2]]) -> i64 {
    let mut s = 0i64;
    for i in 0..r.len() {
        let a = r[i];
        let b = r[(i + 1) % r.len()];
        s += a[0] as i64 * b[1] as i64 - b[0] as i64 * a[1] as i64;
    }
    s
}

/// Round to integers and drop repeated points.
pub fn quantise(pts: &[P]) -> Vec<[i32; 2]> {
    let mut out: Vec<[i32; 2]> = Vec::with_capacity(pts.len());
    for p in pts {
        let q = [p[0].round() as i32, p[1].round() as i32];
        if out.last() != Some(&q) {
            out.push(q);
        }
    }
    out
}

/// Douglas–Peucker simplification (iterative). Endpoints are always kept, so
/// a ring passed with its first point at both ends stays closed.
pub fn simplify(pts: &[[i32; 2]], tol: f64) -> Vec<[i32; 2]> {
    if pts.len() < 3 || tol <= 0.0 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    let tol2 = tol * tol;
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (ax, ay) = (pts[a][0] as f64, pts[a][1] as f64);
        let (bx, by) = (pts[b][0] as f64, pts[b][1] as f64);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let (mut best, mut bi) = (-1.0, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let (px, py) = (p[0] as f64 - ax, p[1] as f64 - ay);
            let d2 = if len2 == 0.0 {
                px * px + py * py
            } else {
                let c = px * dy - py * dx;
                c * c / len2
            };
            if d2 > best {
                best = d2;
                bi = i;
            }
        }
        if best > tol2 {
            keep[bi] = true;
            stack.push((a, bi));
            stack.push((bi, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: Rect = Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };

    #[test]
    fn ring_partly_outside() {
        let sq = [[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]];
        let c = clip_ring(&sq, R);
        let a = signed_area(&quantise(&c)).abs();
        assert_eq!(a, 2 * 25); // 5 x 5 square, doubled area
    }

    #[test]
    fn ring_fully_outside() {
        let sq = [[20.0, 20.0], [30.0, 20.0], [30.0, 30.0]];
        assert!(clip_ring(&sq, R).is_empty());
    }

    #[test]
    fn simplify_drops_near_collinear_points() {
        let l = [[0, 0], [10, 1], [20, 0], [30, 50]];
        assert_eq!(simplify(&l, 2.0), vec![[0, 0], [20, 0], [30, 50]]);
    }

    #[test]
    fn line_crossing_twice() {
        let l = [[-5.0, 5.0], [5.0, 5.0], [5.0, 15.0], [8.0, 15.0], [8.0, 5.0], [15.0, 5.0]];
        let pieces = clip_line(&l, R);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0], vec![[0.0, 5.0], [5.0, 5.0], [5.0, 10.0]]);
        assert_eq!(pieces[1], vec![[8.0, 10.0], [8.0, 5.0], [10.0, 5.0]]);
    }
}
