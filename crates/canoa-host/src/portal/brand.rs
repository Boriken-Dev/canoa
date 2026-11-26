//! The name and mark the portal's pages carry, and their colours: the build's own brand (`app/brand.json` and its icon, which the
//! app reads too), or a `brand.json` the settings name. The brand's colours draw the mark; the buttons, links, page and card are the
//! app's default look (`brand/look.json`).

use std::path::Path;

use serde_json::Value;

use super::page::escape;

const BUILTIN: &str = canoa::brand::JSON;
const BUILTIN_ICON: &str = canoa::brand::ICON_SVG;
const LOOK: &str = include_str!("brand/look.json");

/// The least contrast (WCAG) the accent keeps against the card, the text on a button against the accent, and a notice's text
/// against its tint, unless the settings turn the check off.
const CONTRAST: f64 = 4.5;
const WHITE: [u8; 3] = [0xff; 3];
const BLACK: [u8; 3] = [0; 3];
/// How much of a notice's colour its background takes from the card.
const TINT: f64 = 0.14;

/// The colours of one scheme of the look.
struct Scheme {
    page: [u8; 3],
    card: [u8; 3],
    field: [u8; 3],
    line: [u8; 3],
    text: [u8; 3],
    muted: [u8; 3],
    accent: [u8; 3],
    on_accent: [u8; 3],
    error: [u8; 3],
    ok: [u8; 3],
}

impl Scheme {
    fn read(look: &Value, scheme: &str) -> Result<Self, String> {
        let get = |key: &str| {
            look.get(scheme)
                .and_then(|each| each.get(key))
                .and_then(Value::as_str)
                .and_then(hex)
                .ok_or_else(|| format!("the look has no colour `{scheme}.{key}`"))
        };
        Ok(Self {
            page: get("page")?,
            card: get("card")?,
            field: get("field")?,
            line: get("line")?,
            text: get("text")?,
            muted: get("muted")?,
            accent: get("accent")?,
            on_accent: get("on_accent")?,
            error: get("error")?,
            ok: get("ok")?,
        })
    }

    /// The custom properties of this scheme. With `adjust`, each text colour is moved toward `toward` until it holds `CONTRAST`
    /// where it is read.
    fn properties(&self, adjust: bool, toward: [u8; 3]) -> String {
        let accent = if adjust {
            reach(self.accent, self.card, toward)
        } else {
            self.accent
        };
        let on_accent = if adjust && contrast(self.on_accent, accent) < CONTRAST {
            on(accent)
        } else {
            self.on_accent
        };
        let error_bg = mix(self.card, self.error, TINT);
        let ok_bg = mix(self.card, self.ok, TINT);
        let (error, ok) = if adjust {
            (
                reach(self.error, error_bg, toward),
                reach(self.ok, ok_bg, toward),
            )
        } else {
            (self.error, self.ok)
        };
        let vars = [
            ("bg", self.page),
            ("card", self.card),
            ("line", self.line),
            ("text", self.text),
            ("muted", self.muted),
            ("field", self.field),
            ("accent", accent),
            ("on-accent", on_accent),
            ("error-bg", error_bg),
            ("error-text", error),
            ("ok-bg", ok_bg),
            ("ok-text", ok),
        ];
        vars.iter()
            .map(|(name, rgb)| format!("--{name}: {};", css_colour(*rgb)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `from` moved toward `to` by `part` of the way.
fn mix(from: [u8; 3], to: [u8; 3], part: f64) -> [u8; 3] {
    let one = |from: u8, to: u8| {
        (f64::from(from) + (f64::from(to) - f64::from(from)) * part).round() as u8
    };
    [
        one(from[0], to[0]),
        one(from[1], to[1]),
        one(from[2], to[2]),
    ]
}

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
    /// The brand this build carries. `adjust` moves the look's text colours until they hold 4.5 where they are read; off, they
    /// are used as given.
    pub fn builtin(adjust: bool) -> Self {
        Self::from_parts(BUILTIN, BUILTIN_ICON, adjust).expect("the built-in brand is valid")
    }

    /// The brand `brand.json` at `path` names, its icon read from the path in its `icon`, relative to the file.
    pub fn load(path: &Path, adjust: bool) -> Result<Self, String> {
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
        Self::from_parts(&text, &svg, adjust).map_err(fail)
    }

    fn from_parts(json: &str, svg: &str, adjust: bool) -> Result<Self, String> {
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
        let look: Value = serde_json::from_str(LOOK).map_err(|error| error.to_string())?;
        let light = Scheme::read(&look, "light")?.properties(adjust, BLACK);
        let night = Scheme::read(&look, "dark")?.properties(adjust, WHITE);
        let css = format!(
            "{}\n:root {{ {light} --mark: {}; }}\n@media (prefers-color-scheme: dark) {{ :root {{ {night} --mark: {}; }} }}\n.mark [fill=\"{accent_text}\" i] {{ fill: var(--mark); }}\n",
            include_str!("page/login.css"),
            css_colour(accent),
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
    if contrast(WHITE, accent) >= contrast(BLACK, accent) {
        WHITE
    } else {
        BLACK
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

    fn scheme(name: &str) -> Scheme {
        Scheme::read(&serde_json::from_str(LOOK).unwrap(), name).unwrap()
    }

    #[test]
    fn the_built_in_brand_is_the_apps() {
        let brand = Brand::builtin(true);
        assert_eq!(brand.name_html(), "Canoa");
        assert!(brand.mark().starts_with("<svg class=\"mark\""));
        assert!(!brand.mark().contains("<title") && !brand.mark().contains("<!--"));
        assert!(brand.css().contains("--mark: #3dafa1"));
        assert!(brand.css().contains("--mark: #6de2d2"));
        assert!(brand.css().contains("[fill=\"#3DAFA1\" i]"));
    }

    #[test]
    fn the_buttons_links_page_and_card_are_the_default_looks() {
        let css = Brand::builtin(true).css().to_owned();
        for colour in [
            "--accent: #9a5b1e",
            "--on-accent: #ffffff",
            "--bg: #fffdf8",
            "--card: #faf7f1",
            "--line: #d2cfc8",
            "--text: #2b2822",
            "--muted: #6a6358",
            "--accent: #d9a066",
            "--on-accent: #241707",
            "--bg: #262523",
            "--card: #232220",
            "--line: #504e4a",
        ] {
            assert!(css.contains(colour), "{colour}");
        }
        // The brand's teal is the mark's alone.
        assert!(!css.contains("--accent: #3dafa1") && !css.contains("--accent: #6de2d2"));
    }

    /// The default look's amber already holds 4.5 on its card, so the check moves nothing.
    #[test]
    fn the_default_look_needs_no_adjustment() {
        for (name, toward) in [("light", BLACK), ("dark", WHITE)] {
            let look = scheme(name);
            let accent = contrast(look.accent, look.card);
            let button = contrast(look.on_accent, look.accent);
            let error = contrast(look.error, mix(look.card, look.error, TINT));
            let ok = contrast(look.ok, mix(look.card, look.ok, TINT));
            eprintln!(
                "{name}: accent {accent:.2}, button {button:.2}, error {error:.2}, ok {ok:.2}"
            );
            assert!(accent >= CONTRAST && button >= CONTRAST, "{name}");
            assert_eq!(reach(look.accent, look.card, toward), look.accent);
        }
        assert_eq!(Brand::builtin(true).css(), Brand::builtin(false).css());
    }

    #[test]
    fn the_accent_holds_its_contrast_on_the_card_and_the_text_on_it() {
        let card = scheme("light").card;
        let night = scheme("dark").card;
        for (accent, dark) in [
            ("#3DAFA1", "#6DE2D2"),
            ("#ffff00", "#101010"),
            ("#000000", "#ffffff"),
        ] {
            let (accent, dark) = (hex(accent).unwrap(), hex(dark).unwrap());
            let light = reach(accent, card, BLACK);
            let dim = reach(dark, night, WHITE);
            assert!(contrast(light, card) >= CONTRAST, "{light:?}");
            assert!(contrast(on(light), light) >= CONTRAST, "{light:?}");
            assert!(contrast(dim, night) >= CONTRAST, "{dim:?}");
            assert!(contrast(on(dim), dim) >= CONTRAST, "{dim:?}");
        }
    }

    /// A look whose accent is too pale for its card is adjusted when the check is on and used as given when it is off.
    #[test]
    fn the_contrast_check_adjusts_an_accent_and_off_leaves_it() {
        let mut look = scheme("light");
        look.accent = hex("#e0b070").unwrap();
        look.on_accent = hex("#ffffff").unwrap();
        assert!(contrast(look.accent, look.card) < CONTRAST);
        let strict = look.properties(true, BLACK);
        let off = look.properties(false, BLACK);
        assert!(off.contains("--accent: #e0b070;") && off.contains("--on-accent: #ffffff;"));
        let adjusted = strict
            .split("--accent: ")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .and_then(hex)
            .unwrap();
        assert_ne!(adjusted, look.accent);
        assert!(contrast(adjusted, look.card) >= CONTRAST);
        assert!(!strict.contains("--accent: #e0b070;"));
    }
}
