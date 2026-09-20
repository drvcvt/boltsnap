use ab_glyph::{FontVec, VariableFont};
use std::path::PathBuf;
use std::process::Command;

pub(crate) use crate::shelf::font::fallback_popup_font;

fn parse_gsettings_font(output: &str) -> Option<String> {
    let value = output.trim().trim_matches('\'');
    let (family, size) = value.rsplit_once(' ')?;
    size.parse::<f32>().ok()?;
    (!family.is_empty()).then(|| family.to_owned())
}

fn parse_fc_match(output: &str) -> Option<(PathBuf, u32)> {
    let (path, index) = output.lines().next()?.split_once('\t')?;
    (!path.is_empty()).then_some((PathBuf::from(path), index.trim().parse().ok()?))
}

/// Ask fontconfig which face backs `query`. One `fc-match` costs about 10 ms,
/// so callers that need more than one instance of a face resolve it once.
fn fontconfig_source(query: &str) -> Option<(PathBuf, u32)> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\t%{index}\n", query])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_fc_match(&String::from_utf8(output.stdout).ok()?)
}

fn load_fontconfig_font(query: &str) -> Option<FontVec> {
    let (path, index) = fontconfig_source(query)?;
    FontVec::try_from_vec_and_index(std::fs::read(path).ok()?, index).ok()
}

/// The face the chrome should use, as bytes, so one read serves every weight.
fn ui_font_bytes(family: Option<&str>) -> Option<(Vec<u8>, u32)> {
    let (path, index) = family
        .filter(|family| !family.is_empty())
        .and_then(fontconfig_source)
        .or_else(|| desktop_font_family().as_deref().and_then(fontconfig_source))
        .or_else(|| fontconfig_source("sans-serif"))?;
    Some((std::fs::read(path).ok()?, index))
}

/// Load the family boltsnap's chrome is configured with, falling back to the
/// desktop font when the config names none.
pub fn load_ui_font(family: Option<&str>) -> FontVec {
    family
        .filter(|family| !family.is_empty())
        .and_then(load_fontconfig_font)
        .unwrap_or_else(load_popup_font)
}

/// The chrome font at two variable weights, resolving fontconfig and reading
/// the face once for both. Fontconfig names a variable font's instances as
/// separate styles, but its index for them is not a face index, so the axis is
/// set here instead. A static face has no axis and comes back unchanged twice.
pub fn load_ui_font_weights(family: Option<&str>, weights: (f32, f32)) -> (FontVec, FontVec) {
    let source = ui_font_bytes(family);
    let instance = |weight: f32| {
        let mut font = source
            .as_ref()
            .and_then(|(bytes, index)| FontVec::try_from_vec_and_index(bytes.clone(), *index).ok())
            .unwrap_or_else(fallback_popup_font);
        font.set_variation(b"wght", weight);
        font
    };
    (instance(weights.0), instance(weights.1))
}

/// The desktop's configured UI family, when the desktop names one.
fn desktop_font_family() -> Option<String> {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| parse_gsettings_font(&output))
}

pub fn load_popup_font() -> FontVec {
    desktop_font_family()
        .as_deref()
        .and_then(load_fontconfig_font)
        .or_else(|| load_fontconfig_font("sans-serif"))
        .unwrap_or_else(fallback_popup_font)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font;

    #[test]
    fn parses_gsettings_family_without_trailing_size() {
        assert_eq!(
            parse_gsettings_font("'GeistMono Nerd Font 11'\n"),
            Some("GeistMono Nerd Font".into())
        );
        assert_eq!(
            parse_gsettings_font("'Inter Variable 10.5'"),
            Some("Inter Variable".into())
        );
    }

    #[test]
    fn parses_fontconfig_collection_index() {
        assert_eq!(
            parse_fc_match("/usr/share/fonts/inter/Inter.ttc\t2\n"),
            Some((PathBuf::from("/usr/share/fonts/inter/Inter.ttc"), 2))
        );
    }

    #[test]
    fn embedded_fallback_contains_popup_and_selector_glyphs() {
        let font = fallback_popup_font();
        for ch in "RECORDING PAUSED SAVING... SHELF DISK DISCARD AUDIO ON OFF 01:23 ×".chars() {
            if !ch.is_whitespace() {
                assert_ne!(font.glyph_id(ch), ab_glyph::GlyphId(0), "missing {ch:?}");
            }
        }
    }
}
