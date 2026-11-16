//! Translation memory (experiment): remember every translated line, reuse it instantly next time.
//!
//! Keys are normalised English (lower-case, no punctuation, button icons as `{1}`, `{2}`), so the same
//! game line matches whatever icon images or punctuation OCR produced. The Arabic is stored as a
//! template: `{n}` for the n-th icon and `<cK>…</cK>` for the K-th highlighted (coloured) phrase, so
//! icons and colours from the current frame are put back on reuse.
//!
//! Everything lives in one folder ("translation DB"):
//!  - `memory.jsonl`  — lines translated live by the chosen translator (appended, one JSON per line);
//!  - `reviewed.jsonl` — the same shape: reviewed, high-quality translations
//!                      (what a future backend will ship); it always wins and is never overwritten;
//!  - `lines-log.jsonl` — every English line seen: area, start/end time — so lines that follow each
//!                      other closely (one conversation) can be translated together, with context.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::translate::Segments;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    /// normalised English key
    pub en: String,
    /// Arabic template with `{n}` icons and `<cK>…</cK>` colours
    pub ar: String,
    /// which translator made it ("claude", "deepl", "local", "phrasebook", "reviewed" …)
    pub source: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Stats {
    pub enabled: bool,
    pub entries: usize,
    /// how many of them are reviewed translations
    pub reviewed: usize,
    pub hits: u64,
    pub misses: u64,
    /// average time from the English line appearing to its Arabic being ready
    pub avg_hit_ms: f64,
    pub avg_miss_ms: f64,
    pub file: String,
}

pub struct Memory {
    dir: PathBuf,
    /// target language: each has its own memory file
    lang: String,
    path: PathBuf,
    map: HashMap<String, Entry>,
    /// word -> keys containing it, so near matches only compare lines that share most words
    index: HashMap<String, Vec<String>>,
    hits: u64,
    misses: u64,
    hit_ms: f64,
    miss_ms: f64,
    /// translations downloaded in the background (kept through reload / clear)
    shared: Vec<Entry>,
}

/// How much we trust each source: a better translation is never replaced by a worse one.
fn rank(source: &str) -> u8 {
    match source {
        "reviewed" => 6,
        "phrasebook" => 5,
        "claude" => 4,
        "deepl" | "azure" | "google" => 3,
        "libre" => 2,
        _ => 1, // local model
    }
}

/// Lower-case words; icons "⟦n⟧" -> "{n}" (renumbered in order); punctuation dropped.
pub fn key_of(line: &str) -> String {
    let mut out = Vec::new();
    let mut icon = 0;
    for tok in line.split_whitespace() {
        let core = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '⟦' && c != '⟧'); // "⟦1⟧." is still an icon
        // "{n}" too, so a key that is already normalised (reviewed files) stays the same
        let braced = tok.len() > 2 && tok.starts_with('{') && tok.ends_with('}') && tok[1..tok.len() - 1].chars().all(|c| c.is_ascii_digit());
        if braced || (core.starts_with('⟦') && core.ends_with('⟧')) {
            icon += 1;
            out.push(format!("{{{icon}}}"));
            continue;
        }
        let w: String = tok.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase();
        if !w.is_empty() {
            out.push(w);
        }
    }
    out.join(" ")
}

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

/// Segments -> template (icons -> {n}, coloured runs -> <cK>…</cK>).
fn to_template(segs: &Segments) -> String {
    let (mut out, mut icon, mut col) = (String::new(), 0, 0);
    for (t, c) in segs {
        match c.as_deref() {
            Some(c) if c.starts_with("icon:") => {
                icon += 1;
                out.push_str(&format!("{{{icon}}}"));
            }
            Some(_) => {
                col += 1;
                out.push_str(&format!("<c{col}>{t}</c{col}>"));
            }
            None => out.push_str(t),
        }
    }
    out
}

/// Template -> segments with this frame's icons and highlight colours.
fn fill(tpl: &str, icons: &[String], phrases: &[(String, String)]) -> Segments {
    let mut segs: Segments = Vec::new();
    let mut rest = tpl;
    loop {
        let next_icon = rest.find('{');
        let next_col = rest.find("<c");
        let at = match (next_icon, next_col) {
            (None, None) => break,
            (Some(a), None) | (None, Some(a)) => a,
            (Some(a), Some(b)) => a.min(b),
        };
        if at > 0 {
            segs.push((rest[..at].to_string(), None));
        }
        rest = &rest[at..];
        if rest.starts_with('{') {
            let Some(close) = rest.find('}') else { break };
            if let Some(png) = rest[1..close].parse::<usize>().ok().and_then(|n| icons.get(n.wrapping_sub(1))) {
                segs.push((String::new(), Some(format!("icon:{png}"))));
            }
            rest = &rest[close + 1..];
        } else {
            let Some(gt) = rest.find('>') else { break };
            let k = rest[2..gt].to_string();
            let close_tag = format!("</c{k}>");
            let Some(end) = rest.find(&close_tag) else { break };
            let text = rest[gt + 1..end].to_string();
            let color = k.parse::<usize>().ok().and_then(|k| phrases.get(k.wrapping_sub(1))).map(|p| p.1.clone());
            segs.push((text, color));
            rest = &rest[end + close_tag.len()..];
        }
    }
    if !rest.is_empty() {
        segs.push((rest.to_string(), None));
    }
    segs
}

impl Memory {
    /// Load the memory for a target language from the translation-DB folder (Arabic:
    /// memory.jsonl and reviewed.jsonl; others: memory-<lang>.jsonl).
    pub fn load(dir: PathBuf, lang: &str) -> Self {
        let arabic = lang == "ar";
        let path = if arabic { dir.join("memory.jsonl") } else { dir.join(format!("memory-{lang}.jsonl")) };
        let mut map = HashMap::new();
        let files = [(path.clone(), None), (dir.join("reviewed.jsonl"), Some("reviewed"))];
        for (file, forced) in files.into_iter().take(if arabic { 2 } else { 1 }) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            for line in text.lines() {
                if let Ok(mut e) = serde_json::from_str::<Entry>(line) {
                    if let Some(src) = forced {
                        e.source = src.into();
                        e.en = key_of(&e.en); // a hand-edited file may keep raw English
                    }
                    // later lines win unless a better source is already there
                    let keep_old = map.get(&e.en).is_some_and(|old: &Entry| rank(&old.source) > rank(&e.source));
                    if !keep_old {
                        map.insert(e.en.clone(), e);
                    }
                }
            }
        }
        let mut m = Self { dir, lang: lang.to_string(), path, map: HashMap::new(), index: HashMap::new(), hits: 0, misses: 0, hit_ms: 0.0, miss_ms: 0.0, shared: Vec::new() };
        for (k, e) in map {
            m.insert(k, e);
        }
        m
    }

    fn insert(&mut self, key: String, e: Entry) {
        if !self.map.contains_key(&key) {
            let mut words: Vec<&str> = key.split(' ').collect();
            words.sort_unstable();
            words.dedup();
            for w in words {
                self.index.entry(w.to_string()).or_default().push(key.clone());
            }
        }
        self.map.insert(key, e);
    }

    /// Best entry for this English: exact key, else a close one (≥ 92 %, same number of icons).
    /// Near matches are looked for only among lines sharing ≥ 70 % of the words (via the word
    /// index), best 20 first — so a miss stays fast even with many thousands of lines.
    fn find(&self, key: &str) -> Option<&Entry> {
        if let Some(e) = self.map.get(key) {
            return Some(e);
        }
        let mut words: Vec<&str> = key.split(' ').collect();
        words.sort_unstable();
        words.dedup();
        let mut shared: HashMap<&str, usize> = HashMap::new();
        for w in &words {
            if let Some(keys) = self.index.get(*w) {
                for k in keys {
                    *shared.entry(k.as_str()).or_default() += 1;
                }
            }
        }
        let need = (words.len() * 7).div_ceil(10).max(1);
        let mut cands: Vec<(&str, usize)> = shared.into_iter().filter(|(_, n)| *n >= need).collect();
        cands.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let icons = key.matches('{').count();
        let len = key.chars().count() as f64;
        let mut best: Option<(f64, &Entry)> = None;
        for (k, _) in cands.into_iter().take(20) {
            let kl = k.chars().count() as f64;
            if (kl - len).abs() > len * 0.1 + 2.0 || k.matches('{').count() != icons {
                continue; // can't reach 92 % anyway
            }
            let s = similarity(k, key);
            if s >= 0.92 && best.is_none_or(|(b, _)| s > b) {
                best = Some((s, &self.map[k]));
            }
        }
        best.map(|(_, e)| e)
    }

    pub fn knows(&self, line: &str) -> bool {
        let key = key_of(line);
        !key.is_empty() && self.find(&key).is_some()
    }

    /// The remembered Arabic for this line (with this frame's icons and colours) and its source.
    pub fn get(&self, line: &str, icons: &[String], phrases: &[(String, String)]) -> Option<(Segments, String)> {
        let key = key_of(line);
        if key.is_empty() {
            return None;
        }
        self.find(&key).map(|e| (fill(&e.ar, icons, phrases), e.source.clone()))
    }

    /// Note an English line that was on screen from `start_ms` to `end_ms` (Unix ms) in `area`.
    pub fn log_line(&self, area: &str, en: &str, start_ms: u64, end_ms: u64) {
        let key = key_of(en);
        if key.is_empty() {
            return;
        }
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("lines-log.jsonl")) {
            let row = serde_json::json!({ "area": area, "key": key, "en": en, "start": start_ms, "end": end_ms });
            let _ = writeln!(f, "{row}");
        }
    }

    pub fn put(&mut self, line: &str, segs: &Segments, source: &str) {
        let key = key_of(line);
        if key.is_empty() || segs.iter().all(|(t, c)| t.trim().is_empty() && c.is_none()) {
            return;
        }
        if self.map.get(&key).is_some_and(|old| rank(&old.source) > rank(source)) {
            return;
        }
        let e = Entry { en: key.clone(), ar: to_template(segs), source: source.into() };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default());
        }
        self.insert(key, e);
    }

    pub fn record(&mut self, hit: bool, ms: f64) {
        if hit {
            self.hits += 1;
            self.hit_ms += ms;
        } else {
            self.misses += 1;
            self.miss_ms += ms;
        }
    }

    pub fn stats(&self, enabled: bool) -> Stats {
        let avg = |sum: f64, n: u64| if n == 0 { 0.0 } else { sum / n as f64 };
        Stats {
            enabled,
            entries: self.map.len(),
            reviewed: self.map.values().filter(|e| e.source == "reviewed").count(),
            hits: self.hits,
            misses: self.misses,
            avg_hit_ms: avg(self.hit_ms, self.hits),
            avg_miss_ms: avg(self.miss_ms, self.misses),
            file: self.dir.display().to_string(),
        }
    }

    /// Forget the live translations (the lines log stays).
    pub fn clear(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let shared = std::mem::take(&mut self.shared);
        *self = Self::load(self.dir.clone(), &self.lang);
        self.set_shared(shared);
    }

    /// Switch to another target language (its own memory; downloads come again for it).
    pub fn set_lang(&mut self, lang: &str) {
        if self.lang != lang {
            let (h, m, hm, mm) = (self.hits, self.misses, self.hit_ms, self.miss_ms);
            *self = Self::load(self.dir.clone(), lang);
            (self.hits, self.misses, self.hit_ms, self.miss_ms) = (h, m, hm, mm);
        }
    }

    /// Add translations downloaded in the background (they replace weaker ones).
    pub fn set_shared(&mut self, entries: Vec<Entry>) {
        for e in &entries {
            if self.map.get(&e.en).is_none_or(|old| rank(&old.source) <= rank(&e.source)) {
                self.insert(e.en.clone(), e.clone());
            }
        }
        self.shared = entries;
    }

    /// Re-read the files (e.g. after new reviewed translations); session counters are kept.
    pub fn reload(&mut self) {
        let (h, m, hm, mm) = (self.hits, self.misses, self.hit_ms, self.miss_ms);
        let shared = std::mem::take(&mut self.shared);
        *self = Self::load(self.dir.clone(), &self.lang);
        self.set_shared(shared);
        (self.hits, self.misses, self.hit_ms, self.miss_ms) = (h, m, hm, mm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("vi-translator-mem-{}", std::process::id()))
    }
    fn text(segs: &Segments) -> String {
        segs.iter().map(|(t, c)| match c.as_deref() {
            Some(c) if c.starts_with("icon:") => format!("[{}]", &c[5..]),
            Some(c) => format!("<{t}|{c}>"),
            None => t.clone(),
        }).collect()
    }

    #[test]
    fn remembers_with_new_icons_and_colours_and_survives_restart() {
        let path = tmp();
        let _ = std::fs::remove_dir_all(&path);
        let mut m = Memory::load(path.clone(), "ar");
        let segs: Segments = vec![
            ("اذهب إلى ".into(), None),
            ("البنك".into(), Some("#67caff".into())),
            (" واضغط ".into(), None),
            (String::new(), Some("icon:0.8|OLD".into())),
        ];
        m.put("Go to the bank and press ⟦1⟧.", &segs, "local");
        // same line later: different punctuation/case, a new icon image, a new highlight colour
        let got = m.get("go to the bank and press ⟦1⟧", &["0.9|NEW".into()], &[("bank".into(), "#ffd455".into())]).unwrap().0;
        assert_eq!(text(&got), "اذهب إلى <البنك|#ffd455> واضغط [0.9|NEW]");
        // reloaded from disk
        let m2 = Memory::load(path.clone(), "ar");
        assert!(m2.knows("Go to the bank and press ⟦1⟧!"));
        assert_eq!(key_of(&key_of("Press ⟦1⟧ ⟦2⟧ to zoom.")), "press {1} {2} to zoom");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn small_ocr_slips_hit_but_different_lines_dont() {
        let path = tmp().with_extension("2");
        let _ = std::fs::remove_dir_all(&path);
        let mut m = Memory::load(path.clone(), "ar");
        m.put("Blame the pricks who called you out here!", &vec![("ألقِ باللوم على الأوغاد الذين استدعوك إلى هنا!".into(), None)], "local");
        assert!(m.knows("Blame the pricks who caled you out here"));
        assert!(!m.knows("Blame the cops who called you out here"));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn better_sources_are_not_overwritten() {
        let path = tmp().with_extension("3");
        let _ = std::fs::remove_dir_all(&path);
        let mut m = Memory::load(path.clone(), "ar");
        m.put("Take cover.", &vec![("احتمِ.".into(), None)], "claude");
        m.put("Take cover.", &vec![("خذ الغطاء".into(), None)], "local");
        assert_eq!(text(&m.get("Take cover.", &[], &[]).unwrap().0), "احتمِ.");
        // a reviewed line wins over everything and survives clearing the memory
        std::fs::write(path.join("reviewed.jsonl"), "{\"en\":\"Take cover!\",\"ar\":\"احتمِ بساتر!\",\"source\":\"x\"}\n").unwrap();
        m.reload();
        let (segs, src) = m.get("take cover", &[], &[]).unwrap();
        assert_eq!((text(&segs).as_str(), src.as_str()), ("احتمِ بساتر!", "reviewed"));
        m.clear();
        assert!(m.knows("Take cover."));
        let _ = std::fs::remove_dir_all(&path);
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    #[test]
    fn lookup_stays_fast_with_thousands_of_lines() {
        let path = std::env::temp_dir().join(format!("vi-translator-mem-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let mut m = Memory::load(path.clone(), "ar");
        for i in 0..5000 {
            let en = format!("This is game line number {i} about the heist and the cops near the bank");
            m.insert(key_of(&en), Entry { en: key_of(&en), ar: "نص".into(), source: "local".into() });
        }
        let t = std::time::Instant::now();
        let hit = m.knows("This is game line number 4321 about the heist and the cops near the bank");
        let exact_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let miss = m.knows("A completely different sentence that is not stored anywhere at all here");
        let miss_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let near = m.knows("This is game line number 4321 about the heist and the cops near the bnk");
        let near_ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("5000 entries: exact hit {exact_ms:.2} ms, miss {miss_ms:.2} ms, near hit {near_ms:.2} ms");
        assert!(hit && !miss && near);
        let _ = std::fs::remove_dir_all(&path);
    }
}
