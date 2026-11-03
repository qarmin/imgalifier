//! End-to-end tests of the container and the shared pipeline using a fake one-byte-per-sample codec.

use imgalifier_heif::{ChromaFormat, CodecBackend, ColorInfo, Decoder, Error, FourCc, Limits, Result, YuvImage};

struct FakeCodec;

impl CodecBackend for FakeCodec {
    fn handles(&self, item_type: FourCc) -> bool {
        &item_type == b"fake"
    }

    /// Payload: width, height, then a flat luma value; chroma is neutral.
    fn decode(&self, _config: &[u8], payload: &[u8]) -> Result<YuvImage> {
        let [w, h, luma] = payload else {
            return Err(Error::Invalid("fake payload"));
        };
        let mut y = YuvImage::new(u32::from(*w), u32::from(*h), 8, ChromaFormat::Yuv444)?;
        y.color = ColorInfo {
            matrix: 1,
            full_range: true,
            ..ColorInfo::default()
        };
        y.planes[0].fill(u16::from(*luma));
        y.planes[1].fill(128);
        y.planes[2].fill(128);
        Ok(y)
    }
}

fn bx(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(&kind);
    v.extend_from_slice(body);
    v
}

fn full(kind: [u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version, 0, 0, 0];
    b.extend_from_slice(body);
    bx(kind, &b)
}

struct ItemSpec {
    id: u16,
    kind: &'static [u8; 4],
    payload: Vec<u8>,
    props: Vec<u8>,
}

/// Builds `ftyp + meta + mdat`; `refs` are (type, from, to-list) entries of `iref`.
fn build(items: &[ItemSpec], properties: &[Vec<u8>], refs: &[(&[u8; 4], u16, Vec<u16>)], primary: u16) -> Vec<u8> {
    let ftyp = bx(*b"ftyp", b"mif1\0\0\0\0mif1");
    let meta = |base: u32| {
        let mut m = Vec::new();
        m.extend(full(*b"pitm", 0, &primary.to_be_bytes()));
        let mut iinf = (items.len() as u16).to_be_bytes().to_vec();
        for it in items {
            let mut infe = it.id.to_be_bytes().to_vec();
            infe.extend_from_slice(&[0, 0]);
            infe.extend_from_slice(it.kind);
            infe.push(0);
            iinf.extend(full(*b"infe", 2, &infe));
        }
        m.extend(full(*b"iinf", 0, &iinf));
        let mut iloc = vec![0x44, 0x00];
        iloc.extend_from_slice(&(items.len() as u16).to_be_bytes());
        let mut off = base;
        for it in items {
            iloc.extend_from_slice(&it.id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 1]);
            iloc.extend_from_slice(&off.to_be_bytes());
            iloc.extend_from_slice(&(it.payload.len() as u32).to_be_bytes());
            off += it.payload.len() as u32;
        }
        m.extend(full(*b"iloc", 0, &iloc));
        if !refs.is_empty() {
            let mut iref = Vec::new();
            for (kind, from, to) in refs {
                let mut r = from.to_be_bytes().to_vec();
                r.extend_from_slice(&(to.len() as u16).to_be_bytes());
                for t in to {
                    r.extend_from_slice(&t.to_be_bytes());
                }
                iref.extend(bx(**kind, &r));
            }
            m.extend(full(*b"iref", 0, &iref));
        }
        let ipco = bx(*b"ipco", &properties.concat());
        let mut ipma = (items.len() as u32).to_be_bytes().to_vec();
        for it in items {
            ipma.extend_from_slice(&it.id.to_be_bytes());
            ipma.push((it.props.len()) as u8);
            ipma.extend_from_slice(&it.props);
        }
        m.extend(bx(*b"iprp", &[ipco, full(*b"ipma", 0, &ipma)].concat()));
        full(*b"meta", 0, &m)
    };
    let mdat_payload: Vec<u8> = items.iter().flat_map(|i| i.payload.clone()).collect();
    let base = (ftyp.len() + meta(0).len() + 8) as u32;
    [ftyp, meta(base), bx(*b"mdat", &mdat_payload)].concat()
}

fn hvcc_prop() -> Vec<u8> {
    bx(*b"hvcC", &[1])
}

fn decode(file: &[u8]) -> Result<imgalifier_heif::RgbaImage> {
    Decoder::new(file, FakeCodec, Limits::default())?.decode_primary()
}

#[test]
fn single_item_with_rotation() {
    let props = vec![hvcc_prop(), bx(*b"irot", &[1])];
    let items = [ItemSpec {
        id: 1,
        kind: b"fake",
        payload: vec![2, 1, 255],
        props: vec![1, 2],
    }];
    let img = decode(&build(&items, &props, &[], 1)).unwrap();
    assert_eq!((img.width, img.height), (1, 2));
    assert_eq!(&img.data[..4], &[255, 255, 255, 255]);
}

#[test]
fn grid_of_two_tiles() {
    let props = vec![hvcc_prop()];
    let items = [
        ItemSpec {
            id: 1,
            kind: b"grid",
            payload: vec![0, 0, 0, 1, 0, 4, 0, 2],
            props: vec![],
        },
        ItemSpec {
            id: 2,
            kind: b"fake",
            payload: vec![2, 2, 0],
            props: vec![1],
        },
        ItemSpec {
            id: 3,
            kind: b"fake",
            payload: vec![2, 2, 255],
            props: vec![1],
        },
    ];
    let img = decode(&build(&items, &props, &[(b"dimg", 1, vec![2, 3])], 1)).unwrap();
    assert_eq!((img.width, img.height), (4, 2));
    assert_eq!(img.data[0], 0);
    assert_eq!(img.data[3 * 4], 255);
}

#[test]
fn alpha_auxiliary_item() {
    let auxc = full(*b"auxC", 0, b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0");
    let props = vec![hvcc_prop(), auxc];
    let items = [
        ItemSpec {
            id: 1,
            kind: b"fake",
            payload: vec![1, 1, 200],
            props: vec![1],
        },
        ItemSpec {
            id: 2,
            kind: b"fake",
            payload: vec![1, 1, 64],
            props: vec![1, 2],
        },
    ];
    let img = decode(&build(&items, &props, &[(b"auxl", 2, vec![1])], 1)).unwrap();
    assert_eq!(&img.data[..4], &[200, 200, 200, 64]);
}

#[test]
fn rejects_truncated_and_oversized_input() {
    let props = vec![hvcc_prop()];
    let items = [ItemSpec {
        id: 1,
        kind: b"fake",
        payload: vec![1, 1, 1],
        props: vec![1],
    }];
    let file = build(&items, &props, &[], 1);
    for cut in 0..file.len() - 1 {
        // must never panic, only return an error or (for cuts inside mdat) fail on the extent
        let _ = Decoder::new(&file[..cut], FakeCodec, Limits::default()).and_then(|d| d.decode_primary());
    }
    let limits = Limits {
        max_pixels: 0,
        ..Limits::default()
    };
    let ispe = full(*b"ispe", 0, &[0, 0, 0, 1, 0, 0, 0, 1]);
    let big = build(
        &[ItemSpec {
            id: 1,
            kind: b"fake",
            payload: vec![1, 1, 1],
            props: vec![1, 2],
        }],
        &[hvcc_prop(), ispe],
        &[],
        1,
    );
    assert!(matches!(
        Decoder::new(&big, FakeCodec, limits).unwrap().decode_primary(),
        Err(Error::LimitExceeded(_))
    ));
}

struct PanicCodec;

impl CodecBackend for PanicCodec {
    fn handles(&self, item_type: FourCc) -> bool {
        &item_type == b"fake"
    }

    fn decode(&self, _config: &[u8], _payload: &[u8]) -> Result<YuvImage> {
        panic!("boom");
    }
}

#[test]
fn codec_panic_becomes_error() {
    let items = [ItemSpec {
        id: 1,
        kind: b"fake",
        payload: vec![1, 1, 1],
        props: vec![1],
    }];
    let file = build(&items, &[hvcc_prop()], &[], 1);
    let r = Decoder::new(&file, PanicCodec, Limits::default())
        .unwrap()
        .decode_primary();
    assert_eq!(r, Err(Error::Invalid("decoder panicked on this input")));
}

struct HvcFake;

impl CodecBackend for HvcFake {
    fn handles(&self, item_type: FourCc) -> bool {
        &item_type == b"hvc1"
    }

    fn decode(&self, config: &[u8], payload: &[u8]) -> Result<YuvImage> {
        assert_eq!(config, [7, 7]);
        FakeCodec.decode(&[], payload)
    }
}

/// `ftyp` + `moov` (one `pict` track with an `hvc1` entry) + `mdat` holding two samples; no `meta` at all.
fn sequence_file() -> Vec<u8> {
    let ftyp = bx(*b"ftyp", b"msf1\0\0\0\0msf1");
    let build_moov = |sample_offset: u32| {
        let mut entry = vec![0; 6 + 2 + 16];
        entry.extend_from_slice(&[0, 2, 0, 1]);
        entry.extend_from_slice(&[0; 4 + 4 + 4 + 2 + 32 + 2 + 2]);
        entry.extend(bx(*b"hvcC", &[7, 7]));
        let mut stsd = 1u32.to_be_bytes().to_vec();
        stsd.extend(bx(*b"hvc1", &entry));
        let stsz = full(
            *b"stsz",
            0,
            &[
                &0u32.to_be_bytes()[..],
                &2u32.to_be_bytes(),
                &3u32.to_be_bytes(),
                &3u32.to_be_bytes(),
            ]
            .concat(),
        );
        let stco = full(
            *b"stco",
            0,
            &[&1u32.to_be_bytes()[..], &sample_offset.to_be_bytes()].concat(),
        );
        let stbl = bx(*b"stbl", &[full(*b"stsd", 0, &stsd), stsz, stco].concat());
        let hdlr = full(*b"hdlr", 0, &[&[0u8; 4][..], b"pict"].concat());
        let mdia = bx(*b"mdia", &[hdlr, bx(*b"minf", &stbl)].concat());
        bx(*b"moov", &bx(*b"trak", &mdia))
    };
    let base = (ftyp.len() + build_moov(0).len() + 8) as u32;
    [ftyp, build_moov(base), bx(*b"mdat", &[2, 1, 255, 9, 9, 9])].concat()
}

#[test]
fn first_frame_of_an_image_sequence() {
    let file = sequence_file();
    let img = Decoder::new(&file, HvcFake, Limits::default())
        .unwrap()
        .decode_primary()
        .unwrap();
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(&img.data[..4], &[255, 255, 255, 255]);
}

#[test]
fn info_applies_rotation_without_decoding() {
    let ispe = full(*b"ispe", 0, &[0, 0, 0, 2, 0, 0, 0, 1]);
    let pixi = full(*b"pixi", 0, &[1, 10]);
    let props = vec![hvcc_prop(), bx(*b"irot", &[1]), ispe, pixi];
    let items = [ItemSpec {
        id: 1,
        kind: b"fake",
        payload: vec![],
        props: vec![1, 2, 3, 4],
    }];
    let file = build(&items, &props, &[], 1);
    let info = Decoder::new(&file, PanicCodec, Limits::default())
        .unwrap()
        .info()
        .unwrap();
    assert_eq!(
        (info.width, info.height, info.bit_depth, info.has_alpha),
        (1, 2, Some(10), false)
    );
}
