//! GTA V phrasebook: exact Arabic for the game's common on-screen instructions.
//!
//! Machine translators handle these short commands badly ("take cover" -> "to cover it"), and GTA
//! uses a fixed set of them, so known lines get a hand-written translation. `{1}`, `{2}` mark the
//! controller button icons; the Arabic puts each icon where it belongs in the sentence.
//! Lines not in the book go to the translator as usual.

use std::sync::RwLock;

use crate::translate::Segments;

/// Phrases from the game's layout update (any game, any language): (game, language, phrases).
static EXTRA: RwLock<(String, String, Vec<(String, String)>)> = RwLock::new((String::new(), String::new(), Vec::new()));

/// The phrases a layout update brings for this game and language (`"phrases": {"ar": {"Press {1} to …": "…"}}`).
pub fn set_extra(game: &str, lang: &str, phrases: Vec<(String, String)>) {
    *EXTRA.write().unwrap() = (game.to_string(), lang.to_string(), phrases);
}

/// The hand-written translation of a line for this game and language, if there is one: the layout
/// update's phrases first, then the app's own book (GTA V, Arabic).
pub fn find(game: &str, lang: &str, line: &str, icons: &[String]) -> Option<Segments> {
    {
        let extra = EXTRA.read().unwrap();
        if extra.0 == game && extra.1 == lang && !extra.2.is_empty() {
            if let Some(segs) = best(extra.2.iter().map(|(e, a)| (e.as_str(), a.as_str())), line, icons) {
                return Some(segs);
            }
        }
    }
    (game == "gta5" && lang == "ar").then(|| lookup(line, icons)).flatten()
}

/// (English, Arabic). Matching ignores case and punctuation; `{n}` = n-th icon in the line.
const PHRASES: &[(&str, &str)] = &[
    // --- cover & shooting
    ("Press {1} to take cover.", "اضغط {1} للاحتماء."),
    ("Press {1} to take cover behind objects.", "اضغط {1} للاحتماء خلف الأشياء."),
    ("Press {1} to leave cover.", "اضغط {1} لترك الغطاء."),
    ("Hold {1} to aim at enemies while in cover, then shoot with {2}", "اضغط مطوّلاً على {1} للتصويب على الأعداء أثناء الاحتماء، ثم أطلق النار بـ {2}"),
    ("Hold {1} to aim, then shoot with {2}", "اضغط مطوّلاً على {1} للتصويب، ثم أطلق النار بـ {2}"),
    ("Press {1} to blind fire.", "اضغط {1} لإطلاق النار دون تصويب."),
    ("Aim using {1}", "صوّب باستخدام {1}"),
    ("Hold {1} to aim.", "اضغط مطوّلاً على {1} للتصويب."),
    ("Press {1} to shoot.", "اضغط {1} لإطلاق النار."),
    ("Press {1} to fire.", "اضغط {1} لإطلاق النار."),
    ("Press {1} to reload a weapon.", "اضغط {1} لإعادة تعبئة السلاح."),
    ("Press {1} to reload.", "اضغط {1} لإعادة التعبئة."),
    ("Press {1} to zoom in while aiming.", "اضغط {1} للتقريب أثناء التصويب."),
    ("Use {1} to aim at the target and take him out with one headshot.", "استخدم {1} للتصويب على الهدف واقضِ عليه بطلقة واحدة في الرأس."),
    ("Use {1} to aim.", "استخدم {1} للتصويب."),
    ("Hold {1} to open the weapon wheel.", "اضغط مطوّلاً على {1} لفتح قائمة الأسلحة."),
    ("Press {1} to switch weapons.", "اضغط {1} لتبديل السلاح."),
    ("Press {1} to throw.", "اضغط {1} للرمي."),
    ("Press {1} to attack.", "اضغط {1} للهجوم."),
    ("Press {1} to melee attack.", "اضغط {1} للهجوم القريب."),
    ("Press {1} to dodge.", "اضغط {1} للمراوغة."),
    ("Press {1} to activate your special ability.", "اضغط {1} لتفعيل قدرتك الخاصة."),
    ("Press {1} and {2} to activate your special ability.", "اضغط {1} و{2} لتفعيل قدرتك الخاصة."),
    ("Press {1} + {2} to activate your special ability.", "اضغط {1} + {2} لتفعيل قدرتك الخاصة."),
    // --- moving on foot
    ("Use {1} to move.", "استخدم {1} للتحرك."),
    ("Use {1} to walk.", "استخدم {1} للمشي."),
    ("Use {1} to aim.", "استخدم {1} للتصويب."),
    ("Use {1} to move the camera.", "استخدم {1} لتحريك الكاميرا."),
    ("Use {1} to steer.", "استخدم {1} للتوجيه."),
    ("Hold {1} to accelerate.", "اضغط مطوّلاً على {1} للتسارع."),
    ("Hold {1} to brake or reverse.", "اضغط مطوّلاً على {1} للفرملة أو الرجوع للخلف."),
    ("To play GTAV in first-person mode cycle the camera using {1}", "للعب GTAV بمنظور الشخص الأول، بدّل زاوية الكاميرا باستخدام {1}"),
    ("Use {1} to look around.", "استخدم {1} للنظر حولك."),
    ("Tap {1} to sprint.", "اضغط {1} بشكل متكرر للركض بسرعة."),
    ("Hold {1} to sprint.", "اضغط مطوّلاً على {1} للركض بسرعة."),
    ("Press {1} to jump.", "اضغط {1} للقفز."),
    ("Press {1} to climb.", "اضغط {1} للتسلق."),
    ("Press {1} to crouch.", "اضغط {1} للانحناء."),
    ("Press {1} to enter stealth mode.", "اضغط {1} للدخول في وضع التسلل."),
    ("Press {1} to pick up.", "اضغط {1} للالتقاط."),
    ("Press {1} to change camera view.", "اضغط {1} لتغيير زاوية الكاميرا."),
    ("Press {1} to look behind.", "اضغط {1} للنظر إلى الخلف."),
    // --- vehicles
    ("Press {1} to enter the vehicle.", "اضغط {1} لركوب المركبة."),
    ("Press {1} to get in.", "اضغط {1} للركوب."),
    ("Press {1} to exit the vehicle.", "اضغط {1} للنزول من المركبة."),
    ("Use {1} to accelerate and {2} to brake or reverse.", "استخدم {1} للتسارع و{2} للفرملة أو الرجوع للخلف."),
    ("Press {1} to use the handbrake.", "اضغط {1} لاستخدام فرامل اليد."),
    ("Press {1} to change the radio station.", "اضغط {1} لتغيير محطة الراديو."),
    ("Press {1} to use the horn.", "اضغط {1} لاستخدام البوق."),
    ("Press {1} to turn on the headlights.", "اضغط {1} لتشغيل المصابيح الأمامية."),
    // --- phone, menus, characters
    ("Press {1} to use your phone.", "اضغط {1} لاستخدام هاتفك."),
    ("Press {1} to take out your phone.", "اضغط {1} لإخراج هاتفك."),
    ("Press {1} to answer the call.", "اضغط {1} للرد على المكالمة."),
    ("Press {1} to hang up.", "اضغط {1} لإنهاء المكالمة."),
    ("Hold {1} to switch character.", "اضغط مطوّلاً على {1} لتبديل الشخصية."),
    ("Press {1} to skip.", "اضغط {1} للتخطي."),
    ("Press {1} to continue.", "اضغط {1} للمتابعة."),
    ("Press {1} to cancel.", "اضغط {1} للإلغاء."),
    ("Press {1} to select.", "اضغط {1} للاختيار."),
    ("Press {1} to confirm.", "اضغط {1} للتأكيد."),
    ("Press {1} to open the map.", "اضغط {1} لفتح الخريطة."),
    ("Press {1} to pause.", "اضغط {1} للإيقاف المؤقت."),
    // --- objectives without icons
    ("Lose the cops.", "تخلّص من الشرطة."),
    ("Lose your wanted level.", "تخلّص من مستوى المطلوبية."),
    ("Get in the car.", "اركب السيارة."),
    ("Get back in the car.", "عُد إلى السيارة."),
    ("Get back in the vehicle.", "عُد إلى المركبة."),
    ("Take out the enemies.", "اقضِ على الأعداء."),
    ("Kill the enemies.", "اقتل الأعداء."),
    ("Take cover.", "احتمِ."),
];

/// Lower-case words; icons become "{1}", "{2}"... (by order in the line); punctuation dropped.
fn normalize_line(line: &str) -> String {
    let mut out = Vec::new();
    let mut icon = 0;
    for tok in line.split_whitespace() {
        let core = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '⟦' && c != '⟧'); // "⟦1⟧." is still an icon
        if core.starts_with('⟦') && core.ends_with('⟧') {
            icon += 1;
            out.push(format!("{{{icon}}}"));
            continue;
        }
        let w: String = tok.chars().filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '+').collect::<String>().to_lowercase();
        if !w.is_empty() {
            out.push(w);
        }
    }
    out.join(" ")
}

fn normalize_template(t: &str) -> String {
    t.split_whitespace()
        .map(|tok| {
            if tok.starts_with('{') {
                tok.chars().take_while(|c| *c != '}').chain(std::iter::once('}')).collect::<String>()
            } else {
                tok.chars().filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '+').collect::<String>().to_lowercase()
            }
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Edit-distance similarity 0..1.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()) as f64
}

/// The book's Arabic for this English line (with `⟦n⟧` icon placeholders), as segments with the
/// icons put back in. Exact match first; then a close match (≥ 90 %, same icons) to tolerate small
/// OCR slips like a dropped letter.
pub fn lookup(line: &str, icons: &[String]) -> Option<Segments> {
    best(PHRASES.iter().copied(), line, icons)
}

fn best<'a>(phrases: impl Iterator<Item = (&'a str, &'a str)>, line: &str, icons: &[String]) -> Option<Segments> {
    let norm = normalize_line(line);
    if norm.is_empty() {
        return None;
    }
    let n_icons = norm.matches('{').count();
    let mut best: Option<(f64, &str)> = None;
    for (en, ar) in phrases {
        let t = normalize_template(en);
        if t.matches('{').count() != n_icons {
            continue;
        }
        let sim = if t == norm { 1.0 } else { similarity(&t, &norm) };
        if sim >= 0.9 && best.is_none_or(|(s, _)| sim > s) {
            best = Some((sim, ar));
        }
    }
    let (_, ar) = best?;
    Some(fill(ar, icons))
}

/// "اضغط {1} للاحتماء." -> segments, with icon n taken from `icons[n-1]`.
fn fill(ar: &str, icons: &[String]) -> Segments {
    let mut segs: Segments = Vec::new();
    let mut rest = ar;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| c + open) else { break };
        if open > 0 {
            segs.push((rest[..open].to_string(), None));
        }
        if let Some(png) = rest[open + 1..close].parse::<usize>().ok().and_then(|n| icons.get(n.wrapping_sub(1))) {
            segs.push((String::new(), Some(format!("icon:{png}"))));
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        segs.push((rest.to_string(), None));
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(segs: &Segments) -> String {
        segs.iter().map(|(t, c)| if c.is_some() { "🎮".to_string() } else { t.clone() }).collect()
    }
    fn icons(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("0.8|icon{i}")).collect()
    }

    #[test]
    fn recorded_instructions_get_exact_arabic() {
        // exactly what the app read from the recorded frames
        let cases = [
            ("Press ⟦1⟧ to take cover", 1, "اضغط 🎮 للاحتماء."),
            ("Use ⟦1⟧ to walk.", 1, "استخدم 🎮 للمشي."),
            ("To play GTAV in first-person mode cycle the camera using ⟦1⟧", 1, "للعب GTAV بمنظور الشخص الأول، بدّل زاوية الكاميرا باستخدام 🎮"),
            ("Hold ⟦1⟧ to aim at enemies while in cover, then shoot with ⟦2⟧", 2,
             "اضغط مطوّلاً على 🎮 للتصويب على الأعداء أثناء الاحتماء، ثم أطلق النار بـ 🎮"),
            ("Press ⟦1⟧ to reload a weapon", 1, "اضغط 🎮 لإعادة تعبئة السلاح."),
            ("Aim using ⟦1⟧", 1, "صوّب باستخدام 🎮"),
            ("Press ⟦1⟧ to shoot", 1, "اضغط 🎮 لإطلاق النار."),
            ("Press ⟦1⟧ to zoom in while aiming.", 1, "اضغط 🎮 للتقريب أثناء التصويب."),
            ("Use ⟦1⟧ to aim at the target and take him out with one headshot.", 1,
             "استخدم 🎮 للتصويب على الهدف واقضِ عليه بطلقة واحدة في الرأس."),
        ];
        for (en, n, want) in cases {
            let got = lookup(en, &icons(n)).unwrap_or_else(|| panic!("no match for {en}"));
            assert_eq!(text(&got), want, "{en}");
        }
    }

    #[test]
    fn small_ocr_slips_still_match() {
        assert!(lookup("Press ⟦1⟧ to reload a wepon", &icons(1)).is_some());
        assert!(lookup("press ⟦1⟧ to take cover..", &icons(1)).is_some());
    }

    #[test]
    fn unknown_lines_and_wrong_icon_counts_dont_match() {
        assert!(lookup("Blame the pricks who called you out here!", &[]).is_none());
        assert!(lookup("Press to take cover", &[]).is_none()); // icon missing: don't invent one
        assert!(lookup("Press ⟦1⟧ to call Lamar.", &icons(1)).is_none());
    }

    #[test]
    fn layout_phrases_for_any_game() {
        set_extra("rdr2", "fr", vec![("Press {1} to mount.".into(), "Appuyez sur {1} pour monter.".into())]);
        assert_eq!(text(&find("rdr2", "fr", "Press ⟦1⟧ to mount", &icons(1)).unwrap()), "Appuyez sur 🎮 pour monter.");
        assert!(find("rdr2", "ar", "Press ⟦1⟧ to mount", &icons(1)).is_none(), "another language");
        assert!(find("gta5", "ar", "Press ⟦1⟧ to take cover", &icons(1)).is_some(), "the app's own book still");
        assert!(find("rdr2", "ar", "Press ⟦1⟧ to take cover", &icons(1)).is_none(), "GTA V's book is GTA V's");
        set_extra("", "", Vec::new());
    }

    #[test]
    fn icons_land_in_order() {
        let segs = lookup("Use ⟦1⟧ to accelerate and ⟦2⟧ to brake or reverse.", &icons(2)).unwrap();
        let order: Vec<&str> = segs.iter().filter_map(|(_, c)| c.as_deref()).collect();
        assert_eq!(order, vec!["icon:0.8|icon1", "icon:0.8|icon2"]);
    }
}
