//! Target languages. The game's own languages (English, French, Italian, German, Russian,
//! Japanese, Polish, Korean, Chinese, Spanish, Portuguese) are left out: players can switch the
//! game to them. These are the most-spoken languages it doesn't have.

use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug)]
pub struct Lang {
    /// our id (ISO 639-1, "fil" for Filipino)
    pub code: &'static str,
    pub name: &'static str,
    pub native: &'static str,
    /// written right to left
    pub rtl: bool,
    pub google: &'static str,
    pub azure: &'static str,
    pub deepl: &'static str,
    pub libre: &'static str,
    /// free offline model (Hugging Face repo with an ONNX export of OPUS-MT), if there is one
    pub local: Option<&'static str>,
    /// fonts for this script, after the user's chosen font (CSS list)
    pub fonts: &'static str,
}

const LATIN: &str = "\"Segoe UI\", Tahoma, sans-serif";
const INDIC: &str = "\"Nirmala UI\", \"Segoe UI\", sans-serif";
const ARABIC: &str = "\"Segoe UI\", Tahoma, sans-serif";

macro_rules! lang {
    ($code:literal, $name:literal, $native:literal, $rtl:literal, $g:literal, $az:literal, $dl:literal, $lb:literal, $local:expr, $fonts:expr) => {
        Lang { code: $code, name: $name, native: $native, rtl: $rtl, google: $g, azure: $az, deepl: $dl, libre: $lb, local: $local, fonts: $fonts }
    };
}

pub const LANGS: [Lang; 20] = [
    lang!("ar", "Arabic", "العربية", true, "ar", "ar", "AR", "ar", Some("Xenova/opus-mt-en-ar"), ARABIC),
    lang!("hi", "Hindi", "हिन्दी", false, "hi", "hi", "HI", "hi", Some("Xenova/opus-mt-en-hi"), INDIC),
    lang!("bn", "Bengali", "বাংলা", false, "bn", "bn", "BN", "bn", None, INDIC),
    lang!("ur", "Urdu", "اردو", true, "ur", "ur", "UR", "ur", None, ARABIC),
    lang!("id", "Indonesian", "Bahasa Indonesia", false, "id", "id", "ID", "id", Some("Xenova/opus-mt-en-id"), LATIN),
    lang!("tr", "Turkish", "Türkçe", false, "tr", "tr", "TR", "tr", None, LATIN),
    lang!("vi", "Vietnamese", "Tiếng Việt", false, "vi", "vi", "VI", "vi", Some("Xenova/opus-mt-en-vi"), LATIN),
    lang!("fa", "Persian", "فارسی", true, "fa", "fa", "FA", "fa", None, ARABIC),
    lang!("th", "Thai", "ไทย", false, "th", "th", "TH", "th", None, "\"Leelawadee UI\", \"Segoe UI\", sans-serif"),
    lang!("ta", "Tamil", "தமிழ்", false, "ta", "ta", "TA", "ta", None, INDIC),
    lang!("te", "Telugu", "తెలుగు", false, "te", "te", "TE", "te", None, INDIC),
    lang!("mr", "Marathi", "मराठी", false, "mr", "mr", "MR", "mr", None, INDIC),
    lang!("uk", "Ukrainian", "Українська", false, "uk", "uk", "UK", "uk", Some("Xenova/opus-mt-en-uk"), LATIN),
    lang!("nl", "Dutch", "Nederlands", false, "nl", "nl", "NL", "nl", Some("Xenova/opus-mt-en-nl"), LATIN),
    lang!("fil", "Filipino", "Filipino", false, "tl", "fil", "TL", "tl", None, LATIN),
    lang!("ms", "Malay", "Bahasa Melayu", false, "ms", "ms", "MS", "ms", None, LATIN),
    lang!("sw", "Swahili", "Kiswahili", false, "sw", "sw", "SW", "sw", None, LATIN),
    lang!("ro", "Romanian", "Română", false, "ro", "ro", "RO", "ro", Some("Xenova/opus-mt-en-ro"), LATIN),
    lang!("el", "Greek", "Ελληνικά", false, "el", "el", "EL", "el", None, LATIN),
    lang!("cs", "Czech", "Čeština", false, "cs", "cs", "CS", "cs", Some("Xenova/opus-mt-en-cs"), LATIN),
];

/// The language for a code (Arabic for an unknown one).
pub fn get(code: &str) -> &'static Lang {
    LANGS.iter().find(|l| l.code == code).unwrap_or(&LANGS[0])
}
