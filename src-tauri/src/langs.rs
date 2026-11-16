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

/// Letters each language needs (a font with all of them can write it).
pub const NEEDS: [(&str, &[u32]); 20] = [
    ("ar", &[0x0639, 0x0644]),          // ع ل
    ("ur", &[0x0639, 0x06D2, 0x0679]),  // ع ے ٹ
    ("fa", &[0x0639, 0x067E, 0x06AF]),  // ع پ گ
    ("hi", &[0x0915, 0x093F]),          // क ि
    ("mr", &[0x0915, 0x0933]),          // क ळ
    ("bn", &[0x0995, 0x09BF]),          // ক ি
    ("ta", &[0x0B95, 0x0BBF]),          // க ி
    ("te", &[0x0C15, 0x0C3F]),          // క ి
    ("th", &[0x0E01, 0x0E33]),          // ก ำ
    ("el", &[0x03B1, 0x03A9]),          // α Ω
    ("uk", &[0x0436, 0x0456, 0x0457]),  // ж і ї
    ("vi", &[0x01A1, 0x1EA1, 0x1EBF]),  // ơ ạ ế
    ("tr", &[0x011F, 0x0131, 0x015F]),  // ğ ı ş
    ("cs", &[0x0159, 0x016F, 0x010D]),  // ř ů č
    ("ro", &[0x0219, 0x021B, 0x0103]),  // ș ț ă
    ("id", &[0x0061, 0x0067]),
    ("ms", &[0x0061, 0x0067]),
    ("nl", &[0x0061, 0x00EB]),          // ë
    ("fil", &[0x0061, 0x00F1]),         // ñ
    ("sw", &[0x0061, 0x0067]),
];

