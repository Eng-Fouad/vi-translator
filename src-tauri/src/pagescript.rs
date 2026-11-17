//! Game pages (Mission Failed, captions, …) recognised by scripts, so a new game's pages can come
//! from the server instead of an app update.
//!
//! A script is JavaScript, run by an embedded QuickJS in a sandbox: no files, no network, nothing of
//! the PC — only the frame it's given, through a few fast helpers the app provides (the pixel work
//! runs here in Rust; a script only decides). Each call has a time limit and the engine a memory cap.
//!
//! The script registers its pages:
//!
//! ```js
//! pages.add({
//!   id: "gta5_caption",            // the page's id (the overlay draws it by its layout)
//!   option: "captions",            // the profile's switch for it ("" = always on)
//!   present(f) { … },              // cheap: is it on screen? → anything (passed to read) or null
//!   read(f, found) { … },          // the page's layout (see screens::ScreenLayout) or null
//!   same(f, old) { … },            // while it's up: still the same page? true / false
//! });
//! ```
//!
//! A page's `option` is one of the app's switches ("mission_failed", "pause_menu", "captions") or one
//! the script declares, which then shows in Game pages with its name and explanation (text, or text
//! by interface language, `en` the fallback):
//!
//! ```js
//! pages.option({ id: "heists", name: { en: "Heist board", ar: "لوحة السرقات" }, about: "…", on: true });
//! ```
//!
//! and gets, in each call, a frame `f` with: `f.w`, `f.h` (pixels), and
//!  - `f.pixels(rect, {step, bright: [min, max], spread: [min, max], near: [r, g, b], tol})` →
//!    `{count, box}`: how many sampled pixels match (bright = brightest channel, spread = brightest −
//!    darkest; near/tol: within tol of a colour) and the box around them (or null);
//!  - `f.ocr(rect, {scale})` → `{text, words: [{text, x, y, w, h}]}` (frame pixels);
//!  - `f.box(x, y, w, h)` → the same rectangle as fractions of the frame, for layouts.
//!
//! Scripts come from the server inside the game's layout update, signed with the app's update key
//! (they're code: an unsigned or changed one never runs). The app also carries its own copy of each
//! (`pages/<game>.js`); the newer one wins, by its `// pages-version: N` first line.

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow};
use rquickjs::{Context, Function, Runtime};
use serde::Deserialize;

use crate::capture::FrameData;
use crate::ocr::{self, Crop, Ocr};
use crate::screens::ScreenLayout;

/// The scripts the app carries, by game.
fn bundled(game: &str) -> Option<&'static str> {
    match game {
        "gta5" => Some(include_str!("../pages/gta5.js")),
        _ => None,
    }
}

/// Bumped when a layout update saves a new script: the detection loop loads it again.
pub static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Is a page's switch on in the profile ("" or an unknown name: always on)?
pub fn option_on(opts: &crate::config::ProfileOpts, option: &str) -> bool {
    match option {
        "mission_failed" => opts.mission_failed,
        "pause_menu" => opts.pause_menu,
        "captions" => opts.captions,
        _ => true,
    }
}

/// The switch of a page the app reads itself (when no script has it).
pub fn option_of(id: &str) -> &'static str {
    match id {
        crate::screens::PAUSE_ID => "pause_menu",
        crate::screens::CAPTION_ID => "captions",
        crate::screens::ALERT_ID | "gta5_mission_failed" => "mission_failed",
        _ => "",
    }
}

/// A layout update may carry a newer pages script: `"pages": {"script": "...", "sig": "<base64>"}`.
/// Saved (and loaded by the detection loop) only if it is signed with the app's key.
pub fn save_from_layout(data_dir: &Path, game: &str, layout: &serde_json::Value) {
    let (Some(script), Some(sig)) = (layout["pages"]["script"].as_str(), layout["pages"]["sig"].as_str()) else { return };
    if !signed(script, sig) {
        eprintln!("[pages] the {game} script in the layout update is not signed by VI Translator: ignored");
        return;
    }
    let (js, sg) = files(data_dir, game);
    if std::fs::read_to_string(&js).is_ok_and(|old| old == script) {
        return;
    }
    if std::fs::write(&js, script).is_ok() && std::fs::write(&sg, sig).is_ok() {
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Longest a single call may run (a script that loops forever is stopped).
const CALL_LIMIT_MS: u64 = 250;

/// A page a script recognises.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PageInfo {
    pub id: String,
    /// the profile's switch for it: "mission_failed" | "pause_menu" | "captions" | "" (always on)
    pub option: String,
    /// While it's up (`hold` in the script): keep showing it this long after `same` stops seeing it
    /// (a title that blinks out while the scene fades); 0 = gone at once.
    pub hold_ms: u64,
    /// …but end it straight away once the screen has gone dark.
    pub end_when_dark: bool,
    /// Read it again this often while it's up (new words: a new reason, Yes / No buttons); 0 = never.
    /// The measured places are kept (the translation doesn't jump), only the words change.
    pub reread_ms: u64,
}

/// A switch a script declares for its pages (shown in Game pages).
#[derive(Deserialize, serde::Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct OptionInfo {
    pub id: String,
    /// text, or `{"en": …, "ar": …}`
    pub name: serde_json::Value,
    pub about: serde_json::Value,
    /// on unless the profile says otherwise
    pub on: bool,
}

pub struct Pages {
    _rt: Runtime,
    ctx: Context,
    deadline: Arc<AtomicU64>,
    started: Instant,
    pub list: Vec<PageInfo>,
    /// the switches it declares
    pub options: Vec<OptionInfo>,
    /// where the script came from and its version, for the log
    pub origin: String,
}

thread_local! {
    /// The frame (and OCR) of the call in progress, for the helpers.
    static CURRENT: RefCell<Option<(*const FrameData, Option<*const Ocr>)>> = const { RefCell::new(None) };
    /// Time spent in the app's helpers during the call in progress (ms): it doesn't count against the
    /// script's time limit, which is there to stop runaway script code, not slow OCR.
    static NATIVE_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `// pages-version: N` on the first line (0 if none).
pub fn version_of(script: &str) -> u32 {
    script.lines().next().and_then(|l| l.trim().strip_prefix("// pages-version:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// The script file a layout update saved for a game, and its signature.
pub fn files(data_dir: &Path, game: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    (data_dir.join(format!("pages-{game}.js")), data_dir.join(format!("pages-{game}.js.sig")))
}

/// A downloaded script, if its signature is the app's.
fn downloaded(data_dir: &Path, game: &str) -> Option<String> {
    let (js, sig) = files(data_dir, game);
    let script = std::fs::read_to_string(js).ok()?;
    let sig = std::fs::read_to_string(sig).ok()?;
    crate::updater::verify(script.as_bytes(), sig.trim(), crate::updater::PUBLIC_KEY).ok()?;
    Some(script)
}

/// Check a script's signature (for a layout update, before it's saved).
pub fn signed(script: &str, sig: &str) -> bool {
    crate::updater::verify(script.as_bytes(), sig.trim(), crate::updater::PUBLIC_KEY).is_ok()
}

impl Pages {
    /// The pages script for a game: the downloaded one if it's signed and newer than the app's own.
    pub fn for_game(game: &str, data_dir: &Path) -> Option<Pages> {
        let own = bundled(game);
        let got = downloaded(data_dir, game);
        let (script, origin) = match (own, got) {
            (Some(o), Some(d)) if version_of(&d) > version_of(o) => (d, "downloaded"),
            (Some(o), _) => (o.to_string(), "built in"),
            (None, Some(d)) => (d, "downloaded"),
            (None, None) => return None,
        };
        match Pages::load(&script) {
            Ok(mut p) => {
                p.origin = format!("{origin}, version {}", version_of(&script));
                Some(p)
            }
            Err(e) => {
                eprintln!("[pages] {game} script ({origin}) failed to load: {e:#}");
                // a broken download must not take the app's own pages away
                (origin == "downloaded").then(|| own.and_then(|o| Pages::load(o).ok())).flatten()
            }
        }
    }

    /// Start the engine with a script.
    pub fn load(script: &str) -> Result<Pages> {
        let rt = Runtime::new()?;
        rt.set_memory_limit(32 * 1024 * 1024);
        rt.set_max_stack_size(512 * 1024);
        let started = Instant::now();
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let d = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            let script_ms = (started.elapsed().as_millis() as u64).saturating_sub(NATIVE_MS.with(|n| n.get()));
            script_ms > d.load(Ordering::Relaxed)
        })));
        let ctx = Context::full(&rt)?;
        let pages = Pages { _rt: rt, ctx, deadline, started, list: Vec::new(), options: Vec::new(), origin: String::new() };
        pages.arm();
        let list = pages.ctx.with(|ctx| -> Result<(String, String)> {
            let g = ctx.globals();
            g.set("__w", Function::new(ctx.clone(), || with_frame(|f, _| f.w).unwrap_or(0))?)?;
            g.set("__h", Function::new(ctx.clone(), || with_frame(|f, _| f.h).unwrap_or(0))?)?;
            g.set("__pixels", Function::new(ctx.clone(), |rect: String, opts: String| helper_pixels(&rect, &opts))?)?;
            g.set("__ocr", Function::new(ctx.clone(), |rect: String, opts: String| helper_ocr(&rect, &opts))?)?;
            g.set("__profile", Function::new(ctx.clone(), |rect: String, opts: String| helper_profile(&rect, &opts))?)?;
            g.set("__run", Function::new(ctx.clone(), |rect: String, opts: String| helper_run(&rect, &opts))?)?;
            g.set("__mean", Function::new(ctx.clone(), |rect: String, opts: String| helper_mean(&rect, &opts))?)?;
            g.set("__blobs", Function::new(ctx.clone(), |rect: String, opts: String| helper_blobs(&rect, &opts))?)?;
            g.set("__icon", Function::new(ctx.clone(), |rect: String, thr: f64| helper_icon(&rect, thr))?)?;
            g.set("__letters", Function::new(ctx.clone(), |s: String| crate::screens::letters(&s))?)?;
            g.set("__similar", Function::new(ctx.clone(), |a: String, b: String| ocr::similar(&a, &b))?)?;
            g.set("__button_word", Function::new(ctx.clone(), |read: String| helper_button_word(&read))?)?;
            g.set("__runs", Function::new(ctx.clone(), |rect: String, opts: String| helper_runs(&rect, &opts))?)?;
            g.set("__median", Function::new(ctx.clone(), |rect: String, opts: String| helper_median(&rect, &opts))?)?;
            g.set("__contrast", Function::new(ctx.clone(), |rect: String, against: String, share: f64| helper_contrast(&rect, &against, share))?)?;
            g.set("__letter_height", Function::new(ctx.clone(), |rect: String| helper_letter_height(&rect))?)?;
            g.set("__tab_word", Function::new(ctx.clone(), |read: String| {
                crate::screens::tab_words().iter().find(|(e, _)| *e == read).map(|(e, a)| serde_json::json!([e, a]).to_string()).unwrap_or_else(|| "null".into())
            })?)?;
            g.set("__menu_item_word", Function::new(ctx.clone(), |read: String| {
                crate::screens::menu_item_word(&read).map(|(e, a)| serde_json::json!([e, a]).to_string()).unwrap_or_else(|| "null".into())
            })?)?;
            g.set("__fix_contractions", Function::new(ctx.clone(), |s: String| ocr::fix_contractions(&s))?)?;
            ctx.eval::<(), _>(PRELUDE).map_err(|e| js_error(&ctx, e))?;
            ctx.eval::<(), _>(script).map_err(|e| js_error(&ctx, e))?;
            let list: String = ctx.eval("__list()").map_err(|e| js_error(&ctx, e))?;
            let options: String = ctx.eval("__option_list()").map_err(|e| js_error(&ctx, e))?;
            Ok((list, options))
        })?;
        pages.disarm();
        let mut pages = pages;
        let (list, options) = list;
        pages.list = serde_json::from_str(&list).context("the page list")?;
        pages.options = serde_json::from_str(&options).context("the switches")?;
        Ok(pages)
    }

    fn arm(&self) {
        NATIVE_MS.with(|n| n.set(0));
        self.deadline.store(self.started.elapsed().as_millis() as u64 + CALL_LIMIT_MS, Ordering::Relaxed);
    }
    fn disarm(&self) {
        self.deadline.store(u64::MAX, Ordering::Relaxed);
    }

    /// Call `page.<func>(frame, arg)`; its result as JSON (null when the page has no such function,
    /// or on any error — a failing script just finds nothing).
    fn call(&self, id: &str, func: &str, frame: &FrameData, ocr: Option<&Ocr>, arg: &serde_json::Value) -> serde_json::Value {
        CURRENT.with(|c| *c.borrow_mut() = Some((frame as *const _, ocr.map(|o| o as *const _))));
        self.arm();
        let out = self.ctx.with(|ctx| -> Result<String> {
            let call: Function = ctx.globals().get("__call")?;
            call.call((id, func, arg.to_string())).map_err(|e| js_error(&ctx, e))
        });
        self.disarm();
        CURRENT.with(|c| *c.borrow_mut() = None);
        match out {
            Ok(json) => serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
            Err(e) => {
                if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                    eprintln!("[pages] {id}.{func}: {e:#}");
                }
                serde_json::Value::Null
            }
        }
    }

    /// Is the page on screen? Whatever `present` found (handed on to `read`), or None.
    pub fn present(&self, id: &str, frame: &FrameData, ocr: &Ocr) -> Option<serde_json::Value> {
        Some(self.call(id, "present", frame, Some(ocr), &serde_json::Value::Null)).filter(|v| !v.is_null() && v != &serde_json::Value::Bool(false))
    }

    /// Read the page into a layout the overlay draws.
    pub fn read(&self, id: &str, frame: &FrameData, ocr: &Ocr, found: &serde_json::Value) -> Option<ScreenLayout> {
        let v = self.call(id, "read", frame, Some(ocr), found);
        if v.is_null() {
            return None;
        }
        let mut l: ScreenLayout = serde_json::from_value(v).ok()?;
        l.id = id.to_string();
        crate::screens::take_sigs(frame, &mut l);
        Some(l)
    }

    /// Read a page that's up again, quickly (`reread(f, {old, unchanged})` in the script): None if
    /// the script has no `reread` or found nothing.
    pub fn reread(&self, id: &str, frame: &FrameData, ocr: &Ocr, old: &ScreenLayout) -> Option<ScreenLayout> {
        let arg = serde_json::json!({ "old": old, "unchanged": crate::screens::unchanged(frame, old) });
        let v = self.call(id, "reread", frame, Some(ocr), &arg);
        if v.is_null() {
            return None;
        }
        let mut l: ScreenLayout = serde_json::from_value(v).ok()?;
        l.id = id.to_string();
        crate::screens::take_sigs(frame, &mut l);
        Some(l)
    }

    pub fn can(&self, id: &str, func: &str) -> bool {
        self.has(id)
            && self.ctx.with(|ctx| ctx.eval::<bool, _>(format!("(() => {{ const p = __pages.get({}); return !!p && typeof p[{}] === 'function'; }})()", serde_json::json!(id), serde_json::json!(func))).unwrap_or(false))
    }

    /// While the page is up: is it still the same page? None if the script doesn't say.
    pub fn same(&self, id: &str, frame: &FrameData, ocr: &Ocr, old: &ScreenLayout) -> Option<bool> {
        self.call(id, "same", frame, Some(ocr), &serde_json::to_value(old).ok()?).as_bool()
    }

    pub fn has(&self, id: &str) -> bool {
        self.list.iter().any(|p| p.id == id)
    }

    pub fn info(&self, id: &str) -> Option<&PageInfo> {
        self.list.iter().find(|p| p.id == id)
    }

    /// Is a switch on in the profile? The app's own switches as the profile has them; one the script
    /// declares as the profile set it, else the script's default.
    pub fn switch_on(&self, opts: &crate::config::ProfileOpts, option: &str) -> bool {
        match option {
            "mission_failed" | "pause_menu" | "captions" | "" => option_on(opts, option),
            _ => opts.more.get(option).copied().unwrap_or_else(|| self.options.iter().find(|o| o.id == option).is_none_or(|o| o.on)),
        }
    }
}

/// The JavaScript side of the helpers and the page registry.
const PRELUDE: &str = r#"
const __pages = new Map();
const __options = new Map();
globalThis.pages = Object.freeze({
  add(p) {
    if (!p || typeof p.id !== "string" || !p.id) throw new Error("a page needs an id");
    __pages.set(p.id, p);
  },
  option(o) {
    if (!o || typeof o.id !== "string" || !/^[a-z0-9_]{1,32}$/.test(o.id)) throw new Error("a switch needs an id (a-z, 0-9, _)");
    if (["mission_failed", "pause_menu", "captions"].includes(o.id)) throw new Error("that switch is the app's");
    const text = (v) => (typeof v === "string" ? v : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).filter(([, t]) => typeof t === "string")) : "");
    __options.set(o.id, { id: o.id, name: text(o.name) || o.id, about: text(o.about), on: o.on !== false });
  },
});
function __frame() {
  const w = __w(), h = __h();
  return Object.freeze({
    w, h,
    pixels: (r, o) => JSON.parse(__pixels(JSON.stringify(r), JSON.stringify(o || {}))),
    profile: (r, o) => JSON.parse(__profile(JSON.stringify(r), JSON.stringify(o || {}))),
    run: (r, o) => __run(JSON.stringify(r), JSON.stringify(o || {})),
    mean: (r, o) => __mean(JSON.stringify(r), JSON.stringify(o || {})),
    blobs: (r, o) => JSON.parse(__blobs(JSON.stringify(r), JSON.stringify(o || {}))),
    ocr: (r, o) => JSON.parse(__ocr(JSON.stringify(r), JSON.stringify(o || {}))),
    icon: (r, threshold) => __icon(JSON.stringify(r), threshold ?? 140) || null,
    runs: (r, o) => JSON.parse(__runs(JSON.stringify(r), JSON.stringify(o || {}))),
    median: (r, o) => JSON.parse(__median(JSON.stringify(r), JSON.stringify(o || {}))),
    contrast: (r, against, share) => JSON.parse(__contrast(JSON.stringify(r), JSON.stringify(against), share ?? 0.125)),
    letterHeight: (r) => __letter_height(JSON.stringify(r)),
    box: (x, y, bw, bh) => ({ x: x / w, y: y / h, w: bw / w, h: bh / h }),
  });
}
// text and the game's word lists (built in, plus layout updates)
globalThis.vi = Object.freeze({
  letters: (s) => __letters(String(s)),
  similar: (a, b) => __similar(String(a), String(b)),
  buttonWord: (read) => JSON.parse(__button_word(String(read))),
  tabWord: (read) => JSON.parse(__tab_word(String(read))),
  menuItemWord: (read) => JSON.parse(__menu_item_word(String(read))),
  fixContractions: (s) => __fix_contractions(String(s)),
});
globalThis.__list = () => JSON.stringify([...__pages.values()].map((p) => ({
  id: p.id, option: p.option || "",
  hold_ms: (p.hold && p.hold.ms) || 0, end_when_dark: !!(p.hold && p.hold.endWhenDark), reread_ms: (p.hold && p.hold.rereadMs) || 0,
})));
globalThis.__option_list = () => JSON.stringify([...__options.values()]);
globalThis.__call = (id, fn, arg) => {
  const p = __pages.get(id);
  if (!p || typeof p[fn] !== "function") return "null";
  const r = p[fn](__frame(), JSON.parse(arg));
  return JSON.stringify(r === undefined ? null : r);
};
"#;

/// A script error with its message (QuickJS keeps the exception aside).
fn js_error(ctx: &rquickjs::Ctx, e: rquickjs::Error) -> anyhow::Error {
    if let rquickjs::Error::Exception = e {
        let ex = ctx.catch();
        let msg = ex.as_exception().map(|x| format!("{}", x.message().unwrap_or_default())).unwrap_or_else(|| format!("{ex:?}"));
        return anyhow!("script error: {msg}");
    }
    anyhow!("{e}")
}

fn with_frame<R>(f: impl FnOnce(&FrameData, Option<&Ocr>) -> R) -> Option<R> {
    let t = Instant::now();
    let out = CURRENT.with(|c| {
        let cur = *c.borrow();
        // (set only for the duration of a `Pages::call`, which holds both alive)
        cur.map(|(frame, ocr)| unsafe { f(&*frame, ocr.map(|o| &*o)) })
    });
    let spent = t.elapsed().as_millis() as u64;
    NATIVE_MS.with(|n| n.set(n.get() + spent));
    out
}

#[derive(Deserialize)]
struct PxRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// A rectangle in frame pixels, inside the frame: x0, y0, x1, y1 (exclusive).
fn clamp_rect(r: &PxRect, fw: u32, fh: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = r.x.max(0.0).min(fw as f64) as u32;
    let y0 = r.y.max(0.0).min(fh as f64) as u32;
    let x1 = (r.x + r.w).max(0.0).min(fw as f64) as u32;
    let y1 = (r.y + r.h).max(0.0).min(fh as f64) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Which pixels count: every given range must hold. `bright` is the brightest channel, `low` the
/// darkest, `spread` brightest − darkest; `rg`, `rb`, `gb` are channel differences (r − g …);
/// `near` + `tol`: within `tol` of a colour.
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct Rule {
    r: Option<[i32; 2]>,
    g: Option<[i32; 2]>,
    b: Option<[i32; 2]>,
    bright: Option<[i32; 2]>,
    low: Option<[i32; 2]>,
    spread: Option<[i32; 2]>,
    rg: Option<[i32; 2]>,
    rb: Option<[i32; 2]>,
    gb: Option<[i32; 2]>,
    near: Option<[i32; 3]>,
    tol: Option<i32>,
}

impl Rule {
    fn ok(&self, r: i32, g: i32, b: i32) -> bool {
        let inr = |v: i32, range: &Option<[i32; 2]>| range.is_none_or(|[lo, hi]| v >= lo && v <= hi);
        let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
        inr(r, &self.r)
            && inr(g, &self.g)
            && inr(b, &self.b)
            && inr(hi, &self.bright)
            && inr(lo, &self.low)
            && inr(hi - lo, &self.spread)
            && inr(r - g, &self.rg)
            && inr(r - b, &self.rb)
            && inr(g - b, &self.gb)
            && self.near.is_none_or(|[nr, ng, nb]| {
                let t = self.tol.unwrap_or(24);
                (r - nr).abs() <= t && (g - ng).abs() <= t && (b - nb).abs() <= t
            })
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PxOpts {
    step: Option<u32>,
    /// also the median colour of the matching pixels
    median: bool,
    /// profile: "rows" (one count per sampled row) or "cols"
    axis: Option<String>,
    /// blobs: columns this far apart (or less) belong to the same blob
    gap: Option<u32>,
    #[serde(flatten)]
    rule: Rule,
}

fn px(f: &FrameData, x: u32, y: u32) -> (i32, i32, i32) {
    let i = ((y * f.w + x) * 4) as usize;
    (f.rgba[i] as i32, f.rgba[i + 1] as i32, f.rgba[i + 2] as i32)
}

fn parse(rect: &str, opts: &str) -> Option<(PxRect, PxOpts)> {
    Some((serde_json::from_str(rect).ok()?, serde_json::from_str(opts).ok()?))
}

/// `f.pixels(rect, {step, median, ...rule})`: how many sampled pixels match, their box (and median colour).
fn helper_pixels(rect: &str, opts: &str) -> String {
    const NONE: &str = r#"{"count":0,"box":null,"median":null}"#;
    let Some((r, o)) = parse(rect, opts) else { return NONE.into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return NONE.to_string() };
        let step = o.step.unwrap_or(1).clamp(1, 64) as usize;
        let (mut count, mut bx0, mut by0, mut bx1, mut by1) = (0u64, u32::MAX, u32::MAX, 0u32, 0u32);
        let (mut rs, mut gs, mut bs) = (Vec::new(), Vec::new(), Vec::new());
        for y in (y0..y1).step_by(step) {
            for x in (x0..x1).step_by(step) {
                let (pr, pg, pb) = px(f, x, y);
                if o.rule.ok(pr, pg, pb) {
                    count += 1;
                    (bx0, by0, bx1, by1) = (bx0.min(x), by0.min(y), bx1.max(x), by1.max(y));
                    if o.median {
                        rs.push(pr);
                        gs.push(pg);
                        bs.push(pb);
                    }
                }
            }
        }
        let bx = (count > 0).then(|| serde_json::json!({ "x": bx0, "y": by0, "w": bx1 - bx0 + 1, "h": by1 - by0 + 1 }));
        let med = |v: &mut Vec<i32>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        let median = (!rs.is_empty()).then(|| serde_json::json!({ "r": med(&mut rs), "g": med(&mut gs), "b": med(&mut bs) }));
        serde_json::json!({ "count": count, "box": bx, "median": median }).to_string()
    })
    .unwrap_or_else(|| NONE.into())
}

/// `f.profile(rect, {axis: "rows" | "cols", step, ...rule})`: matching pixels per sampled row (or column).
fn helper_profile(rect: &str, opts: &str) -> String {
    let Some((r, o)) = parse(rect, opts) else { return "[]".into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return "[]".to_string() };
        let step = o.step.unwrap_or(1).clamp(1, 64) as usize;
        let cols = o.axis.as_deref() == Some("cols");
        let counts: Vec<u32> = if cols {
            (x0..x1).step_by(step).map(|x| (y0..y1).step_by(step).filter(|&y| { let (a, b, c) = px(f, x, y); o.rule.ok(a, b, c) }).count() as u32).collect()
        } else {
            (y0..y1).step_by(step).map(|y| (x0..x1).step_by(step).filter(|&x| { let (a, b, c) = px(f, x, y); o.rule.ok(a, b, c) }).count() as u32).collect()
        };
        serde_json::to_string(&counts).unwrap_or_else(|_| "[]".into())
    })
    .unwrap_or_else(|| "[]".into())
}

/// `f.run(rect, rule)`: the longest unbroken row of matching pixels, over the rect's rows.
fn helper_run(rect: &str, opts: &str) -> u32 {
    let Some((r, o)) = parse(rect, opts) else { return 0 };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return 0 };
        let mut best = 0u32;
        for y in y0..y1 {
            let mut run = 0u32;
            for x in x0..x1 {
                let (a, b, c) = px(f, x, y);
                if o.rule.ok(a, b, c) {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 0;
                }
            }
        }
        best
    })
    .unwrap_or(0)
}

/// `f.mean(rect, {step})`: the average brightness (brightest channel) of the sampled pixels.
fn helper_mean(rect: &str, opts: &str) -> f64 {
    let Some((r, o)) = parse(rect, opts) else { return 0.0 };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return 0.0 };
        let step = o.step.unwrap_or(1).clamp(1, 64) as usize;
        let (mut sum, mut n) = (0u64, 0u64);
        for y in (y0..y1).step_by(step) {
            for x in (x0..x1).step_by(step) {
                let (a, b, c) = px(f, x, y);
                sum += a.max(b).max(c) as u64;
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { (sum / n) as f64 }
    })
    .unwrap_or(0.0)
}

/// `f.blobs(rect, {gap, ...rule})`: shapes side by side on a line (words, icons): columns holding a
/// matching pixel, grouped when at most `gap` columns apart; each with its own height. Left to right.
fn helper_blobs(rect: &str, opts: &str) -> String {
    let Some((r, o)) = parse(rect, opts) else { return "[]".into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return "[]".to_string() };
        let on = |x: u32, y: u32| { let (a, b, c) = px(f, x, y); o.rule.ok(a, b, c) };
        let max_gap = o.gap.unwrap_or(3);
        let col_on: Vec<bool> = (x0..x1).map(|x| (y0..y1).any(|y| on(x, y))).collect();
        let mut out = Vec::new();
        let close = |s: u32, e: u32, out: &mut Vec<serde_json::Value>| {
            let ys: Vec<u32> = (y0..y1).filter(|&y| (s..=e).any(|x| on(x, y))).collect();
            if let (Some(&t), Some(&b)) = (ys.first(), ys.last()) {
                out.push(serde_json::json!({ "x": s, "y": t, "w": e - s + 1, "h": b - t + 1 }));
            }
        };
        let (mut start, mut end, mut gap): (Option<u32>, u32, u32) = (None, 0, 0);
        for (i, &c) in col_on.iter().enumerate() {
            let x = x0 + i as u32;
            if c {
                start.get_or_insert(x);
                end = x;
                gap = 0;
            } else if let Some(s) = start {
                gap += 1;
                if gap > max_gap {
                    close(s, end, &mut out);
                    start = None;
                }
            }
        }
        if let Some(s) = start {
            close(s, end, &mut out);
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
    })
    .unwrap_or_else(|| "[]".into())
}

/// `f.icon(rect, threshold)`: a picture of a button icon (PNG data URL) for the overlay's replica.
fn helper_icon(rect: &str, thr: f64) -> Option<String> {
    let r: PxRect = serde_json::from_str(rect).ok()?;
    with_frame(|f, _| {
        // a margin around it (the icon cut-out pads by 2), then the icon's box inside that
        let m = 4.0;
        let outer = PxRect { x: r.x - m, y: r.y - m, w: r.w + 2.0 * m, h: r.h + 2.0 * m };
        let (x0, y0, x1, y1) = clamp_rect(&outer, f.w, f.h)?;
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            let s = ((y * f.w + x0) * 4) as usize;
            rgba.extend_from_slice(&f.rgba[s..s + (cw * 4) as usize]);
        }
        let c = Crop { w: cw, h: ch, rgba };
        let inner = ((r.x as u32).saturating_sub(x0), (r.y as u32).saturating_sub(y0), r.w as u32, r.h as u32);
        ocr::icon_png(&c, inner, thr.clamp(0.0, 255.0) as u8)
    })
    .flatten()
}

/// `f.runs(rect, rule)`: for each row, its longest unbroken run of matching pixels: `[start, length]`
/// (start in frame pixels; [0, 0] for a row without one).
fn helper_runs(rect: &str, opts: &str) -> String {
    let Some((r, o)) = parse(rect, opts) else { return "[]".into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return "[]".to_string() };
        let mut out = Vec::with_capacity((y1 - y0) as usize);
        for y in y0..y1 {
            let (mut best, mut best_s, mut run, mut s) = (0u32, 0u32, 0u32, 0u32);
            for x in x0..x1 {
                let (a, b, c) = px(f, x, y);
                if o.rule.ok(a, b, c) {
                    if run == 0 {
                        s = x;
                    }
                    run += 1;
                    if run > best {
                        (best, best_s) = (run, s);
                    }
                } else {
                    run = 0;
                }
            }
            out.push([best_s, best]);
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
    })
    .unwrap_or_else(|| "[]".into())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MedianOpts {
    step_x: Option<u32>,
    step_y: Option<u32>,
    /// pixels inside this rectangle (inclusive edges) are left out
    exclude: Option<[i64; 4]>,
}

/// `f.median(rect, {step_x, step_y, exclude: [x0, y0, x1, y1]})`: the median colour of the sampled
/// pixels (a background around a word, say), or null if none.
fn helper_median(rect: &str, opts: &str) -> String {
    let (Ok(r), Ok(o)) = (serde_json::from_str::<PxRect>(rect), serde_json::from_str::<MedianOpts>(opts)) else { return "null".into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return "null".to_string() };
        let (sx, sy) = (o.step_x.unwrap_or(1).clamp(1, 64) as usize, o.step_y.unwrap_or(1).clamp(1, 64) as usize);
        let (mut rs, mut gs, mut bs) = (Vec::new(), Vec::new(), Vec::new());
        for y in (y0..y1).step_by(sy) {
            for x in (x0..x1).step_by(sx) {
                if let Some([ex0, ey0, ex1, ey1]) = o.exclude {
                    let (xi, yi) = (x as i64, y as i64);
                    if xi >= ex0 && xi <= ex1 && yi >= ey0 && yi <= ey1 {
                        continue;
                    }
                }
                let (a, b, c) = px(f, x, y);
                rs.push(a);
                gs.push(b);
                bs.push(c);
            }
        }
        if rs.is_empty() {
            return "null".to_string();
        }
        let med = |v: &mut Vec<i32>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        serde_json::json!({ "r": med(&mut rs), "g": med(&mut gs), "b": med(&mut bs) }).to_string()
    })
    .unwrap_or_else(|| "null".into())
}

#[derive(Deserialize)]
struct Rgb {
    r: i32,
    g: i32,
    b: i32,
}

/// `f.contrast(rect, background, share)`: the colour of the letters in a box: the median of the
/// pixels least like the background (the top `share` of them).
fn helper_contrast(rect: &str, against: &str, share: f64) -> String {
    let (Ok(r), Ok(bg)) = (serde_json::from_str::<PxRect>(rect), serde_json::from_str::<Rgb>(against)) else { return "null".into() };
    with_frame(|f, _| {
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return "null".to_string() };
        let mut ins: Vec<(i32, i32, i32)> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| px(f, x, y)).collect();
        if ins.is_empty() {
            return "null".to_string();
        }
        let dist = |p: &(i32, i32, i32)| (p.0 - bg.r).abs() + (p.1 - bg.g).abs() + (p.2 - bg.b).abs();
        ins.sort_by_key(|p| std::cmp::Reverse(dist(p)));
        let n = ((ins.len() as f64 * share.clamp(0.0, 1.0)) as usize).max(1);
        let top = &ins[..n.min(ins.len())];
        let med = |mut v: Vec<i32>| {
            v.sort_unstable();
            v.get(v.len() / 2).copied().unwrap_or(0)
        };
        serde_json::json!({
            "r": med(top.iter().map(|p| p.0).collect()),
            "g": med(top.iter().map(|p| p.1).collect()),
            "b": med(top.iter().map(|p| p.2).collect()),
        })
        .to_string()
    })
    .unwrap_or_else(|| "null".into())
}

/// `f.letterHeight(rect)`: the height of the bright letters inside a box (pixels).
fn helper_letter_height(rect: &str) -> u32 {
    let Ok(r) = serde_json::from_str::<PxRect>(rect) else { return 0 };
    with_frame(|f, _| {
        let (x0, y0) = (r.x.max(0.0) as u32, r.y.max(0.0) as u32);
        let (x1, y1) = ((r.x + r.w - 1.0).max(0.0) as u32, (r.y + r.h - 1.0).max(0.0) as u32);
        crate::screens::letter_height(f, x0, y0, x1, y1)
    })
    .unwrap_or(0)
}

/// `vi.buttonWord(read)`: the game's button word read here ([english, translation]) or null.
fn helper_button_word(read: &str) -> String {
    let read = crate::screens::letters(read);
    crate::screens::button_words()
        .iter()
        .find(|(en, _)| read == crate::screens::letters(en) || (read.len() >= 4 && ocr::similar(&read, &crate::screens::letters(en))))
        .map(|(en, ar)| serde_json::json!([en, ar]).to_string())
        .unwrap_or_else(|| "null".into())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OcrOpts {
    /// read the crop enlarged this many times (small text reads better)
    scale: Option<u32>,
    /// or shrunk to at most this many pixels tall (huge title letters read better at text size)
    shrink: Option<u32>,
    /// put the crop on a black canvas with this margin first (a word OCR skips next to an icon)
    isolate: Option<u32>,
    /// resizing filter: "catmullrom" (default), "lanczos", "triangle"
    filter: Option<String>,
    /// "brightest": OCR's preparation for light text on mixed backgrounds (else the default)
    prep: Option<String>,
    /// faint text: each channel becomes (v − 6) × boost before reading
    boost: Option<i32>,
    /// words exactly as OCR saw them (in the enlarged picture), with the crop's origin `x0`, `y0`
    /// and the enlargement `k`: for arithmetic that must divide sums, not parts
    raw: bool,
}

/// `f.ocr(rect, opts)`: the text in a rectangle, cleaned, and its words (frame pixels).
fn helper_ocr(rect: &str, opts: &str) -> String {
    const NONE: &str = r#"{"text":"","words":[]}"#;
    let (Ok(r), Ok(o)) = (serde_json::from_str::<PxRect>(rect), serde_json::from_str::<OcrOpts>(opts)) else { return NONE.into() };
    with_frame(|f, ocr| {
        let Some(ocr) = ocr else { return NONE.to_string() };
        let Some((x0, y0, x1, y1)) = clamp_rect(&r, f.w, f.h) else { return NONE.to_string() };
        let (cw, ch) = (x1 - x0, y1 - y0);
        if cw < 8 || ch < 8 {
            return NONE.to_string();
        }
        let mut rgba = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            let s = ((y * f.w + x0) * 4) as usize;
            rgba.extend_from_slice(&f.rgba[s..s + (cw * 4) as usize]);
        }
        let filter = match o.filter.as_deref() {
            Some("lanczos") => image::imageops::FilterType::Lanczos3,
            Some("triangle") => image::imageops::FilterType::Triangle,
            _ => image::imageops::FilterType::CatmullRom,
        };
        let mut img = image::RgbaImage::from_raw(cw, ch, rgba).expect("crop size");
        if let Some(boost) = o.boost.filter(|&b| b > 0) {
            for p in img.pixels_mut() {
                for ch in 0..3 {
                    p.0[ch] = ((p.0[ch] as i32 - 6).max(0) * boost).min(255) as u8;
                }
            }
        }
        // on black, with a margin (words are then reported relative to the crop: the margin comes off)
        let pad = o.isolate.unwrap_or(0).min(200);
        if pad > 0 {
            let mut canvas = image::RgbaImage::from_pixel(cw + 2 * pad, ch + 2 * pad, image::Rgba([0, 0, 0, 255]));
            image::imageops::overlay(&mut canvas, &img, pad as i64, pad as i64);
            img = canvas;
        }
        let (iw, ih) = img.dimensions();
        // scale up (k) or down (to `shrink` pixels tall); positions are mapped back below
        let (nw, nh) = if let Some(max_h) = o.shrink.filter(|&m| ih > m && m >= 8) {
            (((iw as u64 * max_h as u64) / ih as u64).max(8) as u32, max_h)
        } else {
            let k = o.scale.unwrap_or(1).clamp(1, 4);
            (iw * k, ih * k)
        };
        let img = if (nw, nh) == (iw, ih) {
            img
        } else {
            image::imageops::resize(&img, nw, nh, if o.shrink.is_some() { image::imageops::FilterType::Triangle } else { filter })
        };
        let crop = Crop { w: nw, h: nh, rgba: img.into_raw() };
        let read = if o.prep.as_deref() == Some("brightest") { ocr.recognize_with(&crop, ocr::Prep::Brightest) } else { ocr.recognize(&crop) };
        let Ok(mut words) = read else { return NONE.to_string() };
        let text = ocr::clean(&words);
        if o.raw {
            let ws: Vec<serde_json::Value> =
                words.iter().map(|w| serde_json::json!({ "text": w.text, "x": w.rect.0, "y": w.rect.1, "w": w.rect.2, "h": w.rect.3, "line": w.line })).collect();
            return serde_json::json!({ "text": text, "words": ws, "x0": x0, "y0": y0, "k": nw / iw.max(1), "pad": pad }).to_string();
        }
        // back to the crop's pixels: whole-number enlargements divide exactly; a shrink scales
        let k = nw / iw.max(1);
        for w in words.iter_mut() {
            let (x, y, ww, wh) = if nw >= iw && k >= 1 && nw == iw * k {
                (w.rect.0 / k, w.rect.1 / k, w.rect.2 / k, w.rect.3 / k)
            } else {
                let (sx, sy) = (iw as f64 / nw as f64, ih as f64 / nh as f64);
                ((w.rect.0 as f64 * sx) as u32, (w.rect.1 as f64 * sy) as u32, (w.rect.2 as f64 * sx) as u32, (w.rect.3 as f64 * sy) as u32)
            };
            w.rect = (x.saturating_sub(pad), y.saturating_sub(pad), ww, wh);
        }
        let ws: Vec<serde_json::Value> = words
            .iter()
            .map(|w| serde_json::json!({ "text": w.text, "x": x0 + w.rect.0, "y": y0 + w.rect.1, "w": w.rect.2, "h": w.rect.3, "line": w.line }))
            .collect();
        serde_json::json!({ "text": text, "words": ws }).to_string()
    })
    .unwrap_or_else(|| NONE.into())
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A black 1920×1080 frame with a white caption-like block of "letters" in the middle.
    fn caption_frame() -> FrameData {
        let (w, h) = (1920u32, 1080u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for i in (3..rgba.len()).step_by(4) {
            rgba[i] = 255;
        }
        // 40 "letters": 3×24 white strokes with gaps, centred, at mid-height
        for k in 0..40u32 {
            let x0 = 960 - 200 + k * 10;
            for y in 528..552u32 {
                for x in x0..x0 + 3 {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 3].copy_from_slice(&[235, 235, 235]);
                }
            }
        }
        FrameData { w, h, rgba, seq: 1 }
    }

    #[test]
    fn the_built_in_script_loads_and_finds_a_caption_like_the_rust_code() {
        let p = Pages::load(bundled("gta5").unwrap()).expect("loads");
        assert!(p.has("gta5_caption"));
        assert_eq!(p.list[0].option, "captions");
        let f = caption_frame();
        let rust = crate::screens::find_caption(&f).expect("rust finds it");
        let js = p.call("gta5_caption", "present", &f, None, &serde_json::Value::Null);
        assert_eq!((js["x0"].as_u64(), js["y0"].as_u64(), js["x1"].as_u64(), js["y1"].as_u64()),
                   (Some(rust.0 as u64), Some(rust.1 as u64), Some(rust.2 as u64), Some(rust.3 as u64)));
        // a black frame: nothing, in both
        let black = FrameData { w: 1920, h: 1080, rgba: vec![0; 1920 * 1080 * 4], seq: 2 };
        assert!(crate::screens::find_caption(&black).is_none());
        assert!(p.call("gta5_caption", "present", &black, None, &serde_json::Value::Null).is_null());
    }

    #[test]
    fn a_script_that_never_ends_is_stopped() {
        let p = Pages::load(r#"pages.add({ id: "loop", present(f) { for (;;) {} } });"#).expect("loads");
        let f = caption_frame();
        let t = Instant::now();
        assert!(p.call("loop", "present", &f, None, &serde_json::Value::Null).is_null());
        assert!(t.elapsed().as_millis() < 2000, "stopped after {} ms", t.elapsed().as_millis());
        // and the engine still works afterwards
        assert!(p.call("loop", "nothing", &f, None, &serde_json::Value::Null).is_null());
    }

    #[test]
    fn time_in_the_apps_helpers_doesnt_count_against_the_script() {
        // ~40 full-frame pixel passes: well over the limit in total, but all of it in the helpers
        let p = Pages::load(r#"pages.add({ id: "busy", present(f) {
            let n = 0;
            for (let i = 0; i < 40; i++) n += f.pixels({ x: 0, y: 0, w: f.w, h: f.h }, { bright: [200, 255] }).count;
            return n;
        } });"#).expect("loads");
        let f = caption_frame();
        let t = Instant::now();
        let v = p.call("busy", "present", &f, None, &serde_json::Value::Null);
        assert!(t.elapsed().as_millis() > CALL_LIMIT_MS as u128 || !v.is_null(), "the test should exceed the limit in helpers");
        assert!(!v.is_null(), "stopped although only the helpers were slow");
    }

    #[test]
    fn a_script_gets_nothing_of_the_pc() {
        let p = Pages::load(
            r#"pages.add({ id: "probe", present(f) {
                 return [typeof require, typeof fetch, typeof XMLHttpRequest, typeof std, typeof os, typeof process, typeof Deno].join(",");
               } });"#,
        )
        .expect("loads");
        let v = p.call("probe", "present", &caption_frame(), None, &serde_json::Value::Null);
        assert_eq!(v, "undefined,undefined,undefined,undefined,undefined,undefined,undefined");
    }

    #[test]
    fn a_broken_script_is_an_error_not_a_crash() {
        assert!(Pages::load("this is not javascript (").is_err());
        assert!(Pages::load(r#"pages.add({});"#).is_err(), "a page needs an id");
        assert!(Pages::load(r#"pages.option({ id: "pause_menu" });"#).is_err(), "the app's switches are the app's");
        let p = Pages::load(r#"
            pages.option({ id: "heists", name: { en: "Heist board", ar: "لوحة" }, about: "Plans", on: false });
            pages.option({ id: "camp", name: "Camp" });
            pages.add({ id: "board", option: "heists" });
            pages.add({ id: "fire", option: "camp" });"#).expect("loads");
        assert_eq!(p.options.len(), 2);
        assert_eq!(p.options[0].name["ar"], "لوحة");
        let mut o = crate::config::ProfileOpts::default();
        assert!(!p.switch_on(&o, "heists") && p.switch_on(&o, "camp") && p.switch_on(&o, "unknown"));
        o.more.insert("heists".into(), true);
        o.pause_menu = false;
        assert!(p.switch_on(&o, "heists") && !p.switch_on(&o, "pause_menu"));
        assert_eq!(version_of("// pages-version: 7\npages.add({id:'x'})"), 7);
    }

    #[test]
    fn only_a_signed_script_counts() {
        assert!(!signed("pages.add({id:'x'})", "AAAA"));
    }
}
