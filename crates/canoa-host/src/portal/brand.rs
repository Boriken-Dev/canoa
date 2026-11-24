//! The name, mark and colours the portal's pages carry: the build's own brand (`app/brand.json` and its icon, which the app reads
//! too), or a `brand.json` the settings name.

use std::path::Path;

use serde_json::Value;

use super::page::escape;

const BUILTIN: &str = include_str!("brand/brand.json");
const BUILTIN_ICON: &str = include_str!("brand/canoa.svg");

/// The least contrast (WCAG) the accent keeps against the card, and the text on a button against the accent.
const CONTRAST: f64 = 4.5;
const CARD_LIGHT: [u8; 3] = [0xfa, 0xf7, 0xf1];
const CARD_DARK: [u8; 3] = [0x23, 0x22, 0x20];
const TEXT_LIGHT: [u8; 3] = [0xff, 0xfd, 0xf8];
const TEXT_DARK: [u8; 3] = [0x1b, 0x1a, 0x18];

/// What the pages say and draw for the product: its name, its mark as inline SVG, the icon as served for the favicon, and the
/// stylesheet that sets the accent for light and dark.
#[derive(Clone, Debug)]
pub struct Brand {
    name: String,
    mark: String,
    icon: String,
    css: String,
}

impl Brand {
    /// The brand this build carries.
    pub fn builtin() -> Self {
        Self::from_parts(BUILTIN, BUILTIN_ICON).expect("the built-in brand is valid")
    }

    /// The brand `brand.json` at `path` names, its icon read from the path in its `icon`, relative to the file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let fail = |why: String| format!("brand: {}: {why}", path.display());
        let text = std::fs::read_to_string(path).map_err(|error| fail(error.to_string()))?;
        let json: Value = serde_json::from_str(&text).map_err(|error| fail(error.to_string()))?;
        let icon = json
            .get("icon")
            .and_then(Value::as_str)
            .ok_or_else(|| fail("no `icon`".to_owned()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let icon_path = base.join(icon);
        let svg = std::fs::read_to_string(&icon_path)
            .map_err(|error| fail(format!("{}: {error}", icon_path.display())))?;
        Self::from_parts(&text, &svg).map_err(fail)
    }

    fn from_parts(json: &str, svg: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let field = |key: &str| {
            json.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("no `{key}`"))
        };
        let name = field("name")?.to_owned();
        let accent_text = field("accent")?;
        let accent = hex(accent_text).ok_or("`accent` is not a #RRGGBB colour")?;
        let dark =
            hex(field("accent_on_dark")?).ok_or("`accent_on_dark` is not a #RRGGBB colour")?;
        let mark = inline(svg, &name)?;
        let light_accent = reach(accent, CARD_LIGHT, [0, 0, 0]);
        let dark_accent = reach(dark, CARD_DARK, [255, 255, 255]);
        let css = format!(
            "{}\n:root {{ --accent: {}; --on-accent: {}; --mark: {}; }}\n@media (prefers-color-scheme: dark) {{ :root {{ --accent: {}; --on-accent: {}; --mark: {}; }} }}\n.mark [fill=\"{accent_text}\" i] {{ fill: var(--mark); }}\n",
            include_str!("page/login.css"),
            css_colour(light_accent),
            css_colour(on(light_accent)),
            css_colour(accent),
            css_colour(dark_accent),
            css_colour(on(dark_accent)),
            css_colour(dark),
        );
        Ok(Self {
            name,
            mark,
            icon: svg.to_owned(),
            css,
        })
    }

    /// The product's name, escaped for HTML.
    pub fn name_html(&self) -> String {
        escape(&self.name)
    }

    /// The mark, an inline `<svg class="mark">`.
    pub fn mark(&self) -> &str {
        &self.mark
    }

    /// The icon file as it is, for the favicon.
    pub fn icon(&self) -> &str {
        &self.icon
    }

    /// The pages' stylesheet.
    pub fn css(&self) -> &str {
        &self.css
    }
}

fn hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let pair = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        6 => Some([pair(0)?, pair(2)?, pair(4)?]),
        3 => {
            let one = |at: usize| {
                u8::from_str_radix(digits.get(at..at + 1)?, 16)
                    .ok()
                    .map(|d| d * 17)
            };
            Some([one(0)?, one(1)?, one(2)?])
        }
        _ => None,
    }
}

fn css_colour(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn luminance(rgb: [u8; 3]) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `colour` moved toward `toward` in small steps until it holds `CONTRAST` against `card`; the colour itself when it already
/// does, and `toward` when nothing before it does.
fn reach(colour: [u8; 3], card: [u8; 3], toward: [u8; 3]) -> [u8; 3] {
    for step in 0..=20u32 {
        let mix = |from: u8, to: u8| {
            let (from, to) = (f64::from(from), f64::from(to));
            (from + (to - from) * f64::from(step) / 20.0).round() as u8
        };
        let candidate = [
            mix(colour[0], toward[0]),
            mix(colour[1], toward[1]),
            mix(colour[2], toward[2]),
        ];
        if contrast(candidate, card) >= CONTRAST {
            return candidate;
        }
    }
    toward
}

/// The text colour with the better contrast on a button of `accent`.
fn on(accent: [u8; 3]) -> [u8; 3] {
    if contrast(TEXT_LIGHT, accent) >= contrast(TEXT_DARK, accent) {
        TEXT_LIGHT
    } else {
        TEXT_DARK
    }
}

/// `svg` as the page's inline mark: no prolog, comments or title, drawn at 72 px and named for the product.
fn inline(svg: &str, name: &str) -> Result<String, String> {
    let lower = svg.to_ascii_lowercase();
    if lower.contains("<script") || lower.contains("<foreignobject") {
        return Err("the icon holds a script or foreign content".to_owned());
    }
    let mut text = svg.to_owned();
    for (open, close) in [("<!--", "-->"), ("<?", "?>"), ("<title", "</title>")] {
        while let Some(start) = text.find(open) {
            let end = text[start..]
                .find(close)
                .ok_or("the icon is not well formed SVG")?;
            text.replace_range(start..start + end + close.len(), "");
        }
    }
    let start = text.find("<svg").ok_or("the icon is not an SVG")?;
    let end = start
        + text[start..]
            .find('>')
            .ok_or("the icon is not well formed SVG")?;
    let tag = &text[start..end];
    let mut kept = String::from("<svg class=\"mark\" width=\"72\" height=\"72\" role=\"img\"");
    kept.push_str(&format!(" aria-label=\"{}\"", escape(name)));
    // The root's own attributes, but for the ones set above.
    let mut rest = tag["<svg".len()..].trim_start();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|q| *q == '"' || *q == '\'');
        let Some(quote) = quote else {
            return Err("the icon is not well formed SVG".to_owned());
        };
        let close = after[1..]
            .find(quote)
            .ok_or("the icon is not well formed SVG")?;
        let value = &after[1..1 + close];
        if !matches!(
            key,
            "class" | "width" | "height" | "role" | "aria-label" | "style"
        ) {
            kept.push_str(&format!(" {key}=\"{}\"", value.replace('"', "&quot;")));
        }
        rest = after[close + 2..].trim_start();
    }
    kept.push('>');
    text.replace_range(start..=end, &kept);
    Ok(text[start..].trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_brand_is_the_apps() {
        let brand = Brand::builtin();
        assert_eq!(brand.name_html(), "Canoa");
        assert!(brand.mark().starts_with("<svg class=\"mark\""));
        assert!(!brand.mark().contains("<title") && !brand.mark().contains("<!--"));
        assert!(brand.css().contains("--mark: #3dafa1"));
        assert!(brand.css().contains("--mark: #6de2d2"));
        assert!(brand.css().contains("[fill=\"#3DAFA1\" i]"));
    }

    #[test]
    fn the_accent_holds_its_contrast_on_the_card_and_the_text_on_it() {
        for (accent, dark) in [
            ("#3DAFA1", "#6DE2D2"),
            ("#ffff00", "#101010"),
            ("#000000", "#ffffff"),
        ] {
            let (accent, dark) = (hex(accent).unwrap(), hex(dark).unwrap());
            let light = reach(accent, CARD_LIGHT, [0, 0, 0]);
            let night = reach(dark, CARD_DARK, [255, 255, 255]);
            assert!(contrast(light, CARD_LIGHT) >= CONTRAST, "{light:?}");
            assert!(contrast(on(light), light) >= CONTRAST, "{light:?}");
            assert!(contrast(night, CARD_DARK) >= CONTRAST, "{night:?}");
            assert!(contrast(on(night), night) >= CONTRAST, "{night:?}");
        }
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;

    /// The crate builds packaged on its own, so it carries a copy of the app's brand; a checkout holds both.
    #[test]
    fn the_built_in_brand_is_a_copy_of_the_apps() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (copy, original) in [
            (BUILTIN, "app/brand.json"),
            (BUILTIN_ICON, "app/assets/icons/canoa.svg"),
        ] {
            let Ok(app) = std::fs::read_to_string(root.join(original)) else {
                continue;
            };
            assert_eq!(
                copy.replace("\r\n", "\n"),
                app.replace("\r\n", "\n"),
                "the portal's copy of {original} is stale: copy {original} over crates/canoa-host/src/portal/brand/ (brand.json, canoa.svg)"
            );
        }
    }
}
