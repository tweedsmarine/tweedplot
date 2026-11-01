//! Development only: print installation codes for made-up computers (fingerprint n, device n),
//! one per line, and with `release <activation code> <hw_id>` a release code.
use chart::enc::{device::Device, oem};

fn main() {
    let n: u8 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    for i in 1..=n {
        let d = Device::from_secret([i; 32]);
        println!("{}", oem::installation_code(&[i; 16], &d.public));
    }
}
