// pages-version: 1
// Grand Theft Auto V: the game's own pages the app translates — Mission Failed, the alert page, the
// pause menu and captions on a black screen (see src/pagescript.rs for the API).
// The app carries this file; a newer signed one can come from the server with a layout update.

// ------------------------------------------------------------------ captions on a black screen
// Place and time cards ("Ludendorff, North Yankton, nine years ago."): white text, centred, mid-screen,
// on black. The translation is drawn centred just below the English, like the Mission Failed reason.

/** The caption's box (frame pixels, inclusive) or null. */
function findCaption(f) {
  const x0 = Math.floor((f.w * 3) / 100), x1 = Math.floor((f.w * 97) / 100);
  const y0 = Math.floor((f.h * 3) / 100), y1 = Math.floor((f.h * 97) / 100);
  const area = { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  // white-ish text, and anything else that is lit (letter edges, or a picture)
  const text = f.pixels(area, { step: 2, bright: [121, 255], spread: [0, 39] });
  const lit = f.pixels(area, { step: 2, bright: [41, 255] });
  const other = lit.count - text.count;
  // a few words' worth of white, and not much else lit (edges of the letters only)
  if (text.count < 40 || other > text.count * 3 + 60) return null;
  const b = text.box;
  const [bx0, by0, bx1, by1] = [b.x, b.y, b.x + b.w - 1, b.y + b.h - 1];
  const bw = (bx1 - bx0) / f.w, bh = (by1 - by0) / f.h;
  const cx = (bx0 + bx1) / 2 / f.w, cy = (by0 + by1) / 2 / f.h;
  // centred, mid-screen, one to three lines
  if (bw < 0.04 || bw > 0.8 || bh < 0.012 || bh > 0.15 || Math.abs(cx - 0.5) > 0.03 || cy < 0.25 || cy > 0.75) return null;
  // text, not a white shape: letters cover only part of their box
  const fill = (text.count * 4) / ((bx1 - bx0 + 1) * (by1 - by0 + 1));
  if (fill < 0.04 || fill > 0.6) return null;
  return { x0: bx0, y0: by0, x1: bx1, y1: by1 };
}

const asBox = (f, b) => f.box(b.x0, b.y0, b.x1 - b.x0 + 1, b.y1 - b.y0 + 1);

pages.add({
  id: "gta5_caption",
  option: "captions",

  present(f) {
    return findCaption(f);
  },

  // The page's `title` holds the caption's box (to tell when it changes; no title is drawn) and
  // `reason` the English, so the translation is drawn centred under it.
  read(f, b) {
    if (!b) return null;
    const title = asBox(f, b);
    const pad = { x: title.x - 0.02, y: title.y - 0.015, w: title.w + 0.04, h: title.h + 0.03 };
    const clamp = (v) => Math.min(Math.max(v, 0), 1);
    const px0 = Math.floor(clamp(pad.x) * f.w), py0 = Math.floor(clamp(pad.y) * f.h);
    const px1 = Math.floor(clamp(pad.x + pad.w) * f.w), py1 = Math.floor(clamp(pad.y + pad.h) * f.h);
    // small text reads better enlarged (below 4K)
    const r = f.ocr({ x: px0, y: py0, w: px1 - px0, h: py1 - py0 }, { scale: f.h < 1600 ? 2 : 1 });
    if ((r.text.match(/\p{Alphabetic}/gu) || []).length < 4 || !r.words.length) return null;
    const x0 = Math.min(...r.words.map((w) => w.x)), y0 = Math.min(...r.words.map((w) => w.y));
    const x1 = Math.max(...r.words.map((w) => w.x + w.w)), y1 = Math.max(...r.words.map((w) => w.y + w.h));
    const hs = r.words.map((w) => w.h).sort((a, b) => a - b);
    return {
      title,
      reason: f.box(x0, y0, x1 - x0, y1 - y0),
      reason_line_h: hs[Math.min(Math.floor((hs.length * 3) / 4), hs.length - 1)] / f.h,
      reason_en: r.text,
    };
  },

  // While it's up: the same caption as long as its box hasn't moved.
  same(f, old) {
    const b = findCaption(f);
    if (!b) return false;
    const r = asBox(f, b), t = old.title, tol = 0.012;
    return Math.abs(r.x - t.x) < tol && Math.abs(r.y - t.y) < tol && Math.abs(r.x + r.w - t.x - t.w) < tol && Math.abs(r.y + r.h - t.y - t.h) < tol;
  },
});

// ------------------------------------------------------------------ shared pieces

const RED = { r: [111, 255], rg: [71, 255], rb: [61, 255] };     // the title's red, also faded / dark
const YELLOW = { r: [181, 255], g: [151, 255], b: [0, 139], rb: [81, 255] };
const hex = (c) => "#" + [c.r, c.g, c.b].map((v) => v.toString(16).padStart(2, "0")).join("");
const clamp01 = (v) => Math.min(Math.max(v, 0), 1);
/** A rectangle given as fractions → the same pixels the app's own crop takes. */
function pxRect(f, r) {
  const x0 = Math.floor(clamp01(r.x) * f.w), y0 = Math.floor(clamp01(r.y) * f.h);
  const x1 = Math.floor(clamp01(r.x + r.w) * f.w), y1 = Math.floor(clamp01(r.y + r.h) * f.h);
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}
/** The box around OCR words (frame pixels) and one line's height (a tall-ish word: 75th percentile). */
function wordsBox(words) {
  const x0 = Math.min(...words.map((w) => w.x)), y0 = Math.min(...words.map((w) => w.y));
  const x1 = Math.max(...words.map((w) => w.x + w.w)), y1 = Math.max(...words.map((w) => w.y + w.h));
  const hs = words.map((w) => w.h).sort((a, b) => a - b);
  return { x0, y0, x1, y1, lineH: hs[Math.min(Math.floor((hs.length * 3) / 4), hs.length - 1)] };
}

/** Is the frame (almost) black? */
function isDark(f) {
  return f.mean({ x: Math.floor(f.w / 10), y: Math.floor(f.h / 10), w: Math.floor((f.w * 9) / 10) - Math.floor(f.w / 10), h: Math.floor((f.h * 9) / 10) - Math.floor(f.h / 10) }, { step: 8 }) < 25;
}

// ------------------------------------------------------------------ buttons, bottom right
// "Skip ▢  Restart △  Retry ✕" (Skip only after a few failures), or "No ◯  Yes ✕" on the confirmation.
// Found by shape (white word blobs, each followed by a square-ish icon); each word is then read on
// its own. A word that can't be read gets no translation rather than a guess.

function readButtonWord(f, word, row) {
  const m = 10;
  const x0 = Math.max(word.x - m, row.x), y0 = Math.max(word.y - m, row.y);
  const x1 = Math.min(word.x + word.w - 1 + m, row.x + row.w - 1), y1 = Math.min(word.y + word.h - 1 + m, row.y + row.h - 1);
  for (const scale of [1, 2]) {
    const r = f.ocr({ x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 }, { isolate: 40, scale, filter: "lanczos" });
    const hit = vi.buttonWord(r.words.map((w) => w.text).join(""));
    if (hit) return hit;
  }
  return null;
}

function findButtons(f) {
  const row = pxRect(f, { x: 0.55, y: 0.86, w: 0.45, h: 0.13 });
  if (row.w < 8 || row.h < 8) return [];
  const gap = Math.floor(Math.max(f.h * 0.003, 3));
  const blobs = f.blobs(row, { gap, bright: [141, 255] });
  const minIcon = Math.floor(f.h * 0.012);
  // round buttons ~1:1; L1 / R1 are rounded rectangles up to ~1.6:1
  const isIcon = (b) => b.h >= minIcon && b.w / b.h > 0.75 && b.w / b.h < 1.65;
  const pairs = [];
  let i = 0;
  while (i + 1 < blobs.length) {
    let word = blobs[i];
    if (!isIcon(word) && word.w > word.h && isIcon(blobs[i + 1])) {
      // a label of several words ("Hide Menu"): take in the words just before it
      let k = i;
      while (k > 0 && !isIcon(blobs[k - 1]) && Math.max(word.x - (blobs[k - 1].x + blobs[k - 1].w - 1), 0) <= word.h) {
        const b = blobs[k - 1];
        const y0 = Math.min(b.y, word.y), y1 = Math.max(b.y + b.h, word.y + word.h);
        word = { x: b.x, y: y0, w: word.x + word.w - b.x, h: y1 - y0 };
        k--;
      }
      const icons = [];
      let j = i + 1;
      while (j < blobs.length && isIcon(blobs[j])) icons.push(blobs[j++]);
      pairs.push([word, icons]);
      i = j;
    } else {
      i++;
    }
  }
  pairs.reverse(); // right to left: the order an Arabic reader meets them
  const out = [];
  for (const [word, icons] of pairs.slice(0, 5)) {
    const hit = readButtonWord(f, word, row);
    if (!hit) continue;
    const ic = icons[0];
    out.push({
      en: hit[0], ar: hit[1],
      icon: f.icon(ic, 140),
      icon_rect: f.box(ic.x, ic.y, ic.w, ic.h),
      text_rect: f.box(word.x, word.y, word.w, word.h),
      more_icons: icons.slice(1).map((b) => f.icon(b, 140)).filter(Boolean),
    });
  }
  return out;
}

/** The box around the buttons (icons included). */
function buttonsRow(buttons) {
  if (!buttons.length) return null;
  const right = (b) => (b.icon_rect ? b.icon_rect.x + b.icon_rect.w : b.text_rect.x + b.text_rect.w);
  const top = (b) => (b.icon_rect ? b.icon_rect.y : b.text_rect.y);
  const bottom = (b) => (b.icon_rect ? b.icon_rect.y + b.icon_rect.h : b.text_rect.y + b.text_rect.h);
  const x0 = Math.min(...buttons.map((b) => b.text_rect.x)), x1 = Math.max(0, ...buttons.map(right));
  const y0 = Math.min(...buttons.map(top)), y1 = Math.max(0, ...buttons.map(bottom));
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

// ------------------------------------------------------------------ "MISSION FAILED"
// The red title starts over the live (often bright) scene, so a dark screen isn't required: the
// densest band of red in the middle of the screen, confirmed by OCR. Then the reason under it, and
// the buttons.

/** The red title's box (frame pixels, inclusive) and its median colour, or null. */
function findRedTitle(f) {
  const x0 = Math.floor(f.w * 0.12), x1 = Math.floor(f.w * 0.88);
  const y0 = Math.floor(f.h * 0.22), y1 = Math.floor(f.h * 0.72);
  const step = 4;
  const rows = f.profile({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, { axis: "rows", step, ...RED });
  if (!rows.length) return null;
  const peak = Math.max(...rows);
  // the title spans at least a fifth of the screen width
  if (peak * step < Math.floor(f.w / 5)) return null;
  // contiguous dense rows around the densest one
  const pi = rows.indexOf(peak);
  const dense = (i) => rows[i] * 4 >= peak;
  let top = pi, bot = pi;
  while (top > 0 && dense(top - 1)) top--;
  while (bot + 1 < rows.length && dense(bot + 1)) bot++;
  const by0 = y0 + top * step, by1 = y0 + bot * step + step - 1;
  const bh = (by1 - by0) / f.h;
  if (bh < 0.03 || bh > 0.2) return null;
  // exact box at full resolution, a bit above the band too (tall letters stick up past it)
  const sy0 = Math.max(by0 - Math.floor(f.h * 0.04), 0), sy1 = Math.min(by1 + step * 2, f.h - 1);
  const area = { x: x0, y: sy0, w: x1 - x0, h: sy1 - sy0 + 1 };
  const rowN = f.profile(area, { axis: "rows", ...RED });
  const colN = f.profile(area, { axis: "cols", ...RED });
  // grow from the dense band outward while rows stay red (stops at the first empty row)
  const rowMin = Math.max(Math.floor(f.w / 400), 3);
  let t0 = by0 - sy0, t1 = Math.min(by1 - sy0, rowN.length - 1);
  while (t0 > 0 && rowN[t0 - 1] >= rowMin) t0--;
  while (t1 + 1 < rowN.length && rowN[t1 + 1] >= rowMin) t1++;
  const first = colN.findIndex((c) => c >= 3), last = colN.length - 1 - [...colN].reverse().findIndex((c) => c >= 3);
  if (first < 0) return null;
  const tx0 = x0 + first, tx1 = x0 + last;
  const tw = (tx1 - tx0) / f.w;
  if (tw < 0.2 || tw > 0.76) return null;
  const color = f.pixels(area, { median: true, ...RED }).median;
  return { box: [tx0, sy0 + t0, tx1, sy0 + t1], color };
}

pages.add({
  id: "gta5_mission_failed",
  option: "mission_failed",
  // Once shown it stays while the title blinks out as the scene fades; it ends when the screen has
  // gone dark (or 5 s without the title). Read again each second for new words.
  hold: { ms: 5000, endWhenDark: true, rereadMs: 1000 },

  present(f) {
    return findRedTitle(f);
  },

  read(f, found) {
    if (!found) return null;
    const [tx0, ty0, tx1, ty1] = found.box;
    const title = f.box(tx0, ty0, tx1 - tx0 + 1, ty1 - ty0 + 1);
    // Confirm with OCR. The title letters are huge: read at normal text size. Its font reads loosely
    // ("misoneiled"), so a recognisable piece of either word is enough with the red band.
    const pad = pxRect(f, { x: title.x - 0.01, y: title.y - 0.01, w: title.w + 0.02, h: title.h + 0.02 });
    if (pad.w < 8 || pad.h < 8) return null;
    const t = f.ocr(pad, { shrink: 90 });
    const read = vi.letters(t.words.map((w) => w.text).join(""));
    if (!["miss", "mis", "sion", "fail", "aile", "iled"].some((p) => read.includes(p))) return null;

    // the reason: white text just under the title (small text reads better enlarged)
    let reason = null, reason_en = "", reason_line_h = 0;
    const rr = pxRect(f, { x: 0.12, y: title.y + title.h + 0.002, w: 0.76, h: 0.08 });
    if (rr.w >= 8 && rr.h >= 8) {
      const r = f.ocr(rr, { scale: f.h < 1600 ? 2 : 1 });
      if (r.text && r.words.length) {
        const b = wordsBox(r.words);
        reason = f.box(b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0);
        reason_en = r.text;
        reason_line_h = b.lineH / f.h;
      }
    }
    const buttons = findButtons(f);
    return {
      title,
      // the title's red as it is in this picture (it differs between sources)
      title_color: hex(found.color),
      title_ar: "فشلت المهمة",
      reason, reason_line_h, reason_en,
      buttons, buttons_row: buttonsRow(buttons),
    };
  },

  same(f) {
    return findRedTitle(f) !== null;
  },
});

// ------------------------------------------------------------------ alert page
// The yellow title in the middle of a dark screen, the message under it (between two white rules)
// and the buttons, if any.

function findAlertTitle(f) {
  if (!isDark(f)) return null;
  const x0 = Math.floor(f.w / 5), x1 = Math.floor((f.w * 4) / 5);
  const y0 = Math.floor(f.h / 5), y1 = Math.floor((f.h * 3) / 4);
  const p = f.pixels({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, { step: 2, median: true, ...YELLOW });
  if (p.count < 150) return null;
  const b = p.box;
  const [bx0, by0, bx1, by1] = [b.x, b.y, b.x + b.w - 1, b.y + b.h - 1];
  const bw = (bx1 - bx0) / f.w, bh = (by1 - by0) / f.h, cx = (bx0 + bx1) / 2 / f.w;
  if (bw < 0.06 || bw > 0.45 || bh < 0.03 || bh > 0.14 || Math.abs(cx - 0.5) > 0.04) return null;
  return { box: [bx0, by0, bx1, by1], color: p.median };
}

pages.add({
  id: "gta5_alert",
  option: "mission_failed",
  // gone the moment its yellow title is (the page is dark itself); the message is read again once a
  // second (it can change), the title stays put
  hold: { ms: 0, endWhenDark: false, rereadMs: 1000 },

  present(f) {
    return findAlertTitle(f);
  },

  read(f, found) {
    if (!found) return null;
    const [tx0, ty0, tx1, ty1] = found.box;
    const title = f.box(tx0, ty0, tx1 - tx0 + 1, ty1 - ty0 + 1);
    const W = f.w, H = f.h;
    // the white rules under the title (the message sits between them)
    const xa = Math.floor(W / 5), xb = Math.floor((W * 4) / 5);
    const isRule = (y) => f.run({ x: xa, y, w: xb - xa, h: 1 }, { low: [191, 255] }) > Math.floor(W / 6);
    const rules = [];
    let y = ty1 + 2;
    while (y < Math.min(ty1 + Math.floor(H / 6), H - 1) && rules.length < 2) {
      if (isRule(y)) {
        rules.push(y);
        y += Math.floor(H / 100); // past this line's thickness
      } else {
        y += 1;
      }
    }
    let my0, my1;
    if (rules.length === 2) [my0, my1] = [rules[0] + 2, Math.max(rules[1] - 2, 0)];
    else if (rules.length === 1) [my0, my1] = [rules[0] + 2, Math.min(rules[0] + Math.floor(H / 12), H - 1)];
    else [my0, my1] = [ty1 + 2, Math.min(ty1 + Math.floor(H / 10), H - 1)];
    let reason = null, reason_en = "", reason_line_h = 0;
    const area = pxRect(f, f.box(Math.floor(W / 5), my0, Math.floor((W * 3) / 5), Math.max(my1 - my0, 1)));
    if (area.w >= 8 && area.h >= 8) {
      const r = f.ocr(area, { scale: H < 1600 ? 2 : 1 });
      if ((r.text.match(/\p{Alphabetic}/gu) || []).length >= 3 && r.words.length) {
        const b = wordsBox(r.words);
        reason_line_h = b.lineH / H;
        // down to the lower rule, or just under the text when there is none
        const bottom = rules.length === 2 ? rules[1] + 3 : b.y1;
        reason = f.box(b.x0, b.y0, b.x1 - b.x0, Math.max(bottom - b.y0, 0));
        reason_en = r.text;
      }
    }
    const buttons = findButtons(f);
    return {
      title, title_color: hex(found.color), title_ar: "تنبيه",
      reason, reason_line_h, reason_en,
      buttons, buttons_row: buttonsRow(buttons),
    };
  },

  same(f) {
    return findAlertTitle(f) !== null;
  },
});

// ------------------------------------------------------------------ pause menu
// Its tabs become translated replicas in the game's colours, the character / day / time block (top
// right) and the menu's content (list on the left, panel on the right) get translations, and its
// buttons (bottom right) get translated labels. The app follows it while it's up (what changed,
// tab states seen before) and calls `reread` for a quick read of just the menu.

const WHITE = { low: [226, 255], spread: [0, 24] };
const alpha = (s) => (s.match(/\p{Alphabetic}/gu) || []).join("");
const PLAYERS = [["MICHAEL", "مايكل"], ["FRANKLIN", "فرانكلين"], ["TREVOR", "تريفور"]];
const DAYS = [["MON", "الاثنين"], ["TUE", "الثلاثاء"], ["WED", "الأربعاء"], ["THU", "الخميس"], ["FRI", "الجمعة"], ["SAT", "السبت"], ["SUN", "الأحد"]];
const LIST = { x: 0.155, y: 0.212, w: 0.231, h: 0.64 };
const PANEL = { x: 0.390, y: 0.212, w: 0.455, h: 0.64 };
const STORE = { x: 0.155, y: 0.212, w: 0.455, h: 0.64 };

/** The selected (white) tab: [x, y, w, h] in frame pixels, or null. Colour only, every frame. */
function selectedTab(f) {
  const x0 = Math.floor(f.w * 0.12), x1 = Math.floor(f.w * 0.88);
  const ymin = Math.floor(f.h * 0.12), ymax = Math.floor(f.h * 0.26);
  const wmin = Math.floor(f.w * 0.08), wmax = Math.floor(f.w * 0.14);
  // rows whose longest white run is tab-sized
  const runs = f.runs({ x: x0, y: ymin, w: x1 - x0, h: ymax - ymin }, WHITE);
  const rows = [];
  runs.forEach(([s, n], i) => { if (n >= wmin && n <= wmax) rows.push({ y: ymin + i, s, n }); });
  // the same run (same start and length, ±3 px) over several rows: its outer rows are the tab
  const same = (a, b) => Math.abs(a.s - b.s) <= 3 && Math.abs(a.n - b.n) <= 3;
  const key = rows.find((r) => rows.filter((o) => same(r, o)).length >= 8);
  if (!key) return null;
  const ys = rows.filter((o) => same(key, o)).map((o) => o.y);
  const y0 = Math.min(...ys), y1 = Math.max(...ys);
  return y1 - y0 >= f.h * 0.025 ? [key.s, y0, key.n, y1 - y0 + 1] : null;
}

/** "MICHAEL" over "MONDAY 05:00", top right (small letters, read enlarged ×3). */
function readPlayer(f) {
  const area = pxRect(f, { x: 0.62, y: 0.06, w: 0.18, h: 0.10 });
  if (area.w < 8 || area.h < 8) return null;
  const r = f.ocr(area, { scale: 3, prep: "brightest", raw: true });
  const k = r.k || 3;
  const byLine = new Map();
  for (const w of r.words) {
    if (!byLine.has(w.line)) byLine.set(w.line, []);
    byLine.get(w.line).push(w);
  }
  const lines = [...byLine.keys()].sort((a, b) => a - b).map((key) => {
    const ws = byLine.get(key);
    return {
      top: Math.floor(Math.min(...ws.map((w) => w.y)) / k), bottom: Math.floor(Math.max(...ws.map((w) => w.y + w.h)) / k),
      x0: Math.floor(Math.min(...ws.map((w) => w.x)) / k), x1: Math.floor(Math.max(...ws.map((w) => w.x + w.w)) / k),
      text: ws.map((w) => w.text).join(" "),
    };
  });
  lines.sort((a, b) => a.top - b.top);
  const nameI = lines.findIndex((l) => PLAYERS.some(([n]) => l.text.toUpperCase().includes(n.slice(0, 4))));
  if (nameI < 0) return null;
  const name = PLAYERS.find(([n]) => lines[nameI].text.toUpperCase().includes(n.slice(0, 4)));
  const en = [lines[nameI].text], ar = [name[1]], used = [nameI];
  // the day / time line right under it
  const j = lines.findIndex((l) => l.top > lines[nameI].bottom);
  if (j >= 0) {
    const t = lines[j].text.toUpperCase();
    const day = DAYS.find(([d]) => t.includes(d));
    const digits = (t.match(/[0-9]/g) || []).join("");
    let time = null;
    if (digits.length >= 3) {
      const d = digits.slice(Math.max(digits.length - 4, 0));
      time = `${d.slice(0, d.length - 2).padStart(2, "0")}:${d.slice(d.length - 2)}`;
    }
    const line = [day && day[1], time].filter(Boolean);
    if (line.length) {
      en.push(lines[j].text);
      ar.push(line.join(" "));
      used.push(j);
    }
  }
  const ls = used.map((i) => lines[i]);
  const x0 = Math.min(...ls.map((l) => l.x0)), x1 = Math.max(...ls.map((l) => l.x1));
  const y0 = Math.min(...ls.map((l) => l.top)), y1 = Math.max(...ls.map((l) => l.bottom));
  const lineH = (ls[0].bottom - ls[0].top) / f.h;
  return {
    en, ar,
    rect: f.box(r.x0 + x0, r.y0 + y0, x1 - x0, y1 - y0),
    line_h: lineH,
    pitch: ls.length > 1 ? (ls[1].top - ls[0].top) / f.h : lineH * 1.4,
  };
}

/** One part of the menu, enlarged ×2 (brightness × boost when > 0): its lines in frame pixels. */
function readLines(f, area, boost) {
  const px = pxRect(f, area);
  if (px.w < 8 || px.h < 8) return [];
  const r = f.ocr(px, { scale: 2, raw: true, boost: boost || 0 });
  const k = r.k || 2;
  const byLine = new Map();
  for (const w of r.words) {
    if (!byLine.has(w.line)) byLine.set(w.line, []);
    byLine.get(w.line).push(w);
  }
  const out = [];
  for (const key of [...byLine.keys()].sort((a, b) => a - b)) {
    const ws = byLine.get(key).slice().sort((a, b) => a.x - b.x);
    let text = ws.map((w) => w.text).join(" ");
    if (alpha(text).length < 2) continue;
    const x0 = Math.floor(Math.min(...ws.map((w) => w.x)) / k) + r.x0;
    const y0 = Math.floor(Math.min(...ws.map((w) => w.y)) / k) + r.y0;
    const x1 = Math.floor(Math.max(...ws.map((w) => w.x + w.w)) / k) + r.x0;
    const y1 = Math.floor(Math.max(...ws.map((w) => w.y + w.h)) / k) + r.y0;
    // OCR punctuation slips: "Mission;_Prologue" -> "Mission: Prologue"
    text = text.replaceAll("_", " ").replaceAll(";", ":").replaceAll(":", ": ");
    text = text.split(/\s+/).filter(Boolean).join(" ");
    out.push({ x0, y0, x1, y1, text: vi.fixContractions(text) });
  }
  return out;
}

/** Lines → menu items / paragraphs with their place, colours and size. */
function menuBlocks(f, lines, split, tabH) {
  const blocks = [];
  for (const l of lines) {
    const item = l.x0 < split;
    const h = l.y1 - l.y0;
    if (!item) {
      // a wrapped sentence continues on the next line: same left edge, right under it, the previous
      // line not finished (. ! ? :) and this one starting in lower case (or set tightly)
      const b = [...blocks].reverse().find((b) => !b.item);
      if (b) {
        const last = b.lines[b.lines.length - 1];
        const unfinished = !/[.!?:]$/.test(last.trimEnd());
        const lower = /^\p{Lowercase}/u.test(l.text);
        const gap = Math.max(l.y0 - b.y1, 0);
        const bullet = /^[—–\-•]/.test(l.text);
        if (unfinished && !bullet && Math.abs(l.x0 - b.x0) <= h && ((lower && gap < h) || gap * 2 < h)) {
          Object.assign(b, { x0: Math.min(b.x0, l.x0), x1: Math.max(b.x1, l.x1), y1: l.y1, item: false });
          b.lines.push(l.text);
          continue;
        }
      }
    }
    blocks.push({ x0: l.x0, y0: l.y0, x1: l.x1, y1: l.y1, lines: [l.text], item, lh: h });
  }
  // which edge does each line share with others? values line up on the right, lists on the left
  const tol = Math.floor(f.w * 0.004);
  const rightOf = blocks.map((b) => {
    const lefts = blocks.filter((o) => o !== b && Math.abs(o.x0 - b.x0) <= tol).length;
    const rights = blocks.filter((o) => o !== b && Math.abs(o.x1 - b.x1) <= tol && Math.abs(o.x0 - b.x0) > tol).length;
    return rights > lefts || (b.x0 > f.w * 0.6 && b.x1 > f.w * 0.8);
  });
  const hs = blocks.filter((b) => b.lines.length === 1).map((b) => b.lh).sort((a, b) => a - b);
  const menuLh = hs.length ? hs[Math.floor(hs.length / 2)] : 20;
  return blocks.map((b, i) => {
    // background: a ring just outside the text; letters: the pixels least like it
    const pad = Math.max(Math.floor(Math.min(b.lh, tabH) / 2), 3);
    const rx0 = Math.max(b.x0 - pad, 0), ry0 = Math.max(b.y0 - Math.floor(pad / 2), 0);
    const rx1 = Math.min(b.x1 + pad, f.w - 1), ry1 = Math.min(b.y1 + Math.floor(pad / 2), f.h - 1);
    const bg = f.median({ x: rx0, y: ry0, w: rx1 - rx0 + 1, h: ry1 - ry0 + 1 }, { step_x: 2, step_y: 1, exclude: [b.x0, b.y0, b.x1, b.y1] }) || { r: 0, g: 0, b: 0 };
    const fg = f.contrast({ x: b.x0, y: b.y0, w: b.x1 - b.x0 + 1, h: b.y1 - b.y0 + 1 }, bg, 0.125) || { r: 0, g: 0, b: 0 };
    // one size for the menu (from the tab bar); a heading much bigger than the rest keeps its own,
    // measured on its letters (OCR's box around big faint text is far too tall)
    const heading = b.lh > 1.6 * menuLh;
    const lineH = (heading ? f.letterHeight({ x: b.x0, y: b.y0, w: b.x1 - b.x0 + 1, h: b.y1 - b.y0 + 1 }) * 0.7 : tabH * 0.44) / f.h;
    return {
      lines: b.lines.length, en: b.lines.join(" "), ar: "",
      rect: f.box(rx0, ry0, rx1 - rx0, ry1 - ry0),
      bg: hex(bg), fg: hex(fg), line_h: lineH, heading, missed: 0, item: b.item, right: rightOf[i],
    };
  });
}

/** The menu under the tabs (list and panel read apart; the Store is one wide column). */
function readMenu(f, tabH, tab) {
  if (tab === "STORE") return menuBlocks(f, readLines(f, STORE, 0), 0, tabH);
  const split = Math.floor(f.w * 0.388); // the list column ends here
  const lines = [...readLines(f, LIST, 0), ...readLines(f, PANEL, 0)];
  // greyed-out list items are too faint for that read: a boosted read fills the rows it left
  // empty, with known menu words only
  for (const l of readLines(f, LIST, 4)) {
    const h = l.y1 - l.y0;
    const free = !lines.some((o) => o.x0 < split && o.y0 < l.y1 && l.y0 < o.y1);
    if (free && h > 0 && vi.menuItemWord(l.text)) lines.push(l);
  }
  lines.sort((a, b) => a.y0 - b.y0 || a.x0 - b.x0);
  return menuBlocks(f, lines, split, tabH);
}

const selectedName = (labels) => (labels.find((l) => l.fg === "#000000") || {}).en || null;

pages.add({
  id: "gta5_pause",
  option: "pause_menu",

  present(f) {
    return selectedTab(f);
  },

  read(f, tab) {
    if (!tab) return null;
    const [, sy, sw, sh] = tab;
    // the tab row
    const row = pxRect(f, { x: 0.1, y: sy / f.h - 0.004, w: 0.8, h: sh / f.h + 0.008 });
    if (row.w < 8 || row.h < 8) return null;
    let words = f.ocr(row, { prep: "brightest" }).words;
    if (words.length < 3) words = f.ocr(row).words;
    const labels = [];
    for (const w of words) {
      const hit = vi.tabWord(alpha(w.text).toUpperCase());
      if (!hit) continue;
      // the tab around the word: as wide and tall as the selected one, centred on the word
      const cx = w.x + Math.floor(w.w / 2);
      const tx = Math.max(cx - Math.floor(sw / 2), 0), ty = sy;
      // its background: the tab's pixels away from the letters
      const xEnd = Math.min(tx + sw, f.w) - 4, yEnd = ty + sh - 2;
      const bg = f.median({ x: tx + 4, y: ty + 2, w: xEnd - (tx + 4), h: yEnd - (ty + 2) },
        { step_x: 3, step_y: 2, exclude: [w.x - 4, w.y - 4, w.x + w.w + 4, w.y + w.h + 4] });
      if (!bg) continue;
      const light = bg.r + bg.g + bg.b > 3 * 140;
      labels.push({ en: hit[0], ar: hit[1], rect: f.box(tx, ty, sw, sh), bg: hex(bg), fg: light ? "#000000" : "#ffffff" });
    }
    if (labels.length < 3) return null;
    const buttons = findButtons(f);
    const tab_ = selectedName(labels);
    return {
      title: { x: 0, y: 0, w: 0, h: 0 },
      buttons, buttons_row: buttonsRow(buttons),
      menu: tab_ === "MAP" ? [] : readMenu(f, sh, tab_ || ""),
      labels,
      player: readPlayer(f),
    };
  },

  // the menu changed but the tabs didn't: read just the menu (and the buttons if their row changed);
  // when the list on the left is unchanged, only the panel on the right
  reread(f, { old, unchanged }) {
    const tab = selectedTab(f);
    if (!tab) return null;
    const sh = tab[3];
    const name = selectedName(old.labels) || "";
    const items = old.menu.map((m, i) => [m, unchanged.menu[i]]).filter(([m]) => m.item);
    const listSame = name !== "STORE" && items.length > 0 && items.every(([, same]) => same);
    let menu;
    if (name === "MAP") menu = [];
    else if (listSame) menu = [...items.map(([m]) => m), ...menuBlocks(f, readLines(f, PANEL, 0), Math.floor(f.w * 0.388), sh)];
    else menu = readMenu(f, sh, name);
    const buttons = unchanged.buttons ? old.buttons : findButtons(f);
    return { ...old, menu, buttons, buttons_row: unchanged.buttons ? old.buttons_row : buttonsRow(buttons) };
  },

  same(f) {
    return selectedTab(f) !== null;
  },
});
