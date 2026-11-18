//! English text recognition, plus detection of words drawn in a colour (game highlights) instead
//! of white. The OCR engine itself is the platform's: Windows OCR (ocr_windows.rs), Apple Vision on
//! macOS (ocr_vision.rs), Tesseract on Linux (ocr_tesseract.rs).

use anyhow::Result;

#[cfg_attr(windows, path = "ocr_windows.rs")]
#[cfg_attr(target_os = "macos", path = "ocr_vision.rs")]
#[cfg_attr(all(not(windows), not(target_os = "macos")), path = "ocr_tesseract.rs")]
mod engine;

/// A word as the engine reports it, in the (prepared) image's pixels.
pub struct RawWord {
    pub line: u32,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// An RGBA image crop.
pub struct Crop {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Crop {
    /// Cut a rectangle given as fractions (0..1) out of a full RGBA frame.
    pub fn from_frame(rgba: &[u8], fw: u32, fh: u32, x: f64, y: f64, w: f64, h: f64) -> Option<Crop> {
        let x0 = ((x.clamp(0.0, 1.0)) * fw as f64) as u32;
        let y0 = ((y.clamp(0.0, 1.0)) * fh as f64) as u32;
        let x1 = (((x + w).clamp(0.0, 1.0)) * fw as f64) as u32;
        let y1 = (((y + h).clamp(0.0, 1.0)) * fh as f64) as u32;
        let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if cw < 8 || ch < 8 {
            return None;
        }
        let mut out = Vec::with_capacity((cw * ch * 4) as usize);
        for row in y0..y1 {
            let start = ((row * fw + x0) * 4) as usize;
            out.extend_from_slice(&rgba[start..start + (cw * 4) as usize]);
        }
        Some(Crop { w: cw, h: ch, rgba: out })
    }

    fn px(&self, x: u32, y: u32) -> (i32, i32, i32) {
        let i = ((y * self.w + x) * 4) as usize;
        (self.rgba[i] as i32, self.rgba[i + 1] as i32, self.rgba[i + 2] as i32)
    }
}

pub struct Word {
    pub text: String,
    /// Box in crop pixels: x, y, w, h.
    pub rect: (u32, u32, u32, u32),
    /// Which text line the OCR put this word on (0 = first line).
    pub line: u32,
}

pub struct Ocr {
    engine: engine::Engine,
}

impl Ocr {
    /// (on Windows, on a thread that has WinRT initialised: see `init_thread`)
    pub fn new() -> Result<Self> {
        Ok(Self { engine: engine::Engine::new()? })
    }

    pub fn init_thread() {
        engine::Engine::init_thread();
    }

    /// Prepare for OCR: take each pixel's brightest channel (so coloured words are as bright as white
    /// ones), invert to dark-on-light, and upscale tiny text. Returns BGRA bytes, size, and the factor
    /// that maps OCR-image pixels back to crop pixels.
    fn preprocess(crop: &Crop, mode: Prep) -> (Vec<u8>, u32, u32, f64) {
        let scale = if crop.h < 60 { 2 } else { 1 };
        let (w, h) = (crop.w * scale, crop.h * scale);
        let max_dim = engine::Engine::max_dim();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = crop.px(x / scale, y / scale);
                let v = 255 - prep_value(mode, r, g, b);
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        if w > max_dim || h > max_dim {
            // Very wide crops: halve until it fits (rare; OCR limit is ~2600 px).
            let mut img = image::RgbaImage::from_raw(w, h, out).unwrap();
            while img.width() > max_dim || img.height() > max_dim {
                img = image::imageops::resize(&img, img.width() / 2, img.height() / 2, image::imageops::FilterType::Triangle);
            }
            let (nw, nh) = img.dimensions();
            return (img.into_raw(), nw, nh, crop.w as f64 / nw as f64);
        }
        (out, w, h, 1.0 / scale as f64)
    }

    /// Read the crop twice — once as is (best for outlined subtitles), once keeping only near-white
    /// and strongly coloured pixels (best when a lamp or sky shows through GTA's see-through
    /// instructions box) — and keep whichever reading has more real-looking words. A reading can
    /// look clean yet have silently lost words, so both always run.
    pub fn recognize(&self, crop: &Crop) -> Result<Vec<Word>> {
        let first = drop_junk(self.recognize_with(crop, Prep::Brightest)?);
        let second = drop_junk(self.recognize_with(crop, Prep::WhiteOrColour)?);
        let score = |w: &[Word]| {
            let (g, b) = quality(w);
            g - 2 * b
        };
        if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
            let txt = |w: &[Word]| w.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join(" ");
            eprintln!("[ocr] pass1 {:?} (score {}) | pass2 {:?} (score {})", txt(&first), score(&first), txt(&second), score(&second));
        }
        // The second reading must clearly win: a 1-character edge is usually a misread ("time" for "the").
        // (a pass that read nothing never replaces one that read something: "nothing" scores 0, which
        // can beat a junky but useful read such as "gussian FaiLedj" for the Pricedown title)
        Ok(if !second.is_empty() && score(&second) >= score(&first) + 3 { second } else { first })
    }

    pub fn recognize_with(&self, crop: &Crop, mode: Prep) -> Result<Vec<Word>> {
        let (bgra, w, h, back) = Self::preprocess(crop, mode);
        Ok(self
            .engine
            .read(&bgra, w, h)?
            .into_iter()
            .map(|r| Word {
                line: r.line,
                text: r.text,
                rect: ((r.x * back) as u32, (r.y * back) as u32, (r.w * back).ceil() as u32 + 1, (r.h * back).ceil() as u32 + 1),
            })
            .collect())
    }
}

/// Join words and drop OCR noise (almost no letters).
pub fn clean(words: &[Word]) -> String {
    let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let total = text.chars().count().max(1);
    if letters >= 4 && letters as f64 / total as f64 > 0.5 { fix_contractions(&text) } else { String::new() }
}

/// GTA VI shows two people's lines as two subtitle lines: the one speaking now in white, the other
/// in grey ("Do not move." / "Idiot."). Lines whose letters differ clearly in brightness are split
/// into one part per speaker: (text, brightness relative to the brightest part, 0..1]. A line that
/// only wrapped (the same speaker, the same brightness) stays one part. Fewer than two parts: empty.
pub fn speaker_lines(c: &Crop, words: &[Word]) -> Vec<(String, f64)> {
    if words.len() < 2 {
        return Vec::new();
    }
    let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
    hs.sort_unstable();
    let mh = hs[hs.len() / 2].max(4) as f64;
    // the OCR's lines, top to bottom (pieces of one visual line at the same height go together)
    let mut by_line: std::collections::BTreeMap<u32, Vec<usize>> = std::collections::BTreeMap::new();
    for (i, w) in words.iter().enumerate() {
        by_line.entry(w.line).or_default().push(i);
    }
    let cy = |i: usize| words[i].rect.1 as f64 + words[i].rect.3 as f64 / 2.0;
    let mut lines: Vec<(f64, Vec<usize>)> = by_line
        .into_values()
        .map(|idx| {
            let mut ys: Vec<f64> = idx.iter().map(|&i| cy(i)).collect();
            ys.sort_by(|a, b| a.total_cmp(b));
            (ys[ys.len() / 2], idx)
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut joined: Vec<(f64, Vec<usize>)> = Vec::new();
    for (y, idx) in lines {
        match joined.last_mut() {
            Some(last) if (y - last.0).abs() < 0.5 * mh => last.1.extend(idx),
            _ => joined.push((y, idx)),
        }
    }
    if joined.len() < 2 {
        return Vec::new();
    }
    // A word's brightness: its bright letter pixels (the 90th percentile; the rest of its box is
    // the scene behind it). A line's: the median of its words'.
    let word_lum = |r: (u32, u32, u32, u32)| -> f64 {
        let (x1, y1) = ((r.0 + r.2).min(c.w), (r.1 + r.3).min(c.h));
        let mut v: Vec<u8> = Vec::new();
        for y in r.1..y1 {
            for x in r.0..x1 {
                let i = ((y * c.w + x) * 4) as usize;
                v.push(c.rgba[i].max(c.rgba[i + 1]).max(c.rgba[i + 2]));
            }
        }
        if v.is_empty() {
            return 0.0;
        }
        v.sort_unstable();
        v[(v.len() * 9 / 10).min(v.len() - 1)] as f64
    };
    let line_lum = |idx: &[usize]| {
        let mut v: Vec<f64> = idx.iter().map(|&i| word_lum(words[i].rect)).collect();
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    // consecutive lines of about the same brightness are one speaker
    let mut parts: Vec<(Vec<usize>, f64)> = Vec::new();
    for (_, mut idx) in joined {
        let l = line_lum(&idx);
        match parts.last_mut() {
            Some((pidx, pl)) if l.min(*pl) >= 0.82 * l.max(*pl) => {
                pidx.append(&mut idx);
                *pl = pl.max(l);
            }
            _ => parts.push((idx, l)),
        }
    }
    if parts.len() < 2 {
        return Vec::new();
    }
    let top = parts.iter().map(|p| p.1).fold(1.0f64, f64::max);
    let out: Vec<(String, f64)> = parts
        .into_iter()
        .map(|(idx, l)| {
            // (each part's words are already top to bottom, left to right within a line)
            let text = idx.iter().map(|&i| words[i].text.as_str()).collect::<Vec<_>>().join(" ");
            (fix_contractions(&text.split_whitespace().collect::<Vec<_>>().join(" ")), l / top)
        })
        .filter(|(t, _)| t.chars().any(|ch| ch.is_alphabetic()))
        .collect();
    if out.len() < 2 { Vec::new() } else { out }
}

/// Could this be real English, or is it the OCR spelling out something that isn't English letters
/// at all — text in another script (the console's Arabic menu "وسائط" read as "bjlwg"), icons, a map?
/// False only when the line has words made of letters and none of them could be a word: no vowel
/// ("bjlwg"), six consonants in a row, or a capital in the middle of lowercase ("ulnJiJl").
/// Acronyms ("LSPD", "FIB") and the usual vowel-less words ("Mr", "hmm", "shh") are fine.
pub fn looks_like_words(text: &str) -> bool {
    let fixes = FIXES_EXTRA.read().unwrap();
    const NO_VOWEL_OK: [&str; 16] = ["mr", "mrs", "ms", "dr", "st", "jr", "hmm", "hm", "mm", "shh", "sh", "psst", "pst", "brr", "grr", "pfft"];
    let plausible = |w: &str| {
        let lower = w.to_lowercase();
        // (the game's layout update can name its own words, and junk it knows the OCR makes)
        if fixes.junk.contains(&lower) {
            return false;
        }
        if fixes.ok.contains(&lower) {
            return true;
        }
        if NO_VOWEL_OK.contains(&lower.as_str()) || (w.chars().count() <= 5 && w.chars().all(|c| c.is_uppercase())) {
            return true; // "Hmm", "LSPD"
        }
        // accented letters ("ö", "é") count as vowels: "Röckstar" is a word
        let vowel = |c: char| "aeiouy".contains(c) || !c.is_ascii();
        let has_vowel = lower.chars().any(vowel);
        let mut run = 0;
        let long_run = lower.chars().any(|c| {
            run = if vowel(c) { 0 } else { run + 1 };
            run >= 6
        });
        // "ulnJiJl": lowercase then a capital inside the word ("McKay" and "iPhone" are rare in games)
        let chars: Vec<char> = w.chars().collect();
        let odd_caps = chars.windows(2).skip(1).any(|p| p[0].is_lowercase() && p[1].is_uppercase());
        has_vowel && !long_run && !odd_caps
    };
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || ".,!?;:\"()[]…-–—".contains(c))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_alphabetic() || c == '\''))
        .collect();
    words.is_empty() || words.iter().any(|w| plausible(w))
}

#[cfg(test)]
mod speaker_tests {
    use super::*;

    /// A dark picture with "letters" (filled boxes) of the given brightness where the words are.
    fn picture(words: &[(&str, (u32, u32, u32, u32), u32, u8)]) -> (Crop, Vec<Word>) {
        let (w, h) = (400u32, 100u32);
        let mut rgba = vec![30u8; (w * h * 4) as usize];
        for &(_, r, _, lum) in words {
            for y in r.1..r.1 + r.3 {
                for x in r.0..r.0 + r.2 {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 3].copy_from_slice(&[lum, lum, lum]);
                }
            }
        }
        let ws = words.iter().map(|&(t, rect, line, _)| Word { text: t.into(), rect, line }).collect();
        (Crop { w, h, rgba }, ws)
    }

    #[test]
    fn a_white_and_a_grey_line_are_two_speakers() {
        let (c, w) = picture(&[("Do", (100, 10, 30, 20), 0, 210), ("not", (140, 10, 40, 20), 0, 215), ("move.", (190, 10, 60, 20), 0, 205), ("Idiot.", (150, 50, 60, 20), 1, 122)]);
        let parts = speaker_lines(&c, &w);
        assert_eq!(parts.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["Do not move.", "Idiot."]);
        assert_eq!(parts[0].1, 1.0);
        assert!((0.5..0.65).contains(&parts[1].1), "{}", parts[1].1);
    }

    #[test]
    fn a_wrapped_line_is_one_speaker() {
        let (c, w) = picture(&[("You", (100, 10, 40, 20), 0, 210), ("never", (150, 10, 60, 20), 0, 200), ("listen.", (120, 50, 70, 20), 1, 205)]);
        assert!(speaker_lines(&c, &w).is_empty());
        // one line: nothing to split
        let (c, w) = picture(&[("Okay,", (100, 10, 60, 20), 0, 210), ("pull", (170, 10, 40, 20), 0, 120)]);
        assert!(speaker_lines(&c, &w).is_empty());
    }
}

#[cfg(test)]
mod word_tests {
    use super::looks_like_words as ok;
    #[test]
    fn tells_words_from_misread_script() {
        assert!(!ok("bjlwg")); // the PS5's Arabic «وسائط» read by the English OCR
        assert!(!ok("ulnJiJl hjluJ9"));
        assert!(!ok("bjlwg qrtzm"));
        assert!(ok("Go to the guard."));
        assert!(ok("Hmm."));
        assert!(ok("LSPD"));
        assert!(ok("Mr. K"));
        assert!(ok("C'mon mister"));
        assert!(ok("Press {1} to bring up the phone"));
        assert!(ok("Trevor"));
        assert!(ok("Strengths")); // "ngths": a long but real consonant run
        assert!(ok("Röckstar Editör"));
        assert!(ok("$500")); // no words at all: left as it was
    }
}

/// OCR fixes from the game's layout update:
/// `"ocr": {"words": {"cmon": "c'mon"}, "junk": ["bjlwg"], "ok": ["hmm"]}` — whole words, any case
/// (a replacement keeps a leading capital); junk words make a line not text, ok words are real words.
#[derive(Default, Clone, Debug)]
pub struct Fixes {
    pub words: Vec<(String, String)>,
    pub junk: Vec<String>,
    pub ok: Vec<String>,
}

static FIXES_EXTRA: std::sync::RwLock<Fixes> = std::sync::RwLock::new(Fixes { words: Vec::new(), junk: Vec::new(), ok: Vec::new() });

pub fn set_fixes(f: Fixes) {
    *FIXES_EXTRA.write().unwrap() = f;
}

/// Read a layout's `"ocr"` block.
pub fn fixes_from(v: &serde_json::Value) -> Fixes {
    let list = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.to_lowercase()).collect()).unwrap_or_default();
    Fixes {
        words: v["words"].as_object().map(|m| m.iter().filter_map(|(k, x)| Some((k.to_lowercase(), x.as_str()?.to_string()))).collect()).unwrap_or_default(),
        junk: list("junk"),
        ok: list("ok"),
    }
}

/// Windows OCR drops the apostrophe in GTA's subtitle font ("Cmon mister, were giving you ...").
/// Put it back where the word can't be anything else; and "were" + an "-ing" verb at the start of a
/// sentence or after a comma ("mister, were giving") is "we're" ("they were giving" stays).
pub fn fix_contractions(text: &str) -> String {
    const FIXES: [(&str, &str); 34] = [
        ("cmon", "c'mon"), ("im", "i'm"), ("ive", "i've"), ("youre", "you're"), ("youve", "you've"),
        ("youll", "you'll"), ("theyre", "they're"), ("theyve", "they've"), ("theyll", "they'll"),
        ("weve", "we've"), ("dont", "don't"), ("doesnt", "doesn't"), ("didnt", "didn't"), ("cant", "can't"),
        ("wont", "won't"), ("isnt", "isn't"), ("arent", "aren't"), ("wasnt", "wasn't"), ("werent", "weren't"),
        ("wouldnt", "wouldn't"), ("couldnt", "couldn't"), ("shouldnt", "shouldn't"), ("havent", "haven't"),
        ("hasnt", "hasn't"), ("aint", "ain't"), ("thats", "that's"), ("whats", "what's"), ("hes", "he's"),
        ("shes", "she's"), ("theres", "there's"), ("heres", "here's"), ("wheres", "where's"), ("whos", "who's"),
        ("yall", "y'all"),
    ];
    let words: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let start = w.find(|c: char| c.is_alphanumeric()).unwrap_or(w.len());
        // (char boundaries, not bytes: OCR text can hold "ö", "é" ...)
        let end = w.char_indices().rev().find(|(_, c)| c.is_alphanumeric()).map(|(e, c)| e + c.len_utf8()).unwrap_or(start).max(start);
        let (pre, core, post) = (&w[..start], &w[start..end], &w[end..]);
        let lower = core.to_lowercase();
        let mut fixed = FIXES.iter().find(|(k, _)| *k == lower).map(|(_, v)| v.to_string());
        if fixed.is_none() {
            fixed = FIXES_EXTRA.read().unwrap().words.iter().find(|(k, _)| *k == lower).map(|(_, v)| v.clone());
        }
        if fixed.is_none() && lower == "were" {
            let next_ing = words
                .get(i + 1)
                .is_some_and(|n| n.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase().ends_with("ing"));
            // a clause starts here: the line's start, or after , . ! ? ; :
            let clause_start = i == 0 || words[i - 1].ends_with([',', '.', '!', '?', ';', ':']);
            if next_ing && clause_start {
                fixed = Some("we're".into());
            }
        }
        match fixed {
            Some(f) => {
                // keep the capital ("Cmon" -> "C'mon"); "I'm" is always capital
                let f = if core.starts_with(|c: char| c.is_uppercase()) || f.starts_with("i'") {
                    let mut ch = f.chars();
                    ch.next().map(|c| c.to_uppercase().collect::<String>() + ch.as_str()).unwrap_or_default()
                } else {
                    f
                };
                out.push(format!("{pre}{f}{post}"));
            }
            None => out.push(w.to_string()),
        }
    }
    out.join(" ")
}

#[cfg(test)]
mod contraction_tests {
    use super::fix_contractions as f;
    #[test]
    fn puts_lost_apostrophes_back() {
        assert_eq!(f("Cmon mister, were giving you everything you want."), "C'mon mister, we're giving you everything you want.");
        assert_eq!(f("They were giving it away."), "They were giving it away."); // a real "were"
        assert_eq!(f("Dont even think about it. Im serious, youre dead."), "Don't even think about it. I'm serious, you're dead.");
        assert_eq!(f("We were going home."), "We were going home.");
        assert_eq!(f("Were going in!"), "We're going in!"); // line start, no subject
        assert_eq!(f("The cops were waiting."), "The cops were waiting.");
        assert_eq!(f("Go to the guard."), "Go to the guard."); // untouched
        assert_eq!(f("Röckstar Editör, dont stop"), "Röckstar Editör, don't stop"); // non-ASCII letters
    }

    #[test]
    fn layout_fixes() {
        super::set_fixes(super::fixes_from(&serde_json::json!({"words": {"vvanted": "wanted"}, "junk": ["Sogwa"], "ok": ["grrk"]})));
        assert_eq!(f("Vvanted level"), "Wanted level");
        assert!(!super::looks_like_words("sogwa"));
        assert!(super::looks_like_words("Grrk"));
        super::set_fixes(Default::default());
        assert_eq!(f("Vvanted level"), "Vvanted level");
    }
}

/// Similarity 0..1 from edit distance, case-insensitive (used to tell "same line" from "new line").
pub fn similar(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    // (the edit distance is at least the difference in length: too different to be alike, without
    // working it out — the same answer as below, for far less work)
    if a.len().abs_diff(b.len()) * 20 >= a.len().max(b.len()) * 3 {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let dist = prev[b.len()] as f64;
    1.0 - dist / a.len().max(b.len()) as f64 > 0.85
}

/// Highlighted phrases: consecutive words drawn in the same non-white colour → (phrase, "#rrggbb").
/// Only pixels right next to the dark text outline count as letter pixels, so a colourful background
/// behind white text is not mistaken for a highlight.
pub fn colored_phrases(crop: &Crop, words: &[Word]) -> Vec<(String, String)> {
    let (w, h) = (crop.w as usize, crop.h as usize);
    let mut maxc = vec![0u8; w * h];
    let mut minc = vec![0u8; w * h];
    for i in 0..w * h {
        let p = &crop.rgba[i * 4..i * 4 + 3];
        maxc[i] = p[0].max(p[1]).max(p[2]);
        minc[i] = p[0].min(p[1]).min(p[2]);
    }
    // "near outline": dilate the dark mask by 3 px (separable max filter).
    let dark: Vec<bool> = maxc.iter().map(|&m| m < 70).collect();
    let mut horiz = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(3), (x + 3).min(w - 1));
            horiz[y * w + x] = (a..=b).any(|xx| dark[y * w + xx]);
        }
    }
    let mut near = vec![false; w * h];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(3), (y + 3).min(h - 1));
        for x in 0..w {
            near[y * w + x] = (a..=b).any(|yy| horiz[yy * w + x]);
        }
    }

    let word_color = |rect: (u32, u32, u32, u32)| -> Option<(i32, i32, i32)> {
        let (x0, y0) = (rect.0 as usize, rect.1 as usize);
        let (x1, y1) = ((x0 + rect.2 as usize).min(w), (y0 + rect.3 as usize).min(h));
        let (mut letters, mut colored) = (0usize, Vec::new());
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                if near[i] && maxc[i] > 150 {
                    letters += 1;
                    if maxc[i] as i32 - minc[i] as i32 > 60 {
                        let p = &crop.rgba[i * 4..i * 4 + 3];
                        colored.push((p[0] as i32, p[1] as i32, p[2] as i32));
                    }
                }
            }
        }
        if letters < 12 || (colored.len() as f64) < 0.5 * letters as f64 {
            return None;
        }
        let med = |f: fn(&(i32, i32, i32)) -> i32| {
            let mut v: Vec<i32> = colored.iter().map(f).collect();
            v.sort_unstable();
            v[v.len() / 2]
        };
        let (r, g, b) = (med(|p| p.0), med(|p| p.1), med(|p| p.2));
        let m = r.max(g).max(b).max(1) as f64;
        let k = 255.0 / m; // show it at full brightness
        Some(((r as f64 * k) as i32, (g as f64 * k) as i32, (b as f64 * k) as i32))
    };
    let close = |a: (i32, i32, i32), b: (i32, i32, i32)| {
        let d = (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2) + (a.2 - b.2).pow(2);
        d < 80 * 80
    };
    let hex = |c: (i32, i32, i32)| format!("#{:02x}{:02x}{:02x}", c.0.clamp(0, 255), c.1.clamp(0, 255), c.2.clamp(0, 255));

    let mut phrases = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_c: Option<(i32, i32, i32)> = None;
    for word in words {
        let c = word_color(word.rect);
        match (c, cur_c) {
            (Some(c), Some(cc)) if close(c, cc) => {
                cur.push(&word.text);
                continue;
            }
            _ => {}
        }
        if let Some(cc) = cur_c {
            phrases.push((cur.join(" "), hex(cc)));
        }
        cur.clear();
        cur_c = c;
        if c.is_some() {
            cur.push(&word.text);
        }
    }
    if let Some(cc) = cur_c {
        if !cur.is_empty() {
            phrases.push((cur.join(" "), hex(cc)));
        }
    }
    phrases
}

// ------------------------------------------------------------------------------------- button icons

/// Placeholder for the n-th button icon (1-based) inside English text, e.g. "Press ⟦1⟧ to jump."
pub fn icon_token(n: usize) -> String {
    format!("⟦{n}⟧")
}

/// Cut an icon out of a crop as a PNG data URL. `thr` is the brightness that counts as "icon" (it
/// depends on the background: ~140 on a black page, higher on GTA's see-through grey help box).
/// Inside the icon's outline everything stays solid (the dark "R3" / ✕ drawn inside a white disc is
/// kept); outside it the background becomes fully transparent, with a soft 1-pixel edge.
pub fn icon_png(c: &Crop, r: (u32, u32, u32, u32), thr: u8) -> Option<String> {
    use base64::Engine as _;
    let pad = 2;
    let (x0, y0) = (r.0.saturating_sub(pad), r.1.saturating_sub(pad));
    let (x1, y1) = ((r.0 + r.2 + pad).min(c.w), (r.1 + r.3 + pad).min(c.h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let lum = |x: u32, y: u32| {
        let i = ((y * c.w + x) * 4) as usize;
        c.rgba[i].max(c.rgba[i + 1]).max(c.rgba[i + 2])
    };
    let mut img = image::RgbaImage::new(x1 - x0, y1 - y0);
    for y in y0..y1 {
        let first = (x0..x1).find(|&x| lum(x, y) > thr);
        let last = (x0..x1).rev().find(|&x| lum(x, y) > thr);
        for x in x0..x1 {
            let i = ((y * c.w + x) * 4) as usize;
            let (rr, g, b) = (c.rgba[i], c.rgba[i + 1], c.rgba[i + 2]);
            let a = match (first, last) {
                (Some(f), Some(l)) if x >= f && x <= l => 255,
                (Some(f), Some(l)) if x + 1 == f || x == l + 1 => 110, // anti-aliased rim
                _ => 0,
            };
            img.put_pixel(x - x0, y - y0, image::Rgba([rr, g, b, a]));
        }
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// Find controller button icons inside text and rebuild the line with placeholders where they sit:
/// "Press ⟦1⟧ to enter the vehicle." plus the icons cut from the picture (PNG data URLs).
///
/// Icons are found two ways: square-ish bright shapes in the gaps OCR left between words, and
/// "words" OCR made out of an icon (e.g. "@", "©", "8") that are square and taller than the text.
/// Also returns the box (crop pixels: x, y, w, h) the whole text block occupies, words and icons
/// together: GTA's instructions box hugs this block, so the Arabic box can line up with it.
pub fn text_with_icons(c: &Crop, words: &[Word]) -> (String, Vec<String>, Option<(u32, u32, u32, u32)>) {
    if words.is_empty() {
        return (String::new(), Vec::new(), None);
    }
    let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
    hs.sort_unstable();
    let mh = hs[hs.len() / 2].max(4) as f64; // typical text height

    let square = |w: u32, h: u32| {
        let r = w as f64 / h.max(1) as f64;
        (0.6..=1.8).contains(&r) // round icons ~1:1, shoulder buttons (L2/R2) up to ~1.7:1
    };
    // Is there a solid bright shape at this OCR box (an icon), rather than letters? Measured on the
    // tight box of near-white pixels around it: an icon fills ~0.7 of it, letters ≤ ~0.45 (thin
    // strokes), and an icon is about as tall as capitals.
    let solid_shape = |r: (u32, u32, u32, u32)| {
        let (x0, y0) = (r.0.saturating_sub(4), r.1.saturating_sub(4));
        let (x1, y1) = ((r.0 + r.2 + 4).min(c.w), (r.1 + r.3 + 4).min(c.h));
        let (mut tx0, mut ty0, mut tx1, mut ty1, mut n) = (u32::MAX, u32::MAX, 0u32, 0u32, 0u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * c.w + x) * 4) as usize;
                if c.rgba[i].max(c.rgba[i + 1]).max(c.rgba[i + 2]) > 200 {
                    (tx0, ty0, tx1, ty1, n) = (tx0.min(x), ty0.min(y), tx1.max(x), ty1.max(y), n + 1);
                }
            }
        }
        if n == 0 {
            return false;
        }
        let (w, h) = (tx1 - tx0 + 1, ty1 - ty0 + 1);
        square(w, h) && h as f64 >= 0.6 * mh && n as f64 / (w * h) as f64 >= 0.6
    };
    // OCR "words" that are really icons
    let misread = |w: &Word| {
        let t = w.text.trim();
        // shoulder/stick buttons OCR reads as text: "L1", "R2", "R3", "LB", "RT" ...
        let label = t.len() == 2
            && matches!(t.as_bytes()[0], b'L' | b'R')
            && matches!(t.as_bytes()[1], b'1' | b'2' | b'3' | b'B' | b'T' | b'I' | b'l' | b'|'); // OCR often reads 1 as I or l
        let letters = t.chars().filter(|ch| ch.is_alphanumeric()).count();
        // an icon OCR read as a letter or two ("e" for L2 at 1080p): the box is a solid bright shape
        let solid = t.chars().count() <= 2 && solid_shape(w.rect);
        label
            || solid
            || (t.chars().count() <= 2
                && square(w.rect.2, w.rect.3)
                && w.rect.3 as f64 >= 1.1 * mh
                && (letters == 0 || matches!(t, "O" | "o" | "0" | "X" | "x" | "8" | "Q" | "R" | "L" | "®" | "©")))
    };

    // Items of each line, left to right: Ok(word index) or Err(icon box)
    let mut lines: Vec<(f64, Vec<(u32, Result<usize, (u32, u32, u32, u32)>)>)> = Vec::new();
    // Use the OCR's own lines: grouping by height would split a line whose words have tall
    // ("called") and low ("you") letters, scrambling the word order.
    let mut by_line: std::collections::BTreeMap<u32, Vec<usize>> = std::collections::BTreeMap::new();
    for (i, w) in words.iter().enumerate() {
        by_line.entry(w.line).or_default().push(i);
    }
    for idxs in by_line.into_values() {
        let mut cys: Vec<f64> = idxs.iter().map(|&i| words[i].rect.1 as f64 + words[i].rect.3 as f64 / 2.0).collect();
        cys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let cy = cys[cys.len() / 2];
        let items = idxs.iter().map(|&i| (words[i].rect.0, if misread(&words[i]) { Err(words[i].rect) } else { Ok(i) })).collect();
        lines.push((cy, items));
    }
    // Windows OCR sometimes splits one visual line in two around an icon ("Press" | "to zoom in…"):
    // put pieces at the same height, side by side, back together (otherwise both pieces would find
    // the same icon between them).
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let x_range = |items: &Vec<(u32, Result<usize, (u32, u32, u32, u32)>)>| {
        let ends = items.iter().map(|(_, it)| match it {
            Ok(i) => (words[*i].rect.0, words[*i].rect.0 + words[*i].rect.2),
            Err(r) => (r.0, r.0 + r.2),
        });
        ends.fold((u32::MAX, 0), |acc, (a, b)| (acc.0.min(a), acc.1.max(b)))
    };
    let mut joined: Vec<(f64, Vec<(u32, Result<usize, (u32, u32, u32, u32)>)>)> = Vec::new();
    for (cy, items) in lines {
        if let Some(last) = joined.last_mut() {
            let (a, b) = (x_range(&last.1), x_range(&items));
            if (cy - last.0).abs() < 0.5 * mh && (a.1 <= b.0 || b.1 <= a.0) {
                last.1.extend(items);
                continue;
            }
        }
        joined.push((cy, items));
    }
    let mut lines = joined;

    // Bright shapes in the gaps between words on each line. "Bright" is relative to the line's own
    // background: GTA's help box is see-through grey, often over lamps or sky.
    let lum = |x: u32, y: u32| {
        let i = ((y * c.w + x) * 4) as usize;
        c.rgba[i].max(c.rgba[i + 1]).max(c.rgba[i + 2])
    };
    let mut thr_used: u8 = 150;
    let text_boxes: Vec<(u32, u32, u32, u32)> = words.iter().filter(|w| !misread(w)).map(|w| w.rect).collect();
    let in_text = |x: u32, y: u32| {
        text_boxes.iter().any(|&(bx, by, bw, bh)| x + 2 >= bx && x <= bx + bw + 2 && y + 2 >= by && y <= by + bh + 2)
    };
    let mut found_icons: Vec<(u32, u32, u32, u32)> = Vec::new(); // x0, y0, x1, y1 of accepted icons
    for (cy, items) in lines.iter_mut() {
        // icons are about as tall as the text; a taller band would reach past GTA's help box
        let y0 = (*cy - 0.85 * mh).max(0.0) as u32;
        let y1 = ((*cy + 0.85 * mh) as u32).min(c.h - 1);
        // only look near this line's words (icons sit between or right next to them)
        let word_x: Vec<(u32, u32)> =
            items.iter().filter_map(|(_, it)| it.as_ref().ok().map(|&i| (words[i].rect.0, words[i].rect.0 + words[i].rect.2))).collect();
        let lx0 = word_x.iter().map(|w| w.0).min().unwrap_or(0).saturating_sub((3.0 * mh) as u32);
        let lx1 = (word_x.iter().map(|w| w.1).max().unwrap_or(c.w) + (3.0 * mh) as u32).min(c.w - 1);
        // background level of the line (text is a small part of it) -> what counts as icon-bright
        let mut samples: Vec<u8> = (y0..=y1).step_by(2).flat_map(|y| (lx0..=lx1).step_by(3).map(move |x| (x, y))).map(|(x, y)| lum(x, y)).collect();
        samples.sort_unstable();
        let bg = samples.get(samples.len() / 2).copied().unwrap_or(0) as u32;
        // icons and text are near-white; lamps or sky behind the see-through box are dimmer
        let thr = (bg + 80).clamp(220, 245) as u8;
        thr_used = thr;
        let bright = |x: u32, y: u32| lum(x, y) > thr;
        // Connected groups of near-white pixels that aren't part of a word. (Grouping by columns
        // instead would glue an icon to a lamp right under GTA's see-through box.)
        let on = |x: u32, y: u32| bright(x, y) && !in_text(x, y);
        let (bw, bh) = (lx1 - lx0 + 1, y1 - y0 + 1);
        let at = |x: u32, y: u32| ((y - y0) * bw + (x - lx0)) as usize;
        let mut seen = vec![false; (bw * bh) as usize];
        let mut comps: Vec<(u32, u32, u32, u32, u32)> = Vec::new(); // x0, y0, x1, y1, pixel count
        for y in y0..=y1 {
            for x in lx0..=lx1 {
                if seen[at(x, y)] || !on(x, y) {
                    continue;
                }
                seen[at(x, y)] = true;
                let mut stack = vec![(x, y)];
                let mut cc = (x, y, x, y, 0u32);
                while let Some((cx, cy)) = stack.pop() {
                    cc = (cc.0.min(cx), cc.1.min(cy), cc.2.max(cx), cc.3.max(cy), cc.4 + 1);
                    let mut push = |nx: u32, ny: u32| {
                        if nx >= lx0 && nx <= lx1 && ny >= y0 && ny <= y1 && !seen[at(nx, ny)] && on(nx, ny) {
                            seen[at(nx, ny)] = true;
                            stack.push((nx, ny));
                        }
                    };
                    if cx > 0 {
                        push(cx - 1, cy);
                    }
                    push(cx + 1, cy);
                    if cy > 0 {
                        push(cx, cy - 1);
                    }
                    push(cx, cy + 1);
                }
                comps.push(cc);
            }
        }
        // An icon's white parts can be split by its dark symbol (e.g. ✕): merge small groups that
        // touch. Big groups (a lamp, the sky) never merge, or they would swallow a nearby icon.
        let small = |c: &(u32, u32, u32, u32, u32)| ((c.2 - c.0 + 1) as f64) <= 2.5 * mh && ((c.3 - c.1 + 1) as f64) <= 2.5 * mh;
        let mut merged = true;
        while merged {
            merged = false;
            'outer: for i in 0..comps.len() {
                for j in i + 1..comps.len() {
                    let (a, b) = (comps[i], comps[j]);
                    if small(&a) && small(&b) && a.0 <= b.2 + 2 && b.0 <= a.2 + 2 && a.1 <= b.3 + 2 && b.1 <= a.3 + 2 {
                        comps[i] = (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3), a.4 + b.4);
                        comps.remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
            }
        }
        for (s, ty, e, by, count) in comps {
            let (w, h) = (e - s + 1, by - ty + 1);
            if count < 6 {
                continue;
            }
            if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                eprintln!("[icons] line bg {bg} thr {thr} mh {mh:.0}: blob x {s}-{e} y {ty}-{by} ({w}x{h})");
            }
            if (h as f64) < 0.55 * mh || (h as f64) > 2.5 * mh || !square(w, h) {
                continue;
            }
            // an icon sits inside the line; something spanning the whole band is letters/scenery
            if ty == y0 && by == y1 {
                continue;
            }
            let fill = count as f64 / (w * h) as f64;
            if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                eprintln!("[icons] line bg {bg} thr {thr} mh {mh:.0}: candidate x {s}-{e} y {ty}-{by} ({w}x{h}) fill {fill:.2}");
            }
            // icons are solid shapes (≥ ~0.45, e.g. R2's white box with dark letters); stray letter strokes ~0.3
            if !(0.42..=0.95).contains(&fill) {
                continue;
            }
            // ...with a dark symbol inside (✕ △ "R3" "L2"); a plain bright patch of scenery has none
            let (ix0, ix1) = (s + w / 5, e - w / 5);
            let (iy0, iy1) = (ty + h / 5, by - h / 5);
            let inner = ((ix1 - ix0 + 1) * (iy1 - iy0 + 1)).max(1) as f64;
            let dark = (iy0..=iy1).flat_map(|y| (ix0..=ix1).map(move |x| (x, y))).filter(|&(x, y)| lum(x, y) < 110).count();
            if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                eprintln!("[icons]    dark symbol share {:.2}", dark as f64 / inner);
            }
            // (inside GTA's solid black box a bright solid shape can only be an icon — e.g. the touchpad,
            // a light bar with faint lines; over see-through scenery it could be a lamp)
            if (dark as f64) < 0.04 * inner && !(bg < 60 && fill >= 0.7) {
                continue;
            }
            // a real icon has a space on both sides; a stroke OCR left outside a word ("w?nna") touches it
            let line_words: Vec<(u32, u32)> = items
                .iter()
                .filter_map(|(_, it)| it.as_ref().ok().map(|&i| (words[i].rect.0, words[i].rect.0 + words[i].rect.2)))
                .collect();
            let min_gap = (0.2 * mh) as u32;
            let left_gap = line_words.iter().filter(|w| w.1 <= s).map(|w| s - w.1).min().unwrap_or(u32::MAX);
            let right_gap = line_words.iter().filter(|w| w.0 >= e).map(|w| w.0 - e).min().unwrap_or(u32::MAX);
            let overlaps_word = line_words.iter().any(|w| w.0 < e && w.1 > s);
            if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                eprintln!("[icons]    gaps left {left_gap} right {right_gap} (need {min_gap}), overlaps word {overlaps_word}");
            }
            if overlaps_word || left_gap < min_gap || right_gap < min_gap {
                continue;
            }
            // skip if it overlaps an icon already found as a misread word
            if items.iter().any(|(_, it)| matches!(it, Err(r) if s < r.0 + r.2 && e > r.0)) {
                continue;
            }
            // ...or one another line already took
            if found_icons.iter().any(|f| s <= f.2 && e >= f.0 && ty <= f.3 && by >= f.1) {
                continue;
            }
            found_icons.push((s, ty, e, by));
            items.push((s, Err((s, ty, w, h))));
        }
        items.sort_by_key(|(x, _)| *x);
    }
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let mut parts: Vec<String> = Vec::new();
    let mut icons = Vec::new();
    let mut ext: Option<(u32, u32, u32, u32)> = None; // x0, y0, x1, y1
    let mut grow = |r: (u32, u32, u32, u32)| {
        let (x1, y1) = (r.0 + r.2, r.1 + r.3);
        ext = Some(match ext {
            None => (r.0, r.1, x1, y1),
            Some(e) => (e.0.min(r.0), e.1.min(r.1), e.2.max(x1), e.3.max(y1)),
        });
    };
    for (_, items) in &lines {
        for (_, it) in items {
            match it {
                Ok(i) => {
                    grow(words[*i].rect);
                    parts.push(words[*i].text.clone())
                }
                Err(r) => {
                    grow(*r);
                    if let Some(png) = icon_png(c, *r, thr_used) {
                        // "<height relative to the text>|<png>": the Arabic draws it at the same proportion
                        // (GTA's L2/R2 are smaller than the text, ✕/△ discs a bit larger)
                        // (+4: the cut-out has a 2 px transparent margin above and below the icon)
                        icons.push(format!("{:.3}|{png}", (r.3 + 4) as f64 / mh));
                        parts.push(icon_token(icons.len()));
                    }
                }
            }
        }
    }
    (fix_contractions(&parts.join(" ")), icons, ext.map(|e| (e.0, e.1, e.2 - e.0, e.3 - e.1)))
}


// ------------------------------------------------------------------------------------- OCR passes

/// How pixels are turned into the black-on-white picture the OCR reads.
#[derive(Clone, Copy)]
pub enum Prep {
    /// Brightest colour channel (white AND coloured text stay bright). Best for outlined subtitles.
    Brightest,
    /// Only near-white or strongly coloured pixels, stretched: drops a lamp or sky showing through
    /// GTA's grey see-through box, and dull red/brown scenery, while keeping white and highlight text.
    WhiteOrColour,
}

fn prep_value(mode: Prep, r: i32, g: i32, b: i32) -> u8 {
    match mode {
        Prep::Brightest => r.max(g).max(b) as u8,
        Prep::WhiteOrColour => {
            let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
            let v = if mx - mn > 80 { mx } else { mn }; // coloured: its brightness; grey/white: its whiteness
            (((v - 150).max(0) * 255) / 105).min(255) as u8
        }
    }
}

/// (good, bad) characters in an OCR reading: words made of plain letters with sane capitals are
/// good; accented letters in English, stray symbols, punctuation inside words, "shoZOith"-style
/// capitals are bad. Lone symbols (often a misread button icon) count as neither.
pub fn quality(words: &[Word]) -> (i32, i32) {
    let (mut good, mut bad) = (0, 0);
    for w in words {
        let t = w.text.trim_matches(|c: char| ".,!?;:\"'()".contains(c));
        let n = t.chars().count() as i32;
        if n == 0 {
            continue;
        }
        if n == 1 && !t.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue; // "@", "©": an icon, judged elsewhere
        }
        let letters_ok = t.chars().all(|c| c.is_ascii_alphabetic() || c == '\'');
        let rest: String = t.chars().skip(1).filter(|c| c.is_ascii_alphabetic()).collect();
        let case_ok = rest.chars().all(|c| c.is_ascii_lowercase()) || t.chars().all(|c| !c.is_ascii_lowercase());
        let number = t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '$');
        if (letters_ok && case_ok) || number {
            good += n;
        } else {
            bad += n;
        }
    }
    (good, bad)
}


/// Remove scenery read as text: tiny fragments carrying quote marks or bullets (`"c'`, `Y'"`, `•`).
fn drop_junk(words: Vec<Word>) -> Vec<Word> {
    words
        .into_iter()
        .filter(|w| {
            let t = w.text.trim();
            let letters = t.chars().filter(|c| c.is_alphabetic()).count();
            let junky = t.chars().any(|c| "\"“”•|=~_^`".contains(c))
                || ((t.starts_with('\'') || t.ends_with('\'')) && letters <= 2);
            !(junky && letters <= 2)
        })
        .collect()
}
