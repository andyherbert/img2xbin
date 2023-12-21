use clap::Parser;
use image::{DynamicImage, GenericImageView};
use imagequant::RGBA;
use oklab::srgb_to_oklab;
use std::path::Path;
use std::{io::Write, path::PathBuf};

#[derive(Parser, Debug)]
struct Args {
    #[clap(value_name = "INPUT")]
    input: PathBuf,
    #[clap(value_name = "OUTPUT")]
    output: PathBuf,
}

struct Palettes {
    vga: Vec<RGBA>,
    rgba: Vec<RGBA>,
}

impl Palettes {
    fn new(rgba: &[RGBA]) -> Self {
        let mut vga: Vec<RGBA> = rgba
            .iter()
            .map(|color| RGBA {
                r: (color.r as f32 / 255.0 * 63.0).round() as u8,
                g: (color.g as f32 / 255.0 * 63.0).round() as u8,
                b: (color.b as f32 / 255.0 * 63.0).round() as u8,
                a: 255,
            })
            .collect();
        while vga.len() < 16 {
            vga.push(RGBA {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            });
        }
        let rgba: Vec<RGBA> = vga
            .iter()
            .map(|color| RGBA {
                r: color.r * 4,
                g: color.g * 4,
                b: color.b * 4,
                a: 255,
            })
            .collect();
        Palettes { vga, rgba }
    }
}

fn quantize_image_16(image: &DynamicImage) -> (Palettes, Vec<u8>) {
    let (width, height) = image.dimensions();
    let pixels: Vec<RGBA> = image
        .pixels()
        .map(|(_, _, pixel)| RGBA {
            r: pixel[0],
            g: pixel[1],
            b: pixel[2],
            a: 255,
        })
        .collect();
    let mut liq = imagequant::new();
    liq.set_speed(1).unwrap();
    liq.set_max_colors(16).unwrap();
    let mut image = liq
        .new_image(&pixels[..], width as usize, height as usize, 0.0)
        .unwrap();
    let mut res = liq.quantize(&mut image).unwrap();
    let (palette, indexes) = res.remapped(&mut image).unwrap();
    let palettes = Palettes::new(&palette);
    let mut quantized = DynamicImage::new_rgb8(width, height);
    quantized
        .as_mut_rgb8()
        .unwrap()
        .rchunks_exact_mut(3)
        .zip(indexes.iter().rev())
        .for_each(|(dst, index)| {
            let color = palettes.rgba[*index as usize];
            dst[0] = color.r;
            dst[1] = color.g;
            dst[2] = color.b;
        });
    (palettes, indexes)
}

fn find_closest(rgba: &RGBA, palette: &[RGBA]) -> u8 {
    let mut closest: Option<u8> = None;
    let mut closest_distance: f32 = 0.0;
    let ok_palette: Vec<_> = palette
        .iter()
        .map(|color| {
            srgb_to_oklab(oklab::RGB {
                r: color.r,
                g: color.g,
                b: color.b,
            })
        })
        .collect();
    let ok_rgba = srgb_to_oklab(oklab::RGB {
        r: rgba.r,
        g: rgba.g,
        b: rgba.b,
    });
    for (index, color) in ok_palette.iter().enumerate() {
        let distance = (color.l - ok_rgba.l).powi(2)
            + (color.a - ok_rgba.a).powi(2)
            + (color.b - ok_rgba.b).powi(2);
        if closest.is_none() || distance < closest_distance {
            closest_distance = distance;
            closest = Some(index as u8);
        }
    }
    closest.expect("match")
}

struct Chunk {
    fg: u8,
    bg: u8,
    codepoint: u8,
}

fn break_into_chunks(palettes: &Palettes, mut indexes: Vec<u8>) -> Vec<Chunk> {
    indexes
        .chunks_exact_mut(8)
        .map(|chunk| {
            let mut scores: Vec<(u8, usize)> = (0..16)
                .map(|index| {
                    let mut score = 0;
                    for color_index in chunk.iter() {
                        if *color_index == index {
                            score += 1;
                        }
                    }
                    (index, score)
                })
                .collect();
            scores.sort_by(|(_, a), (_, b)| b.cmp(a));
            scores.resize(2, (0, 0));
            let common_indexes: Vec<u8> = scores.into_iter().map(|(index, _)| index).collect();
            let common_palette: Vec<RGBA> = common_indexes
                .iter()
                .map(|index| palettes.rgba[*index as usize])
                .collect();
            for color_index in chunk.iter_mut() {
                if !common_indexes.contains(color_index) {
                    let rgba = &palettes.rgba[*color_index as usize];
                    *color_index = find_closest(rgba, &common_palette);
                }
            }
            let bg = common_indexes[0];
            let fg = common_indexes[1];
            let bitmask = chunk.iter().map(|index| if *index == bg { 0 } else { 1 });
            let codepoint = bitmask.fold(0, |acc, bit| (acc << 1) + bit);
            Chunk { fg, bg, codepoint }
        })
        .collect()
}

fn palette_to_bytes(palette: &[RGBA]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for color in palette {
        bytes.push(color.r);
        bytes.push(color.g);
        bytes.push(color.b);
    }
    bytes
}

fn chunks_to_bytes(chunks: &[Chunk]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in chunks {
        bytes.push(chunk.codepoint);
        bytes.push((chunk.bg << 4) + chunk.fg);
    }
    bytes
}

fn save_xbin(path: impl AsRef<Path>, image: &DynamicImage, palette: &[RGBA], chunks: &[Chunk]) {
    let mut file = std::fs::File::create(path).unwrap();
    let palette_bytes = palette_to_bytes(palette);
    let font_bytes: Vec<u8> = (0..=255).collect();
    let chunk_bytes = chunks_to_bytes(chunks);
    let columns = image.width() / 8;
    let rows = image.height();
    file.write_all(b"XBIN\x1a").unwrap();
    file.write_all((columns as u16).to_le_bytes().as_ref())
        .unwrap();
    file.write_all((rows as u16).to_le_bytes().as_ref())
        .unwrap();
    file.write_all(b"\x01\x0b").unwrap();
    file.write_all(&palette_bytes).unwrap();
    file.write_all(&font_bytes).unwrap();
    file.write_all(&chunk_bytes).unwrap();
}

fn main() {
    let args = Args::parse();
    let image = image::open(args.input).unwrap();
    let (palettes, indexes) = quantize_image_16(&image);
    let chunks = break_into_chunks(&palettes, indexes);
    save_xbin(args.output, &image, &palettes.vga, &chunks);
}
