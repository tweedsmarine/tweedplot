//! Which depth areas contain a point: `depth_at <dir>... -- lon lat`.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let k = a.iter().position(|s| s == "--").unwrap();
    let dirs: Vec<std::path::PathBuf> = a[..k].iter().map(Into::into).collect();
    let (x, y): (f64, f64) = (a[k + 1].parse().unwrap(), a[k + 2].parse().unwrap());
    let s = chart::ChartStore::open_dirs(&dirs).unwrap();
    let inside = |r: &[[f64; 2]]| {
        let mut c = false;
        let n = r.len();
        for i in 0..n {
            let (p, q) = (r[i], r[(i + n - 1) % n]);
            if (p[1] > y) != (q[1] > y) && x < (q[0] - p[0]) * (y - p[1]) / (q[1] - p[1]) + p[0] {
                c = !c;
            }
        }
        c
    };
    let mut n = 0;
    s.for_each_depth_area([x - 0.001, y - 0.001, x + 0.001, y + 0.001], |band, d, rings| {
        if inside(&rings[0]) && !rings[1..].iter().any(|h| inside(h)) {
            n += 1;
            println!("band {band} drval1 {d:?}");
        }
    });
    println!("{n} depth areas contain {x} {y}");
}
