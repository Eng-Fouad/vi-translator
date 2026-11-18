//! Translation memory: remember every translated line, reuse it instantly next time.
//!
//! Keys are normalised English (lower-case, no punctuation, button icons as `{1}`, `{2}`), so the same
//! game line matches whatever icon images or punctuation OCR produced. The Arabic is stored as a
//! template: `{n}` for the n-th icon and `<cK>…</cK>` for the K-th highlighted (coloured) phrase, so
//! icons and colours from the current frame are put back on reuse.
//!
//! Everything lives in one folder ("translation DB"), encrypted (see `secure`):
//!  - `memory-<lang>.dat` — lines translated live by the chosen translator (appended, one sealed
//!                      record per line). Lines that come from a downloaded shared pack replace them;
//!  - `lines-log.dat`  — every English line seen: area, start/end time (shared with the server when
//!                      the player allows it);
//!  - `reviewed.jsonl` — only on the maintainer's PC: reviewed translations being prepared (plain
//!                      text, read if present; the app never writes it).
//! Files from before encryption (`memory.jsonl`, `memory-<lang>.jsonl`) are converted once, then removed.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::secure;
use crate::translate::Segments;

/// The lines log (encrypted), shared with the server by `sync`.
pub const LOG_FILE: &str = "lines-log.dat";
const LOG_KIND: &str = "lines-log";

/// Read a sealed log record (for `sync`).
pub fn open_log_line(line: &str) -> Option<String> {
    secure::open(LOG_KIND, line)
}

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
    pub entries: usize,
    /// how many of them are reviewed translations
    pub reviewed: usize,
    pub hits: u64,
    pub misses: u64,
    /// average time from the English line appearing to its Arabic being ready
    pub avg_hit_ms: f64,
    pub avg_miss_ms: f64,
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
    /// translations downloaded in the background, by language (the one in use is applied; the others
    /// wait for a switch back): kept through reload, clear and language switches
    packs: HashMap<String, Vec<Entry>>,
}

/// What lines logged now are, for sharing: (game, confirmed, held, sharing on). Confirmed: one of
/// the game's own pages has been on screen this session, so this is really that game. Held: another
/// game the app knows is on screen (the player is playing something else with this profile). Sharing
/// on: "Help improve translations" is on now — a line seen while it's off is never sent, even if it's
/// turned on again later.
static SHARE: std::sync::RwLock<(String, bool, bool, bool)> = std::sync::RwLock::new((String::new(), false, false, false));

pub fn set_share(game: &str, confirmed: bool, held: bool, on: bool) {
    let mut s = SHARE.write().unwrap();
    if s.0 != game || s.1 != confirmed || s.2 != held || s.3 != on {
        *s = (game.to_string(), confirmed, held, on);
    }
}

/// How much we trust each source: a better translation is never replaced by a worse one.
fn rank(source: &str) -> u8 {
    match source {
        "reviewed" => 6,
        "phrasebook" => 5,
        "claude" | "openai" => 4,
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
        // (lower case first, then only letters, numbers and apostrophes: so a key is its own key)
        let w: String = tok.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect();
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
pub(crate) fn fill(tpl: &str, icons: &[String], phrases: &[(String, String)]) -> Segments {
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
    /// Load the memory for a target language from the translation-DB folder (`memory-<lang>.dat`, plus
    /// `reviewed.jsonl` for Arabic when it's there). A memory file from before encryption is converted.
    pub fn load(dir: PathBuf, lang: &str) -> Self {
        convert_other_languages(&dir, lang);
        let path = dir.join(format!("memory-{lang}.dat"));
        let kind = format!("memory/{lang}");
        let mut map: HashMap<String, Entry> = HashMap::new();
        let add = |e: Entry, map: &mut HashMap<String, Entry>| {
            // later lines win unless a better source is already there
            let keep_old = map.get(&e.en).is_some_and(|old: &Entry| rank(&old.source) > rank(&e.source));
            if !keep_old {
                map.insert(e.en.clone(), e);
            }
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                if let Some(e) = secure::open(&kind, line).and_then(|j| serde_json::from_str::<Entry>(&j).ok()) {
                    add(e, &mut map);
                }
            }
        }
        // before encryption: memory.jsonl (Arabic) or memory-<lang>.jsonl, in plain text
        let legacy = if lang == "ar" { dir.join("memory.jsonl") } else { dir.join(format!("memory-{lang}.jsonl")) };
        if let Ok(text) = std::fs::read_to_string(&legacy) {
            let moved: Vec<Entry> = text.lines().filter_map(|l| serde_json::from_str::<Entry>(l).ok()).collect();
            if append_sealed(&path, &kind, moved.iter().map(|e| serde_json::to_string(e).unwrap_or_default())) {
                let _ = std::fs::remove_file(&legacy);
            }
            for e in moved {
                add(e, &mut map);
            }
        }
        if lang == "ar" {
            if let Ok(text) = std::fs::read_to_string(dir.join("reviewed.jsonl")) {
                for line in text.lines() {
                    if let Ok(mut e) = serde_json::from_str::<Entry>(line) {
                        e.source = "reviewed".into();
                        e.en = key_of(&e.en); // a hand-edited file may keep raw English
                        add(e, &mut map);
                    }
                }
            }
        }
        let mut m = Self { dir, lang: lang.to_string(), path, map: HashMap::new(), index: HashMap::new(), hits: 0, misses: 0, hit_ms: 0.0, miss_ms: 0.0, packs: HashMap::new() };
        for (k, e) in map {
            m.insert(k, e);
        }
        m
    }

    fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            for w in key.split(' ') {
                if let Some(keys) = self.index.get_mut(w) {
                    keys.retain(|k| k != key);
                }
            }
        }
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
        // (half the words in common: OCR noise breaks a word or two of a line — "Yepr", "cometo")
        let need = words.len().div_ceil(2).max(1);
        let mut cands: Vec<(&str, usize)> = shared.into_iter().filter(|(_, n)| *n >= need).collect();
        cands.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let icons = key.matches('{').count();
        let len = key.chars().count() as f64;
        let mut best: Option<(f64, &Entry)> = None;
        // compared without spaces (a lost or extra space is no difference); long lines may differ a
        // little more than short ones, where one letter can change the meaning
        let bare = |t: &str| t.replace(' ', "");
        let key_bare = bare(key);
        let need_sim = if key_bare.chars().count() >= 12 { 0.86 } else { 0.92 };
        for (k, _) in cands.into_iter().take(20) {
            let kl = k.chars().count() as f64;
            if (kl - len).abs() > len * 0.15 + 2.0 || k.matches('{').count() != icons {
                continue; // can't be that alike anyway
            }
            let s = similarity(&bare(k), &key_bare);
            if s >= need_sim && best.is_none_or(|(b, _)| s > b) {
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

    /// Note an English line that was on screen from `start_ms` to `end_ms` (Unix ms) in `area`, with
    /// the game it was seen in, whether that game was confirmed on screen, and whether it is held back
    /// from sharing (see `set_share`).
    pub fn log_line(&self, area: &str, en: &str, start_ms: u64, end_ms: u64) {
        let key = key_of(en);
        if key.is_empty() {
            return;
        }
        let (game, confirmed, held, on) = SHARE.read().unwrap().clone();
        let row = serde_json::json!({ "area": area, "key": key, "en": en, "start": start_ms, "end": end_ms, "game": game, "confirmed": confirmed, "held": held, "share": on });
        append_sealed(&self.dir.join(LOG_FILE), LOG_KIND, [row.to_string()]);
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
        // (the same again adds nothing: the file only grows with what's new)
        if self.map.get(&key).is_some_and(|old| old.ar == e.ar && old.source == e.source) {
            return;
        }
        append_sealed(&self.path, &format!("memory/{}", self.lang), [serde_json::to_string(&e).unwrap_or_default()]);
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

    pub fn stats(&self) -> Stats {
        let avg = |sum: f64, n: u64| if n == 0 { 0.0 } else { sum / n as f64 };
        Stats {
            entries: self.map.len(),
            reviewed: self.map.values().filter(|e| e.source == "reviewed").count(),
            hits: self.hits,
            misses: self.misses,
            avg_hit_ms: avg(self.hit_ms, self.hits),
            avg_miss_ms: avg(self.miss_ms, self.misses),
        }
    }

    /// Forget the live translations (the lines log stays).
    pub fn clear(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let packs = std::mem::take(&mut self.packs);
        *self = Self::load(self.dir.clone(), &self.lang);
        self.take_packs(packs);
    }

    /// Switch to another target language (its own memory, and what was downloaded for it).
    pub fn set_lang(&mut self, lang: &str) {
        if self.lang != lang {
            let (h, m, hm, mm) = (self.hits, self.misses, self.hit_ms, self.miss_ms);
            let packs = std::mem::take(&mut self.packs);
            *self = Self::load(self.dir.clone(), lang);
            (self.hits, self.misses, self.hit_ms, self.miss_ms) = (h, m, hm, mm);
            self.take_packs(packs);
        }
    }

    /// The target language this memory is for.
    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// The downloads kept from before a reload: the language in use's applied again.
    fn take_packs(&mut self, mut packs: HashMap<String, Vec<Entry>>) {
        let mine = packs.remove(&self.lang);
        self.packs = packs;
        if let Some(entries) = mine {
            let lang = self.lang.clone();
            self.set_shared(&lang, entries);
        }
    }

    /// Add translations downloaded in the background: they replace the player's own lines for the
    /// same English, here and in the memory file (which then keeps only what the pack doesn't have).
    pub fn set_shared(&mut self, lang: &str, entries: Vec<Entry>) {
        // (downloaded for another language: the player switched meanwhile; kept for a switch back)
        if lang != self.lang {
            self.packs.insert(lang.to_string(), entries);
            return;
        }
        // lines the last pack had and this one doesn't (taken out on the server): not used any more
        let gone: Vec<String> = {
            let now: std::collections::HashSet<&str> = entries.iter().map(|e| e.en.as_str()).collect();
            self.packs
                .get(lang)
                .into_iter()
                .flatten()
                .filter(|e| !now.contains(e.en.as_str()))
                .filter(|e| self.map.get(&e.en).is_some_and(|m| m.source == e.source && m.ar == e.ar))
                .map(|e| e.en.clone())
                .collect()
        };
        for k in gone {
            self.remove(&k);
        }
        for e in &entries {
            if self.map.get(&e.en).is_none_or(|old| rank(&old.source) <= rank(&e.source)) {
                self.insert(e.en.clone(), e.clone());
            }
        }
        let covered: std::collections::HashSet<&str> = entries.iter().map(|e| e.en.as_str()).collect();
        self.drop_from_file(&covered);
        self.packs.insert(lang.to_string(), entries);
    }

    /// Rewrite the memory file without the lines for these English keys (if it has any).
    fn drop_from_file(&self, keys: &std::collections::HashSet<&str>) {
        let kind = format!("memory/{}", self.lang);
        let Ok(text) = std::fs::read_to_string(&self.path) else { return };
        let mut removed = 0;
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| {
                let gone = secure::open(&kind, l)
                    .and_then(|j| serde_json::from_str::<Entry>(&j).ok())
                    .is_some_and(|e| keys.contains(e.en.as_str()));
                removed += gone as usize;
                !gone
            })
            .collect();
        if removed == 0 {
            return;
        }
        // written next to it, then swapped in: a crash can't leave half a file
        let tmp = self.path.with_extension("dat.tmp");
        let body: String = kept.iter().map(|l| format!("{l}\n")).collect();
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

/// Memory files from before encryption for the other languages (memory-<lang>.jsonl): converted too,
/// so nothing stays readable on disk (the one for `lang` is converted by `load` itself).
fn convert_other_languages(dir: &std::path::Path, lang: &str) {
    let Ok(files) = std::fs::read_dir(dir) else { return };
    for f in files.flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        let Some(other) = name.strip_prefix("memory-").and_then(|n| n.strip_suffix(".jsonl")) else { continue };
        if other == lang || other.is_empty() || !other.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        let lines = text.lines().filter(|l| serde_json::from_str::<Entry>(l).is_ok()).map(String::from);
        if append_sealed(&dir.join(format!("memory-{other}.dat")), &format!("memory/{other}"), lines) {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

/// Append records, each sealed on its own line. False if the file couldn't be written.
fn append_sealed(path: &std::path::Path, kind: &str, records: impl IntoIterator<Item = String>) -> bool {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return false };
    let body: String = records.into_iter().map(|r| secure::seal(kind, &r) + "\n").collect();
    f.write_all(body.as_bytes()).is_ok()
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
    fn files_are_encrypted_old_ones_converted_and_shared_lines_replace_local_ones() {
        let path = std::env::temp_dir().join(format!("vi-translator-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        // a memory file from before encryption
        std::fs::write(
            path.join("memory.jsonl"),
            "{\"en\":\"take cover\",\"ar\":\"احتمِ\",\"source\":\"local\"}\n{\"en\":\"follow me\",\"ar\":\"اتبعني\",\"source\":\"local\"}\n",
        )
        .unwrap();
        let mut m = Memory::load(path.clone(), "ar");
        assert!(!path.join("memory.jsonl").exists(), "the plain file is converted, then removed");
        let dat = std::fs::read_to_string(path.join("memory-ar.dat")).unwrap();
        assert!(!dat.contains("take cover") && !dat.contains("احتمِ"), "nothing readable on disk");
        assert!(m.get("Take cover!", &[], &[]).is_some(), "still remembered");
        m.put("Get in the car.", &vec![("اركب السيارة.".into(), None)], "local");
        m.log_line("subtitles", "Get in the car.", 1, 2);
        assert!(!std::fs::read_to_string(path.join(LOG_FILE)).unwrap().contains("car"), "the log is sealed too");
        // each logged line says which game it was seen in, and whether it may be shared
        set_share("rdr2", true, true, true);
        m.log_line("subtitles", "Saddle up.", 5, 9);
        set_share("", false, false, false);
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(path.join(LOG_FILE)).unwrap().lines().filter_map(open_log_line).filter_map(|l| serde_json::from_str(&l).ok()).collect();
        let last = rows.last().unwrap();
        assert_eq!((last["en"].as_str(), last["game"].as_str(), last["confirmed"].as_bool(), last["held"].as_bool()), (Some("Saddle up."), Some("rdr2"), Some(true), Some(true)));
        assert_eq!(last["share"].as_bool(), Some(true), "seen while sharing was on");
        // after a restart: the same lines, read back from the encrypted file
        let mut m = Memory::load(path.clone(), "ar");
        assert_eq!(m.get("get in the car", &[], &[]).unwrap().0[0].0, "اركب السيارة.");
        // a shared pack with "take cover": it wins, and the local line leaves the file
        m.set_shared("ar", vec![Entry { en: "take cover".into(), ar: "احتمِ بساتر!".into(), source: "reviewed".into() }]);
        assert_eq!(m.get("take cover", &[], &[]).unwrap().1, "reviewed");
        let m2 = Memory::load(path.clone(), "ar");
        assert!(m2.get("take cover", &[], &[]).is_none(), "the player's own line for it is gone");
        assert!(m2.get("follow me", &[], &[]).is_some(), "lines the pack doesn't have stay");
        // a pack for another language (the player switched languages meanwhile): not taken
        let mut m3 = Memory::load(path.clone(), "ar");
        m3.set_shared("ar", vec![Entry { en: "take cover".into(), ar: "احتمِ بساتر!".into(), source: "reviewed".into() }]);
        m3.set_shared("hi", vec![Entry { en: "follow me".into(), ar: "मेरे पीछे आओ".into(), source: "reviewed".into() }]);
        assert_ne!(m3.get("follow me", &[], &[]).unwrap().1, "reviewed");
        // a line taken out of the pack on the server: not used any more
        m3.set_shared("ar", vec![]);
        assert!(m3.get("take cover", &[], &[]).is_none());
        // switching languages and back (before the next download): the download is used again
        m3.set_shared("ar", vec![Entry { en: "take cover".into(), ar: "احتمِ بساتر!".into(), source: "reviewed".into() }]);
        m3.set_lang("hi");
        assert_eq!(m3.lang(), "hi");
        assert_eq!(m3.get("follow me", &[], &[]).unwrap().1, "reviewed", "hi's own download, kept from before");
        assert!(m3.get("take cover", &[], &[]).is_none());
        m3.set_lang("ar");
        assert_eq!(m3.get("take cover", &[], &[]).unwrap().1, "reviewed");
        let _ = std::fs::remove_dir_all(&path);
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
        // (the server's KeyNormalizerTest has the same cases)
        assert_eq!(key_of("Go\u{a0}to the car."), "go to the car");
        assert_eq!(key_of("Press\u{a0}\u{27e6}1\u{27e7} to jump"), "press {1} to jump");
        assert_eq!(key_of("1\u{bd} miles"), "1\u{bd} miles");
        assert_eq!(key_of("E=mc\u{b2}"), "emc\u{b2}");
        assert_eq!(key_of("Chapter \u{2165}"), "chapter \u{2175}");
        assert_eq!(key_of("\u{2003}Lucia\u{2003}"), "lucia");
        assert_eq!(key_of("\u{130}stanbul"), "istanbul");
        assert_eq!(key_of(&key_of("\u{130}stanbul")), key_of("\u{130}stanbul"));
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
        let mut m = Memory::load(path.clone(), "ar");
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
