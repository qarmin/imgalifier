//! Prints NAL units, SPS/PPS summary and the decode result of the primary HEVC item: hevc_debug <file>

#![expect(clippy::print_stdout, reason = "developer tool")]

use imgalifier_heif::hevc::HevcConfig;
use imgalifier_heif::hevc::decoder::{DecodeOptions, decode_picture};
use imgalifier_heif::hevc::nal::{NalUnit, split_annex_b};
use imgalifier_heif::hevc::params::ParameterSets;
use imgalifier_heif::{Container, Limits, Property};

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let data = std::fs::read(path).expect("read");
    let c = Container::parse(&data, Limits::default()).expect("container");
    let item = c.primary().expect("primary");
    let cfg = c
        .find_property(item, |p| match p {
            Property::CodecConfig { data, .. } => Some(data.clone()),
            _ => None,
        })
        .expect("hvcC");
    let cfg = HevcConfig::parse(&cfg).expect("hvcC parse");
    let payload = c.item_data(item).expect("payload");
    let stream = cfg.annex_b(&payload).expect("annexb");
    let mut sets = ParameterSets::default();
    for raw in split_annex_b(&stream) {
        let nal = NalUnit::parse(raw).expect("nal");
        println!("NAL {:?} rbsp {} bytes", nal.header.unit_type, nal.rbsp.len());
        if !nal.header.unit_type.is_slice() {
            let _ = sets.add(&nal);
        }
    }
    for sps in sets.sps.iter().flatten() {
        println!("{sps:#?}");
    }
    for pps in sets.pps.iter().flatten() {
        println!("{pps:#?}");
    }
    println!(
        "{:?}",
        decode_picture(&stream, DecodeOptions::default()).map(|f| (f.widths, f.heights))
    );
}
