//! Speaker names in front of subtitles ("Dobbs: Not to be annoying, can you fix my leg?"), as GTA VI
//! shows them when its "speaker name" option is on: the name in a colour of its own, then a colon.
//!
//! Only the sentence is translated (and remembered), so the same line finds the same translation
//! with or without the name. The name is written in the target language's letters where it uses the
//! Arabic script (a reviewed name from the downloaded translations first, then the game's known
//! names, then letter by letter), and drawn back in front in the game's colour.

use crate::translate::Segments;

/// The segment colour marking a speaker name: "name:#rrggbb" (always drawn in that colour, also with
/// highlight colours turned off, like the grey of the other speaker's line).
pub const NAME_PREFIX: &str = "name:";

/// A line starting with a speaker name: (name, its colour, the rest). The name must be coloured in the
/// game (a highlighted phrase starting with it): a white "Tip:" or "Note:" is part of the text.
pub fn split<'a>(en: &'a str, phrases: &[(String, String)]) -> Option<(String, String, &'a str)> {
    let (tagged, rest) = crate::ocr::split_name(en)?;
    // the phrase is the name, or a highlighted run that starts with it
    let first = tagged.split_whitespace().next()?;
    let color = phrases
        .iter()
        .find(|(p, _)| p.starts_with(tagged.as_str()) || (p.split_whitespace().next() == Some(first) && tagged.starts_with(p.as_str())))?
        .1
        .clone();
    if rest.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return None;
    }
    Some((tagged.trim_end_matches(':').to_string(), color, rest))
}

/// The line without a "Name:" in front (colour not known yet: for the memory check only).
pub fn without_name(en: &str) -> Option<&str> {
    let rest = crate::ocr::split_name(en).map(|(_, rest)| rest).unwrap_or(en);
    let rest = language_tag(rest).map(|(_, s)| s).unwrap_or(rest);
    (rest.len() < en.len()).then_some(rest)
}

/// The highlighted phrases of the sentence without the name (the name is not a highlighted word in it).
pub fn phrases_without(phrases: &[(String, String)], name: &str) -> Vec<(String, String)> {
    let tag = format!("{name}:");
    phrases
        .iter()
        .filter_map(|(p, c)| {
            let rest = p.strip_prefix(&tag).map(str::trim).unwrap_or(p);
            (!rest.is_empty()).then(|| (rest.to_string(), c.clone()))
        })
        .collect()
}

/// Languages written in the Arabic script: the name is written in it. Others keep the game's name.
fn arabic_script(lang: &str) -> bool {
    matches!(lang, "ar" | "fa" | "ur")
}

/// The game's people, as their names are usually written in Arabic.
const KNOWN: &[(&str, &str)] = &[
    ("lucia", "لوسيا"),
    ("jason", "جيسون"),
    ("cal", "كال"),
    ("boobie", "بوبي"),
    ("dre'quan", "دريكوان"),
    ("drequan", "دريكوان"),
    ("brian", "براين"),
    ("raul", "راؤول"),
    ("bae-luxe", "بي-لوكس"),
    ("roxy", "روكسي"),
    ("real dimez", "ريال دايمز"),
    ("mike", "مايك"),
    ("michael", "مايكل"),
    ("trevor", "تريفور"),
    ("franklin", "فرانكلين"),
    ("lamar", "لامار"),
];

/// The name as the target language writes it. `reviewed`: a reviewed translation of the name alone
/// (the downloaded translations can carry the names the game's people go by).
pub fn name_in(lang: &str, name: &str, reviewed: Option<String>) -> String {
    if !arabic_script(lang) {
        return name.to_string();
    }
    if let Some(r) = reviewed.filter(|r| !r.trim().is_empty()) {
        return r.trim().trim_end_matches(':').trim().to_string();
    }
    let low = name.to_lowercase();
    if let Some((_, ar)) = KNOWN.iter().find(|(en, _)| *en == low) {
        return ar.to_string();
    }
    // each word on its own ("Officer Smith"); a word that is one of the known names is written as they are
    low.split(' ')
        .map(|word| {
            KNOWN
                .iter()
                .find(|(en, _)| *en == word)
                .map(|(_, ar)| ar.to_string())
                .unwrap_or_else(|| word.split('-').map(transliterate).collect::<Vec<_>>().join("-"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// English letters -> Arabic letters, by sound ("dobbs" -> "دوبس", "lucia" -> "لوسيا").
pub fn transliterate(word: &str) -> String {
    let w: Vec<char> = word.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let n = w.len();
    let soft = |i: usize| i < n && matches!(w[i], 'e' | 'i' | 'y');
    // a "magic e" ("Kyle", "Mike", "Kate", "Jones"): the vowel before the last consonant is long
    let magic = n >= 4 && w[n - 1] == 'e' && !is_vowel(w[n - 2]) && is_vowel(w[n - 3]) && (n < 4 || !is_vowel(w[n - 4]));
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        let c = w[i];
        let next = if i + 1 < n { w[i + 1] } else { ' ' };
        // a doubled consonant is one sound
        if i > 0 && c == w[i - 1] && !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') {
            i += 1;
            continue;
        }
        let two: String = [c, next].iter().collect();
        let digraph = match two.as_str() {
            "sh" => Some("ش"),
            "ch" => Some("تش"),
            "th" => Some("ث"),
            "ph" => Some("ف"),
            "kh" => Some("خ"),
            "gh" => Some("غ"),
            "ck" => Some("ك"),
            "qu" => Some("كو"),
            "oo" | "ou" => Some("و"),
            "ee" | "ea" | "ie" => Some(if i == 0 { "إي" } else { "ي" }),
            "ai" | "ay" => Some(if i == 0 { "اي" } else { "اي" }),
            _ => None,
        };
        if let Some(d) = digraph {
            out.push_str(d);
            i += 2;
            continue;
        }
        let last = i + 1 == n;
        if magic && i == n - 3 {
            out.push_str(match c {
                'i' | 'y' => "اي",
                'a' => "اي",
                'o' => "و",
                _ => "و",
            });
            i += 1;
            continue;
        }
        let s = match c {
            'a' => if i == 0 { "ا" } else { "ا" },
            'e' => {
                if i == 0 {
                    "إ"
                } else if last {
                    "" // a silent final e ("Mike")
                } else {
                    "ي"
                }
            }
            'i' => if i == 0 { "إي" } else { "ي" },
            'o' => if i == 0 { "أو" } else { "و" },
            'u' => if i == 0 { "أو" } else { "و" },
            'y' => "ي",
            'b' => "ب",
            'c' => if soft(i + 1) { "س" } else { "ك" },
            'd' => "د",
            'f' => "ف",
            'g' => if soft(i + 1) { "ج" } else { "غ" },
            'h' => "ه",
            'j' => "ج",
            'k' => "ك",
            'l' => "ل",
            'm' => "م",
            'n' => "ن",
            'p' => "ب",
            'q' => "ك",
            'r' => "ر",
            's' => "س",
            't' => "ت",
            'v' => "ف",
            'w' => "و",
            'x' => "كس",
            'z' => "ز",
            _ => "",
        };
        out.push_str(s);
        i += 1;
    }
    // a medial short "e" written as ي between two consonants reads oddly long; keep it (it helps the
    // reader say the name) but never two ي in a row
    out.replace("يي", "ي")
}

/// The name in front of the translated sentence: "دوبس:" in the game's colour, then a space.
pub fn with_name(name_shown: &str, color: &str, mut segs: Segments) -> Segments {
    let mut out: Segments = vec![(format!("{name_shown}:"), Some(format!("{NAME_PREFIX}{color}"))), (" ".into(), None)];
    out.append(&mut segs);
    out
}

// ------------------------------------------------------------------------------- language tags

/// Languages the game's people speak in, as GTA VI tags the line ("In Spanish: …"), and how Arabic
/// says "in Spanish".
const LANGUAGES: &[(&str, &str)] = &[
    ("Spanish", "بالإسبانية"),
    ("Italian", "بالإيطالية"),
    ("French", "بالفرنسية"),
    ("Portuguese", "بالبرتغالية"),
    ("Russian", "بالروسية"),
    ("Chinese", "بالصينية"),
    ("Mandarin", "بالماندرين"),
    ("Cantonese", "بالكانتونية"),
    ("Japanese", "باليابانية"),
    ("Korean", "بالكورية"),
    ("German", "بالألمانية"),
    ("Creole", "بالكريولية"),
    ("Haitian Creole", "بالكريولية الهايتية"),
    ("Arabic", "بالعربية"),
    ("Hindi", "بالهندية"),
    ("Vietnamese", "بالفيتنامية"),
    ("Tagalog", "بالتاغالوغية"),
    ("Polish", "بالبولندية"),
    ("Greek", "باليونانية"),
    ("Hebrew", "بالعبرية"),
    ("Dutch", "بالهولندية"),
    ("Turkish", "بالتركية"),
    ("Persian", "بالفارسية"),
    ("Farsi", "بالفارسية"),
    ("Ukrainian", "بالأوكرانية"),
    ("Armenian", "بالأرمنية"),
    ("Patois", "بالباتوا"),
    ("Jamaican Patois", "بالباتوا الجامايكية"),
    ("Swahili", "بالسواحيلية"),
];

/// "In Spanish: Bro, I'm telling you…" -> ("Spanish", "Bro, I'm telling you…"): the language the line
/// is spoken in, kept apart so the sentence is translated (and remembered) on its own. A misread
/// language ("Spahiyh") is matched to the nearest known one.
pub fn language_tag(en: &str) -> Option<(String, &str)> {
    let rest = en.strip_prefix("In ")?;
    let colon = rest.find(':')?;
    let lang = &rest[..colon];
    let words: Vec<&str> = lang.split_whitespace().collect();
    let capitalised = |w: &&str| w.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && w.chars().all(char::is_alphabetic);
    if words.is_empty() || words.len() > 2 || !words.iter().all(capitalised) {
        return None;
    }
    let sentence = rest[colon + 1..].trim_start();
    if sentence.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return None;
    }
    let names = || LANGUAGES.iter().map(|(en, _)| *en);
    let known = names()
        .find(|k| k.eq_ignore_ascii_case(lang))
        .or_else(|| names().filter(|k| closeness(k, lang) >= 0.7).max_by(|a, b| closeness(a, lang).total_cmp(&closeness(b, lang))));
    Some((known.unwrap_or(lang).to_string(), sentence))
}

/// "In Spanish:" in the target language when the app knows it (Arabic), else None: translate it.
pub fn language_in(lang: &str, language: &str) -> Option<String> {
    if lang != "ar" {
        return None;
    }
    LANGUAGES.iter().find(|(en, _)| *en == language).map(|(_, ar)| format!("{ar}:"))
}

/// The tag in front of the translated sentence (plain text, like the game's).
pub fn with_language(tag_shown: &str, mut segs: Segments) -> Segments {
    let mut out: Segments = vec![(format!("{} ", tag_shown.trim()), None)];
    out.append(&mut segs);
    out
}

/// 0..1 likeness of two words (1 - edit distance / longer length), case-insensitive.
fn closeness(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()).max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gold() -> Vec<(String, String)> {
        vec![("Dobbs:".into(), "#ffd69f".into())]
    }

    #[test]
    fn a_coloured_name_is_split_off() {
        let (name, color, rest) = split("Dobbs: Not to be annoying, can you fix my leg?", &gold()).unwrap();
        assert_eq!((name.as_str(), color.as_str(), rest), ("Dobbs", "#ffd69f", "Not to be annoying, can you fix my leg?"));
    }

    #[test]
    fn a_two_word_name_is_split_off() {
        let ph = vec![("Real Dimez:".to_string(), "#c8a0ff".to_string())];
        let (name, color, rest) = split("Real Dimez: Yo, what's up?", &ph).unwrap();
        assert_eq!((name.as_str(), color.as_str(), rest), ("Real Dimez", "#c8a0ff", "Yo, what's up?"));
        assert_eq!(name_in("ar", "Real Dimez", None), "ريال دايمز");
        assert_eq!(name_in("ar", "Officer Jason", None), "أوفيسير جيسون");
        assert_eq!(without_name("Officer Smith: Hands up!"), Some("Hands up!"));
        // a colon later in a sentence is not a name
        assert!(without_name("Listen up: we go now.").is_none());
        assert!(without_name("Hey Jason, come: now").is_none());
    }

    #[test]
    fn a_language_tag_is_kept_apart() {
        assert_eq!(language_tag("In Italian: Go, go, go."), Some(("Italian".to_string(), "Go, go, go.")));
        assert_eq!(language_tag("In Spahiyh: One coffee, please."), Some(("Spanish".to_string(), "One coffee, please.")));
        assert_eq!(language_tag("In Haitian Creole: Let's go."), Some(("Haitian Creole".to_string(), "Let's go.")));
        assert_eq!(language_in("ar", "Italian").as_deref(), Some("بالإيطالية:"));
        assert!(language_tag("In time: we go").is_none());
        assert!(language_tag("In the end, it's fine.").is_none());
        assert_eq!(without_name("In Spanish: Hola."), Some("Hola."));
        let segs = with_language("بالإيطالية:", vec![("هيا، هيا، هيا.".into(), None)]);
        assert_eq!(segs[0].0, "بالإيطالية: ");
    }

    #[test]
    fn a_white_label_is_not_a_name() {
        assert!(split("Tip: Hold L2 to aim.", &[]).is_none());
        assert!(split("Not to be annoying: fine.", &gold()).is_none());
    }

    #[test]
    fn names_in_arabic_letters() {
        assert_eq!(name_in("ar", "Dobbs", None), "دوبس");
        assert_eq!(name_in("ar", "Lucia", None), "لوسيا");
        assert_eq!(name_in("ar", "Jason", None), "جيسون");
        assert_eq!(name_in("ar", "Dobbs", Some("دابس".into())), "دابس");
        assert_eq!(name_in("hi", "Dobbs", None), "Dobbs");
        assert_eq!(transliterate("dennis"), "دينيس");
        assert_eq!(transliterate("bobby"), "بوبي");
        assert_eq!(transliterate("kyle"), "كايل");
        assert_eq!(transliterate("kenton"), "كينتون");
        assert_eq!(transliterate("shirley"), "شيرلي");
        assert_eq!(transliterate("peggy"), "بيغي");
    }

    #[test]
    fn the_name_goes_in_front_in_its_colour() {
        let segs = with_name("دوبس", "#ffd69f", vec![("هل يمكنك إصلاح ساقي؟".into(), None)]);
        assert_eq!(segs[0], ("دوبس:".to_string(), Some("name:#ffd69f".to_string())));
        assert_eq!(phrases_without(&gold(), "Dobbs"), Vec::<(String, String)>::new());
    }
}
