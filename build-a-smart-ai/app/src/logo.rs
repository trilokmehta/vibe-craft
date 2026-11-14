//! Vibe Craft logo, drawn procedurally so the window icon, the header badge
//! and the .exe icon (via build.rs) all come from one place.
//!
//! Design: a rounded square with a violet -> pink -> cyan diagonal gradient,
//! faint vinyl grooves, and a bold white "V" whose right arm turns into a
//! row of equalizer bars.

fn clamp01(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Signed distance to a rounded box centred at (0.5, 0.5).
fn sd_round_box(x: f32, y: f32, half: f32, r: f32) -> f32 {
    let qx = (x - 0.5).abs() - (half - r);
    let qy = (y - 0.5).abs() - (half - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let h = clamp01((pax * bax + pay * bay) / (bax * bax + bay * bay));
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt()
}

/// Colour + alpha of the logo at a point in unit space.
fn sample(x: f32, y: f32) -> [f32; 4] {
    let bg = sd_round_box(x, y, 0.47, 0.20);
    if bg > 0.0 {
        return [0.0; 4];
    }
    // Diagonal gradient.
    let violet = [0.42, 0.16, 0.85];
    let pink = [0.93, 0.28, 0.60];
    let cyan = [0.13, 0.83, 0.93];
    let t = clamp01((x + y) * 0.5);
    let mut c = if t < 0.55 { mix(violet, pink, t / 0.55) } else { mix(pink, cyan, (t - 0.55) / 0.45) };
    // Soft top highlight.
    let hl = clamp01(1.0 - y * 1.8) * 0.12;
    c = mix(c, [1.0, 1.0, 1.0], hl);
    // Vinyl grooves.
    let (dx, dy) = (x - 0.5, y - 0.5);
    let r = (dx * dx + dy * dy).sqrt();
    for ring in [0.30f32, 0.37, 0.43] {
        if (r - ring).abs() < 0.008 {
            c = mix(c, [1.0, 1.0, 1.0], 0.18);
        }
    }
    // The V: left arm is a solid stroke, right arm is equalizer bars.
    let th = 0.065;
    let left = sd_segment(x, y, 0.27, 0.28, 0.47, 0.74);
    let mut white = left < th;
    let bars = [(0.56, 0.58), (0.635, 0.46), (0.71, 0.34)];
    for (bx, top) in bars {
        if (x - bx).abs() < 0.03 && y > top && y < 0.74 {
            let d = sd_segment(x, y, bx, top + 0.03, bx, 0.71);
            if d < 0.03 {
                white = true;
            }
        }
    }
    if white {
        c = [1.0, 1.0, 1.0];
    }
    [c[0], c[1], c[2], 1.0]
}

/// Render the logo as straight-alpha RGBA, size x size, 4x4 supersampled.
pub fn render(size: u32) -> Vec<u8> {
    let n = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..n {
                for sx in 0..n {
                    let x = (px as f32 + (sx as f32 + 0.5) / n as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / n as f32) / size as f32;
                    let s = sample(x, y);
                    acc[0] += s[0] * s[3];
                    acc[1] += s[1] * s[3];
                    acc[2] += s[2] * s[3];
                    acc[3] += s[3];
                }
            }
            let a = acc[3] / (n * n) as f32;
            let un = |v: f32| if acc[3] > 0.0 { (v / acc[3] * 255.0).round() as u8 } else { 0 };
            out.extend_from_slice(&[un(acc[0]), un(acc[1]), un(acc[2]), (a * 255.0).round() as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn write_preview() {
        let px = super::render(256);
        image::save_buffer("shots/logo_preview.png", &px, 256, 256, image::ColorType::Rgba8).unwrap();
    }
}
