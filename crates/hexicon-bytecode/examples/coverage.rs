//! Compares function discovery against the method header list of a debug
//! build. Usage: `cargo run --example coverage -- <image.t3>...`

use std::collections::BTreeSet;

use hexicon_bytecode::{discover, discover_without_debug_info};
use hexicon_image::Image;

fn main() {
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).expect("cannot read image");
        let image = Image::parse(&data).expect("cannot parse image");
        let listed: BTreeSet<u32> = image.debug.method_headers.iter().copied().collect();
        let with: BTreeSet<u32> = discover(&image).by_address.keys().copied().collect();
        let found = discover_without_debug_info(&image);
        let without: BTreeSet<u32> = found.by_address.keys().copied().collect();
        let missed: Vec<u32> = listed.difference(&without).copied().collect();
        let extra: Vec<u32> = without.difference(&listed).copied().collect();
        println!(
            "{path}: listed {}, with debug info {}, without {}, missed {}, not listed {}, failures {}",
            listed.len(),
            with.len(),
            without.len(),
            missed.len(),
            extra.len(),
            found.errors().count()
        );
        println!("  missed: {:x?}", &missed[..missed.len().min(12)]);
        println!("  not listed: {:x?}", &extra[..extra.len().min(12)]);
    }
}
