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
    Some((bare_name(&tagged), color, rest))
}

/// "Lucia.:" / "Jason…:" -> "Lucia" / "Jason": the name without its colon or the dots the OCR reads
/// next to it.
fn bare_name(tagged: &str) -> String {
    tagged.trim_end_matches(':').trim_end_matches(|c: char| matches!(c, '.' | '…' | ',' | ';')).trim().to_string()
}

/// A "Name:" in front that the colour check missed (a pale name, or one over a bright sky): (name,
/// the rest). The caller takes it only for a name it knows (the game's people, a reviewed name, or a
/// sentence it knows without it), so a white "Tip:" stays part of the text. Never a language tag.
pub fn plain_name(en: &str) -> Option<(String, &str)> {
    if language_tag(en).is_some() {
        return None;
    }
    let (tagged, rest) = crate::ocr::split_name(en)?;
    if rest.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return None;
    }
    Some((bare_name(&tagged), rest))
}

/// One of the game's people (as KNOWN lists them).
pub fn is_known(name: &str) -> bool {
    let low = name.to_lowercase();
    KNOWN.iter().any(|(en, _)| *en == low)
}

/// The colour a name the colour check missed is drawn in: the subtitles' own near-white.
pub const PLAIN_NAME_COLOR: &str = "#f2f2f2";

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

/// Roles the game puts where a name goes ("Cop:", "Prison Guard:"): translated, never spelt out in
/// Arabic letters ("كوب"). Arabic, with the article, as the subtitle names a person.
const ROLES: &[(&str, &str)] = &[
    ("cop", "الشرطي"),
    ("cops", "الشرطة"),
    ("police", "الشرطة"),
    ("police officer", "ضابط الشرطة"),
    ("officer", "الضابط"),
    ("deputy", "نائب الشريف"),
    ("sheriff", "الشريف"),
    ("trooper", "شرطي الولاية"),
    ("detective", "المحقق"),
    ("sergeant", "الرقيب"),
    ("captain", "النقيب"),
    ("agent", "العميل"),
    ("soldier", "الجندي"),
    ("guard", "الحارس"),
    ("prison guard", "حارس السجن"),
    ("security guard", "حارس الأمن"),
    ("security", "الأمن"),
    ("bouncer", "حارس الملهى"),
    ("dispatch", "مركز البلاغات"),
    ("dispatcher", "مركز البلاغات"),
    ("operator", "عامل الهاتف"),
    ("pilot", "الطيار"),
    ("driver", "السائق"),
    ("cashier", "أمين الصندوق"),
    ("clerk", "الموظف"),
    ("teller", "الصرّاف"),
    ("bank teller", "صرّاف البنك"),
    ("bartender", "النادل"),
    ("waiter", "النادل"),
    ("waitress", "النادلة"),
    ("customer", "الزبون"),
    ("shopkeeper", "صاحب المتجر"),
    ("owner", "المالك"),
    ("manager", "المدير"),
    ("boss", "الزعيم"),
    ("worker", "العامل"),
    ("mechanic", "الميكانيكي"),
    ("attendant", "العامل"),
    ("salesman", "البائع"),
    ("dealer", "التاجر"),
    ("doctor", "الطبيب"),
    ("nurse", "الممرضة"),
    ("paramedic", "المسعف"),
    ("reporter", "المراسل"),
    ("news anchor", "مذيع الأخبار"),
    ("anchor", "المذيع"),
    ("announcer", "المذيع"),
    ("host", "المقدّم"),
    ("radio host", "مذيع الراديو"),
    ("dj", "منسّق الأغاني"),
    ("caller", "المتصل"),
    ("voice", "الصوت"),
    ("man", "الرجل"),
    ("woman", "المرأة"),
    ("guy", "الرجل"),
    ("girl", "الفتاة"),
    ("boy", "الفتى"),
    ("kid", "الطفل"),
    ("old man", "العجوز"),
    ("old woman", "العجوز"),
    ("homeless man", "المشرّد"),
    ("pedestrian", "أحد المارة"),
    ("passerby", "أحد المارة"),
    ("local", "أحد السكان"),
    ("neighbor", "الجار"),
    ("neighbour", "الجار"),
    ("tourist", "السائح"),
    ("stranger", "الغريب"),
    ("hostage", "الرهينة"),
    ("inmate", "السجين"),
    ("prisoner", "السجين"),
    ("thug", "البلطجي"),
    ("goon", "البلطجي"),
    ("gangster", "رجل العصابة"),
    ("gang member", "عضو العصابة"),
    ("biker", "سائق الدراجة"),
    ("crowd", "الحشد"),
    ("everyone", "الجميع"),
    ("all", "الجميع"),
];

/// A role in place of a name ("Cop", "Prison Guard"), or a title before one ("Officer Smith").
pub fn is_role(name: &str) -> bool {
    let low = name.to_lowercase();
    let first = low.split_whitespace().next().unwrap_or("");
    ROLES.iter().any(|(en, _)| *en == low || *en == first)
}

/// A role (or a title and a name) in Arabic: "Cop" -> "الشرطي", "Officer Smith" -> "الضابط سميث".
fn role_ar(name: &str) -> Option<String> {
    let low = name.to_lowercase();
    if let Some((_, ar)) = ROLES.iter().find(|(en, _)| *en == low) {
        return Some(ar.to_string());
    }
    let (first, rest) = low.split_once(' ')?;
    let (_, title) = ROLES.iter().find(|(en, _)| *en == first)?;
    let person = KNOWN.iter().find(|(en, _)| *en == rest).map(|(_, ar)| ar.to_string()).unwrap_or_else(|| rest.split('-').map(transliterate).collect::<Vec<_>>().join("-"));
    Some(format!("{title} {person}"))
}

/// The name as the target language writes it. `reviewed`: a reviewed translation of the name alone
/// (the downloaded translations can carry the names the game's people go by).
pub fn name_in(lang: &str, name: &str, reviewed: Option<String>) -> String {
    if !arabic_script(lang) {
        return name.to_string();
    }
    let low = name.to_lowercase();
    if let Some((_, ar)) = KNOWN.iter().find(|(en, _)| *en == low) {
        return ar.to_string();
    }
    if lang == "ar" {
        if let Some(r) = role_ar(name) {
            return r;
        }
    }
    // a reviewed translation of the name alone: the downloaded line "Lucia." comes with its full stop
    // ("لوسيا."), and only a name-sized one is a name (not a sentence that happens to match)
    let words = name.split_whitespace().count();
    let reviewed = reviewed.map(|r| r.trim().trim_end_matches(|c: char| matches!(c, ':' | '.' | '…' | '!' | '?' | '،' | '؟' | ',' | '؛')).trim().to_string());
    if let Some(r) = reviewed.filter(|r| !r.is_empty() && r.split_whitespace().count() <= words + 1 && !r.contains(|c: char| matches!(c, '.' | '!' | '?' | '؟' | '،'))) {
        return r;
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
        assert_eq!(name_in("ar", "Officer Jason", None), "الضابط جيسون");
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
    fn dots_by_a_name_are_dropped() {
        let ph = vec![("Lucia.:".to_string(), "#ffd69f".to_string())];
        assert_eq!(split("Lucia.: Let's go.", &ph).unwrap().0, "Lucia");
        assert_eq!(name_in("ar", "Lucia", Some("لوسيا.".into())), "لوسيا");
        assert_eq!(name_in("ar", "Dobbs", Some("دوبس..".into())), "دوبس");
        assert_eq!(name_in("ar", "Dobbs", Some("دوبس، هيا بنا.".into())), "دوبس");
        assert_eq!(plain_name("Jason..: Hold up."), Some(("Jason".to_string(), "Hold up.")));
        assert!(plain_name("In Spanish: Hola, amigo.").is_none());
        assert!(is_known("Jason") && !is_known("Tip"));
    }

    #[test]
    fn a_role_is_translated() {
        assert_eq!(name_in("ar", "Cop", None), "الشرطي");
        assert_eq!(name_in("ar", "Prison Guard", None), "حارس السجن");
        assert_eq!(name_in("ar", "Officer Smith", None), "الضابط سميث");
        assert!(is_role("Cop") && is_role("Officer Smith") && !is_role("Dobbs"));
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
