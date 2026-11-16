//! Installed font families (DirectWrite), with a flag for fonts that contain Arabic letters.

use serde::Serialize;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteLocalizedStrings,
};
use windows::core::{BOOL, w};

#[derive(Serialize)]
pub struct FontInfo {
    pub name: String,
    /// The font has Arabic letters (otherwise Windows substitutes another font for them).
    pub arabic: bool,
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

/// All installed font families, Arabic-capable first, each group sorted by name.
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
            let arabic = family
                .GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL)
                .and_then(|f| f.HasCharacter(0x0639)) // ع
                .map(|b| b.as_bool())
                .unwrap_or(false);
            out.push(FontInfo { name, arabic });
        }
    }
    out.sort_by(|a, b| b.arabic.cmp(&a.arabic).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out.dedup_by(|a, b| a.name == b.name);
    out
}
