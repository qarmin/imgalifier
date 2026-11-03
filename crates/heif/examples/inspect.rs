//! Parses the container of every file given on the command line and prints a one-line summary (or the error).

#![expect(clippy::print_stdout, clippy::print_stderr, reason = "developer tool")]

use imgalifier_heif::{Container, Limits};

fn main() {
    let (mut ok, mut failed) = (0u32, 0u32);
    for path in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&path) else { continue };
        match Container::parse(&data, Limits::default()) {
            Ok(c) => {
                if std::env::var_os("VERBOSE").is_some() {
                    for it in &c.items {
                        let props: Vec<String> = c
                            .item_properties(it)
                            .map(|(p, e)| format!("{}{p:?}", if e { "!" } else { "" }))
                            .collect();
                        println!(
                            "  item {} {} hidden={} extents={} props={}",
                            it.id,
                            String::from_utf8_lossy(&it.item_type),
                            it.hidden,
                            it.extents.len(),
                            props.join(" ")
                        );
                    }
                    println!("  sequence {:?}", c.sequence);
                    for (k, from, to) in c.all_references() {
                        println!("  ref {} {from} -> {to:?}", String::from_utf8_lossy(&k));
                    }
                }
                ok += 1;
                let kinds: Vec<String> = c
                    .items
                    .iter()
                    .map(|i| String::from_utf8_lossy(&i.item_type).into_owned())
                    .collect();
                println!(
                    "OK   {path}: brand={} primary={:?} items={}",
                    String::from_utf8_lossy(&c.major_brand),
                    c.primary_item,
                    kinds.join(",")
                );
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {path}: {e}");
            }
        }
    }
    eprintln!("parsed {ok}, failed {failed}");
}
