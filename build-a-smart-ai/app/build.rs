//! Generates the Vibe Craft .ico from src/logo.rs and embeds it in the Windows exe.

#[path = "src/logo.rs"]
mod logo;

use std::io::Write;

fn ico(sizes: &[u32]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let rgba = logo::render(s);
        let mut d = Vec::new();
        // BITMAPINFOHEADER, height doubled for the AND mask.
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&(s as i32).to_le_bytes());
        d.extend_from_slice(&((s * 2) as i32).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        for y in (0..s).rev() {
            for x in 0..s {
                let i = ((y * s + x) * 4) as usize;
                d.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        let mask_row = ((s + 31) / 32) * 4;
        d.extend(std::iter::repeat(0u8).take((mask_row * s) as usize));
        images.push(d);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (i, &s) in sizes.iter().enumerate() {
        let b = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(images[i].len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += images[i].len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let path = out.join("vibe-craft.ico");
    std::fs::File::create(&path).unwrap().write_all(&ico(&[16, 24, 32, 48, 64, 128, 256])).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(path.to_str().unwrap());
        res.set("ProductName", "Vibe Craft");
        res.set("FileDescription", "Vibe Craft");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed exe icon: {}", e);
        }
    }
}
