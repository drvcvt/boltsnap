use ab_glyph::FontVec;
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

fn load_fontconfig_font(query: &str) -> Option<FontVec> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\t%{index}\n", query])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (path, index) = parse_fc_match(&text)?;
    FontVec::try_from_vec_and_index(std::fs::read(path).ok()?, index).ok()
}

pub fn load_popup_font() -> FontVec {
    let desktop = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| parse_gsettings_font(&output));
    desktop
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
