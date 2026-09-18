//! Dev-time fixture writer used by `tests/fixtures/pdf/generate.py`.
//!
//! Usage: `cargo run --example gen_pdf_fixture -- <out.pdf> <w> <h> <seed> [<w> <h> <seed> ...]`
//!
//! Builds pages with the same frozen 16-bit LCG the crate's golden tests
//! use, runs them through `export_pdf`, and writes the bytes to `<out.pdf>`.
//! This is the ONLY sanctioned way to (re)generate the committed PDF
//! fixtures, so the fixtures are byte-for-byte what the shipped writer
//! produces; the generation script then cross-parses them with an
//! independent PDF parser.

use foldscan_domain::{export_pdf, GrayFrame};

struct Lcg(u16);

impl Lcg {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(25173)
            .wrapping_add(13849)
            .wrapping_shl(3)
            ^ self.0.wrapping_shr(5);
        (self.0 >> 8) as u8
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out_path, rest)) = args.split_first() else {
        eprintln!("usage: gen_pdf_fixture <out.pdf> <w> <h> <seed> [<w> <h> <seed> ...]");
        std::process::exit(2);
    };
    if rest.is_empty() || rest.len() % 3 != 0 {
        eprintln!("pages must be given as (w, h, seed) triples");
        std::process::exit(2);
    }
    let mut pages = Vec::new();
    for triple in rest.chunks(3) {
        let w: u32 = triple[0].parse()?;
        let h: u32 = triple[1].parse()?;
        let seed: u16 = triple[2].parse()?;
        let mut rng = Lcg(seed);
        let pixels = (0..(w * h)).map(|_| rng.next()).collect();
        pages.push(GrayFrame::from_pixels(w, h, pixels)?);
    }
    let doc = export_pdf(&pages)?;
    std::fs::write(out_path, doc)?;
    Ok(())
}
