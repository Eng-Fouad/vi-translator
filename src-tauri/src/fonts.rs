//! Installed font families (DirectWrite), with the translation languages each one can write.

use serde::Serialize;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteLocalizedStrings,
};
use windows::core::{BOOL, w};

#[derive(Serialize)]
pub struct FontInfo {
    pub name: String,
    /// Target languages (codes from `langs`) whose letters the font has; for others, Windows
    /// substitutes another font.
    pub covers: Vec<&'static str>,
}


fn english_name(names: &IDWriteLocalizedStrings) -> Option<String> {
    unsafe {
        let mut index = 0u32;
        let mut exists = BOOL(0);
        names.FindLocaleName(w!("en-us"), &mut index, &mut exists).ok()?;
        if !exists.as_bool() {
            index = 0;
        }
        let len = names.GetStringLength(index).ok()? as usize;
        let mut buf = vec![0u16; len + 1];
        names.GetString(index, &mut buf).ok()?;
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// All installed font families, sorted by name (the app lists the ones for its language first).
pub fn list_fonts() -> Vec<FontInfo> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) else { return out };
        let mut coll: Option<IDWriteFontCollection> = None;
        if factory.GetSystemFontCollection(&mut coll, false).is_err() {
            return out;
        }
        let Some(coll) = coll else { return out };
        for i in 0..coll.GetFontFamilyCount() {
            let Ok(family) = coll.GetFontFamily(i) else { continue };
            let Some(name) = family.GetFamilyNames().ok().and_then(|n| english_name(&n)) else { continue };
            if name.starts_with('@') {
                continue; // vertical-text variants
            }
            let Ok(font) = family.GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL) else { continue };
            let has = |c: u32| font.HasCharacter(c).map(|b| b.as_bool()).unwrap_or(false);
            let covers = crate::langs::NEEDS.iter().filter(|(_, cs)| cs.iter().all(|&c| has(c))).map(|(code, _)| *code).collect();
            out.push(FontInfo { name, covers });
        }
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out.dedup_by(|a, b| a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    /// Windows' own fonts: each one is listed for the scripts it really has.
    #[test]
    fn fonts_know_their_scripts() {
        let fonts = super::list_fonts();
        let covers = |name: &str| fonts.iter().find(|f| f.name == name).map(|f| f.covers.clone()).unwrap_or_default();
        let nirmala = covers("Nirmala UI");
        for code in ["hi", "mr", "bn", "ta", "te"] {
            assert!(nirmala.contains(&code), "Nirmala UI should write {code}: {nirmala:?}");
        }
        assert!(covers("Leelawadee UI").contains(&"th"));
        let segoe = covers("Segoe UI");
        for code in ["ar", "el", "uk", "vi", "tr", "cs", "ro"] {
            assert!(segoe.contains(&code), "Segoe UI should write {code}: {segoe:?}");
        }
        assert!(!segoe.contains(&"th") && !segoe.contains(&"hi"), "Segoe UI has no Thai or Devanagari: {segoe:?}");
    }
}
