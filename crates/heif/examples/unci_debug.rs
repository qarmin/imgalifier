//! Prints the iloc extents and the `icbr` ranges of the primary item: unci_debug <file>

#![expect(clippy::print_stdout, reason = "developer tool")]

use imgalifier_heif::{Container, Limits, Property};

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let data = std::fs::read(path).expect("read");
    let c = Container::parse(&data, Limits::default()).expect("container");
    let item = c.primary().expect("primary");
    println!("file {} bytes, extents {:?}", data.len(), item.extents);
    for (p, _) in c.item_properties(item) {
        if let Property::Icbr(r) = p {
            println!("icbr {r:x?}");
        }
        if let Property::UncC(u) = p {
            println!(
                "uncC tiles {}x{} interleave {} align row {} tile {}",
                u.tile_cols, u.tile_rows, u.interleave, u.row_align_size, u.tile_align_size
            );
        }
    }
}
