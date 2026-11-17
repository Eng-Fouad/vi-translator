//! English → target-language translation engines (Arabic and the other languages in `langs`).
//!
//! - local:  OPUS-MT en→ar (Helsinki-NLP, ONNX export by Xenova) run on the CPU with ONNX Runtime. Free, offline.
//! - deepl / azure / google / libre: their REST translation APIs (free tiers available).
//! - claude: the Claude Messages API (raw HTTP; there is no official Rust SDK).
//!
//! Output is a list of segments `(text, colour)` so highlighted English words can keep their colour.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use ort::session::Session;
use ort::value::Tensor;
use serde_json::{Value, json};
use tokenizers::Tokenizer;

use crate::config::Config;

pub type Segments = Vec<(String, Option<String>)>;

// ------------------------------------------------------------------------------------- local model

const MODEL_FILES: [&str; 4] =
    ["onnx/encoder_model_quantized.onnx", "onnx/decoder_model_quantized.onnx", "tokenizer.json", "config.json"];

/// ONNX Runtime allows one environment per process; create it once and keep it for the app's lifetime.
fn ort_env() -> Result<&'static ort::environment::Environment> {
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(e) = ENV.get() {
        return Ok(e);
    }
    let env = ort::init().with_name("vi-translator").build()?;
    Ok(ENV.get_or_init(|| env))
}

pub struct LocalMt {
    tok: Tokenizer,
    /// target-language token some models need first (">>ara<<", ">>vie<<")
    prefix_id: Option<u32>,
    /// padding = the decoder's start token; end of sentence
    pad_id: i64,
    eos_id: i64,
    enc: Session,
    dec: Session,
}

impl LocalMt {
    /// Loads the model for `lang`, downloading it first (~110 MB, once) if it is missing.
    pub fn load(dir: &Path, lang: &crate::langs::Lang, http: &reqwest::blocking::Client, status: &dyn Fn(String)) -> Result<Self> {
        let repo = lang.local.ok_or_else(|| {
            anyhow!("There is no free offline model for {}: choose another translation service in the Translator tab.", lang.name)
        })?;
        for f in MODEL_FILES {
            let path = dir.join(f);
            if path.exists() {
                continue;
            }
            status(format!("Downloading the free translation model: {f} ..."));
            std::fs::create_dir_all(path.parent().unwrap())?;
            let mut resp = http
                .get(format!("https://huggingface.co/{repo}/resolve/main/{f}"))
                .timeout(Duration::from_secs(600))
                .send()?
                .error_for_status()
                .with_context(|| format!("download {f}"))?;
            let part = path.with_extension("part");
            let mut file = std::fs::File::create(&part)?;
            resp.copy_to(&mut file)?;
            drop(file);
            std::fs::rename(&part, &path)?;
        }
        status("Loading the translation model ...".into());
        // This export has an empty "Precompiled" normalizer, which the Rust tokenizer rejects; drop it.
        let mut json: Value = serde_json::from_slice(&std::fs::read(dir.join("tokenizer.json"))?)?;
        if json["normalizer"]["type"] == "Precompiled" && json["normalizer"]["precompiled_charsmap"].is_null() {
            json["normalizer"] = Value::Null;
        }
        let tok = Tokenizer::from_bytes(serde_json::to_vec(&json)?).map_err(|e| anyhow!("tokenizer: {e}"))?;
        // multi-language models need their target token first (en→ar: ">>ara<<")
        let prefix_id = [">>ara<<", ">>vie<<"].iter().find_map(|t| tok.token_to_id(t));
        let conf: Value = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let pad_id = conf["decoder_start_token_id"].as_i64().or(conf["pad_token_id"].as_i64()).context("model config: no start token")?;
        let eos_id = conf["eos_token_id"].as_i64().unwrap_or(0);
        let env = ort_env()?;
        let enc = Session::builder(env)?.commit_from_file(dir.join(MODEL_FILES[0]))?;
        let dec = Session::builder(env)?.commit_from_file(dir.join(MODEL_FILES[1]))?;
        Ok(Self { tok, prefix_id, pad_id, eos_id, enc, dec })
    }

    pub fn translate(&mut self, text: &str) -> Result<String> {
        // (the target-language token, when the model needs one, is added by id so it isn't split)
        let encoding = self.tok.encode(text, true).map_err(|e| anyhow!("{e}"))?;
        let ids: Vec<i64> =
            self.prefix_id.map(|p| p as i64).into_iter().chain(encoding.get_ids().iter().map(|&i| i as i64)).collect();
        let n = ids.len();
        let mask = vec![1i64; n];

        let (d, hidden) = {
            let out = self.enc.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, n], ids))?,
                "attention_mask" => Tensor::from_array(([1usize, n], mask.clone()))?,
            ])?;
            let (shape, data) = out["last_hidden_state"].try_extract_tensor::<f32>()?;
            (shape[2] as usize, data.to_vec())
        };

        // Greedy decoding (no KV cache; subtitle lines are short).
        let (pad, eos) = (self.pad_id, self.eos_id);
        let mut out_ids = vec![pad];
        for _ in 0..(n * 3).max(32).min(200) {
            let m = out_ids.len();
            let next = {
                let out = self.dec.run(ort::inputs![
                    "input_ids" => Tensor::from_array(([1usize, m], out_ids.clone()))?,
                    "encoder_hidden_states" => Tensor::from_array(([1usize, n, d], hidden.clone()))?,
                    "encoder_attention_mask" => Tensor::from_array(([1usize, n], mask.clone()))?,
                ])?;
                let (shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
                let v = shape[2] as usize;
                let last = &logits[(m - 1) * v..m * v];
                let mut best = (0usize, f32::MIN);
                for (i, &l) in last.iter().enumerate() {
                    if i as i64 != pad && l > best.1 {
                        best = (i, l);
                    }
                }
                best.0 as i64
            };
            if next == eos {
                break;
            }
            out_ids.push(next);
        }
        let ids: Vec<u32> = out_ids[1..].iter().map(|&i| i as u32).collect();
        Ok(self.tok.decode(&ids, true).map_err(|e| anyhow!("{e}"))?.trim().to_string())
    }
}

// ------------------------------------------------------------------------------------- translator

const DIALECTS: [(&str, &str); 4] = [
    ("msa", "Modern Standard Arabic (الفصحى), natural and readable, not stiff"),
    ("egyptian", "Egyptian Arabic dialect (اللهجة المصرية)"),
    ("gulf", "Gulf Arabic dialect (اللهجة الخليجية)"),
    ("levantine", "Levantine Arabic dialect (اللهجة الشامية)"),
];

/// USD per million tokens (input, output) for the cost counter.
fn claude_price(model: &str) -> (f64, f64) {
    match model {
        m if m.starts_with("claude-haiku-5-5") => (0.10, 0.50),
        m if m.starts_with("claude-sonnet-5-5") => (2.0, 10.0),
        m if m.starts_with("claude-opus-5-5") => (4.0, 20.0),
        _ => (0.0, 0.0),
    }
}

pub struct Translator {
    http: reqwest::blocking::Client,
    model_dir: PathBuf,
    local: Option<LocalMt>,
    cache: HashMap<String, Segments>,
    /// Arabic words for a highlighted English phrase (see `recolor`)
    phrase_words: HashMap<String, Vec<String>>,
    /// target language the local model, phrase words and context are for
    lang: String,
    history: VecDeque<(String, String)>,
    pub cost_usd: f64,
    pub lines: u64,
}

impl Translator {
    pub fn new(model_dir: PathBuf) -> Self {
        let http = reqwest::blocking::Client::builder()
            .user_agent("vi-translator/0.1")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");
        Self { http, model_dir, local: None, cache: HashMap::new(), phrase_words: HashMap::new(), lang: String::new(), history: VecDeque::new(), cost_usd: 0.0, lines: 0 }
    }

    /// `line` may contain button-icon placeholders "⟦n⟧" (see `ocr::text_with_icons`); `icons[n-1]` is
    /// the icon image. The icons come back as segments `("", Some("icon:<png data url>"))`.
    /// Translate a game line. Names and terms that must stay as they are (GTA+, GTA$500,000,
    /// web addresses …) go through as "[1]", "[2]" — the one form the local model never changes —
    /// and are put back afterwards.
    pub fn translate(
        &mut self,
        cfg: &Config,
        line: &str,
        phrases: &[(String, String)],
        icons: &[String],
        status: &dyn Fn(String),
    ) -> Result<Segments> {
        // another target language: its own model, word lists and context
        if self.lang != cfg.target_lang {
            self.lang = cfg.target_lang.clone();
            self.local = None;
            self.phrase_words.clear();
            self.history.clear();
        }
        let (protected, terms) = protect_terms(line);
        // nothing but kept terms ("GTA+"): nothing to translate
        let rest: String = protected.chars().filter(|c| c.is_alphabetic()).collect();
        if !terms.is_empty() && rest.is_empty() {
            return Ok(vec![(format!("\u{2066}{line}\u{2069}"), None)]);
        }
        let mut segs = self.translate_inner(cfg, &protected, phrases, icons, status)?;
        // (the phrasebook's hand-written lines carry no colours, and the translator may have missed one)
        segs = self.recolor(cfg, segs, phrases, status);
        if !terms.is_empty() {
            const AR_DIGITS: [char; 10] = ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'];
            for seg in segs.iter_mut() {
                for (i, t) in terms.iter().enumerate() {
                    let n = (i + 1).to_string();
                    let an: String = n.chars().map(|c| AR_DIGITS[c.to_digit(10).unwrap_or(0) as usize]).collect();
                    // the model may answer in Arabic digits, and right-to-left text can flip the brackets
                    // (inside an invisible left-to-right isolate, so right-to-left text can't turn
                    // "GTA+" into "+GTA")
                    let kept = format!("\u{2066}{t}\u{2069}");
                    for form in [format!("[{n}]"), format!("]{n}["), format!("[{an}]"), format!("]{an}[")] {
                        seg.0 = seg.0.replace(&form, &kept);
                    }
                }
            }
        }
        Ok(segs)
    }

    fn translate_inner(
        &mut self,
        cfg: &Config,
        line: &str,
        phrases: &[(String, String)],
        icons: &[String],
        status: &dyn Fn(String),
    ) -> Result<Segments> {
        // Slang machine translators mistake for a name ("C'mon" -> "سيد (كومون)"): give them plain words.
        let expanded = expand_slang(line);
        let line = expanded.as_str();
        let icon_sig: usize = icons.iter().map(|i| i.len()).sum();
        let key = format!("{}\u{1}{}\u{1}{}\u{1}{}\u{1}{:?}\u{1}{icon_sig}", cfg.engine, cfg.target_lang, cfg.dialect, line, phrases);
        if let Some(s) = self.cache.get(&key) {
            return Ok(s.clone());
        }
        // GTA V profile: the game's common instructions have exact, hand-written Arabic.
        // (and any game's phrases from its layout update)
        if let Some(segs) = crate::phrasebook::find(&cfg.game(), &cfg.target_lang, line, icons) {
            self.lines += 1;
            self.cache.insert(key, segs.clone());
            return Ok(segs);
        }
        let segs = if cfg.engine == "claude" {
            self.claude(cfg, line, phrases, icons)?
        } else {
            // Machine translators can't be trusted to keep markers, so translate the line without
            // the icons and put each icon back after the Arabic for the English word before it.
            let (plain, anchors) = icon_anchors(line, icons.len());
            let arabic = cfg.target_lang == "ar";
            // One batch: the line, plus each highlighted phrase in a few forms ("bank" alone is
            // ambiguous, "the bank" isn't) so we can find which Arabic words to colour.
            let mut batch = vec![plain.clone()];
            let mut groups = Vec::new();
            for (phrase, color) in phrases {
                let bare = phrase.trim_matches(|c: char| " .,!?;:\"'".contains(c)).to_string();
                let variants: Vec<String> = {
                    let mut v = vec![phrase.clone(), bare.clone(), format!("the {bare}")];
                    v.dedup();
                    v
                };
                groups.push((batch.len(), variants.len(), color.clone()));
                batch.extend(variants);
            }
            let out = self.mt_batch(cfg, &batch, status)?;
            let sentence = out.first().cloned().unwrap_or_default();
            let mut targets: Vec<(Vec<String>, String)> = Vec::new();
            for (start, len, color) in groups {
                let words = out[start..start + len]
                    .iter()
                    .flat_map(|t| t.split_whitespace().map(ar_norm).collect::<Vec<_>>())
                    .filter(|w| !w.is_empty())
                    .collect();
                targets.push((words, color));
            }
            let segs = color_tokens(&sentence, &targets);
            if icons.is_empty() {
                segs
            } else {
                // Anchor words that aren't known button verbs: translate them alone to find them.
                let unknown: Vec<String> =
                    anchors.iter().filter_map(|a| a.prev.clone()).filter(|w| !arabic || verb_stems(w).is_none()).collect();
                let found = if unknown.is_empty() { Vec::new() } else { self.mt_batch(cfg, &unknown, status)? };
                let lookup: HashMap<String, Vec<String>> = unknown
                    .into_iter()
                    .zip(found)
                    .map(|(en, ar)| (en, ar.split_whitespace().map(ar_norm).filter(|w| !w.is_empty()).collect()))
                    .collect();
                place_icons(segs, &anchors, icons, &lookup, arabic)
            }
        };
        self.lines += 1;
        self.cache.insert(key, segs.clone());
        Ok(segs)
    }

    /// Colour the game's highlighted words ("Go to <green>Lamar</green>") in an Arabic line that has
    /// no colours: a line from memory saved before (or without) its highlight was
    /// seen, or a hand-written phrasebook line. Each phrase is translated on its own (a few forms,
    /// cached) and the matching Arabic words take its colour. Anything that fails leaves the line
    /// as it was.
    pub fn recolor(&mut self, cfg: &Config, segs: Segments, phrases: &[(String, String)], status: &dyn Fn(String)) -> Segments {
        let has_color = segs.iter().any(|s| s.1.as_deref().is_some_and(|c| !c.starts_with("icon:")));
        if phrases.is_empty() || has_color || cfg.engine == "claude" {
            return segs;
        }
        let mut targets: Vec<(Vec<String>, String)> = Vec::new();
        for (phrase, color) in phrases {
            let bare = phrase.trim_matches(|c: char| " .,!?;:\"'".contains(c)).to_string();
            if bare.is_empty() {
                continue;
            }
            let words = match self.phrase_words.get(&bare.to_lowercase()) {
                Some(w) => w.clone(),
                None => {
                    let mut variants = vec![bare.clone(), format!("the {bare}")];
                    if *phrase != bare {
                        variants.push(phrase.clone());
                    }
                    let Ok(out) = self.mt_batch(cfg, &variants, status) else { return segs };
                    let w: Vec<String> =
                        out.iter().flat_map(|t| t.split_whitespace().map(ar_norm).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect();
                    self.phrase_words.insert(bare.to_lowercase(), w.clone());
                    w
                }
            };
            if !words.is_empty() {
                targets.push((words, color.clone()));
            }
        }
        if targets.is_empty() {
            return segs;
        }
        let mut out: Segments = Vec::new();
        for (text, c) in segs {
            if c.is_some() || text.trim().is_empty() {
                out.push((text, c));
                continue;
            }
            // keep the spaces around this piece (an icon may sit on either side)
            if text.starts_with(char::is_whitespace) {
                out.push((" ".into(), None));
            }
            out.extend(color_tokens(&text, &targets));
            if text.ends_with(char::is_whitespace) {
                out.push((" ".into(), None));
            }
        }
        out
    }

    pub fn stats(&self, cfg: &Config) -> String {
        if cfg.engine == "claude" {
            format!("{} lines, ${:.4}", self.lines, self.cost_usd)
        } else {
            format!("{} lines", self.lines)
        }
    }

    fn mt_batch(&mut self, cfg: &Config, texts: &[String], status: &dyn Fn(String)) -> Result<Vec<String>> {
        match cfg.engine.as_str() {
            "local" => {
                if self.local.is_none() {
                    let lang = crate::langs::get(&cfg.target_lang);
                    // (Arabic's model stays where it always was; the others get their own folder)
                    let dir = if lang.code == "ar" { self.model_dir.clone() } else { self.model_dir.join(lang.code) };
                    self.local = Some(LocalMt::load(&dir, lang, &self.http, status)?);
                    status("Free local translator ready.".into());
                }
                let mt = self.local.as_mut().unwrap();
                texts.iter().map(|t| mt.translate(t)).collect()
            }
            "deepl" => {
                need(&cfg.deepl_key, "DeepL API key")?;
                let host = if cfg.deepl_key.ends_with(":fx") { "api-free.deepl.com" } else { "api.deepl.com" };
                let v = self.post_json(
                    self.http
                        .post(format!("https://{host}/v2/translate"))
                        .header("Authorization", format!("DeepL-Auth-Key {}", cfg.deepl_key))
                        .json(&json!({ "text": texts, "source_lang": "EN", "target_lang": crate::langs::get(&cfg.target_lang).deepl })),
                )?;
                strings(&v["translations"], |t| t["text"].as_str())
            }
            "azure" => {
                need(&cfg.azure_key, "Azure Translator key")?;
                let mut req = self
                    .http
                    .post(format!(
                        "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&from=en&to={}",
                        crate::langs::get(&cfg.target_lang).azure
                    ))
                    .header("Ocp-Apim-Subscription-Key", &cfg.azure_key);
                if !cfg.azure_region.trim().is_empty() {
                    req = req.header("Ocp-Apim-Subscription-Region", cfg.azure_region.trim());
                }
                let body: Vec<Value> = texts.iter().map(|t| json!({ "Text": t })).collect();
                let v = self.post_json(req.json(&body))?;
                strings(&v, |t| t["translations"][0]["text"].as_str())
            }
            "google" => {
                need(&cfg.google_key, "Google Cloud API key")?;
                let v = self.post_json(
                    self.http
                        .post("https://translation.googleapis.com/language/translate/v2")
                        .header("X-goog-api-key", &cfg.google_key)
                        .json(&json!({ "q": texts, "source": "en", "target": crate::langs::get(&cfg.target_lang).google, "format": "text" })),
                )?;
                strings(&v["data"]["translations"], |t| t["translatedText"].as_str())
            }
            "libre" => {
                need(&cfg.libre_url, "LibreTranslate URL")?;
                let mut body = json!({ "q": texts, "source": "en", "target": crate::langs::get(&cfg.target_lang).libre, "format": "text" });
                if !cfg.libre_key.is_empty() {
                    body["api_key"] = json!(cfg.libre_key);
                }
                let url = format!("{}/translate", cfg.libre_url.trim_end_matches('/'));
                let v = self.post_json(self.http.post(url).json(&body))?;
                match &v["translatedText"] {
                    Value::Array(_) => strings(&v["translatedText"], |t| t.as_str()),
                    Value::String(s) => Ok(vec![s.clone()]),
                    _ => bail!("Unexpected LibreTranslate reply"),
                }
            }
            other => bail!("Unknown translator \"{other}\""),
        }
    }

    fn post_json(&self, req: reqwest::blocking::RequestBuilder) -> Result<Value> {
        let resp = req.send().context("network error")?;
        let code = resp.status();
        let text = resp.text().unwrap_or_default();
        if !code.is_success() {
            let snippet: String = text.chars().take(200).collect();
            bail!("HTTP {code}: {snippet}");
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn claude(&mut self, cfg: &Config, line: &str, phrases: &[(String, String)], icons: &[String]) -> Result<Segments> {
        need(&cfg.claude_key, "Anthropic API key")?;
        let lang = crate::langs::get(&cfg.target_lang);
        let target = if lang.code == "ar" {
            DIALECTS.iter().find(|d| d.0 == cfg.dialect).unwrap_or(&DIALECTS[0]).1.to_string()
        } else {
            format!("{} ({}), natural and readable", lang.name, lang.native)
        };
        let name = lang.name;
        let system = format!(
            "You translate English video-game text (subtitles and on-screen instructions) into {name} for players reading live.\n\
             Target language: {target}.\n\
             Rules:\n\
             - Output ONLY the {name} translation of the NEW line. No quotes, notes or English.\n\
             - Keep it short enough to read in 2-3 seconds; meaning and tone over literal wording.\n\
             - Translate slang, jokes and insults naturally (tone down only the most graphic slurs).\n\
             - The line came from OCR and may have small errors; silently fix obvious ones.\n\
             - Use earlier lines only as context; never re-translate them.\n\
             - Words wrapped in tags like <c1>...</c1> are highlighted in the game. Wrap the {name} words that\n\
               translate them in the same tag (same number). Never add tags that weren't in the input.\n\
             - Tags like <b1/> are controller button icons (e.g. the X or triangle button). Put each one where it\n\
               belongs in the {name} sentence, e.g. in Arabic \"Press <b1/> to jump\" -> \"اضغط <b1/> للقفز\". Keep every\n\
               icon tag exactly once."
        );
        // Tag the highlighted phrases: "Go to the <c1>bank</c1>."
        let mut tagged = line.to_string();
        for n in (1..=icons.len()).rev() {
            tagged = tagged.replace(&crate::ocr::icon_token(n), &format!("<b{n}/>"));
        }
        let mut colors = HashMap::new();
        let mut pos = 0;
        for (i, (phrase, color)) in phrases.iter().enumerate() {
            if let Some(at) = tagged[pos..].find(phrase.as_str()).map(|a| a + pos) {
                let n = i + 1;
                let wrapped = format!("<c{n}>{phrase}</c{n}>");
                tagged.replace_range(at..at + phrase.len(), &wrapped);
                pos = at + wrapped.len();
                colors.insert(n.to_string(), color.clone());
            }
        }
        let context: Vec<String> = self.history.iter().map(|(en, ar)| format!("EN: {en}\nAR: {ar}")).collect();
        let prompt = if context.is_empty() {
            format!("NEW line:\n{tagged}")
        } else {
            format!("Earlier lines:\n{}\n\nNEW line:\n{tagged}", context.join("\n"))
        };

        let model = cfg.claude_model.trim();
        let mut body = json!({
            "model": model,
            "max_tokens": 1024,
            "system": system,
            "messages": [{ "role": "user", "content": prompt }],
            "output_config": { "effort": "low" },
        });
        let mut req = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &cfg.claude_key)
            .header("anthropic-version", "2026-10-21");
        if model.starts_with("claude-haiku") {
            // Cheapest path: thinking off, so we only pay for the short Arabic line.
            body["thinking"] = json!({ "type": "disabled" });
            body["max_tokens"] = json!(300);
        } else {
            // On a policy decline, let the API retry on a fallback model in the same call.
            req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            body["fallbacks"] = json!("default");
        }
        let v = self.post_json(req.json(&body))?;

        let (pin, pout) = claude_price(model);
        let usage = &v["usage"];
        self.cost_usd += (usage["input_tokens"].as_f64().unwrap_or(0.0) * pin
            + usage["output_tokens"].as_f64().unwrap_or(0.0) * pout)
            / 1e6;
        if v["stop_reason"] == "refusal" {
            return Ok(Vec::new());
        }
        let ar: String = v["content"]
            .as_array()
            .map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
            .unwrap_or_default();
        let ar = ar.trim();

        // Split "<c1>...</c1>" into coloured segments; drop any stray tags.
        let mut segs: Segments = Vec::new();
        let mut rest = ar;
        while let Some(open) = rest.find("<c") {
            let after = &rest[open + 2..];
            let Some(gt) = after.find('>') else { break };
            let num = &after[..gt];
            let close_tag = format!("</c{num}>");
            let inner_start = open + 2 + gt + 1;
            let Some(close) = rest[inner_start..].find(&close_tag) else { break };
            segs.push((rest[..open].to_string(), None));
            segs.push((rest[inner_start..inner_start + close].to_string(), colors.get(num).cloned()));
            rest = &rest[inner_start + close + close_tag.len()..];
        }
        segs.push((rest.to_string(), None));
        let segs: Segments = segs
            .into_iter()
            .map(|(t, c)| (strip_tags(&t), c))
            .filter(|(t, _)| !t.is_empty())
            .collect();

        // Button icons: "<b1/>" -> icon segment (an icon Claude dropped is added at the end).
        let segs = split_icon_tags(segs, icons);
        let plain: String = segs.iter().filter(|s| !is_icon(s)).map(|s| s.0.as_str()).collect();
        self.history.push_back((line.to_string(), plain));
        while self.history.len() > 3 {
            self.history.pop_front();
        }
        Ok(segs)
    }
}

fn need(v: &str, what: &str) -> Result<()> {
    if v.trim().is_empty() { bail!("Enter your {what} in Settings → Translator.") } else { Ok(()) }
}

fn strings(v: &Value, f: impl Fn(&Value) -> Option<&str>) -> Result<Vec<String>> {
    v.as_array()
        .ok_or_else(|| anyhow!("Unexpected reply: {v}"))?
        .iter()
        .map(|t| f(t).map(str::to_string).ok_or_else(|| anyhow!("Unexpected reply item: {t}")))
        .collect()
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("<") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let is_tag = tail.starts_with("<c") || tail.starts_with("</c"); // <bN/> icons are kept
        match (is_tag, tail.find('>')) {
            (true, Some(j)) => rest = &tail[j + 1..],
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------------------------- colour matching

/// Normalise an Arabic word for matching: drop diacritics/tatweel/punctuation, unify alef/ya/ta marbuta,
/// strip one leading conjunction/preposition letter and the article "ال".
pub fn ar_norm(word: &str) -> String {
    let mut w: String = word
        .chars()
        .filter(|c| !('\u{064B}'..='\u{0652}').contains(c) && *c != '\u{0640}')
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ى' => 'ي',
            'ة' => 'ه',
            c => c,
        })
        .collect();
    if w.chars().count() > 3 && w.starts_with(['و', 'ف', 'ب', 'ل', 'ك']) {
        w = w.chars().skip(1).collect();
    }
    if w.chars().count() > 3 && w.starts_with("ال") {
        w = w.chars().skip(2).collect();
    }
    w
}

fn close_enough(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la.min(lb) < 3 {
        return false;
    }
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=bv.len()).collect();
    for i in 1..=av.len() {
        let mut cur = vec![i; bv.len() + 1];
        for j in 1..=bv.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(av[i - 1] != bv[j - 1]));
        }
        prev = cur;
    }
    1.0 - prev[bv.len()] as f64 / la.max(lb) as f64 >= 0.8
}

/// Colour the words of `sentence` that match a highlighted phrase's translation.
fn color_tokens(sentence: &str, targets: &[(Vec<String>, String)]) -> Segments {
    let tokens: Vec<&str> = sentence.split_whitespace().collect();
    let mut segs: Segments = Vec::new();
    for tok in &tokens {
        let n = ar_norm(tok);
        let color = if n.is_empty() {
            None
        } else {
            targets.iter().find(|(words, _)| words.iter().any(|t| close_enough(&n, t))).map(|(_, c)| c.clone())
        };
        match segs.last_mut() {
            None => segs.push((tok.to_string(), color)),
            // Same colour as the previous run: extend it ("منزل فرانكلين" stays one coloured run).
            Some(last) if last.1 == color => {
                last.0.push(' ');
                last.0.push_str(tok);
            }
            // Uncoloured word after a coloured run: the space goes with the plain word.
            Some(_) if color.is_none() => segs.push((format!(" {tok}"), None)),
            // Coloured word after plain text: keep the space uncoloured.
            Some(_) => {
                segs.push((" ".into(), None));
                segs.push((tok.to_string(), color));
            }
        }
    }
    segs
}


// ------------------------------------------------------------------------------------- button icons

/// Where an icon sat in the English line.
pub struct IconAnchor {
    /// The English word just before the icon (lower-case, no punctuation); None = start of the line.
    pub prev: Option<String>,
    /// The icon was the last thing on the line.
    pub at_end: bool,
}

/// Remove "⟦n⟧" placeholders: returns the plain line and, for each icon in order, its anchor.
fn icon_anchors(line: &str, n_icons: usize) -> (String, Vec<IconAnchor>) {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut plain = Vec::new();
    let mut anchors: Vec<(usize, IconAnchor)> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let num = t.strip_prefix('⟦').and_then(|r| r.strip_suffix('⟧')).and_then(|n| n.parse::<usize>().ok());
        match num {
            Some(n) if n >= 1 && n <= n_icons => {
                let prev = plain.last().map(|w: &&str| {
                    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
                });
                let at_end = tokens[i + 1..].iter().all(|t| t.starts_with('⟦'));
                anchors.push((n, IconAnchor { prev: prev.filter(|p| !p.is_empty()), at_end }));
            }
            _ => plain.push(*t),
        }
    }
    anchors.sort_by_key(|(n, _)| *n);
    (plain.join(" "), anchors.into_iter().map(|(_, a)| a).collect())
}

/// Arabic stems for the verbs that come before button icons ("Press ✕", "Hold △", ...).
fn verb_stems(en: &str) -> Option<&'static [&'static str]> {
    Some(match en {
        "press" | "push" | "mash" | "click" => &["اضغط", "انقر", "ادفع"],
        "hold" => &["اضغط", "استمر", "امسك", "ابق"],
        "tap" => &["انقر", "اضغط"],
        "use" => &["استخدم", "استعمل"],
        "release" => &["حرر", "افلت", "اترك", "ارفع"],
        "pull" => &["اسحب"],
        "select" | "choose" => &["اختر", "حدد"],
        "toggle" | "switch" => &["بدل", "غير"],
        _ => return None,
    })
}

pub fn is_icon(seg: &(String, Option<String>)) -> bool {
    seg.1.as_deref().is_some_and(|c| c.starts_with("icon:"))
}

fn icon_seg(png: &str) -> (String, Option<String>) {
    (String::new(), Some(format!("icon:{png}")))
}

/// Put icons back into machine-translated Arabic: after the Arabic for the English word before the
/// icon ("Press ✕" -> after "اضغط"); at the start or end when the icon started or ended the line.
fn place_icons(segs: Segments, anchors: &[IconAnchor], icons: &[String], lookup: &HashMap<String, Vec<String>>, arabic: bool) -> Segments {
    // the Arabic button verbs ("اضغط") only help in Arabic
    let verb_stems = |w: &str| if arabic { verb_stems(w) } else { None };
    // word-level tokens, keeping colours
    let mut toks: Vec<(String, Option<String>)> = Vec::new();
    for (t, c) in segs {
        for w in t.split_whitespace() {
            toks.push((w.to_string(), c.clone()));
        }
    }
    let mut floor = 0; // keep icons in their English order
    for (a, png) in anchors.iter().zip(icons) {
        let idx = match (&a.prev, a.at_end) {
            (None, _) => floor,
            (Some(prev), at_end) => {
                let found = (floor..toks.len()).find(|&i| {
                    if is_icon(&toks[i]) {
                        return false;
                    }
                    let n = ar_norm(&toks[i].0);
                    match verb_stems(prev) {
                        Some(stems) => stems.iter().any(|s| n.starts_with(&ar_norm(s)) || ar_norm(&toks[i].0).starts_with(s)),
                        None => lookup.get(prev).is_some_and(|ws| ws.iter().any(|w| close_enough(&n, w))),
                    }
                });
                match found {
                    Some(i) => i + 1,
                    None if at_end => toks.len(),
                    // a button verb we couldn't find: Arabic puts the verb first
                    None if verb_stems(prev).is_some() => (floor + 1).min(toks.len()),
                    None => toks.len(),
                }
            }
        };
        toks.insert(idx, icon_seg(png));
        floor = idx + 1;
    }
    // back to segments: same-colour words share a run, icons stand alone, spaces stay uncoloured
    let mut out: Segments = Vec::new();
    for (i, (w, c)) in toks.into_iter().enumerate() {
        if i > 0 {
            out.push((" ".into(), None));
        }
        let icon = c.as_deref().is_some_and(|c| c.starts_with("icon:"));
        match out.iter_mut().rev().nth(1) {
            Some(prev) if !icon && prev.1 == c && !is_icon(prev) && i > 0 => {
                out.pop(); // drop the space we just pushed
                let last = out.last_mut().unwrap();
                last.0.push(' ');
                last.0.push_str(&w);
            }
            _ => out.push((w, c)),
        }
    }
    out
}

/// Claude's reply keeps icons as "<bN/>" tags: turn them into icon segments.
fn split_icon_tags(segs: Segments, icons: &[String]) -> Segments {
    let mut out: Segments = Vec::new();
    let mut used = vec![false; icons.len()];
    for (text, color) in segs {
        let mut rest = text.as_str();
        while let Some(start) = rest.find("<b") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("/>") else { break };
            let Ok(n) = after[..end].trim().parse::<usize>() else {
                out.push((rest[..start + 2].to_string(), color.clone()));
                rest = after;
                continue;
            };
            if !rest[..start].is_empty() {
                out.push((rest[..start].to_string(), color.clone()));
            }
            if let Some(png) = icons.get(n.wrapping_sub(1)) {
                out.push(icon_seg(png));
                used[n - 1] = true;
            }
            rest = &after[end + 2..];
        }
        if !rest.is_empty() {
            out.push((rest.to_string(), color));
        }
    }
    for (i, png) in icons.iter().enumerate() {
        if !used[i] {
            out.push((" ".into(), None));
            out.push(icon_seg(png));
        }
    }
    out
}

/// Spoken forms translators don't know, in their plain form ("C'mon" -> "Come on").
pub fn expand_slang(line: &str) -> String {
    let mut out = line.to_string();
    for (from, to) in [("C'mon", "Come on"), ("c'mon", "come on"), ("C\u{2019}mon", "Come on"), ("c\u{2019}mon", "come on")] {
        out = out.replace(from, to);
    }
    out
}

/// Terms kept exactly as the game writes them: GTA+ / GTA$ amounts / GTA Online / GTAV, web
/// addresses, and ™ names. Returns the line with "[1]", "[2]" … in their place, and the terms.
pub fn protect_terms(line: &str) -> (String, Vec<String>) {
    let mut terms: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let words: Vec<&str> = line.split(' ').collect();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let core = w.trim_end_matches(|c: char| ",.;:!?)".contains(c));
        let tail = &w[core.len()..];
        let up = core.to_uppercase();
        let is_term = up.starts_with("GTA+")
            || up.starts_with("GTA$")
            || up == "GTAV"
            || core.contains('™')
            || (core.contains('.') && core.contains('/'))
            || core.ends_with(".com");
        // "GTA Online" / "GTA V" are two words
        let two = up == "GTA" && words.get(i + 1).is_some_and(|n| matches!(n.trim_end_matches(|c: char| ",.;:!?".contains(c)), "Online" | "V"));
        if two {
            let n = words[i + 1];
            let ncore = n.trim_end_matches(|c: char| ",.;:!?".contains(c));
            terms.push(format!("{core} {ncore}"));
            out.push(format!("[{}]{}", terms.len(), &n[ncore.len()..]));
            i += 2;
            continue;
        }
        if is_term {
            // OCR reads the zeros of "GTA$500,000" as letter O
            let core = if up.starts_with("GTA$") { format!("GTA${}", core[4..].replace(['O', 'o'], "0")) } else { core.to_string() };
            terms.push(core);
            out.push(format!("[{}]{tail}", terms.len()));
        } else {
            out.push(w.to_string());
        }
        i += 1;
    }
    (out.join(" "), terms)
}

#[cfg(test)]
mod term_tests {
    use super::protect_terms as p;
    #[test]
    fn game_terms_are_kept() {
        let (t, terms) = p("In addition to the monthly GTA$500,000 deposited, GTA+ Members who've signed up get GTA Online cars. Visit rsg.ms/gtaplus for more.");
        assert_eq!(t, "In addition to the monthly [1] deposited, [2] Members who've signed up get [3] cars. Visit [4] for more.");
        assert_eq!(terms, vec!["GTA$500,000", "GTA+", "GTA Online", "rsg.ms/gtaplus"]);
        assert_eq!(p("Go to the guard.").1.len(), 0);
    }
}

/// Does this Arabic keep every term of the English that must stay as is (GTA+, GTA$ …)? An old
/// translation made before they were protected may have mangled them ("م م +").
pub fn keeps_terms(en: &str, ar: &str) -> bool {
    let ar = ar.replace(['\u{2066}', '\u{2069}'], "");
    protect_terms(en).1.iter().all(|t| ar.contains(t.as_str()))
}
