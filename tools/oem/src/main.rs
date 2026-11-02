//! tweedplot-oem: check what a computer's TweedPlot fingerprint is made from.
//!
//!   tweedplot-oem fingerprint     this computer's fingerprint sources and installation code
//!
//! Licences, seats, activation codes and the M_KEY are in the Licence Manager
//! (its own private repository, tweedplot-licence, cloned beside this one), which keeps them in its vault.

use chart::enc::{device::Device, machine, oem};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fingerprint") => {
            // a throwaway device key: the code shows the fingerprint part the app would send
            let mut s = [0u8; 32];
            getrandom::fill(&mut s).expect("random numbers");
            let d = Device::from_secret(s);
            println!("Fingerprint {}", machine::fingerprint().iter().map(|b| format!("{b:02x}")).collect::<String>());
            for l in machine::describe() {
                println!("  {l}");
            }
            println!("(an installation code for a throwaway device key: {})", oem::installation_code(&machine::fingerprint(), &d.public));
        }
        _ => {
            println!("usage: tweedplot-oem fingerprint");
            println!("Licences, seats and activation codes are in the Licence Manager (C:\\dev\\tweedplot-licence: cargo run --release)");
        }
    }
}
