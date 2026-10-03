//! Color as text: hex, RGB, HSL, or OKLCH. Any of them parses, whichever
//! format is shown.
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Hex,
    Rgb,
    Hsl,
    Oklch,
}

static PREFERRED: AtomicU8 = AtomicU8::new(0);

/// The format every color field shows; choosing one in any picker sets it
/// for all of them.
pub fn preferred() -> Format {
    FORMATS[PREFERRED.load(Ordering::Relaxed) as usize % FORMATS.len()].0
}

pub fn set_preferred(format: Format) {
    let index = FORMATS.iter().position(|(f, _)| *f == format).unwrap_or(0);
    PREFERRED.store(index as u8, Ordering::Relaxed);
}

pub fn name(format: Format) -> &'static str {
    FORMATS
        .iter()
        .find(|(f, _)| *f == format)
        .map_or("Hex", |(_, n)| n)
}

/// The format with this name, as `name` gives it.
pub fn format_named(name: &str) -> Option<Format> {
    FORMATS.iter().find(|(_, n)| *n == name).map(|(f, _)| *f)
}

pub const FORMATS: [(Format, &str); 4] = [
    (Format::Hex, "Hex"),
    (Format::Rgb, "RGB"),
    (Format::Hsl, "HSL"),
    (Format::Oklch, "OKLCH"),
];

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    }
}

/// sRGB to OKLCH: lightness 0 to 1, chroma, hue in degrees.
fn to_oklch([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let hue = b.atan2(a).to_degrees().rem_euclid(360.);
    (lightness, a.hypot(b), hue)
}

fn from_oklch(lightness: f32, chroma: f32, hue: f32) -> [f32; 3] {
    let (a, b) = (
        chroma * hue.to_radians().cos(),
        chroma * hue.to_radians().sin(),
    );
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
    .map(|c| from_linear(c.clamp(0., 1.)))
}

fn to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = (max + min) / 2.;
    let delta = max - min;
    if delta == 0. {
        return (0., 0., lightness);
    }
    let saturation = delta / (1. - (2. * lightness - 1.).abs());
    let hue = if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    (hue, saturation, lightness)
}

fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let c = (1. - (2. * lightness - 1.).abs()) * saturation;
    let (r, g, b) = crate::color_picker::hue_rgb(hue, c);
    let m = lightness - c / 2.;
    [r + m, g + m, b + m]
}

fn alpha_suffix(alpha: u8) -> String {
    if alpha == 255 {
        String::new()
    } else {
        format!(" / {:.0}%", alpha as f32 / 2.55)
    }
}

pub fn format(color: [u8; 4], format: Format) -> String {
    let [r, g, b, a] = color;
    let unit = [r, g, b].map(|c| c as f32 / 255.);
    match format {
        Format::Hex if a == 255 => format!("{r:02X}{g:02X}{b:02X}"),
        Format::Hex => format!("{r:02X}{g:02X}{b:02X}{a:02X}"),
        Format::Rgb => format!("rgb({r} {g} {b}{})", alpha_suffix(a)),
        Format::Hsl => {
            let (h, s, l) = to_hsl(unit);
            format!(
                "hsl({h:.0} {:.0}% {:.0}%{})",
                s * 100.,
                l * 100.,
                alpha_suffix(a)
            )
        }
        Format::Oklch => {
            let (l, c, h) = to_oklch(unit);
            format!("oklch({:.1}% {c:.3} {h:.1}{})", l * 100., alpha_suffix(a))
        }
    }
}

/// Numbers in `text`, with a flag for each that ended in `%`.
fn numbers(text: &str) -> Vec<(f32, bool)> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == '/' || c == '(' || c == ')')
        .filter_map(|word| {
            let percent = word.ends_with('%');
            let digits = word.trim_end_matches('%').trim_end_matches("deg");
            digits.parse::<f32>().ok().map(|value| (value, percent))
        })
        .collect()
}

/// Reads `text` in any format; bare numbers use `shown`.
pub fn parse(text: &str, shown: Format) -> Option<[u8; 4]> {
    let text = text.trim().to_ascii_lowercase();
    let format = if text.starts_with("rgb") {
        Format::Rgb
    } else if text.starts_with("hsl") {
        Format::Hsl
    } else if text.starts_with("oklch") {
        Format::Oklch
    } else if text.starts_with('#') || shown == Format::Hex {
        Format::Hex
    } else {
        shown
    };
    if format == Format::Hex {
        let hex = text.trim_start_matches('#');
        let value = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => {
                let [_, r, g, b] = value.to_be_bytes();
                Some([r, g, b, 255])
            }
            8 => Some(value.to_be_bytes()),
            _ => None,
        };
    }
    let values = numbers(&text);
    let [(x, xp), (y, yp), (z, zp)] = values.get(..3)?.try_into().ok()?;
    let alpha = values.get(3).map_or(
        255.,
        |(a, percent)| {
            if *percent { a * 2.55 } else { a * 255. }
        },
    );
    let unit = |v: f32, percent: bool, scale: f32| if percent { v / 100. } else { v / scale };
    let rgb = match format {
        Format::Rgb => [unit(x, xp, 255.), unit(y, yp, 255.), unit(z, zp, 255.)],
        Format::Hsl => from_hsl(x, unit(y, true, 1.), unit(z, true, 1.)),
        _ => from_oklch(unit(x, xp, 1.), unit(y, yp, 1.), z),
    };
    let [r, g, b] = rgb.map(|c| (c.clamp(0., 1.) * 255.).round() as u8);
    Some([r, g, b, alpha.clamp(0., 255.).round() as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_round_trips() {
        for color in [[229, 72, 77, 255], [74, 127, 224, 128], [16, 16, 16, 255]] {
            for (shown, _) in FORMATS {
                let text = format(color, shown);
                let back = parse(&text, shown).unwrap();
                // Whole-number HSL percentages round by up to a few steps.
                let slack = if shown == Format::Hsl { 3 } else { 1 };
                for (a, b) in color.iter().zip(back) {
                    assert!(
                        (*a as i32 - b as i32).abs() <= slack,
                        "{text}: {color:?} {back:?}"
                    );
                }
            }
        }
    }
}
