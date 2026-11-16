//! Profile "screens": whole game pages that get their own hand-made Arabic layout instead of
//! plain subtitles. First one: GTA V's "MISSION FAILED" page.
//!
//! Detection is two-step: a cheap pixel check every frame (big red title on a dark screen),
//! then OCR to confirm and to locate the reason line and the Skip / Restart / Retry buttons.
//! Every position is reported as a fraction of the frame, so the overlay can place the Arabic
//! exactly relative to the English at any window size.

use serde::Serialize;

use crate::capture::FrameData;
use crate::ocr::{self, Crop, Ocr};

/// Rectangle as fractions (0..1) of the frame.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Button {
    pub en: String,
    pub ar: String,
    /// The game's own button icon, cut out of the frame (PNG data URL), so the replica is exact.
    pub icon: Option<String>,
    pub icon_rect: Option<Rect>,
    /// The English word's box (its height sets the Arabic font size).
    pub text_rect: Rect,
    /// More icons after the first ("Browse [L1] [R1]").
    pub more_icons: Vec<String>,
}

/// A piece of the game's UI covered by an Arabic replica (e.g. a pause-menu tab): same box, same
/// background colour, Arabic in the matching text colour.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Label {
    pub en: String,
    pub ar: String,
    pub rect: Rect,
    pub bg: String,
    pub fg: String,
    /// what the game showed under it when read (see `region_sig`)
    #[serde(skip)]
    pub sig: Vec<u8>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ScreenLayout {
    pub id: String,
    pub title: Rect,
    pub title_color: String,
    pub title_ar: String,
    pub reason: Option<Rect>,
    /// Height of one line of the reason text (the reason can wrap onto several lines).
    pub reason_line_h: f64,
    pub reason_en: String,
    /// Buttons from right to left (the order an Arabic reader meets them).
    pub buttons: Vec<Button>,
    pub buttons_row: Option<Rect>,
    /// UI pieces replaced by Arabic replicas (pause-menu tabs).
    pub labels: Vec<Label>,
    /// Pause menu, top right: character name, day and time ("MICHAEL / MONDAY 05:00").
    pub player: Option<PlayerInfo>,
    /// Pause menu content under the tabs: list items (left) and text (right), as Arabic replicas.
    pub menu: Vec<MenuText>,
    /// a fine picture of the whole menu area when it was read (see `content_sig`)
    #[serde(skip)]
    pub content_sig: Vec<u8>,
    /// a picture of the button row (bottom right) when it was read
    #[serde(skip)]
    pub buttons_sig: Vec<u8>,
}

/// Where GTA's button row sits (bottom right).
pub const BUTTON_ROW: Rect = Rect { x: 0.55, y: 0.90, w: 0.45, h: 0.06 };

/// One piece of pause-menu content: a list item or a paragraph, covered by an Arabic replica.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MenuText {
    pub en: String,
    /// filled in by the app (built-in words, memory, or the translator); empty = not yet
    pub ar: String,
    /// the English text block, frame fractions
    pub rect: Rect,
    pub bg: String,
    pub fg: String,
    /// one line's height (frame fraction)
    pub line_h: f64,
    /// a list item (left column) rather than a paragraph
    pub item: bool,
    /// the English is right-aligned (values, labels left of the controller): keep that edge
    pub right: bool,
    /// how many lines the English took (1 = keep the Arabic on one line)
    pub lines: u32,
    /// a heading much bigger than the menu text ("Replay"): keeps its own size
    pub heading: bool,
    /// reads in a row this item was not seen again (kept on screen for a moment meanwhile)
    pub missed: u32,
    /// what the game showed under it when read (see `region_sig`)
    #[serde(skip)]
    pub sig: Vec<u8>,
}

/// The pause-menu tab the player is on.
pub fn selected_tab_name(l: &ScreenLayout) -> Option<&str> {
    selected_label(&l.labels)
}

/// The pause menu's character / day / time block: Arabic lines drawn just left of the English.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlayerInfo {
    pub en: Vec<String>,
    pub ar: Vec<String>,
    /// the English block, frame fractions
    pub rect: Rect,
    /// one line's letter height and the distance between line tops (frame fractions)
    pub line_h: f64,
    pub pitch: f64,
}

/// Id of the pause-menu page.
pub const PAUSE_ID: &str = "gta5_pause";

fn px(f: &FrameData, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * f.w + x) * 4) as usize;
    (f.rgba[i], f.rgba[i + 1], f.rgba[i + 2])
}

/// GTA V's "MISSION FAILED" red on the black screen (measured on captured frames).
const TITLE_RED: &str = "#df333b";

/// Is the frame (almost) black? The Mission Failed page ends on a black screen.
pub fn is_dark(f: &FrameData) -> bool {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in ((f.h / 10)..(f.h * 9 / 10)).step_by(8) {
        for x in ((f.w / 10)..(f.w * 9 / 10)).step_by(8) {
            let (r, g, b) = px(f, x, y);
            sum += r.max(g).max(b) as u64;
            n += 1;
        }
    }
    n > 0 && sum / n < 25
}

/// Cheap per-frame check (colour only, no OCR): is the red title still on screen?
pub fn title_present(f: &FrameData) -> bool {
    find_red_title(f).is_some()
}

fn is_title_red((r, g, b): (u8, u8, u8)) -> bool {
    // Strong red, also when the title is fading in over a bright scene (it looks pink then), and on
    // the final black screen through a capture card, where it is darker (~148, 20, 20).
    let (r, g, b) = (r as i32, g as i32, b as i32);
    r > 110 && r - g > 70 && r - b > 60
}

/// Find GTA V's red "MISSION FAILED" title: returns its pixel box (x0, y0, x1, y1) and median colour.
/// The page starts over the live (often bright) game scene, so we don't require a dark screen; we take
/// the densest horizontal band of red in the middle of the screen, and OCR confirms it afterwards.
fn find_red_title(f: &FrameData) -> Option<((u32, u32, u32, u32), (u8, u8, u8))> {
    let (w, h) = (f.w, f.h);
    let (x0, x1) = ((w as f64 * 0.12) as u32, (w as f64 * 0.88) as u32);
    let (y0, y1) = ((h as f64 * 0.22) as u32, (h as f64 * 0.72) as u32);
    let step = 4u32;
    // red count per sampled row
    let rows: Vec<(u32, u32)> = (y0..y1)
        .step_by(step as usize)
        .map(|y| (y, (x0..x1).step_by(step as usize).filter(|&x| is_title_red(px(f, x, y))).count() as u32))
        .collect();
    let peak = rows.iter().map(|r| r.1).max()?;
    // the title spans at least a fifth of the screen width
    if (peak * step) < w / 5 {
        return None;
    }
    // contiguous dense rows around the densest one
    let pi = rows.iter().position(|r| r.1 == peak)?;
    let dense = |i: usize| rows[i].1 * 4 >= peak;
    let mut top = pi;
    while top > 0 && dense(top - 1) {
        top -= 1;
    }
    let mut bot = pi;
    while bot + 1 < rows.len() && dense(bot + 1) {
        bot += 1;
    }
    let (by0, by1) = (rows[top].0, rows[bot].0 + step - 1);
    let bh = (by1 - by0) as f64 / h as f64;
    if !(0.03..=0.2).contains(&bh) {
        return None;
    }
    // Exact box and colour at full resolution inside that band; ignore columns/rows with stray pixels.
    // search a bit above the band too: tall letters ("d", "L") stick up past the dense rows
    let (sy0, sy1) = (by0.saturating_sub((h as f64 * 0.04) as u32), (by1 + step * 2).min(h - 1));
    let mut row_n = vec![0u32; (sy1 - sy0 + 1) as usize];
    let mut col_n = vec![0u32; (x1 - x0) as usize];
    let (mut rs, mut gs, mut bs) = (Vec::new(), Vec::new(), Vec::new());
    for y in sy0..=sy1 {
        for x in x0..x1 {
            let p = px(f, x, y);
            if is_title_red(p) {
                row_n[(y - sy0) as usize] += 1;
                col_n[(x - x0) as usize] += 1;
                rs.push(p.0);
                gs.push(p.1);
                bs.push(p.2);
            }
        }
    }
    // grow from the dense band outward while rows stay red (stops at the first empty row)
    let row_min = (w / 400).max(3);
    let (b0, b1) = ((by0 - sy0) as usize, ((by1 - sy0) as usize).min(row_n.len() - 1));
    let mut t0 = b0;
    while t0 > 0 && row_n[t0 - 1] >= row_min {
        t0 -= 1;
    }
    let mut t1 = b1;
    while t1 + 1 < row_n.len() && row_n[t1 + 1] >= row_min {
        t1 += 1;
    }
    let (ty0, ty1) = (sy0 + t0 as u32, sy0 + t1 as u32);
    let tx0 = x0 + col_n.iter().position(|&c| c >= 3)? as u32;
    let tx1 = x0 + col_n.iter().rposition(|&c| c >= 3)? as u32;
    let tw = (tx1 - tx0) as f64 / w as f64;
    if !(0.2..=0.76).contains(&tw) {
        return None;
    }
    let med = |v: &mut Vec<u8>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    Some(((tx0, ty0, tx1, ty1), (med(&mut rs), med(&mut gs), med(&mut bs))))
}

fn frac(f: &FrameData, x: u32, y: u32, w: u32, h: u32) -> Rect {
    Rect { x: x as f64 / f.w as f64, y: y as f64 / f.h as f64, w: w as f64 / f.w as f64, h: h as f64 / f.h as f64 }
}

fn crop(f: &FrameData, r: &Rect) -> Option<(Crop, u32, u32)> {
    let c = Crop::from_frame(&f.rgba, f.w, f.h, r.x, r.y, r.w, r.h)?;
    Some((c, (r.x.max(0.0) * f.w as f64) as u32, (r.y.max(0.0) * f.h as f64) as u32))
}

fn letters(s: &str) -> String {
    s.chars().filter(|c| c.is_alphabetic()).collect::<String>().to_lowercase()
}

pub fn detect_gta5_mission_failed(f: &FrameData, ocr: &Ocr) -> Option<ScreenLayout> {
    let found = find_red_title(f);
    if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[screens] red check: {found:?}");
    }
    let ((tx0, ty0, tx1, ty1), _measured_colour) = found?;
    let title = frac(f, tx0, ty0, tx1 - tx0 + 1, ty1 - ty0 + 1);

    // Confirm with OCR: GTA's title font reads loosely, so accept "mission" or "fail" anywhere.
    let pad = Rect { x: title.x - 0.01, y: title.y - 0.01, w: title.w + 0.02, h: title.h + 0.02 };
    let (tc, _, _) = crop(f, &pad)?;
    // The title letters are huge (~10% of the screen); OCR reads best at normal text size.
    let tc = shrink_to_height(&tc, 90);
    let tw = ocr.recognize(&tc).ok()?;
    let ttext = letters(&tw.iter().map(|w| w.text.as_str()).collect::<String>());
    if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[screens] red title {title:?}, OCR read {ttext:?}");
    }
    // The Pricedown title reads as junk at 1080p ("misoneiled", "gussian FaiLedj"); together with the
    // dense red band, a recognisable piece of either word is enough.
    if !["miss", "mis", "sion", "fail", "aile", "iled"].iter().any(|p| ttext.contains(p)) {
        return None;
    }

    // Reason line: white text just under the title.
    let mut reason = None;
    let mut reason_en = String::new();
    let mut reason_line_h = 0.0;
    let rr = Rect { x: 0.12, y: title.y + title.h + 0.002, w: 0.76, h: 0.08 };
    if let Some((rc, ox, oy)) = crop(f, &rr) {
        // Small text (1080p capture cards) reads better enlarged: "A hostage" came out "Abostage".
        let k = if f.h < 1600 { 2 } else { 1 };
        let rc = if k == 1 {
            rc
        } else {
            let img = image::RgbaImage::from_raw(rc.w, rc.h, rc.rgba).expect("crop size");
            let big = image::imageops::resize(&img, rc.w * k, rc.h * k, image::imageops::FilterType::CatmullRom);
            Crop { w: rc.w * k, h: rc.h * k, rgba: big.into_raw() }
        };
        if let Ok(mut words) = ocr.recognize(&rc) {
            for w in words.iter_mut() {
                w.rect = (w.rect.0 / k, w.rect.1 / k, w.rect.2 / k, w.rect.3 / k);
            }
            let text = ocr::clean(&words);
            if !text.is_empty() {
                let x0 = words.iter().map(|w| w.rect.0).min().unwrap_or(0);
                let y0 = words.iter().map(|w| w.rect.1).min().unwrap_or(0);
                let x1 = words.iter().map(|w| w.rect.0 + w.rect.2).max().unwrap_or(0);
                let y1 = words.iter().map(|w| w.rect.1 + w.rect.3).max().unwrap_or(0);
                reason = Some(frac(f, ox + x0, oy + y0, x1 - x0, y1 - y0));
                reason_en = text;
                // One line's height, not the whole block's: take a tall-ish word (75th percentile).
                let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
                hs.sort_unstable();
                reason_line_h = hs[(hs.len() * 3 / 4).min(hs.len() - 1)] as f64 / f.h as f64;
            }
        }
    }

    // Buttons, bottom right: "Skip ▢  Restart △  Retry ✕" (Skip only after a few failures),
    // or "No ◯  Yes ✕" on the restart confirmation.
    let buttons = find_buttons(f, ocr);
    let buttons_row = (!buttons.is_empty()).then(|| {
        let x0 = buttons.iter().map(|b| b.text_rect.x).fold(f64::MAX, f64::min);
        let x1 = buttons
            .iter()
            .map(|b| b.icon_rect.as_ref().map(|i| i.x + i.w).unwrap_or(b.text_rect.x + b.text_rect.w))
            .fold(0.0, f64::max);
        let y0 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.y).unwrap_or(b.text_rect.y)).fold(f64::MAX, f64::min);
        let y1 = buttons
            .iter()
            .map(|b| b.icon_rect.as_ref().map(|i| i.y + i.h).unwrap_or(b.text_rect.y + b.text_rect.h))
            .fold(0.0, f64::max);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    });

    Some(ScreenLayout {
        id: "gta5_mission_failed".into(),
        title,
        // GTA's title red as it appears on the black screen (#df333b). While the title fades in over
        // the game it looks pinker; we always use the final colour so the Arabic doesn't change colour.
        title_color: TITLE_RED.into(),
        title_ar: "فشلت المهمة".into(),
        reason,
        reason_line_h,
        reason_en,
        buttons,
        buttons_row,
        labels: Vec::new(),
        player: None,
        menu: Vec::new(),
        content_sig: Vec::new(),
        buttons_sig: Vec::new(),
    })
}

/// Downscale a crop so it is at most `max_h` pixels tall (keeps aspect ratio).
fn shrink_to_height(c: &Crop, max_h: u32) -> Crop {
    if c.h <= max_h {
        return Crop { w: c.w, h: c.h, rgba: c.rgba.clone() };
    }
    let img = image::RgbaImage::from_raw(c.w, c.h, c.rgba.clone()).expect("crop size");
    let nw = (c.w as u64 * max_h as u64 / c.h as u64).max(8) as u32;
    let small = image::imageops::resize(&img, nw, max_h, image::imageops::FilterType::Triangle);
    Crop { w: nw, h: max_h, rgba: small.into_raw() }
}

/// Button words GTA V shows in its prompts, and their Arabic.
const BUTTON_WORDS: [(&str, &str); 21] = [
    ("Enter PlayStation Store", "دخول متجر PlayStation™"),
    ("Browse", "تصفح"), ("Hide Menu", "إخفاء القائمة"), ("Waypoint", "نقطة توجيه"), ("Zoom", "تكبير"),
    ("Legend", "دليل الرموز"), ("Apply", "تطبيق"), ("Reset", "إعادة الضبط"),
    ("Retry", "إعادة المحاولة"), ("Restart", "إعادة البدء"), ("Skip", "تخطي"), ("Yes", "نعم"), ("No", "لا"),
    ("Continue", "متابعة"), ("Cancel", "إلغاء"), ("Back", "رجوع"), ("Quit", "خروج"), ("Exit", "خروج"),
    ("Accept", "قبول"), ("Select", "اختيار"), ("Confirm", "تأكيد"),
];

/// Read one button word on its own. OCR ignores words sitting next to the round icons, but reads
/// them reliably once cut out with a dark margin (enlarged ×2 if the first try doesn't match).
fn read_button_word(c: &Crop, word: (u32, u32, u32, u32), ocr: &Ocr) -> Option<(&'static str, &'static str)> {
    let m = 10u32;
    let (x0, y0) = (word.0.saturating_sub(m), word.2.saturating_sub(m));
    let (x1, y1) = ((word.1 + m).min(c.w - 1), (word.3 + m).min(c.h - 1));
    let pad = 40u32;
    let (w, h) = (x1 - x0 + 1 + 2 * pad, y1 - y0 + 1 + 2 * pad);
    let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = ((y * c.w + x) * 4) as usize;
            canvas.put_pixel(x - x0 + pad, y - y0 + pad, image::Rgba([c.rgba[i], c.rgba[i + 1], c.rgba[i + 2], 255]));
        }
    }
    for scale in [1u32, 2] {
        let img = if scale == 1 {
            canvas.clone()
        } else {
            image::imageops::resize(&canvas, w * scale, h * scale, image::imageops::FilterType::Lanczos3)
        };
        let (iw, ih) = img.dimensions();
        let Ok(words) = ocr.recognize(&Crop { w: iw, h: ih, rgba: img.into_raw() }) else { continue };
        let read = letters(&words.iter().map(|w| w.text.as_str()).collect::<String>());
        if let Some(hit) = button_words()
            .iter()
            // (compared as letters only: "Hide Menu" -> "hidemenu", like what was read)
            .find(|(en, _)| read == letters(en) || (read.len() >= 4 && ocr::similar(&read, &letters(en))))
        {
            return Some(*hit);
        }
    }
    None
}

/// Bottom-right button row by shape: white word blobs, each followed by a square-ish icon.
/// Each word is then read on its own; a word we can't read gets no Arabic rather than a guess.
fn find_buttons(f: &FrameData, ocr: &Ocr) -> Vec<Button> {
    let row = Rect { x: 0.55, y: 0.86, w: 0.45, h: 0.13 };
    let Some((c, ox, oy)) = crop(f, &row) else { return Vec::new() };
    let bright = |x: u32, y: u32| {
        let i = ((y * c.w + x) * 4) as usize;
        c.rgba[i].max(c.rgba[i + 1]).max(c.rgba[i + 2]) > 140
    };
    // Columns that contain any bright pixel, grouped into blobs (small gaps = same word).
    let max_gap = ((f.h as f64) * 0.003).max(3.0) as u32;
    let col_on: Vec<bool> = (0..c.w).map(|x| (0..c.h).any(|y| bright(x, y))).collect();
    let mut blobs: Vec<(u32, u32, u32, u32)> = Vec::new(); // x0, x1, y0, y1
    let mut start: Option<u32> = None;
    let (mut end, mut gap) = (0u32, 0u32);
    let close = |s: u32, e: u32, blobs: &mut Vec<(u32, u32, u32, u32)>| {
        let ys: Vec<u32> = (0..c.h).filter(|&y| (s..=e).any(|x| bright(x, y))).collect();
        if let (Some(&y0), Some(&y1)) = (ys.first(), ys.last()) {
            blobs.push((s, e, y0, y1));
        }
    };
    for x in 0..c.w {
        if col_on[x as usize] {
            start.get_or_insert(x);
            end = x;
            gap = 0;
        } else if let Some(s) = start {
            gap += 1;
            if gap > max_gap {
                close(s, end, &mut blobs);
                start = None;
            }
        }
    }
    if let Some(s) = start {
        close(s, end, &mut blobs);
    }

    let min_icon = (f.h as f64 * 0.012) as u32;
    let is_icon = |b: &(u32, u32, u32, u32)| {
        let (w, h) = (b.1 - b.0 + 1, b.3 - b.2 + 1);
        // round buttons ~1:1; L1 / R1 are rounded rectangles up to ~1.6:1
        h >= min_icon && (w as f64 / h as f64) > 0.75 && (w as f64 / h as f64) < 1.65
    };
    // Each word with the icon(s) right after it ("Retry [X]", "Browse [L1] [R1]"); right to left.
    let mut pairs: Vec<((u32, u32, u32, u32), Vec<(u32, u32, u32, u32)>)> = Vec::new();
    let mut i = 0;
    while i + 1 < blobs.len() {
        let word = blobs[i];
        if !is_icon(&word) && word.1 - word.0 > (word.3 - word.2) && is_icon(&blobs[i + 1]) {
            // a label of several words ("Hide Menu"): take in the words just before, up to the
            // previous button's icon
            let mut word = word;
            let mut k = i;
            while k > 0 && !is_icon(&blobs[k - 1]) && word.0.saturating_sub(blobs[k - 1].1) <= (word.3 - word.2 + 1) {
                let b = blobs[k - 1];
                word = (b.0, word.1, b.2.min(word.2), b.3.max(word.3));
                k -= 1;
            }
            let mut icons = Vec::new();
            let mut j = i + 1;
            while j < blobs.len() && is_icon(&blobs[j]) {
                icons.push(blobs[j]);
                j += 1;
            }
            pairs.push((word, icons));
            i = j;
        } else {
            i += 1;
        }
    }
    pairs.reverse();
    let png = |b: &(u32, u32, u32, u32)| (b.0, b.2, b.1 - b.0 + 1, b.3 - b.2 + 1);
    pairs
        .into_iter()
        .take(5)
        .filter_map(|(word, icons)| {
            let (en, ar) = read_button_word(&c, word, ocr)?;
            let ir = png(&icons[0]);
            Some(Button {
                en: en.into(),
                ar: ar.into(),
                icon: ocr::icon_png(&c, ir, 140),
                icon_rect: Some(frac(f, ox + ir.0, oy + ir.1, ir.2, ir.3)),
                text_rect: frac(f, ox + word.0, oy + word.2, word.1 - word.0 + 1, word.3 - word.2 + 1),
                more_icons: icons[1..].iter().filter_map(|b| ocr::icon_png(&c, png(b), 140)).collect(),
            })
        })
        .collect()
}

/// GTA V fail reasons follow fixed patterns; translate the common ones exactly instead of by machine.
/// "M. died." -> "مات م.", "Amanda died." -> "ماتت أماندا." Unknown text returns None (machine translation).
pub fn reason_rule(en: &str) -> Option<String> {
    const NAMES: [(&str, &str, bool); 28] = [
        ("A hostage", "رهينة", true), ("The hostage", "الرهينة", true),
        ("Michael", "مايكل", false), ("Franklin", "فرانكلين", false), ("Trevor", "تريفور", false),
        ("Lamar", "لامار", false), ("Lester", "ليستر", false), ("Chop", "تشوب", false), ("Brad", "براد", false),
        ("Jimmy", "جيمي", false), ("Dave", "ديف", false), ("Steve", "ستيف", false), ("Wade", "ويد", false),
        ("Ron", "رون", false), ("Simeon", "سيميون", false), ("Devin", "ديفن", false), ("Solomon", "سولومون", false),
        ("Stretch", "ستريتش", false), ("Floyd", "فلويد", false), ("Martin", "مارتن", false),
        ("Amanda", "أماندا", true), ("Tracey", "تريسي", true), ("Denise", "دينيس", true), ("Tanisha", "تانيشا", true),
        ("Patricia", "باتريشيا", true), ("M.", "م.", false), ("T.", "ت.", false), ("B.", "ب.", false),
    ];
    let t = en.trim().trim_end_matches('.').trim();
    let name = t.strip_suffix(" died").or_else(|| t.strip_suffix(" was killed"))?.trim();
    let (_, ar, female) = NAMES.iter().find(|(n, _, _)| n.trim_end_matches('.') == name.trim_end_matches('.'))?;
    let verb = if *female { "ماتت" } else { "مات" };
    // names like "م." already end with a dot
    Some(if ar.ends_with('.') { format!("{verb} {ar}") } else { format!("{verb} {ar}.") })
}

// ------------------------------------------------------------------------------------- pause menu

/// Pause-menu tab names and their Arabic.
const TAB_WORDS: [(&str, &str); 13] = [
    ("MAP", "الخريطة"), ("BRIEF", "الموجز"), ("STATS", "الإحصائيات"), ("SETTINGS", "الإعدادات"),
    ("GAME", "اللعبة"), ("ONLINE", "أونلاين"), ("GALLERY", "المعرض"), ("FRIENDS", "الأصدقاء"),
    ("INFO", "معلومات"), ("STORE", "المتجر"), ("REPLAY", "الإعادة"), ("PLAYERS", "اللاعبون"),
    ("CREWS", "الطواقم"),
];

/// The selected tab of the pause menu: a white block ~11 % of the width, ~3.5 % of the height, in
/// the tab row near the top (measured on 1080p frames). Cheap: no OCR. Returns it in pixels
/// (x, y, w, h).
fn selected_tab(f: &FrameData) -> Option<(u32, u32, u32, u32)> {
    let white = |x: u32, y: u32| {
        let (r, g, b) = px(f, x, y);
        r.min(g).min(b) > 225 && r.max(g).max(b) - r.min(g).min(b) < 25
    };
    let (x0, x1) = ((f.w as f64 * 0.12) as u32, (f.w as f64 * 0.88) as u32);
    let (ymin, ymax) = ((f.h as f64 * 0.12) as u32, (f.h as f64 * 0.26) as u32);
    let (wmin, wmax) = ((f.w as f64 * 0.08) as u32, (f.w as f64 * 0.14) as u32);
    // rows whose longest white run is tab-sized: (y, start, length)
    let mut rows: Vec<(u32, u32, u32)> = Vec::new();
    for y in ymin..ymax {
        let (mut best, mut best_s, mut run, mut s) = (0u32, 0u32, 0u32, 0u32);
        for x in x0..x1 {
            if white(x, y) {
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
        if (wmin..=wmax).contains(&best) {
            rows.push((y, best_s, best));
        }
    }
    // the same run (same start and length, ±3 px) over several rows: its outer rows are the tab
    let same = |a: &(u32, u32, u32), b: &(u32, u32, u32)| a.1.abs_diff(b.1) <= 3 && a.2.abs_diff(b.2) <= 3;
    let key = rows.iter().find(|r| rows.iter().filter(|o| same(r, o)).count() >= 8)?;
    let ys: Vec<u32> = rows.iter().filter(|o| same(key, o)).map(|o| o.0).collect();
    let (y0, y1) = (*ys.iter().min()?, *ys.iter().max()?);
    ((y1 - y0) as f64 >= f.h as f64 * 0.025).then_some((key.1, y0, key.2, y1 - y0 + 1))
}

/// Is the pause menu (still) up? Colour only, every frame.
pub fn pause_present(f: &FrameData) -> bool {
    selected_tab(f).is_some()
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// GTA V pause menu: its tabs become Arabic replicas, its buttons (bottom right) get Arabic labels.
pub fn detect_gta5_pause(f: &FrameData, ocr: &Ocr) -> Option<ScreenLayout> {
    let (sx, sy, sw, sh) = selected_tab(f)?;
    // read the tab row
    let row = Rect { x: 0.1, y: sy as f64 / f.h as f64 - 0.004, w: 0.8, h: (sh as f64 / f.h as f64) + 0.008 };
    let (c, ox, oy) = crop(f, &row)?;
    let words = ocr.recognize_with(&c, ocr::Prep::Brightest).ok().unwrap_or_default();
    let words = if words.len() < 3 { ocr.recognize(&c).ok()? } else { words };
    let debug = std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some();
    let mut labels = Vec::new();
    for w in &words {
        let t: String = w.text.chars().filter(|c| c.is_alphabetic()).collect::<String>().to_uppercase();
        let tabs = tab_words();
        let Some(&(en, ar)) = tabs.iter().find(|(e, _)| *e == t) else { continue };
        // the tab around the word: as wide and tall as the selected tab, centred on the word
        let cx = ox + w.rect.0 + w.rect.2 / 2;
        let (tx, ty) = (cx.saturating_sub(sw / 2), sy);
        // its background: the tab's pixels away from the letters (median)
        let (mut rs, mut gs, mut bs) = (Vec::new(), Vec::new(), Vec::new());
        let (wx0, wy0) = (ox + w.rect.0, oy + w.rect.1);
        let (wx1, wy1) = (wx0 + w.rect.2, wy0 + w.rect.3);
        for y in (ty + 2..ty + sh - 2).step_by(2) {
            for x in (tx + 4..(tx + sw).min(f.w) - 4).step_by(3) {
                if x + 4 >= wx0 && x <= wx1 + 4 && y + 4 >= wy0 && y <= wy1 + 4 {
                    continue;
                }
                let (r, g, b) = px(f, x, y);
                rs.push(r);
                gs.push(g);
                bs.push(b);
            }
        }
        if rs.is_empty() {
            continue;
        }
        let med = |v: &mut Vec<u8>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        let bg = (med(&mut rs), med(&mut gs), med(&mut bs));
        let light = bg.0 as u32 + bg.1 as u32 + bg.2 as u32 > 3 * 140;
        if debug {
            eprintln!("[pause] tab {en} at x {tx} bg {bg:?}");
        }
        let rect = frac(f, tx, ty, sw, sh);
        labels.push(Label {
            en: en.to_string(),
            ar: ar.to_string(),
            sig: region_sig(f, &rect),
            rect,
            bg: hex(bg),
            fg: if light { "#000000".into() } else { "#ffffff".into() },
        });
    }
    if debug {
        eprintln!("[pause] selected tab {sx},{sy} {sw}x{sh}; words {:?}; {} tab(s) matched", words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), labels.len());
    }
    if labels.len() < 3 {
        return None;
    }
    let buttons = find_buttons(f, ocr);
    let buttons_row = (!buttons.is_empty()).then(|| {
        let x0 = buttons.iter().map(|b| b.text_rect.x).fold(f64::MAX, f64::min);
        let x1 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.x + i.w).unwrap_or(b.text_rect.x + b.text_rect.w)).fold(0.0, f64::max);
        let y0 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.y).unwrap_or(b.text_rect.y)).fold(f64::MAX, f64::min);
        let y1 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.y + i.h).unwrap_or(b.text_rect.y + b.text_rect.h)).fold(0.0, f64::max);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    });
    Some(ScreenLayout {
        id: PAUSE_ID.into(),
        title: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        title_color: String::new(),
        title_ar: String::new(),
        reason: None,
        reason_line_h: 0.0,
        reason_en: String::new(),
        buttons,
        buttons_row,
        menu: match selected_label(&labels) {
            Some("MAP") => Vec::new(),
            tab => read_menu(f, ocr, sh, tab.unwrap_or("")),
        },
        labels,
        player: read_player(f, ocr),
        content_sig: content_sig(f),
        buttons_sig: region_sig(f, &BUTTON_ROW),
    })
}

/// The tab the player is on (the light one).
fn selected_label(labels: &[Label]) -> Option<&str> {
    labels.iter().find(|l| l.fg == "#000000").map(|l| l.en.as_str())
}

/// Character names and weekdays the pause menu shows, and their Arabic.
const PLAYERS: [(&str, &str); 3] = [("MICHAEL", "مايكل"), ("FRANKLIN", "فرانكلين"), ("TREVOR", "تريفور")];
const DAYS: [(&str, &str); 7] = [
    ("MON", "الاثنين"), ("TUE", "الثلاثاء"), ("WED", "الأربعاء"), ("THU", "الخميس"),
    ("FRI", "الجمعة"), ("SAT", "السبت"), ("SUN", "الأحد"),
];

/// Top right of the pause menu: "MICHAEL" over "MONDAY 05:00" (small condensed letters, read
/// enlarged ×3). Name and day come from fixed lists; the time keeps its digits.
fn read_player(f: &FrameData, ocr: &Ocr) -> Option<PlayerInfo> {
    let area = Rect { x: 0.62, y: 0.06, w: 0.18, h: 0.10 };
    let (c, ox, oy) = crop(f, &area)?;
    let k = 3;
    let img = image::RgbaImage::from_raw(c.w, c.h, c.rgba).expect("crop size");
    let big = image::imageops::resize(&img, c.w * k, c.h * k, image::imageops::FilterType::CatmullRom);
    let big = Crop { w: c.w * k, h: c.h * k, rgba: big.into_raw() };
    let words = ocr.recognize_with(&big, ocr::Prep::Brightest).ok()?;
    if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[pause] player words {:?}", words.iter().map(|w| (w.text.as_str(), w.line)).collect::<Vec<_>>());
    }
    // OCR lines, top to bottom: (top, bottom, x0, x1, text)
    let mut lines: Vec<(u32, u32, u32, u32, String)> = Vec::new();
    let mut by_line: std::collections::BTreeMap<u32, Vec<&ocr::Word>> = std::collections::BTreeMap::new();
    for w in &words {
        by_line.entry(w.line).or_default().push(w);
    }
    for ws in by_line.values() {
        let top = ws.iter().map(|w| w.rect.1).min()? / k;
        let bottom = ws.iter().map(|w| w.rect.1 + w.rect.3).max()? / k;
        let x0 = ws.iter().map(|w| w.rect.0).min()? / k;
        let x1 = ws.iter().map(|w| w.rect.0 + w.rect.2).max()? / k;
        lines.push((top, bottom, x0, x1, ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")));
    }
    lines.sort_by_key(|l| l.0);
    let name_i = lines.iter().position(|l| PLAYERS.iter().any(|(n, _)| l.4.to_uppercase().contains(&n[..4])))?;
    let (_, name_ar) = PLAYERS.iter().find(|(n, _)| lines[name_i].4.to_uppercase().contains(&n[..4]))?;
    let mut en = vec![lines[name_i].4.clone()];
    let mut ar = vec![name_ar.to_string()];
    let mut used = vec![name_i];
    // the day / time line right under it
    if let Some(j) = lines.iter().position(|l| l.0 > lines[name_i].1) {
        let t = lines[j].4.to_uppercase();
        let day = DAYS.iter().find(|(d, _)| t.contains(d)).map(|(_, a)| a.to_string());
        let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
        let time = (digits.len() >= 3).then(|| {
            let d = &digits[digits.len().saturating_sub(4)..];
            let (h, m) = d.split_at(d.len() - 2);
            format!("{h:0>2}:{m}")
        });
        let line: Vec<String> = [day, time].into_iter().flatten().collect();
        if !line.is_empty() {
            en.push(lines[j].4.clone());
            ar.push(line.join(" "));
            used.push(j);
        }
    }
    let ls: Vec<&(u32, u32, u32, u32, String)> = used.iter().map(|&i| &lines[i]).collect();
    let (x0, x1) = (ls.iter().map(|l| l.2).min()?, ls.iter().map(|l| l.3).max()?);
    let (y0, y1) = (ls.iter().map(|l| l.0).min()?, ls.iter().map(|l| l.1).max()?);
    let line_h = (ls[0].1 - ls[0].0) as f64 / f.h as f64;
    let pitch = if ls.len() > 1 { (ls[1].0 - ls[0].0) as f64 / f.h as f64 } else { line_h * 1.4 };
    Some(PlayerInfo { en, ar, rect: frac(f, ox + x0, oy + y0, x1 - x0, y1 - y0), line_h, pitch })
}

/// Common pause-menu words and their Arabic (anything else goes to the translator).
const MENU_WORDS: [(&str, &str); 157] = [
    // Brief
    ("Mission", "المهمة"), ("Help", "المساعدة"), ("Dialogue", "الحوار"), ("Notifications", "الإشعارات"),
    ("Newswire", "الأخبار"),
    // Stats
    ("Skills", "المهارات"), ("General", "عام"), ("Crimes", "الجرائم"), ("Vehicles", "المركبات"),
    ("Cash", "المال"), ("Combat", "القتال"), ("Weapons", "الأسلحة"), ("100% Completion", "الإتمام 100%"),
    ("Checklist", "قائمة الإنجاز"), ("Special", "خاص"), ("Strength", "القوة"), ("Stamina", "التحمل"),
    ("Shooting", "الرماية"), ("Driving", "القيادة"), ("Flying", "الطيران"), ("Stealth", "التخفي"),
    ("Lung Capacity", "سعة الرئة"),
    // Settings
    ("Controls", "التحكم"), ("Display", "العرض"), ("Graphics", "الرسوميات"), ("Audio", "الصوت"),
    ("Camera", "الكاميرا"), ("Language", "اللغة"), ("Saving and Startup", "الحفظ والتشغيل"),
    ("Voice Chat", "الدردشة الصوتية"), ("Keyboard / Mouse", "لوحة المفاتيح / الفأرة"),
    ("Advanced Graphics", "رسوميات متقدمة"), ("First Person", "منظور الشخص الأول"), ("Facebook", "فيسبوك"),
    ("On", "تشغيل"), ("Off", "إيقاف"), ("Default", "افتراضي"), ("Restore Defaults", "استعادة الإعدادات الافتراضية"),
    // Game
    ("Replay Mission", "إعادة مهمة"), ("Replay Strangers and Freaks", "إعادة مهام الغرباء والغريبي الأطوار"),
    ("Load Game", "تحميل لعبة"), ("New Game", "لعبة جديدة"), ("Credits & Legal", "الشكر والمعلومات القانونية"),
    ("Upload Photo", "رفع صورة"), ("Save Game", "حفظ اللعبة"), ("Story Mode", "وضع القصة"),
    // Stats / Game: more items and names
    ("Replay", "إعادة اللعب"),
    ("Download Save Game", "تنزيل حفظ اللعبة"),
    ("Quit to Main Menu", "الخروج إلى القائمة الرئيسية"),
    ("Michael", "مايكل"),
    ("Franklin", "فرانكلين"),
    ("Trevor", "تريفور"),
    ("100% Checklist", "قائمة الإتمام 100%"),
    // Online
    ("Join Friends", "الانضمام إلى الأصدقاء"),
    ("Join Crew Members", "الانضمام إلى أعضاء الطاقم"),
    ("Crews", "الطواقم"),
    ("Rockstar Creator", "محرر Rockstar للمحتوى"),
    ("Manage Characters", "إدارة الشخصيات"),
    ("Play GTA Online", "العب GTA أونلاين"),
    ("Find New Session", "البحث عن جلسة جديدة"),
    ("Friends", "الأصدقاء"),
    // the controller diagram's other labels (it alternates between two sets)
    ("Aim Weapon / Lock-On", "التصويب / تثبيت الهدف"),
    ("Interaction Menu", "قائمة التفاعل"),
    ("Sniper / Camera Zoom", "القنّاصة / تقريب الكاميرا"),
    ("Dive", "الغوص"),
    ("Hang Up Phone", "إنهاء المكالمة"),
    ("Answer Phone", "الرد على الهاتف"),
    ("Climb", "التسلق"),
    ("Zoom Aim", "تقريب التصويب"),
    ("Melee Dodge", "مراوغة (قتال قريب)"),
    ("Skip Phone Conversation", "تخطي المكالمة"),
    ("Phone Cancel", "الهاتف: إلغاء"),
    ("Phone Select", "الهاتف: اختيار"),
    ("Character Selector", "اختيار الشخصية"),
    ("Cinematic Camera", "الكاميرا السينمائية"),
    ("Special Ability", "القدرة الخاصة"),
    ("Duck", "الانحناء"),
    ("Cover", "الاحتماء"),
    ("Attack", "الهجوم"),
    ("Grenade", "القنبلة"),
    ("Exit Vehicle", "الخروج من المركبة"),
    ("Horn", "البوق"),
    ("Handbrake", "فرامل اليد"),
    ("Accelerate", "التسارع"),
    ("Brake / Reverse", "الفرامل / الرجوع"),
    ("Steer", "التوجيه"),
    ("Headlights", "المصابيح الأمامية"),
    ("Look Behind", "النظر للخلف"),
    ("Vehicle Weapon", "سلاح المركبة"),
    ("In Vehicle", "في المركبة"),
    ("In Vehicle (Third Person)", "في المركبة (منظور الشخص الثالث)"),
    ("On Foot (First Person)", "سيراً على الأقدام (منظور الشخص الأول)"),
    ("Phone", "الهاتف"),
    // Settings > Camera
    ("Allow Independent Camera Modes", "السماح بأوضاع كاميرا مستقلة"),
    ("Vehicle Camera Height", "ارتفاع كاميرا المركبة"),
    ("First Person Auto Level Camera", "تسوية الكاميرا تلقائياً (منظور الشخص الأول)"),
    ("First Person On Foot Field of View", "مجال الرؤية سيراً (منظور الشخص الأول)"),
    ("First Person Ragdoll", "ارتخاء الجسد عند السقوط (منظور الشخص الأول)"),
    ("First Person Combat Roll", "التدحرج القتالي (منظور الشخص الأول)"),
    ("First Person Head Bobbing", "اهتزاز الرأس (منظور الشخص الأول)"),
    ("First Person Third Person Cover", "الاحتماء بمنظور الشخص الثالث (أثناء منظور الشخص الأول)"),
    ("First Person Vehicle Auto-Center", "توسيط الكاميرا تلقائياً في المركبة (منظور الشخص الأول)"),
    ("First Person Vehicle Hood", "منظور غطاء محرك المركبة (منظور الشخص الأول)"),
    ("First Person Drive-By Camera Relative to Vehicle", "كاميرا إطلاق النار من المركبة نسبةً إليها (منظور الشخص الأول)"),
    // Settings: more options and the controller diagram
    ("Rockstar Editor", "محرر روكستار"),
    ("Saving", "الحفظ"),
    ("Motion Sensor Function", "مستشعر الحركة"),
    ("On Foot", "سيراً على الأقدام"),
    ("On Foot (Third Person)", "سيراً على الأقدام (منظور الشخص الثالث)"),
    ("(Third Person)", "(منظور الشخص الثالث)"),
    ("Target Lock On", "تثبيت الهدف"),
    ("Weapon Wheel", "عجلة الأسلحة"),
    ("Switch Camera", "تبديل الكاميرا"),
    ("Use Phone", "استخدام الهاتف"),
    ("Phone Left", "الهاتف يسار"),
    ("Phone Down", "الهاتف أسفل"),
    ("Phone Right", "الهاتف يمين"),
    ("Phone Up", "الهاتف أعلى"),
    ("Move", "التحرك"),
    ("Stealth Mode", "وضع التخفي"),
    ("Fire Weapon", "إطلاق النار"),
    ("Take Cover", "الاحتماء"),
    ("Pause Menu", "قائمة الإيقاف"),
    ("Enter Vehicle", "ركوب المركبة"),
    ("Light Melee Attack", "هجوم خفيف"),
    ("Heavy Melee Attack", "هجوم قوي"),
    ("Jump", "القفز"),
    ("Look", "النظر"),
    ("Look Behind (On Foot)", "النظر للخلف (سيراً)"),
    ("Sprint", "الركض"),
    ("Reload", "إعادة التعبئة"),
    ("Show Controls For", "عرض التحكم لـ"),
    ("Targeting Mode", "وضع التصويب"),
    ("Assisted Aim - Partial", "تصويب مساعد - جزئي"),
    ("Assisted Aim - Full", "تصويب مساعد - كامل"),
    ("Free Aim", "تصويب حر"),
    ("Vibration", "الاهتزاز"),
    ("Adaptive Triggers", "المشغلات التكيفية"),
    ("High", "مرتفع"),
    ("Medium", "متوسط"),
    ("Low", "منخفض"),
    ("Invert Look", "عكس النظر"),
    ("Third Person Control Type", "نوع التحكم بمنظور الشخص الثالث"),
    ("First Person Control Type", "نوع التحكم بمنظور الشخص الأول"),
    ("Third Person Aiming Sensitivity", "حساسية التصويب بمنظور الشخص الثالث"),
    ("First Person Aiming Sensitivity", "حساسية التصويب بمنظور الشخص الأول"),
    ("Standard", "قياسي"),
    ("Alternate", "بديل"),
    ("Brightness", "السطوع"),
    ("Subtitles", "الترجمة النصية"),
    ("Radar", "الرادار"),
    ("HUD", "واجهة اللعب"),
    ("Safezone Size", "حجم المنطقة الآمنة"),
    ("SFX Volume", "مستوى المؤثرات الصوتية"),
    ("Music Volume", "مستوى الموسيقى"),
    ("Dialogue Volume", "مستوى الحوار"),
    ("Radio Station", "محطة الراديو"),
];

/// Built-in Arabic for a common pause-menu word or phrase.
pub fn menu_word(en: &str) -> Option<&'static str> {
    let k = letters(en);
    let words = menu_words();
    words
        .iter()
        .find(|(e, _)| letters(e) == k)
        // a small OCR slip ("Rockstar Edito/") still counts, for longer phrases
        .or_else(|| words.iter().find(|(e, _)| k.len() >= 8 && ocr::similar(&letters(e), &k)))
        .map(|(_, a)| *a)
}

/// Words from the game's downloaded layout, looked up before the built-in lists (so a layout
/// can add a word or fix one). Set with `set_layout_words`.
static LAYOUT_WORDS: std::sync::RwLock<[Vec<(&'static str, &'static str)>; 3]> =
    std::sync::RwLock::new([Vec::new(), Vec::new(), Vec::new()]);

/// Replace the layout's words (menu, buttons, tabs). Layout updates are rare and small, so the
/// strings are simply kept for the rest of the run.
pub fn set_layout_words(menu: Vec<(String, String)>, buttons: Vec<(String, String)>, tabs: Vec<(String, String)>) {
    let keep = |v: Vec<(String, String)>| -> Vec<(&'static str, &'static str)> {
        v.into_iter().map(|(e, a)| (&*Box::leak(e.into_boxed_str()), &*Box::leak(a.into_boxed_str()))).collect()
    };
    *LAYOUT_WORDS.write().unwrap() = [keep(menu), keep(buttons), keep(tabs)];
}

fn with_layout(i: usize, built_in: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut v = LAYOUT_WORDS.read().unwrap()[i].clone();
    v.extend_from_slice(built_in);
    v
}

fn menu_words() -> Vec<(&'static str, &'static str)> {
    with_layout(0, &MENU_WORDS)
}

fn button_words() -> Vec<(&'static str, &'static str)> {
    with_layout(1, &BUTTON_WORDS)
}

fn tab_words() -> Vec<(&'static str, &'static str)> {
    with_layout(2, &TAB_WORDS)
}

/// The pause menu's content under the tabs. Left column (list): one item per line. Right panel:
/// lines that follow each other closely and start at the same x are one paragraph.
/// `tab_h`: the selected tab's height in pixels; the menu's text size is a fixed share of it (the
/// same on every tab and every read, unlike OCR's boxes, which change with the letters).
fn read_menu(f: &FrameData, ocr: &Ocr, tab_h: u32, tab: &str) -> Vec<MenuText> {
    // The Store has no list: one wide text column (and a picture on the right, left alone).
    if tab == "STORE" {
        let lines = read_lines(f, ocr, &Rect { x: 0.155, y: 0.212, w: 0.455, h: 0.64 }, 0);
        return menu_blocks(f, lines, 0, tab_h);
    }
    // The list column and the panel are read apart (text across the divider runs together
    // otherwise), each enlarged ×2: the menu's small grey letters over the blurred game read
    // much better that way.
    let split = (f.w as f64 * 0.388) as u32; // the list column ends here
    let list = Rect { x: 0.155, y: 0.212, w: 0.231, h: 0.64 };
    let mut lines = read_lines(f, ocr, &list, 0);
    lines.extend(read_lines(f, ocr, &Rect { x: 0.390, y: 0.212, w: 0.455, h: 0.64 }, 0));
    // Greyed-out (disabled) list items are so faint (letters ~55 / 255 on near-black) that the
    // read above misses them. A second, strongly boosted read of the list fills only the rows the
    // first one left empty, and only with known menu words (the boost also lifts background noise).
    for l in read_lines(f, ocr, &list, 4) {
        let h = l.3 - l.1;
        let free = !lines.iter().any(|o| o.0 < split && o.1 < l.3 && l.1 < o.3);
        if free && h > 0 && menu_item_word(&l.4).is_some() {
            lines.push(l);
        }
    }
    lines.sort_by_key(|l| (l.1, l.0));
    menu_blocks(f, lines, split, tab_h)
}

/// OCR one part of the menu, enlarged ×2 (and brightness × `boost` when > 0): its lines as
/// (x0, y0, x1, y1, text) in frame pixels.
fn read_lines(f: &FrameData, ocr: &Ocr, area: &Rect, boost: i32) -> Vec<(u32, u32, u32, u32, String)> {
    let mut lines = Vec::new();
    {
        let Some((c, ox, oy)) = crop(f, area) else { return lines };
        let k = 2;
        let mut img = image::RgbaImage::from_raw(c.w, c.h, c.rgba).expect("crop size");
        if boost > 0 {
            for p in img.pixels_mut() {
                for ch in 0..3 {
                    p.0[ch] = ((p.0[ch] as i32 - 6).max(0) * boost).min(255) as u8;
                }
            }
        }
        let big = image::imageops::resize(&img, c.w * k, c.h * k, image::imageops::FilterType::CatmullRom);
        let Ok(words) = ocr.recognize(&Crop { w: c.w * k, h: c.h * k, rgba: big.into_raw() }) else { return lines };
        let mut by_line: std::collections::BTreeMap<u32, Vec<&ocr::Word>> = std::collections::BTreeMap::new();
        for w in &words {
            by_line.entry(w.line).or_default().push(w);
        }
        for ws in by_line.values_mut() {
            ws.sort_by_key(|w| w.rect.0);
            let text = ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            if text.chars().filter(|c| c.is_alphabetic()).count() < 2 {
                continue;
            }
            let x0 = ws.iter().map(|w| w.rect.0).min().unwrap_or(0) / k + ox;
            let y0 = ws.iter().map(|w| w.rect.1).min().unwrap_or(0) / k + oy;
            let x1 = ws.iter().map(|w| w.rect.0 + w.rect.2).max().unwrap_or(0) / k + ox;
            let y1 = ws.iter().map(|w| w.rect.1 + w.rect.3).max().unwrap_or(0) / k + oy;
            // OCR punctuation slips: "Mission;_Prologue" -> "Mission: Prologue"
            let text = text.replace('_', " ").replace(';', ":").replace(":", ": ");
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            lines.push((x0, y0, x1, y1, ocr::fix_contractions(&text)));
        }
    }
    lines
}

/// Lines -> menu items / paragraphs with their place, colours and size.
fn menu_blocks(f: &FrameData, lines: Vec<(u32, u32, u32, u32, String)>, split: u32, tab_h: u32) -> Vec<MenuText> {
    let mut blocks: Vec<(u32, u32, u32, u32, Vec<String>, bool, u32)> = Vec::new(); // x0,y0,x1,y1,lines,item,line_h
    for (x0, y0, x1, y1, t) in lines {
        let item = x0 < split;
        let h = y1 - y0;
        if !item {
            // a wrapped sentence continues on the next line: same left edge, right under it, the
            // previous line not finished (. ! ? :) and this one starting in lower case
            if let Some(b) = blocks.iter_mut().rev().find(|b| !b.5) {
                // (or simply set tightly, under half a line apart: menu rows are further apart)
                let unfinished = !b.4.last().is_some_and(|l| l.trim_end().ends_with(['.', '!', '?', ':']));
                let lower = t.chars().next().is_some_and(|c| c.is_lowercase());
                let gap = y0.saturating_sub(b.3);
                // a bullet ("— The Grotti Itali RSX") starts its own item
                let bullet = t.starts_with(['—', '–', '-', '•']);
                if unfinished && !bullet && x0.abs_diff(b.0) <= h && ((lower && gap < h) || gap * 2 < h) {
                    *b = (b.0.min(x0), b.1, b.2.max(x1), y1, b.4.clone(), false, b.6);
                    b.4.push(t);
                    continue;
                }
            }
        }
        blocks.push((x0, y0, x1, y1, vec![t], item, h));
    }
    let med = |v: &mut Vec<u8>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    // Which edge does each line share with others? Values and the labels left of the controller
    // line up on the right; lists and option names on the left. One text size for the whole menu
    // (OCR boxes are taller for "Display" than for "Audio").
    let tol = (f.w as f64 * 0.004) as u32;
    let right_of: Vec<bool> = blocks
        .iter()
        .map(|b| {
            let lefts = blocks.iter().filter(|o| !std::ptr::eq(*o, b) && o.0.abs_diff(b.0) <= tol).count();
            let rights = blocks.iter().filter(|o| !std::ptr::eq(*o, b) && o.2.abs_diff(b.2) <= tol && o.0.abs_diff(b.0) > tol).count();
            // short same-width words ("On", "Off") line up on both edges: near the panel's right
            // edge they're values, right-aligned
            rights > lefts || (b.0 as f64 > f.w as f64 * 0.6 && b.2 as f64 > f.w as f64 * 0.80)
        })
        .collect();
    let mut hs: Vec<u32> = blocks.iter().filter(|b| b.4.len() == 1).map(|b| b.6).collect();
    hs.sort_unstable();
    let menu_lh = hs.get(hs.len() / 2).copied().unwrap_or(20);
    blocks
        .into_iter()
        .zip(right_of)
        .map(|((x0, y0, x1, y1, ls, item, lh), right)| {
            // background: a ring just outside the text; letters: the brightest pixels inside
            // (capped: OCR's box around a big faint heading is inflated, which pushed its Arabic left)
            let pad = (lh.min(tab_h) / 2).max(3);
            let (mut br, mut bgc, mut bb) = (Vec::new(), Vec::new(), Vec::new());
            let (rx0, ry0) = (x0.saturating_sub(pad), y0.saturating_sub(pad / 2));
            let (rx1, ry1) = ((x1 + pad).min(f.w - 1), (y1 + pad / 2).min(f.h - 1));
            for y in ry0..=ry1 {
                for x in (rx0..=rx1).step_by(2) {
                    if x >= x0 && x <= x1 && y >= y0 && y <= y1 {
                        continue;
                    }
                    let (r, g, b) = px(f, x, y);
                    br.push(r);
                    bgc.push(g);
                    bb.push(b);
                }
            }
            let bgc_ = (med(&mut br.clone()), med(&mut bgc.clone()), med(&mut bb.clone()));
            let mut ins: Vec<(u8, u8, u8)> = (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).map(|(x, y)| px(f, x, y)).collect();
            // the letters are the pixels least like the background (dark text on the white selected row)
            let dist = |p: &(u8, u8, u8)| (p.0 as i32 - bgc_.0 as i32).abs() + (p.1 as i32 - bgc_.1 as i32).abs() + (p.2 as i32 - bgc_.2 as i32).abs();
            ins.sort_by_key(|p| std::cmp::Reverse(dist(p)));
            let top = &ins[..(ins.len() / 8).max(1)];
            let mut fr: Vec<u8> = top.iter().map(|p| p.0).collect();
            let mut fg: Vec<u8> = top.iter().map(|p| p.1).collect();
            let mut fb: Vec<u8> = top.iter().map(|p| p.2).collect();
            let rect = frac(f, rx0, ry0, rx1 - rx0, ry1 - ry0);
            MenuText {
                sig: region_sig(f, &rect),
                lines: ls.len() as u32,
                en: ls.join(" "),
                ar: String::new(),
                rect,
                bg: hex((med(&mut br), med(&mut bgc), med(&mut bb))),
                fg: hex((med(&mut fr), med(&mut fg), med(&mut fb))),
                // one size for the menu (from the tab bar); a heading much bigger than the rest
                // ("Replay") keeps its own
                // (a heading's height is measured on its letters: OCR's box around big faint text
                // is far too tall — 108 px for 60 px letters)
                line_h: if lh as f64 > 1.6 * menu_lh as f64 { letter_height(f, x0, y0, x1, y1) as f64 * 0.7 } else { tab_h as f64 * 0.44 } / f.h as f64,
                heading: lh as f64 > 1.6 * menu_lh as f64,
                missed: 0,
                item,
                right,
            }
        })
        .collect()
}

/// Stats values: durations ("0d 6h 14m 36s", read as "Od 6h 14m 36"), distances ("61.97 kilometers")
/// and counts ("IO / 30") in Arabic, without a translator. OCR's O / I / l in numbers become 0 / 1.
pub fn menu_rule(en: &str) -> Option<String> {
    let t = en.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let num = |w: &str| -> Option<String> {
        let n: String = w.chars().map(|c| match c { 'O' | 'o' => '0', 'I' | 'l' | '|' => '1', c => c }).collect();
        (n.chars().any(|c| c.is_ascii_digit()) && n.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',')).then_some(n)
    };
    let words: Vec<&str> = t.split_whitespace().collect();
    // durations: every word a number with a unit (d h m s); a cut-off last number is seconds
    let units: Vec<(String, &str)> = words
        .iter()
        .enumerate()
        .filter_map(|(i, w)| {
            let lw = w.to_lowercase();
            let (n, u) = match lw.chars().last()? {
                'd' => (&w[..w.len() - 1], "ي"),
                'h' => (&w[..w.len() - 1], "س"),
                'm' => (&w[..w.len() - 1], "د"),
                's' => (&w[..w.len() - 1], "ث"),
                _ if i + 1 == words.len() && i > 0 => (*w, "ث"),
                _ => return None,
            };
            Some((num(n)?, u))
        })
        .collect();
    if units.len() >= 2 && units.len() == words.len() {
        return Some(units.iter().map(|(n, u)| format!("{n}{u}")).collect::<Vec<_>>().join(" "));
    }
    // distances
    if words.len() == 2 {
        if let Some(n) = num(words[0]) {
            let unit = match words[1].to_lowercase().trim_end_matches('.') {
                "kilometers" | "kilometres" | "km" => Some("كم"),
                "meters" | "metres" | "m" => Some("م"),
                "miles" | "mile" => Some("ميل"),
                "feet" | "ft" => Some("قدم"),
                "times" => Some("مرة"),
                _ => None,
            };
            if let Some(u) = unit {
                return Some(format!("{n} {u}"));
            }
        }
    }
    // counts and plain numbers: "10 / 30", "52%"
    let parts: Vec<String> = words.iter().map(|w| if *w == "/" { Some("/".to_string()) } else { num(w.trim_end_matches('%')).map(|n| if w.ends_with('%') { format!("{n}%") } else { n }) }).collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod menu_rule_tests {
    use super::menu_rule as r;
    #[test]
    fn stats_values() {
        assert_eq!(r("Od 6h 14m 36").as_deref(), Some("0ي 6س 14د 36ث"));
        assert_eq!(r("- Od Oh 8m 18").as_deref(), Some("0ي 0س 8د 18ث"));
        assert_eq!(r("61.97 kilometers").as_deref(), Some("61.97 كم"));
        assert_eq!(r("IO / 30").as_deref(), Some("10 / 30"));
        assert_eq!(r("Total deaths"), None);
        assert_eq!(r("Distance traveled in cars"), None);
    }
}

/// A coarse picture of a region: 24 x 6 brightness samples. Cheap enough for every frame.
pub fn region_sig(f: &FrameData, r: &Rect) -> Vec<u8> {
    let (cols, rows) = (24u32, 6u32);
    let x0 = (r.x * f.w as f64) as u32;
    let y0 = (r.y * f.h as f64) as u32;
    let (w, h) = (((r.w * f.w as f64) as u32).max(1), ((r.h * f.h as f64) as u32).max(1));
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for j in 0..rows {
        for i in 0..cols {
            let x = (x0 + (2 * i + 1) * w / (2 * cols)).min(f.w - 1);
            let y = (y0 + (2 * j + 1) * h / (2 * rows)).min(f.h - 1);
            let (r_, g, b) = px(f, x, y);
            out.push(((r_ as u32 * 3 + g as u32 * 6 + b as u32) / 10) as u8);
        }
    }
    out
}

/// Does the region still show what it did when its signature was taken? A different line, a
/// highlight moving onto it, or the text gone all change it a lot; the blurred game behind
/// the menu barely moves.
pub fn region_same(f: &FrameData, r: &Rect, sig: &[u8]) -> bool {
    if sig.is_empty() {
        return true;
    }
    let now = region_sig(f, r);
    let diff: u32 = now.iter().zip(sig).map(|(a, b)| a.abs_diff(*b) as u32).sum();
    diff / (sig.len() as u32) < 6 // unchanged regions measure 0-1, changed ones 8 and up
}

/// The pause menu's content area (list + panel).
const CONTENT: Rect = Rect { x: 0.15, y: 0.212, w: 0.70, h: 0.64 };

/// A fine picture of the menu's content area (64 x 32 brightness samples): new text appearing
/// anywhere in it changes some samples a lot; the frozen, blurred game behind it doesn't.
pub fn content_sig(f: &FrameData) -> Vec<u8> {
    let (cols, rows) = (64u32, 32u32);
    let (x0, y0) = ((CONTENT.x * f.w as f64) as u32, (CONTENT.y * f.h as f64) as u32);
    let (w, h) = ((CONTENT.w * f.w as f64) as u32, (CONTENT.h * f.h as f64) as u32);
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for j in 0..rows {
        for i in 0..cols {
            let (r, g, b) = px(f, (x0 + (2 * i + 1) * w / (2 * cols)).min(f.w - 1), (y0 + (2 * j + 1) * h / (2 * rows)).min(f.h - 1));
            out.push(((r as u32 * 3 + g as u32 * 6 + b as u32) / 10) as u8);
        }
    }
    out
}

/// Has the menu's content changed since `sig` was taken? (a few samples changed a lot)
pub fn content_changed(f: &FrameData, sig: &[u8]) -> bool {
    !sig.is_empty() && content_sig(f).iter().zip(sig).filter(|(a, b)| a.abs_diff(**b) > 40).count() >= 3
}

/// Letter-level similarity 0..1 (edit distance).
fn ratio(a: &str, b: &str) -> f64 {
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

/// A list item (left column) as one of the menu's known words, even when OCR garbled it badly — a
/// greyed-out item's faint letters come out as "Join cruööds" for "Join Friends". The list is a
/// closed set of words, so a looser match is safe there: same first letters, ~60 % alike.
/// Returns (the English as the game writes it, its Arabic).
pub fn menu_item_word(en: &str) -> Option<(&'static str, &'static str)> {
    // faint letters come back accented ("Join croénds"): compare them plain
    let fold = |s: &str| -> String {
        letters(s)
            .chars()
            .map(|c| match c {
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'ö' | 'ó' | 'ò' | 'ô' => 'o',
                'ä' | 'á' | 'à' | 'â' | 'å' => 'a',
                'ü' | 'ú' | 'ù' => 'u',
                'í' | 'ì' | 'ï' => 'i',
                c => c,
            })
            .collect()
    };
    let k = fold(en);
    if k.len() < 3 {
        return None;
    }
    let words = menu_words();
    if let Some(hit) = words.iter().find(|(e, _)| fold(e) == k) {
        return Some(*hit);
    }
    words
        .iter()
        .map(|w| (w, fold(w.0)))
        .filter(|(_, l)| l.len() >= 4 && l.get(..3) == k.get(..3) && l.len().abs_diff(k.len()) <= 4)
        // ~60 % alike; ~50 % when the first four letters agree ("join…")
        .map(|(w, l)| (w, ratio(&l, &k), l.get(..4) == k.get(..4)))
        .filter(|(_, r, four)| *r >= if *four { 0.5 } else { 0.6 })
        .map(|(w, r, _)| (w, r))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(w, _)| *w)
}

#[cfg(test)]
mod menu_item_tests {
    use super::menu_item_word as w;
    #[test]
    fn faint_items_are_recognised() {
        for garbled in ["Join cruööds", "Join croénds", "JoinCeoéöds", "join cf qéödö", "Join erids"] {
            assert_eq!(w(garbled).map(|x| x.0), Some("Join Friends"), "{garbled}");
        }
        assert_eq!(w("Join Crew Members").map(|x| x.0), Some("Join Crew Members"));
        assert_eq!(w("Manage Characters").map(|x| x.0), Some("Manage Characters"));
        assert_eq!(w("Total deaths"), None);
    }
}

/// Height in pixels of the letters inside a box: the span of rows that hold the bright text
/// (rows whose brightest pixel is above halfway between the dimmest and brightest rows).
fn letter_height(f: &FrameData, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
    let lum = |x: u32, y: u32| {
        let (r, g, b) = px(f, x, y);
        (r as u32 * 3 + g as u32 * 6 + b as u32) / 10
    };
    let rows: Vec<u32> = (y0..=y1.min(f.h - 1)).map(|y| (x0..=x1.min(f.w - 1)).step_by(2).map(|x| lum(x, y)).max().unwrap_or(0)).collect();
    let (lo, hi) = (*rows.iter().min().unwrap_or(&0), *rows.iter().max().unwrap_or(&0));
    let thr = lo + (hi - lo) / 2;
    let first = rows.iter().position(|&v| v > thr);
    let last = rows.iter().rposition(|&v| v > thr);
    match (first, last) {
        (Some(a), Some(b)) if b > a => (b - a + 1) as u32,
        _ => y1 - y0,
    }
}

/// Read the pause menu again after its content changed, quickly: the tabs, the name block and
/// the buttons are kept from `old` (they haven't moved), and when the list on the left is
/// unchanged only the panel on the right is read (the Settings controller diagram swaps its
/// labels every few seconds). ~150 ms instead of ~420 ms for a full read.
pub fn reread_pause_menu(f: &FrameData, ocr: &Ocr, old: &ScreenLayout) -> Option<ScreenLayout> {
    let (_, _, _, sh) = selected_tab(f)?;
    let tab = selected_tab_name(old).unwrap_or("").to_string();
    let list_items: Vec<&MenuText> = old.menu.iter().filter(|m| m.item).collect();
    let list_same = tab != "STORE" && !list_items.is_empty() && list_items.iter().all(|m| region_same(f, &m.rect, &m.sig));
    let menu = if tab == "MAP" {
        Vec::new()
    } else if list_same {
        let split = (f.w as f64 * 0.388) as u32;
        let lines = read_lines(f, ocr, &Rect { x: 0.390, y: 0.212, w: 0.455, h: 0.64 }, 0);
        let mut menu: Vec<MenuText> = list_items.into_iter().cloned().collect();
        menu.extend(menu_blocks(f, lines, split, sh));
        menu
    } else {
        read_menu(f, ocr, sh, &tab)
    };
    // the buttons change with the page ("Back · Select" in a sub-menu): read them again when their row did
    let (buttons, buttons_row, buttons_sig) = if region_same(f, &BUTTON_ROW, &old.buttons_sig) {
        (old.buttons.clone(), old.buttons_row.clone(), old.buttons_sig.clone())
    } else {
        let b = find_buttons(f, ocr);
        let row = row_of(&b);
        (b, row, region_sig(f, &BUTTON_ROW))
    };
    Some(ScreenLayout { menu, content_sig: content_sig(f), buttons, buttons_row, buttons_sig, ..old.clone() })
}

fn row_of(buttons: &[Button]) -> Option<Rect> {
    (!buttons.is_empty()).then(|| {
        let x0 = buttons.iter().map(|b| b.text_rect.x).fold(f64::MAX, f64::min);
        let x1 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.x + i.w).unwrap_or(b.text_rect.x + b.text_rect.w)).fold(0.0, f64::max);
        let y0 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.y).unwrap_or(b.text_rect.y)).fold(f64::MAX, f64::min);
        let y1 = buttons.iter().map(|b| b.icon_rect.as_ref().map(|i| i.y + i.h).unwrap_or(b.text_rect.y + b.text_rect.h)).fold(0.0, f64::max);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    })
}

/// Id of the centre-caption page ("Ludendorff, North Yankton, nine years ago.").
pub const CAPTION_ID: &str = "gta5_caption";

/// GTA V's centre captions: white text in the middle of an otherwise black screen. Pixels only, no
/// OCR (cheap enough for every frame): returns the text's pixel box (x0, y0, x1, y1).
/// Anything else on screen (a game scene, a menu, a subtitle at the bottom, a red title) fails it.
pub fn find_caption(f: &FrameData) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (f.w, f.h);
    let step = 2usize;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let (mut text, mut other) = (0u32, 0u32);
    for y in ((h * 3 / 100)..(h * 97 / 100)).step_by(step) {
        for x in ((w * 3 / 100)..(w * 97 / 100)).step_by(step) {
            let (r, g, b) = px(f, x, y);
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            if hi > 120 && hi - lo < 40 {
                text += 1;
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            } else if hi > 40 {
                other += 1; // letter edges, or a picture
            }
        }
    }
    // a few words' worth of white, and not much else lit (edges of the letters only)
    if text < 40 || other > text * 3 + 60 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as f64 / w as f64, (y1 - y0) as f64 / h as f64);
    let (cx, cy) = ((x0 + x1) as f64 / 2.0 / w as f64, (y0 + y1) as f64 / 2.0 / h as f64);
    // centred, mid-screen, one to three lines
    if !(0.04..=0.8).contains(&bw) || !(0.012..=0.15).contains(&bh) || (cx - 0.5).abs() > 0.03 || !(0.25..=0.75).contains(&cy) {
        return None;
    }
    // text, not a white shape: letters cover only part of their box
    let fill = (text as f64 * (step * step) as f64) / ((x1 - x0 + 1) as f64 * (y1 - y0 + 1) as f64);
    if !(0.04..=0.6).contains(&fill) {
        return None;
    }
    Some((x0, y0, x1, y1))
}

/// Is the caption still the one in `old` (same box, give or take a few pixels as it fades)?
pub fn caption_same(f: &FrameData, b: (u32, u32, u32, u32), old: &ScreenLayout) -> bool {
    let r = frac(f, b.0, b.1, b.2 - b.0 + 1, b.3 - b.1 + 1);
    let t = &old.title;
    let tol = 0.012;
    (r.x - t.x).abs() < tol && (r.y - t.y).abs() < tol && (r.x + r.w - t.x - t.w).abs() < tol && (r.y + r.h - t.y - t.h).abs() < tol
}

/// Read a centre caption. The page's `title` holds the caption's pixel box (for `caption_same`; no
/// title is drawn) and `reason` the English text, so the Arabic is drawn centred under it like the
/// Mission Failed reason.
pub fn detect_gta5_caption(f: &FrameData, ocr: &Ocr) -> Option<ScreenLayout> {
    let b = find_caption(f)?;
    let title = frac(f, b.0, b.1, b.2 - b.0 + 1, b.3 - b.1 + 1);
    let pad = Rect { x: title.x - 0.02, y: title.y - 0.015, w: title.w + 0.04, h: title.h + 0.03 };
    let (rc, ox, oy) = crop(f, &pad)?;
    let k = if f.h < 1600 { 2 } else { 1 };
    let rc = if k == 1 {
        rc
    } else {
        let img = image::RgbaImage::from_raw(rc.w, rc.h, rc.rgba).expect("crop size");
        let big = image::imageops::resize(&img, rc.w * k, rc.h * k, image::imageops::FilterType::CatmullRom);
        Crop { w: rc.w * k, h: rc.h * k, rgba: big.into_raw() }
    };
    let mut words = ocr.recognize(&rc).ok()?;
    for w in words.iter_mut() {
        w.rect = (w.rect.0 / k, w.rect.1 / k, w.rect.2 / k, w.rect.3 / k);
    }
    let text = ocr::clean(&words);
    if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[screens] caption box {title:?}, OCR read {text:?}");
    }
    if text.chars().filter(|c| c.is_alphabetic()).count() < 4 {
        return None;
    }
    let x0 = words.iter().map(|w| w.rect.0).min().unwrap_or(0);
    let y0 = words.iter().map(|w| w.rect.1).min().unwrap_or(0);
    let x1 = words.iter().map(|w| w.rect.0 + w.rect.2).max().unwrap_or(0);
    let y1 = words.iter().map(|w| w.rect.1 + w.rect.3).max().unwrap_or(0);
    let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
    hs.sort_unstable();
    Some(ScreenLayout {
        id: CAPTION_ID.into(),
        title,
        title_color: String::new(),
        title_ar: String::new(),
        reason: Some(frac(f, ox + x0, oy + y0, x1 - x0, y1 - y0)),
        reason_line_h: hs[(hs.len() * 3 / 4).min(hs.len() - 1)] as f64 / f.h as f64,
        reason_en: text,
        buttons: Vec::new(),
        buttons_row: None,
        labels: Vec::new(),
        player: None,
        menu: Vec::new(),
        content_sig: Vec::new(),
        buttons_sig: Vec::new(),
    })
}

/// Id of GTA V's alert page (yellow "alert" title on black: "Please reconnect controller").
pub const ALERT_ID: &str = "gta5_alert";

fn is_alert_yellow((r, g, b): (u8, u8, u8)) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    r > 180 && g > 150 && b < 140 && r - b > 80
}

/// The yellow "alert" title in the middle of a dark screen: its pixel box and median colour.
/// Pixels only, cheap enough for every frame.
fn find_alert_title(f: &FrameData) -> Option<((u32, u32, u32, u32), (u8, u8, u8))> {
    if !is_dark(f) {
        return None;
    }
    let (w, h) = (f.w, f.h);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let (mut rs, mut gs, mut bs) = (Vec::new(), Vec::new(), Vec::new());
    for y in ((h / 5)..(h * 3 / 4)).step_by(2) {
        for x in ((w / 5)..(w * 4 / 5)).step_by(2) {
            let p = px(f, x, y);
            if is_alert_yellow(p) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                rs.push(p.0);
                gs.push(p.1);
                bs.push(p.2);
            }
        }
    }
    if rs.len() < 150 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as f64 / w as f64, (y1 - y0) as f64 / h as f64);
    let cx = (x0 + x1) as f64 / 2.0 / w as f64;
    if !(0.06..=0.45).contains(&bw) || !(0.03..=0.14).contains(&bh) || (cx - 0.5).abs() > 0.04 {
        return None;
    }
    let med = |v: &mut Vec<u8>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    Some(((x0, y0, x1, y1), (med(&mut rs), med(&mut gs), med(&mut bs))))
}

/// Cheap per-frame check: is the alert page still up?
pub fn alert_present(f: &FrameData) -> bool {
    find_alert_title(f).is_some()
}

/// GTA V's alert page: the yellow title, the message under it (between two white rules) and the
/// buttons, if any. The message is the page's `reason`; its box reaches down to the lower rule,
/// so the translation is drawn below the frame, not over the line.
pub fn detect_gta5_alert(f: &FrameData, ocr: &Ocr) -> Option<ScreenLayout> {
    let ((tx0, ty0, tx1, ty1), (r, g, b)) = find_alert_title(f)?;
    let title = frac(f, tx0, ty0, tx1 - tx0 + 1, ty1 - ty0 + 1);
    // white horizontal rules under the title (the message sits between them)
    let (w, h) = (f.w, f.h);
    let rule = |y: u32| {
        let (mut run, mut best) = (0u32, 0u32);
        for x in (w / 5)..(w * 4 / 5) {
            let (r, g, b) = px(f, x, y);
            if r.min(g).min(b) > 190 { run += 1; best = best.max(run); } else { run = 0; }
        }
        best > w / 6
    };
    let mut rules = Vec::new();
    let mut y = ty1 + 2;
    while y < (ty1 + h / 6).min(h - 1) && rules.len() < 2 {
        if rule(y) {
            rules.push(y);
            y += h / 100; // past this line's thickness
        } else {
            y += 1;
        }
    }
    let (my0, my1) = match rules[..] {
        [a, b] => (a + 2, b.saturating_sub(2)),
        [a] => (a + 2, (a + h / 12).min(h - 1)),
        _ => (ty1 + 2, (ty1 + h / 10).min(h - 1)),
    };
    let mut reason = None;
    let mut reason_en = String::new();
    let mut reason_line_h = 0.0;
    let area = frac(f, w / 5, my0, w * 3 / 5, my1.saturating_sub(my0).max(1));
    if let Some((c, ox, oy)) = crop(f, &area) {
        let k = if f.h < 1600 { 2 } else { 1 };
        let img = image::RgbaImage::from_raw(c.w, c.h, c.rgba).expect("crop size");
        let big = image::imageops::resize(&img, c.w * k, c.h * k, image::imageops::FilterType::CatmullRom);
        if let Ok(mut words) = ocr.recognize(&Crop { w: c.w * k, h: c.h * k, rgba: big.into_raw() }) {
            for wd in words.iter_mut() {
                wd.rect = (wd.rect.0 / k, wd.rect.1 / k, wd.rect.2 / k, wd.rect.3 / k);
            }
            let text = ocr::clean(&words);
            if text.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
                let x0 = words.iter().map(|w| w.rect.0).min().unwrap_or(0);
                let y0 = words.iter().map(|w| w.rect.1).min().unwrap_or(0);
                let x1 = words.iter().map(|w| w.rect.0 + w.rect.2).max().unwrap_or(0);
                let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
                hs.sort_unstable();
                reason_line_h = hs[(hs.len() * 3 / 4).min(hs.len() - 1)] as f64 / f.h as f64;
                // (down to the lower rule, or just under the text when there is none)
                let bottom = if rules.len() == 2 { rules[1] + 3 } else { oy + words.iter().map(|w| w.rect.1 + w.rect.3).max().unwrap_or(0) };
                reason = Some(frac(f, ox + x0, oy + y0, x1 - x0, bottom.saturating_sub(oy + y0)));
                reason_en = text;
            }
        }
    }
    if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[screens] alert title {title:?}, rules {rules:?}, message {reason_en:?}");
    }
    let buttons = find_buttons(f, ocr);
    let buttons_row = row_of(&buttons);
    Some(ScreenLayout {
        id: ALERT_ID.into(),
        title,
        title_color: format!("#{r:02x}{g:02x}{b:02x}"),
        title_ar: "تنبيه".into(),
        reason,
        reason_line_h,
        reason_en,
        buttons,
        buttons_row,
        labels: Vec::new(),
        player: None,
        menu: Vec::new(),
        content_sig: Vec::new(),
        buttons_sig: Vec::new(),
    })
}
