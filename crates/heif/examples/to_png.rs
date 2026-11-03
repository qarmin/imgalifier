//! Decodes a HEIC file to a PNG: to_png <in.heic> <out.png>

#![expect(clippy::print_stderr, clippy::indexing_slicing, reason = "developer tool")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).expect("read input");
    match imgalifier_heif::decode_heic(&data) {
        Ok(img) => {
            let mut enc = png::Encoder::new(
                std::io::BufWriter::new(std::fs::File::create(&args[2]).expect("create output")),
                img.width,
                img.height,
            );
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()
                .expect("header")
                .write_image_data(&img.data)
                .expect("data");
        }
        Err(e) => eprintln!("error: {e}"),
    }
}
