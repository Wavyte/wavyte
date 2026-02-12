use crate::foundation::core::Rgba8Premul;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct ColorDef {
    pub(crate) r: f64,
    pub(crate) g: f64,
    pub(crate) b: f64,
    pub(crate) a: f64,
}

impl ColorDef {
    pub(crate) fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    pub(crate) fn to_rgba8_premul(self) -> Rgba8Premul {
        fn to_u8(x: f64) -> u8 {
            (x.clamp(0.0, 1.0) * 255.0).round() as u8
        }

        let a = self.a.clamp(0.0, 1.0);
        let r = (self.r.clamp(0.0, 1.0) * a).clamp(0.0, 1.0);
        let g = (self.g.clamp(0.0, 1.0) * a).clamp(0.0, 1.0);
        let b = (self.b.clamp(0.0, 1.0) * a).clamp(0.0, 1.0);

        Rgba8Premul {
            r: to_u8(r),
            g: to_u8(g),
            b: to_u8(b),
            a: to_u8(a),
        }
    }
}

impl<'de> Deserialize<'de> for ColorDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Hex(String),
            RgbaObj {
                r: f64,
                g: f64,
                b: f64,
                #[serde(default = "one")]
                a: f64,
            },
            HslaObj {
                h: f64,
                s: f64,
                l: f64,
                #[serde(default = "one")]
                a: f64,
            },
            Arr(Vec<f64>),
        }

        fn one() -> f64 {
            1.0
        }

        match Repr::deserialize(deserializer)? {
            Repr::Hex(s) => parse_hex(&s).map_err(serde::de::Error::custom),
            Repr::RgbaObj { r, g, b, a } => Ok(Self::rgba(r, g, b, a)),
            Repr::HslaObj { h, s, l, a } => Ok(hsla_to_rgba(h, s, l, a)),
            Repr::Arr(v) => {
                if v.len() == 3 {
                    Ok(Self::rgba(v[0], v[1], v[2], 1.0))
                } else if v.len() == 4 {
                    Ok(Self::rgba(v[0], v[1], v[2], v[3]))
                } else {
                    Err(serde::de::Error::custom(
                        "rgba array must have len 3 ([r,g,b]) or 4 ([r,g,b,a])",
                    ))
                }
            }
        }
    }
}

fn parse_hex(s: &str) -> Result<ColorDef, String> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);

    fn hex_byte(pair: &str) -> Result<u8, String> {
        u8::from_str_radix(pair, 16).map_err(|_| format!("invalid hex byte \"{pair}\""))
    }

    let (r, g, b, a) = match s.len() {
        6 => {
            let r = hex_byte(&s[0..2])?;
            let g = hex_byte(&s[2..4])?;
            let b = hex_byte(&s[4..6])?;
            (r, g, b, 255)
        }
        8 => {
            let r = hex_byte(&s[0..2])?;
            let g = hex_byte(&s[2..4])?;
            let b = hex_byte(&s[4..6])?;
            let a = hex_byte(&s[6..8])?;
            (r, g, b, a)
        }
        _ => {
            return Err("hex color must be #RRGGBB or #RRGGBBAA (case-insensitive)".to_owned());
        }
    };

    Ok(ColorDef::rgba(
        (r as f64) / 255.0,
        (g as f64) / 255.0,
        (b as f64) / 255.0,
        (a as f64) / 255.0,
    ))
}

fn hsla_to_rgba(h: f64, s: f64, l: f64, a: f64) -> ColorDef {
    // Standard HSL -> RGB conversion (sRGB space, normalized 0..1 inputs).
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);

    if s == 0.0 {
        return ColorDef::rgba(l, l, l, a);
    }

    fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;

    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);
    ColorDef::rgba(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_hex_rgb_and_rgba() {
        let c: ColorDef = serde_json::from_value(json!("#ff0000")).unwrap();
        assert_eq!(c, ColorDef::rgba(1.0, 0.0, 0.0, 1.0));

        let c: ColorDef = serde_json::from_value(json!("#0000ff80")).unwrap();
        assert!((c.b - 1.0).abs() < 1e-9);
        assert!((c.a - (128.0 / 255.0)).abs() < 1e-9);
    }

    #[test]
    fn parses_rgba_object_and_array() {
        let c: ColorDef = serde_json::from_value(json!({"r": 0.25, "g": 0.5, "b": 0.75})).unwrap();
        assert_eq!(c, ColorDef::rgba(0.25, 0.5, 0.75, 1.0));

        let c: ColorDef = serde_json::from_value(json!([0.25, 0.5, 0.75, 0.9])).unwrap();
        assert_eq!(c, ColorDef::rgba(0.25, 0.5, 0.75, 0.9));
    }

    #[test]
    fn parses_hsla_object() {
        let c: ColorDef = serde_json::from_value(json!({"h": 0.0, "s": 1.0, "l": 0.5})).unwrap();
        // Pure red.
        assert!((c.r - 1.0).abs() < 1e-9);
        assert!((c.g - 0.0).abs() < 1e-9);
        assert!((c.b - 0.0).abs() < 1e-9);
    }
}
