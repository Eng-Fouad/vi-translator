//! Installed font families on Linux and macOS (fontdb: fontconfig folders / the macOS font
//! folders), with the translation languages each one can write.

use serde::Serialize;

#[derive(Serialize)]
pub struct FontInfo {
    pub name: String,
    /// Target languages (codes from `langs`) whose letters the font has; for others, the system
    /// substitutes another font.
    pub covers: Vec<&'static str>,
}

/// All installed font families, sorted by name (the app lists the ones for its language first).
pub fn list_fonts() -> Vec<FontInfo> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut out: Vec<FontInfo> = Vec::new();
    for face in db.faces() {
        let Some((name, _)) = face.families.first() else { continue };
        if name.starts_with('.') || out.iter().any(|f| &f.name == name) {
            continue; // hidden system fonts; one entry per family
        }
        let covers = db
            .with_face_data(face.id, |data, index| {
                let f = ttf_parser::Face::parse(data, index).ok()?;
                let has = |c: u32| char::from_u32(c).and_then(|c| f.glyph_index(c)).is_some();
                Some(crate::langs::NEEDS.iter().filter(|(_, cs)| cs.iter().all(|&c| has(c))).map(|(code, _)| *code).collect())
            })
            .flatten()
            .unwrap_or_default();
        out.push(FontInfo { name: name.clone(), covers });
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}
