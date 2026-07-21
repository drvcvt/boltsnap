use ab_glyph::FontVec;

pub fn fallback_popup_font() -> FontVec {
    FontVec::try_from_vec(include_bytes!("../../assets/fonts/dejavu-badge.ttf").to_vec())
        .expect("embedded DejaVu popup font must be valid")
}
