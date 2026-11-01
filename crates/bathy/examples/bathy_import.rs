//! Import a PBG or XYZ file into a store, printing each stage's time:
//! `cargo run -p bathy --release --example bathy_import -- <store> <file>`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let s = bathy::Store::open(std::path::Path::new(&a[1])).unwrap();
    let p = std::path::Path::new(&a[2]);
    let t = std::time::Instant::now();
    let mut last = String::new();
    let mut prog = |st: &str, _d: usize, _n: usize| {
        let k = st.split(':').next().unwrap_or("").to_string();
        if k != last {
            println!("{:>7.1} s  {st}", t.elapsed().as_secs_f64());
            last = k;
        }
    };
    let n = if p.extension().is_some_and(|x| x == "xyz") {
        s.import_xyz(p, &mut prog, &Default::default())
    } else {
        s.import_pbg(p, &mut prog, &Default::default())
    }
    .unwrap();
    println!("{:>7.1} s  done: {n} cells", t.elapsed().as_secs_f64());
}
