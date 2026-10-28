//! Time the second opinion on a store: `bathy_second <store>`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let s = bathy::Store::open(std::path::Path::new(&a[1])).unwrap();
    let before = s.info().unwrap().rejected;
    let t = std::time::Instant::now();
    let back = s.second_opinion(&mut |_, _, _| {}, &Default::default()).unwrap();
    println!("left out {before}: {back} put back in {:?}; now {}", t.elapsed(), s.info().unwrap().rejected);
}
