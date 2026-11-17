const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

let cfg = null;
let frameSize = { w: 0, h: 0 }; // captured window size (physical px)
let shown = { w: 0, h: 0 }; // preview image size on screen (CSS px)
let selected = null; // selected area id
const lastSegs = {}; // area id -> latest Arabic segments
const lastLineH = {}; // area id -> English line height (fraction of the frame), for size matching
const lastCached = {}; // TEMP: area id -> last line came from the memory
const lastTextBox = {}; // area id -> English text block [x, y, w, h] (fractions of the frame)

// ------------------------------------------------------------------------------------- saving

let saveTimer = null;
function save(redraw = true) {
  syncProfile();
  if (redraw) drawLayer();
  updateGlance();
  clearTimeout(saveTimer);
  $("saved").textContent = "…";
  saveTimer = setTimeout(async () => {
    try {
      await invoke("save_config", { config: cfg });
      $("saved").textContent = t("Saved ✓");
    } catch (e) {
      $("saved").textContent = t("Save failed");
      log({ error: String(e) });
    }
  }, 350);
}

// ------------------------------------------------------------------------------------- tabs

function openTab(name) {
  document.querySelectorAll(".tabs button").forEach((x) => x.classList.toggle("active", x.dataset.tab === name));
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("hidden", t.id !== `tab-${name}`));
}
document.querySelectorAll(".tabs button").forEach((b) => b.addEventListener("click", () => openTab(b.dataset.tab)));

// ------------------------------------------------------------------------------------- Play / Setup

// Play: the live translations big, a small preview and what is running. Setup: the area editor
// and every setting. Remembered on this PC; the first start opens Setup.
function setMode(mode) {
  document.body.dataset.mode = mode;
  document.querySelectorAll(".modes button").forEach((b) => {
    b.classList.toggle("active", b.dataset.mode === mode);
    b.setAttribute("aria-selected", b.dataset.mode === mode);
  });
  try { localStorage.setItem("mode", mode); } catch {}
  if (mode === "play") updateGlance();
  requestAnimationFrame(fitFrame);
}
document.querySelectorAll(".modes button").forEach((b) => b.addEventListener("click", () => setMode(b.dataset.mode)));
// "Change" on a Play card: open that part of Setup
document.querySelectorAll("[data-go]").forEach((b) => b.addEventListener("click", () => { setMode("setup"); openTab(b.dataset.go); }));

// Play: the preview full screen — the window goes full screen and the picture fills it. Back with the
// button, Esc, or a double-click on the picture.
const isFullscreen = () => document.documentElement.classList.contains("fullscreen");
async function setFullscreen(on) {
  document.documentElement.classList.toggle("fullscreen", on);
  $("fullscreen").title = on ? t("Exit full screen (Esc)") : t("Full screen");
  try {
    await window.__TAURI__.window.getCurrentWindow().setFullscreen(on);
  } catch (e) {
    log({ error: String(e) });
  }
  requestAnimationFrame(fitFrame); // the window's resize event fits it again once the size settles
}
$("fullscreen").addEventListener("click", () => setFullscreen(!isFullscreen()));
$("viewport").addEventListener("dblclick", (e) => {
  if (document.body.dataset.mode === "play" && !e.target.closest("#fullscreen")) setFullscreen(!isFullscreen());
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && isFullscreen()) setFullscreen(false);
});

/** The Play cards: source, language + service, read areas. */
function updateGlance() {
  if (!cfg) return;
  const src = $("window_title");
  $("ps-source").textContent = src.selectedOptions[0]?.textContent || cfg.window_title || "—";
  const l = LANGS.find((x) => x.code === cfg.target_lang);
  $("ps-lang").textContent = l ? `${l.name} — ${l.native}` : cfg.target_lang;
  const e = ENGINES.find((x) => x.id === cfg.engine);
  $("ps-engine").textContent = e ? t(e.name) : "";
  const on = cfg.areas.filter((a) => a.enabled);
  $("ps-areas").textContent = on.map((a) => a.name).join(", ") || "—";
  $("ps-profile").textContent = activeProfile()?.name || "";
}

// ------------------------------------------------------------------------------------- preview

function fitFrame() {
  if (!frameSize.w) return;
  const vp = $("viewport").getBoundingClientRect();
  const s = Math.min((vp.width - 16) / frameSize.w, (vp.height - 16) / frameSize.h);
  shown = { w: Math.floor(frameSize.w * s), h: Math.floor(frameSize.h * s) };
  for (const el of [$("frame"), $("frame-gl")]) {
    el.style.width = `${shown.w}px`;
    el.style.height = `${shown.h}px`;
  }
  drawLayer();
}
window.addEventListener("resize", fitFrame);

function sampleSegments() {
  // "*word*" in the sample text shows as a highlighted word
  return $("sample-text")
    .value.split(/\*(.+?)\*/)
    .map((t, i) => [t, i % 2 ? "#5db6e5" : null])
    .filter((s) => s[0]);
}

// Game page currently on screen (a menu, a "mission failed" page …) and its translated reason line.
const screen = { layout: null, reason: [] };

function drawLayer() {
  const layer = $("layer");
  layer.innerHTML = "";
  const sl = $("screen-layer");
  if (!cfg || !shown.w) { sl.innerHTML = ""; return; }
  // Overlays off (header switch): no translations on the game, so none on its picture here either
  // (the regions stay in Setup, to place them).
  const overlays = cfg.overlays_enabled !== false;
  // The profile page replaces the normal subtitles while it is up, exactly like the real overlay.
  AS.renderScreen(sl, overlays ? screen.layout : null, screen.reason, cfg.profile_opts);
  if (screen.layout) {
    for (const a of cfg.areas) layer.appendChild(areaBox(a));
    return;
  }
  const k = shown.h / 1080;
  for (const a of cfg.areas) {
    const px = { x: a.x * shown.w, y: a.y * shown.h, w: a.w * shown.w, h: a.h * shown.h };
    // Arabic region (same placement maths as the real overlay). Drag it to place the Arabic anywhere.
    if (a.enabled) {
      const segs = !overlays ? [] : lastSegs[a.id]?.length ? lastSegs[a.id] : $("show-sample").checked && document.body.dataset.mode !== "play" ? sampleSegments() : [];
      const r = AS.rectFor(a.placement, px, shown.w, shown.h, a);
      const band = document.createElement("div");
      band.className = `band${a.id === selected ? " selected" : ""}`;
      band.title = t("Drag to move the translation, drag the corners to resize its region");
      Object.assign(band.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
      const box = document.createElement("div");
      band.appendChild(box);
      AS.anchor(band, a.placement, a);
      // matched size only for real translations (the sample text has no English line to match)
      const fontPx = lastSegs[a.id]?.length ? AS.matchedPx(a, lastLineH[a.id], shown.h) : 0;
      AS.renderBox(box, segs, cfg.style, k, a.font_scale, a.placement, a, fontPx);
      if (lastSegs[a.id]?.length && lastCached[a.id] && cfg.memory_tint !== false) box.style.color = AS.CACHED_TINT; // lines from the memory
      if (a.placement === "under_text") {
        const tb = lastSegs[a.id]?.length ? lastTextBox[a.id] : null;
        if (tb) AS.placeUnderText(box, tb, lastLineH[a.id], shown.w, shown.h, r.x, r.y);
        else Object.assign(box.style, { position: "absolute", left: "0", top: `${px.h}px`, margin: "0" });
      }
      band.insertAdjacentHTML("beforeend", `<div class="tag">${AS.esc(t("Translation · {name}", { name: a.name }))}</div>`);
      addHandles(band, a, "out");
      band.addEventListener("mousedown", (e) => startDrag(e, a, "move", "out"));
      layer.appendChild(band);
    }
    layer.appendChild(areaBox(a));
  }
}

// The yellow read-area rectangle (draggable).
function areaBox(a) {
  const el = document.createElement("div");
  el.className = `area${a.id === selected ? " selected" : ""}${a.enabled ? "" : " disabled"}`;
  Object.assign(el.style, { left: `${a.x * shown.w}px`, top: `${a.y * shown.h}px`, width: `${a.w * shown.w}px`, height: `${a.h * shown.h}px` });
  el.innerHTML = `<div class="tag">${AS.esc(a.enabled ? a.name : t("{name} (off)", { name: a.name }))}</div>`;
  addHandles(el, a, "read");
  el.addEventListener("mousedown", (e) => startDrag(e, a, "move", "read"));
  return el;
}

function addHandles(el, area, kind) {
  for (const c of ["nw", "ne", "sw", "se"]) {
    const h = document.createElement("div");
    h.className = `handle ${c}`;
    h.addEventListener("mousedown", (e) => startDrag(e, area, c, kind));
    el.appendChild(h);
  }
}

/**
 * A yes / no question in the page (window.confirm does nothing in the macOS webview): resolves true
 * when `okLabel` is clicked, false on Cancel or Esc.
 */
function ask(text, okLabel) {
  return new Promise((resolve) => {
    const d = document.createElement("dialog");
    d.className = "ask";
    d.innerHTML = `<p></p><div class="row"><button class="btn" data-no></button><button class="btn danger" data-ok></button></div>`;
    d.querySelector("p").textContent = text;
    d.querySelector("[data-ok]").textContent = okLabel;
    d.querySelector("[data-no]").textContent = t("Cancel");
    const done = (v) => { d.close(); d.remove(); resolve(v); };
    d.querySelector("[data-ok]").addEventListener("click", () => done(true));
    d.querySelector("[data-no]").addEventListener("click", () => done(false));
    d.addEventListener("cancel", (e) => { e.preventDefault(); done(false); }); // Esc
    document.body.appendChild(d);
    d.showModal();
    d.querySelector("[data-no]").focus();
  });
}

// Turn a preset placement into a custom region at the same spot, so dragging starts from where it is.
function toCustom(area) {
  if (area.placement === "custom") return;
  const W = 1920, H = 1080; // fractions don't depend on the size used here
  const px = { x: area.x * W, y: area.y * H, w: area.w * W, h: area.h * H };
  const r = AS.rectFor(area.placement, px, W, H, area);
  const al = AS.alignOf(area.placement, area);
  Object.assign(area, {
    out_x: round(r.x / W), out_y: round(r.y / H), out_w: round(r.w / W), out_h: round(r.h / H),
    out_valign: al.v, out_halign: al.h, placement: "custom",
  });
}

// kind "read" moves the read area; kind "out" moves the Arabic region (switching it to a custom position).
function startDrag(e, area, mode, kind = "read") {
  e.preventDefault();
  e.stopPropagation();
  if (locked()) return; // a built-in profile: copy it to move its areas
  selected = area.id;
  renderAreas();
  const K = kind === "out" ? { x: "out_x", y: "out_y", w: "out_w", h: "out_h" } : { x: "x", y: "y", w: "w", h: "h" };
  const start = { mx: e.clientX, my: e.clientY, x: area[K.x], y: area[K.y], w: area[K.w], h: area[K.h] };
  const min = 0.02;
  let raf = 0;
  let moved = false;
  const move = (ev) => {
    if (!moved) {
      // (a plain click only selects; a translation region turns into a custom one when it's dragged)
      if (Math.abs(ev.clientX - start.mx) + Math.abs(ev.clientY - start.my) < 4) return;
      moved = true;
      if (kind === "out") {
        toCustom(area);
        Object.assign(start, { x: area[K.x], y: area[K.y], w: area[K.w], h: area[K.h] });
        renderAreas();
      }
    }
    const dx = (ev.clientX - start.mx) / shown.w;
    const dy = (ev.clientY - start.my) / shown.h;
    let { x, y, w, h } = start;
    if (mode === "move") {
      x = Math.min(Math.max(start.x + dx, 0), 1 - w);
      y = Math.min(Math.max(start.y + dy, 0), 1 - h);
    } else {
      if (mode.includes("w")) { x = Math.min(Math.max(start.x + dx, 0), start.x + start.w - min); w = start.x + start.w - x; }
      if (mode.includes("e")) { w = Math.min(Math.max(start.w + dx, min), 1 - start.x); }
      if (mode.includes("n")) { y = Math.min(Math.max(start.y + dy, 0), start.y + start.h - min); h = start.y + start.h - y; }
      if (mode.includes("s")) { h = Math.min(Math.max(start.h + dy, min), 1 - start.y); }
    }
    Object.assign(area, { [K.x]: round(x), [K.y]: round(y), [K.w]: round(w), [K.h]: round(h) });
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => { drawLayer(); updateAreaNumbers(area); });
  };
  const up = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    if (moved) save();
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}
const round = (v) => Math.round(v * 10000) / 10000;

$("show-sample").addEventListener("change", drawLayer);
$("sample-text").addEventListener("input", () => { drawLayer(); drawStyleSample(); });

// ------------------------------------------------------------------------------------- source

// Source values: a window title, "device:<name>" (capture card) or "obs:<source name>".
async function loadWindows() {
  const sel = $("window_title");
  sel.innerHTML = "";
  sel.add(new Option(t("Looking for sources ..."), cfg.window_title));
  const { windows, devices, obs } = await invoke("list_sources");
  sel.innerHTML = "";
  const group = (label) => {
    const g = document.createElement("optgroup");
    g.label = label;
    sel.appendChild(g);
    return g;
  };
  const values = [];
  // The open list is as wide as its longest line: long window titles (a browser tab, a file path)
  // are shortened in the middle, and the whole text is kept as the tooltip.
  const short = (s, n = 46) => (s.length > n ? `${s.slice(0, n - 16).trimEnd()}…${s.slice(-15).trimStart()}` : s);
  // The list follows the interface's direction; each name (usually English) is isolated so it keeps
  // its own left-to-right order inside a right-to-left list.
  const iso = (s) => `⁨${s}⁩`;
  const opt = (text, value, full = text) => Object.assign(new Option(text, value), { title: full });
  const add = (g, text, value, full) => { g.appendChild(opt(text, value, full)); values.push(value); };
  if (devices.length) {
    const g = group(t("Capture cards and cameras"));
    for (const d of devices) add(g, `🎥 ${iso(short(d))}`, `device:${d}`, d);
  }
  const st = $("obs-status");
  st.textContent = obs.ok ? t("Connected to OBS — {n} source(s). Pick one marked 📺 above.", { n: obs.sources.length }) : t("Not connected: {error}", { error: obs.error });
  st.style.color = obs.ok ? "var(--ok, #1a7f37)" : "var(--warn, #b45309)";
  // the Browser Source's width and height: OBS's canvas size, read from OBS
  const cv = $("obs-canvas");
  if (obs.ok && obs.canvas) {
    cv.innerHTML = `<span>${AS.esc(t("Your OBS canvas (width × height):"))}</span>
      <b dir="ltr">${obs.canvas.w} × ${obs.canvas.h}</b>
      <span class="muted small">${AS.esc(t("Set in OBS: Settings → Video → Base (Canvas) Resolution"))}</span>`;
  } else {
    cv.innerHTML = `<span class="muted small">${AS.esc(t("Connect to OBS above to see your canvas size here."))}</span>`;
  }
  const og = group(obs.ok ? t("OBS sources") : t("OBS sources (not connected)"));
  if (obs.ok) for (const s of obs.sources) add(og, `📺 ${iso(`${short(s.name, 36)}  —  ${s.kind}`)}`, `obs:${s.name}`, `${s.name}  —  ${s.kind}`);
  else og.appendChild(Object.assign(opt(short(String(obs.error), 56), "", obs.error), { disabled: true }));
  const wg = group(t("Windows"));
  for (const w of windows) {
    const full = w.process ? `${w.title}  —  ${w.process}` : w.title;
    add(wg, iso(w.process ? `${short(w.title)}  —  ${short(w.process, 24)}` : short(w.title)), w.title, full);
  }
  if (cfg.window_title && !values.includes(cfg.window_title)) {
    sel.insertBefore(opt(t("{title}  (not available now)", { title: iso(short(cfg.window_title)) }), cfg.window_title,
                         t("{title}  (not available now)", { title: cfg.window_title })), sel.firstChild);
  }
  sel.value = cfg.window_title;
}
function renderObs() {
  $("obs_port").value = cfg.obs_port;
  $("obs_password").value = cfg.obs_password;
  $("obs-url").value = `http://127.0.0.1:${cfg.obs_page_port}/obs`;
}
$("obs_port").addEventListener("input", (e) => {
  const v = parseInt(e.target.value, 10);
  cfg.obs_port = v >= 1 && v <= 65535 ? v : 4455;
  save(false);
});
$("obs_password").addEventListener("input", (e) => { cfg.obs_password = e.target.value; save(false); });
$("obs-connect").addEventListener("click", () => { $("obs-status").textContent = t("Connecting ..."); $("refresh-windows").click(); });
// Enter in the password box = connect
$("obs_password").addEventListener("keydown", (e) => { if (e.key === "Enter") $("refresh-windows").click(); });
$("obs-url-copy").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("obs-url").value); $("obs-url-copy").textContent = t("Copied ✓"); }
  catch { $("obs-url").select(); document.execCommand("copy"); $("obs-url-copy").textContent = t("Copied ✓"); }
  setTimeout(() => ($("obs-url-copy").textContent = t("Copy")), 1500);
});
// ------------------------------------------------------------------------------------- updates

function showUpdate(u) {
  if (!u) return;
  $("update-text").textContent = t("Version {version} is available.", { version: u.version });
  $("update-bar").classList.remove("hidden");
}
listen("update-available", (e) => showUpdate(e.payload));
$("update-later").addEventListener("click", () => $("update-bar").classList.add("hidden"));
$("update-now").addEventListener("click", async () => {
  $("update-now").disabled = true;
  $("update-text").textContent = t("Downloading the update…");
  try {
    await invoke("install_update"); // the app restarts into the new version
  } catch (e) {
    $("update-text").textContent = t("Update failed: {e}", { e });
    $("update-now").disabled = false;
  }
});

// Live translations page on the home network: switch, address and QR code.
async function updateLan() {
  const box = $("lan-info");
  $("lan-on").checked = !!cfg.lan_page;
  if (!cfg.lan_page) { box.textContent = t("Off. Turn it on to read the live translations on another device on your home network."); return; }
  try {
    const i = await invoke("lan_info");
    if (!i.url) {
      box.textContent = i.error
        ? t("The page couldn't start ({error}). Another program may be using this port.", { error: i.error })
        : t("Starting… If Windows asks, allow the app on private networks.");
      return;
    }
    box.innerHTML = `<div>${AS.esc(t("Open this address on a phone, tablet or computer on the same Wi-Fi:"))}</div>
      <a href="#" class="lan-url" dir="ltr" title="${AS.esc(t("Open in your browser"))}">${AS.esc(i.url)}</a>
      <div class="lan-qr">${i.qr || ""}</div>`;
    const svg = box.querySelector("svg");
    if (svg) Object.assign(svg.style, { width: "132px", height: "132px" });
  } catch (e) { box.textContent = String(e); }
}
// the address: opens the page in the default browser (the box is redrawn every few seconds, so listen on the box)
$("lan-info").addEventListener("click", (e) => {
  if (!e.target.closest(".lan-url")) return;
  e.preventDefault();
  invoke("open_lan_page").catch((err) => log({ error: String(err) }));
});
$("lan-on").addEventListener("change", (e) => { cfg.lan_page = e.target.checked; save(false); setTimeout(updateLan, 900); });
setInterval(() => { if (cfg && !$("tab-source").classList.contains("hidden")) updateLan(); }, 3000);

// save pending edits (e.g. the OBS password just typed) before asking OBS for its sources
$("refresh-windows").addEventListener("click", async () => {
  clearTimeout(saveTimer);
  try { await invoke("save_config", { config: cfg }); $("saved").textContent = t("Saved ✓"); } catch {}
  loadWindows();
});
$("window_title").addEventListener("change", (e) => {
  cfg.window_title = e.target.value;
  for (const k of Object.keys(lastSegs)) delete lastSegs[k];
  save();
  updateGlance();
});

// ------------------------------------------------------------------------------------- translator

const ENGINES = [
  { id: "local", name: "Local (offline)", badge: "Free", d: "Runs on this PC. No account or internet after a one-time ~110 MB download per language. Basic, literal quality. Not every language has one." },
  { id: "deepl", name: "DeepL", badge: "Free tier", d: "Very good quality. The free API plan includes 500,000 characters a month." },
  { id: "azure", name: "Microsoft Azure Translator", badge: "Free tier", d: "Good quality. The free tier includes 2 million characters a month." },
  { id: "google", name: "Google Cloud Translation", badge: "Free tier", d: "Good quality. Around 500,000 free characters a month." },
  { id: "libre", name: "LibreTranslate", badge: "Free / self-hosted", d: "Open source. Use your own server or a hosted one." },
  { id: "claude", name: "Claude (Anthropic)", badge: "≈ $1 per game", d: "Best with slang, jokes and context. Can use a dialect. Paid per use." },
];
const ENGINE_FIELDS = {
  local: [],
  deepl: [["deepl_key", "DeepL API key", "password", "Free keys end in :fx"]],
  azure: [["azure_key", "Azure Translator key", "password"], ["azure_region", "Region (e.g. westeurope)", "text", "Leave empty for a global resource"]],
  google: [["google_key", "Google Cloud API key", "password"]],
  libre: [["libre_url", "Server URL", "text"], ["libre_key", "API key (if the server needs one)", "password"]],
  claude: [
    ["claude_key", "Anthropic API key", "password"],
    ["claude_model", "Model", "select", null, [["claude-haiku-5-5", "Claude Haiku 5.5 — cheapest, fast"], ["claude-sonnet-5-5", "Claude Sonnet 5.5 — better"], ["claude-opus-5-5", "Claude Opus 5.5 — best, ~40× Haiku's price"]]],
    ["dialect", "Arabic variety", "select", null, [["msa", "Modern Standard (الفصحى)"], ["egyptian", "Egyptian (مصري)"], ["gulf", "Gulf (خليجي)"], ["levantine", "Levantine (شامي)"]]],
  ],
};

let LANGS = [];

function renderLang() {
  const sel = $("target-lang");
  if (!sel.options.length) {
    for (const l of LANGS) sel.add(new Option(`${l.name} — ${l.native}`, l.code));
    sel.addEventListener("change", () => {
      cfg.target_lang = sel.value;
      AS.setLang(cfg.target_lang);
      for (const k of Object.keys(lastSegs)) delete lastSegs[k];
      renderLang();
      renderEngines();
      renderStyleForm();
      renderAreas();
      renderProfile();
      renderPages();
      save();
    });
  }
  sel.value = cfg.target_lang || "ar";
  const l = LANGS.find((x) => x.code === sel.value);
  $("lang-note").textContent = !l ? "" : l.local
    ? t("All translation services work with {name}, including the free offline one.", { name: l.name })
    : t("{name} has no free offline model: choose DeepL, Azure, Google, LibreTranslate or Claude.", { name: l.name });
}

function renderEngines() {
  $("engines").innerHTML = "";
  for (const e of ENGINES) {
    const el = document.createElement("label");
    el.className = `engine${cfg.engine === e.id ? " active" : ""}`;
    el.innerHTML = `<input type="radio" name="engine" ${cfg.engine === e.id ? "checked" : ""} />
      <div><b>${t(e.name)}</b><span class="badge">${t(e.badge)}</span></div><div class="d">${t(e.d)}</div>`;
    el.querySelector("input").addEventListener("change", () => {
      cfg.engine = e.id;
      renderEngines();
      save(false);
    });
    $("engines").appendChild(el);
  }
  const box = $("engine-fields");
  box.innerHTML = "";
  const form = document.createElement("div");
  form.className = "form";
  // (the Arabic variety only applies to Arabic)
  for (const [key, label, type, help, options] of (ENGINE_FIELDS[cfg.engine] || []).filter((f) => f[0] !== "dialect" || (cfg.target_lang || "ar") === "ar")) {
    const f = document.createElement("div");
    f.className = "field";
    f.innerHTML = `<label>${t(label)}</label>`;
    let input;
    if (type === "select") {
      input = document.createElement("select");
      for (const [v, txt] of options) input.add(new Option(t(txt), v));
    } else {
      input = document.createElement("input");
      input.type = type;
      input.spellcheck = false;
    }
    input.value = cfg[key] ?? "";
    input.addEventListener("input", () => { cfg[key] = input.value.trim(); save(false); });
    f.appendChild(input);
    if (help) f.insertAdjacentHTML("beforeend", `<div class="muted small" style="margin:4px 0 0">${t(help)}</div>`);
    form.appendChild(f);
  }
  box.appendChild(form);
}

// ------------------------------------------------------------------------------------- areas

const PLACEMENTS = [["under_text", "Below the game's text box (same left edge, at most its width)"], ["above", "Above the area"], ["below", "Below the area"], ["right", "To the right"], ["left", "To the left"], ["over", "On top of the area"], ["custom", "Custom position (drag it in the preview)"]];
// Where the Arabic box sits inside its region (preset placements).
const TEXT_POS = [
  ["", "Automatic for this placement"],
  ["top-left", "Top left"], ["top-center", "Top centre"], ["top-right", "Top right"],
  ["middle-left", "Middle left"], ["middle-center", "Middle centre"], ["middle-right", "Middle right"],
  ["bottom-left", "Bottom left"], ["bottom-center", "Bottom centre"], ["bottom-right", "Bottom right"],
];
const POS_LABELS = { x: "Left %", y: "Top %", w: "Width %", h: "Height %" }; // (translated where drawn)
const OUT_KEYS = { x: "out_x", y: "out_y", w: "out_w", h: "out_h" };

// Four percent inputs for a rectangle stored as fractions under the given keys.
function posGrid(a, keys, cls) {
  return `<div class="grid4">${["x", "y", "w", "h"].map((k) => `<div><label>${t(POS_LABELS[k])}</label>
    <input type="number" class="${cls}-${k}" step="0.1" min="0" max="100" value="${(a[keys[k]] * 100).toFixed(1)}" /></div>`).join("")}</div>`;
}
function bindPosGrid(card, a, keys, cls) {
  for (const k of ["x", "y", "w", "h"]) {
    card.querySelector(`.${cls}-${k}`).addEventListener("change", (e) => {
      a[keys[k]] = round(Math.min(Math.max(Number(e.target.value) / 100, k === "w" || k === "h" ? 0.02 : 0), 1));
      if (a[keys.x] + a[keys.w] > 1) a[keys.w] = round(1 - a[keys.x]);
      if (a[keys.y] + a[keys.h] > 1) a[keys.h] = round(1 - a[keys.y]);
      updateAreaNumbers(a);
      save();
    });
  }
}

function renderAreas() {
  const list = $("area-list");
  list.innerHTML = "";
  for (const a of cfg.areas) {
    const c = document.createElement("div");
    c.className = `card${a.id === selected ? " selected" : ""}`;
    c.dataset.id = a.id;
    c.innerHTML = `
      <div class="row">
        <input type="checkbox" class="en" ${a.enabled ? "checked" : ""} title="${t("Read this area")}" />
        <input type="text" class="nm grow" value="${AS.esc(a.name)}" />
        <button class="btn icon danger del" title="${t("Delete")}"><svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/></svg></button>
      </div>
      <div class="row">
        <select class="pl grow">${PLACEMENTS.map(([v, txt]) => `<option value="${v}" ${a.placement === v ? "selected" : ""}>${t("Translation: {where}", { where: t(txt) })}</option>`).join("")}</select>
      </div>
      <div class="row field inline-label">
        <label>${t("Font")}</label>
        <select class="ff grow">${fontOptions(a.font_family, t("Main font ({font})", { font: cfg.style.font_family }))}</select>
      </div>
      <div class="row field inline-label">
        <label>${t("Background")}</label>
        <select class="bgm grow">${[["", "Main style (Text style tab)"], ["none", "None — outlined text, like game subtitles"], ["game", "Game-style dark box"]]
          .map(([v, txt]) => `<option value="${v}" ${(a.background || "") === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
      </div>
      <label class="check"><input type="checkbox" class="ms" ${a.match_size !== false ? "checked" : ""}/> ${t("Match the game's text size")}</label>
      <label class="check"><input type="checkbox" class="co" ${a.centered_only ? "checked" : ""}/> ${t("Only text centred in the picture (subtitles)")}</label>
      <div class="row field inline-label">
        <label>${a.match_size !== false ? t("Fine-tune") : t("Text size")}</label>
        <input type="range" class="fs" min="0.5" max="2" step="0.05" value="${a.font_scale}" />
        <span class="val fsv">${Math.round(a.font_scale * 100)}%</span>
      </div>
      <div class="sub">${t("Read area")}</div>
      ${posGrid(a, { x: "x", y: "y", w: "w", h: "h" }, "n")}
      ${a.placement !== "custom" ? `
        <div class="row field inline-label">
          <label>${t("Text sits")}</label>
          <select class="tp grow">${TEXT_POS.map(([v, txt]) => `<option value="${v}" ${(a.text_pos || "") === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
        </div>` : ""}
      ${a.placement === "custom" ? `
        <div class="sub">${t("Translation region")}</div>
        ${posGrid(a, OUT_KEYS, "o")}
        <div class="row">
          <select class="va grow">${[["top", "Text at the top"], ["middle", "Text in the middle"], ["bottom", "Text at the bottom"]]
            .map(([v, txt]) => `<option value="${v}" ${a.out_valign === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
          <select class="ha grow">${[["right", "Right"], ["center", "Centre"], ["left", "Left"]]
            .map(([v, txt]) => `<option value="${v}" ${a.out_halign === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
        </div>` : ""}`;
    c.addEventListener("mousedown", () => { if (selected !== a.id) { selected = a.id; highlightCards(); drawLayer(); } });
    c.querySelector(".en").addEventListener("change", (e) => { a.enabled = e.target.checked; save(); });
    c.querySelector(".nm").addEventListener("input", (e) => { a.name = e.target.value; save(); });
    c.querySelector(".pl").addEventListener("change", (e) => {
      if (e.target.value === "custom") toCustom(a); // start the custom region where the Arabic is now
      else a.placement = e.target.value;
      renderAreas(); // shows/hides the custom fields
      save();
    });
    if (a.placement !== "custom") {
      c.querySelector(".tp").addEventListener("change", (e) => { a.text_pos = e.target.value; save(); });
    }
    if (a.placement === "custom") {
      bindPosGrid(c, a, OUT_KEYS, "o");
      c.querySelector(".va").addEventListener("change", (e) => { a.out_valign = e.target.value; save(); });
      c.querySelector(".ha").addEventListener("change", (e) => { a.out_halign = e.target.value; save(); });
    }
    c.querySelector(".bgm").addEventListener("change", (e) => { a.background = e.target.value; save(); });
    c.querySelector(".ms").addEventListener("change", (e) => { a.match_size = e.target.checked; renderAreas(); save(); });
    c.querySelector(".co").addEventListener("change", (e) => { a.centered_only = e.target.checked; save(); });
    const ff = c.querySelector(".ff");
    paintFonts(ff);
    ff.style.fontFamily = fontCss(a.font_family || cfg.style.font_family);
    ff.addEventListener("change", (e) => {
      a.font_family = e.target.value;
      e.target.style.fontFamily = fontCss(a.font_family || cfg.style.font_family);
      save();
    });
    c.querySelector(".fs").addEventListener("input", (e) => {
      a.font_scale = Number(e.target.value);
      c.querySelector(".fsv").textContent = `${Math.round(a.font_scale * 100)}%`;
      save();
    });
    bindPosGrid(c, a, { x: "x", y: "y", w: "w", h: "h" }, "n");
    c.querySelector(".del").addEventListener("click", async () => {
      if (!(await ask(t('Delete the read area "{name}"?', { name: a.name }), t("Delete")))) return;
      cfg.areas = cfg.areas.filter((x) => x !== a);
      delete lastSegs[a.id];
      renderAreas();
      save();
    });
    list.appendChild(c);
  }
  lockControls();
}
function highlightCards() {
  document.querySelectorAll("#area-list .card").forEach((c) => c.classList.toggle("selected", c.dataset.id === selected));
}
function updateAreaNumbers(a) {
  const c = document.querySelector(`#area-list .card[data-id="${CSS.escape(a.id)}"]`);
  if (!c) return;
  for (const k of ["x", "y", "w", "h"]) {
    c.querySelector(`.n-${k}`).value = (a[k] * 100).toFixed(1);
    const o = c.querySelector(`.o-${k}`);
    if (o) o.value = (a[OUT_KEYS[k]] * 100).toFixed(1);
  }
}
function addArea() {
  if (locked()) return;
  const id = `area_${Date.now().toString(36)}`;
  cfg.areas.push({
    id, name: t("Area {n}", { n: cfg.areas.length + 1 }), enabled: true, x: 0.35, y: 0.4, w: 0.3, h: 0.12, placement: "above", font_scale: 1, font_family: "",
    out_x: 0.35, out_y: 0.25, out_w: 0.3, out_h: 0.15, out_valign: "bottom", out_halign: "center",
  });
  selected = id;
  renderAreas();
  save();
}
$("add-area").addEventListener("click", addArea);
$("add-area-2").addEventListener("click", addArea);

// ------------------------------------------------------------------------------------- style

// Installed fonts from the backend: [{ name, covers: [language codes it can write] }].
// Fonts that can write the target language are listed first.
const fitsLang = (f) => (f.covers || []).includes(cfg.target_lang || "ar");
const langNative = () => (LANGS.find((l) => l.code === (cfg.target_lang || "ar")) || { native: "العربية" }).native;
let FONT_LIST = [];
// Free Arabic fonts shipped with the app (ui/fonts, SIL Open Font License).
const BUNDLED_FONTS = [
  { name: "Lalezar", note: "heavy display, great for game titles" },
  { name: "Changa", note: "bold geometric" },
  { name: "Cairo", note: "clean, many weights" },
  { name: "Tajawal", note: "clean text" },
];

/** A font name as a CSS font-family value (a quoted string): for `el.style.fontFamily` only. */
const fontCss = (name) => JSON.stringify(String(name || ""));

/** Draw each font <option> of a select in its own font (see fontOptions). */
function paintFonts(sel) {
  for (const o of sel.querySelectorAll("option")) if (o.value) o.style.fontFamily = fontCss(o.value);
}

// <option>s for a font <select> (then `paintFonts` draws each name in its own font). `defaultLabel`
// adds a "use main font" choice.
function fontOptions(current, defaultLabel) {
  const opt = (f) =>
    `<option value="${AS.esc(f.name)}" ${f.name === current ? "selected" : ""}>${AS.esc(f.name)}${f.note ? ` — ${AS.esc(t(f.note))}` : ""}</option>`;
  let html = defaultLabel ? `<option value="" ${current ? "" : "selected"}>${AS.esc(defaultLabel)}</option>` : "";
  const isBundled = (n) => BUNDLED_FONTS.some((b) => b.name === n);
  if (current && !isBundled(current) && !FONT_LIST.some((f) => f.name === current)) {
    html += `<option value="${AS.esc(current)}" selected>${AS.esc(t("{font} (not installed)", { font: current }))}</option>`;
  }
  html += `<optgroup label="${t("Included with the app")}">${BUNDLED_FONTS.map(opt).join("")}</optgroup>`;
  const fit = FONT_LIST.filter((f) => fitsLang(f) && !isBundled(f.name));
  const other = FONT_LIST.filter((f) => !fitsLang(f) && !isBundled(f.name));
  const language = langNative();
  if (fit.length) html += `<optgroup label="${AS.esc(t("Fonts for {language}", { language }))}">${fit.map(opt).join("")}</optgroup>`;
  if (other.length) html += `<optgroup label="${AS.esc(t("Other fonts ({language} falls back to a default font)", { language }))}">${other.map(opt).join("")}</optgroup>`;
  return html;
}

const STYLE_FIELDS = [
  ["font_family", "Font", "font"],
  ["font_size", "Text size (px at 1080p; scales with the game window)", "size", 10, 96, 1],
  ["bold", "Bold", "check"],
  ["line_height", "Line spacing", "range", 1, 2.2, 0.05],
  ["text_color", "Text colour", "color"],
  ["keep_colors", "Colour highlighted words like the game does", "check"],
  ["bg_color", "Box colour", "color"],
  ["bg_opacity", "Box opacity", "range", 0, 1, 0.05],
  ["padding", "Box padding", "range", 0, 30, 1],
  ["radius", "Box corner roundness", "range", 0, 30, 1],
  ["outline_color", "Text outline colour", "color"],
  ["outline_width", "Text outline width", "range", 0, 6, 0.5],
  ["shadow", "Text shadow", "check"],
];

function renderStyleForm() {
  const form = $("style-form");
  form.innerHTML = "";
  const s = cfg.style;
  for (const [key, label, type, min, max, step] of STYLE_FIELDS) {
    const f = document.createElement("div");
    f.className = "field";
    if (type === "check") {
      f.innerHTML = `<label class="check" style="color:var(--text)"><input type="checkbox" ${s[key] ? "checked" : ""}/> ${t(label)}</label>`;
      f.querySelector("input").addEventListener("change", (e) => setStyle(key, e.target.checked));
    } else if (type === "font") {
      f.innerHTML = `<label>${t(label)}</label><select class="grow">${fontOptions(s.font_family)}</select>
        <div class="muted small" style="margin:4px 0 0">${t("{n} installed fonts can write {language} (listed first). Each read area can also use its own font.", { n: FONT_LIST.filter(fitsLang).length, language: langNative() })}</div>`;
      const sel = f.querySelector("select");
      paintFonts(sel);
      const showFont = () => (sel.style.fontFamily = `${fontCss(sel.value)}, 'Segoe UI'`);
      showFont();
      sel.addEventListener("change", () => { showFont(); setStyle(key, sel.value); renderAreas(); });
    } else if (type === "size") {
      // slider + exact number, kept in sync
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="range" min="${min}" max="${max}" step="${step}" value="${AS.esc(s[key])}" />
        <input type="number" min="${min}" max="${max}" step="${step}" value="${AS.esc(s[key])}" style="width:72px" /><span class="val" style="min-width:18px">px</span></div>`;
      const [range, num] = f.querySelectorAll("input");
      range.addEventListener("input", () => { num.value = range.value; setStyle(key, Number(range.value)); });
      num.addEventListener("change", () => {
        const v = Math.min(Math.max(Number(num.value) || s[key], min), max);
        num.value = range.value = v;
        setStyle(key, v);
      });
    } else if (type === "color") {
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="color" value="${AS.esc(s[key])}" /><span class="val">${AS.esc(s[key])}</span></div>`;
      f.querySelector("input").addEventListener("input", (e) => { f.querySelector(".val").textContent = e.target.value; setStyle(key, e.target.value); });
    } else {
      const fmt = (v) => (step < 1 ? Number(v).toFixed(2) : String(v));
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="range" min="${min}" max="${max}" step="${step}" value="${AS.esc(s[key])}" /><span class="val">${AS.esc(fmt(s[key]))}</span></div>`;
      f.querySelector("input").addEventListener("input", (e) => { f.querySelector(".val").textContent = fmt(e.target.value); setStyle(key, Number(e.target.value)); });
    }
    form.appendChild(f);
  }
  const reset = document.createElement("button");
  reset.className = "btn";
  reset.textContent = t("Reset to defaults");
  reset.addEventListener("click", () => {
    cfg.style = { font_family: "Segoe UI", font_size: 26, bold: true, text_color: "#ffffff", bg_color: "#000000", bg_opacity: 0.65,
      outline_color: "#000000", outline_width: 0, shadow: true, padding: 10, radius: 10, line_height: 1.5, keep_colors: true };
    renderStyleForm();
    save();
  });
  form.appendChild(reset);
  drawStyleSample();
  lockControls();
}
function setStyle(key, value) {
  cfg.style[key] = value;
  drawStyleSample();
  save();
}
function drawStyleSample() {
  if (!cfg) return;
  const wrap = $("style-sample-wrap");
  AS.anchor(wrap, "over");
  wrap.style.justifyContent = "center";
  AS.renderBox($("style-sample"), sampleSegments(), cfg.style, 0.75, 1, "above");
}

// ------------------------------------------------------------------------------------- timing

// The preview's fps badge: shown or not, and which corner (Advanced → Preview).
const FPS_CORNERS = [["top-left", "Top left"], ["top-right", "Top right"], ["bottom-left", "Bottom left"], ["bottom-right", "Bottom right"]];
function applyFpsBadge() {
  const corner = FPS_CORNERS.some(([v]) => v === cfg.fps_corner) ? cfg.fps_corner : "top-right";
  $("fps").dataset.corner = corner;
  document.body.classList.toggle("no-fps", cfg.fps_badge !== true);
}
function renderPreviewForm() {
  const form = $("preview-form");
  form.innerHTML = `
    <label class="check"><input type="checkbox" id="fps-on" ${cfg.fps_badge === true ? "checked" : ""}/> ${t("Show frames per second (fps) on the preview")}</label>
    <div class="field"><label for="fps-corner">${t("Where it shows")}</label>
      <select id="fps-corner">${FPS_CORNERS.map(([v, txt]) => `<option value="${v}" ${cfg.fps_corner === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select></div>`;
  const corner = $("fps-corner");
  corner.disabled = cfg.fps_badge !== true;
  $("fps-on").addEventListener("change", (e) => { cfg.fps_badge = e.target.checked; corner.disabled = !e.target.checked; applyFpsBadge(); save(false); });
  corner.addEventListener("change", () => { cfg.fps_corner = corner.value; applyFpsBadge(); save(false); });
  applyFpsBadge();
  corner.value = $("fps").dataset.corner; // the corner actually used (an unknown value falls back to top right)
}

function renderTiming() {
  const form = $("timing-form");
  form.innerHTML = "";
  const fields = [
    ["capture_fps", "Frames captured for reading text (per second)", 1, 30, 1, "Full-size frames used for reading text, game pages and saved frames. Also how often the pause menu checks whether its text changed. 8 is plenty; more costs CPU, especially at 4K. From OBS, about 12 is the most it can deliver."],
    ["preview_fps", "Live preview frame rate (per second)", 5, 60, 5, "Only the picture in this app (small frames, made only while it's visible). Doesn't change how text is read."],
    ["interval_ms", "How often areas are read (ms)", 100, 1000, 50, "Lower = faster reaction, more CPU."],
    ["stable_ms", "Wait for text to settle before translating (ms)", 0, 1500, 50, "Avoids translating half-faded lines."],
    ["clear_after_ms", "Treat the English as gone after (ms)", 200, 3000, 100, "Ignores short OCR blinks. The translation then stays as long as the English was on screen, and switches to the next line as soon as it is ready."],
  ];
  for (const [key, label, min, max, step, help] of fields) {
    const f = document.createElement("div");
    f.className = "field";
    f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="range" min="${min}" max="${max}" step="${step}" value="${cfg[key]}" /><span class="val">${cfg[key]}</span></div>${help ? `<div class="muted small" style="margin:4px 0 0">${t(help)}</div>` : ""}`;
    f.querySelector("input").addEventListener("input", (e) => { cfg[key] = Number(e.target.value); f.querySelector(".val").textContent = e.target.value; save(false); });
    form.appendChild(f);
  }
  const note = document.createElement("div");
  note.className = "hint";
  note.textContent = t("Left as they are, the last three follow the game profile's own timing when it has one.");
  form.appendChild(note);
}

// ------------------------------------------------------------------------------------- translation memory

function renderMemory() {
  $("memory-form").innerHTML = `
    <label class="check" style="color:var(--text);font-weight:600"><input type="checkbox" id="mem-on" ${cfg.memory_enabled ? "checked" : ""}/> ${t("Use translation memory")}</label>
    <label class="check" style="color:var(--text)"><input type="checkbox" id="mem-tint" ${cfg.memory_tint !== false ? "checked" : ""}/> ${t("Show lines from the memory in {green}", { green: `<b style="color:${AS.CACHED_TINT}">${t("green")}</b>` })}</label>
    <div class="callout" id="mem-stats" style="margin:0"></div>
    <div class="row"><button class="btn" id="mem-open">${t("Open its folder")}</button><button class="btn" id="mem-reload">${t("Reload")}</button><button class="btn danger" id="mem-clear">${t("Clear memory")}</button></div>
    <div class="row"><button class="btn" id="size-reset">${t("Re-measure text sizes")}</button></div>
    <div class="hint">${t("Text sizes are measured on the first {n} lines of each kind, then fixed.", { n: 5 })}</div>
    <label class="check" style="color:var(--text);font-weight:600;margin-top:10px"><input type="checkbox" id="share-on" ${cfg.share_lines !== false ? "checked" : ""}/> ${t("Help improve translations")}</label>
    <div class="hint">${t("Shares the English lines you see in the game (subtitles, instructions and captions; never menus) with the VI Translator server every few minutes, so they can be translated better for everyone. Anonymous: a random id, no account, no screenshots.")}</div>
    <label class="check" style="color:var(--text)"><input type="checkbox" id="layout-auto" ${cfg.auto_layout !== false ? "checked" : ""}/> ${t("Automatic layout updates")}</label>
    <div class="hint">${t("Keeps the game profile's read areas, fonts, text style and timing up to date.")}</div>
    <label class="check" style="color:var(--text)"><input type="checkbox" id="auto-game" ${cfg.auto_game ? "checked" : ""}/> ${t("Switch games automatically")}</label>
    <div class="hint">${t("When the picture shows another game the app knows (by its game pages), that game's profile is used. A game that looks much like another may be taken for it.")}</div>
    <label class="check" style="color:var(--text)"><input type="checkbox" id="early-pages" ${cfg.early_pages ? "checked" : ""}/> ${t("Get test versions of game pages")}</label>
    <div class="hint">${t("New game pages go to a few players first. A test version may not work as well yet. This install's id: {id}", { id: `<code>${AS.esc(cfg.install_id || "—")}</code>` })}</div>`;
  $("size-reset").addEventListener("click", () => invoke("reset_text_sizes"));
  $("mem-reload").addEventListener("click", async () => { await invoke("memory_reload"); updateMemory(); });
  $("mem-on").addEventListener("change", (e) => { cfg.memory_enabled = e.target.checked; save(false); updateMemory(); });
  $("mem-tint").addEventListener("change", (e) => { cfg.memory_tint = e.target.checked; save(); });
  $("share-on").addEventListener("change", (e) => { cfg.share_lines = e.target.checked; save(false); });
  $("layout-auto").addEventListener("change", (e) => { cfg.auto_layout = e.target.checked; save(false); });
  $("auto-game").addEventListener("change", (e) => { cfg.auto_game = e.target.checked; save(false); });
  $("early-pages").addEventListener("change", (e) => { cfg.early_pages = e.target.checked; save(false); });
  $("mem-open").addEventListener("click", () => invoke("memory_open_folder").catch((e) => log({ error: String(e) })));
  $("mem-clear").addEventListener("click", async () => {
    if (!(await ask(t("Forget all remembered translations?"), t("Clear memory")))) return;
    await invoke("memory_clear");
    updateMemory();
  });
  updateMemory();
}

async function updateMemory() {
  const el = $("mem-stats");
  if (!el) return;
  try {
    const m = await invoke("memory_status");
    const lines = [t("{n} remembered line(s)", { n: m.entries }), t("Folder: {path}", { path: m.file })];
    if (m.hits + m.misses > 0) {
      const rate = Math.round((100 * m.hits) / (m.hits + m.misses));
      lines.push(t("This session: {hits} from memory, {misses} translated ({rate}% hits)", { hits: m.hits, misses: m.misses, rate }));
      lines.push(t("Translation ready after the English appeared: memory {a} · translator {b}", {
        a: m.hits ? Math.round(m.avg_hit_ms) + " ms" : "—", b: m.misses ? Math.round(m.avg_miss_ms) + " ms" : "—" }));
      if (m.hits && m.misses) lines.push(t("Saved about {ms} ms per remembered line", { ms: Math.round(m.avg_miss_ms - m.avg_hit_ms) }));
    } else {
      lines.push(m.enabled ? t("No lines seen yet this session.") : t("Off — turn it on to start remembering."));
    }
    el.innerHTML = lines.map(AS.esc).join("<br>");
  } catch (e) {
    el.textContent = String(e);
  }
}
setInterval(() => { if (!$("tab-advanced").classList.contains("hidden")) updateMemory(); }, 2000);

// ------------------------------------------------------------------------------------- debugging

let saver = { on: false, count: 0, folder: null };

function renderDebug() {
  const form = $("debug-form");
  form.innerHTML = `
    <label class="check" style="color:var(--text);font-weight:600"><input type="checkbox" id="dbg-save" ${cfg.debug_save_frames ? "checked" : ""}/> ${t("Save frames to disk")}</label>
    <div class="muted small" style="margin:-6px 0 0">${AS.esc(t("Saves full-size PNG files of the captured window into a new folder (in Pictures\\VI Translator\\debug-frames) each time you turn it on. Turning it off opens the folder. 4K frames are a few MB each."))}</div>
    <div class="field"><label>${t("Save a frame every")}</label><div class="inline">
      <input type="range" id="dbg-int" min="100" max="5000" step="100" value="${cfg.debug_frame_interval_ms}" />
      <span class="val" id="dbg-int-v">${t("{s} s", { s: (cfg.debug_frame_interval_ms / 1000).toFixed(1) })}</span></div></div>
    <div class="callout" id="dbg-status" style="margin:0"></div>
    <button class="btn" id="dbg-open">${t("Open frames folder")}</button>`;
  $("dbg-save").addEventListener("change", (e) => setSaveFrames(e.target.checked));
  $("dbg-int").addEventListener("input", (e) => {
    cfg.debug_frame_interval_ms = Number(e.target.value);
    $("dbg-int-v").textContent = t("{s} s", { s: (cfg.debug_frame_interval_ms / 1000).toFixed(1) });
    save(false);
  });
  $("dbg-open").addEventListener("click", openFramesFolder);
  updateSaver();
}

function setSaveFrames(on) {
  cfg.debug_save_frames = on;
  save(false);
  if ($("dbg-save")) $("dbg-save").checked = on;
  // turning it off: show what was recorded (wait a moment so the last frame is written)
  if (!on) setTimeout(openFramesFolder, 400);
}

async function openFramesFolder() {
  try {
    const dir = await invoke("open_frames_folder");
    log({ info: t("Opened {dir}", { dir }) });
  } catch (e) {
    log({ error: t("Could not open the frames folder: {e}", { e }) });
  }
}

function updateSaver() {
  const rec = $("rec");
  rec.classList.toggle("hidden", !saver.on);
  rec.textContent = t("● Saving frames ({n})", { n: saver.count });
  const st = $("dbg-status");
  if (st) {
    st.textContent = saver.on
      ? t("Saving… {n} frame(s) in {folder}", { n: saver.count, folder: saver.folder ?? "…" })
      : saver.folder ? t("Last recording: {n} frame(s) in {folder}", { n: saver.count, folder: saver.folder }) : t("Not saving.");
  }
}

$("rec").addEventListener("click", () => setSaveFrames(false));
listen("frame-saver", (e) => {
  saver = e.payload;
  updateSaver();
});

// ------------------------------------------------------------------------------------- log

function log(ev) {
  const el = document.createElement("div");
  el.className = "item";
  const area = cfg?.areas.find((a) => a.id === ev.area);
  // when the English appeared (or, for messages, now), as HH:MM:SS in the interface's language
  const when = new Date(ev.at || Date.now());
  const time = when.toLocaleTimeString(I18N.lang, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  if (ev.error) {
    el.innerHTML = `<div class="err">⚠ ${AS.esc(ev.error)}</div><div class="meta">${time}${area ? ` · ${AS.esc(area.name)}` : ""}</div>`;
  } else if (ev.info) {
    el.innerHTML = `<div class="meta">${time} · ${AS.esc(ev.info)}</div>`;
  } else {
    const ar = AS.segHtml(ev.segments);
    const where = area?.name ?? (ev.area === "__screen_reason" ? t("Game page text") : ev.area);
    el.innerHTML = `<div class="head"><span class="when" title="${AS.esc(when.toLocaleString(I18N.lang))}">${time}</span><span class="en" dir="ltr">${AS.esc(ev.en)}</span></div><div class="ar" dir="auto">${ar}</div><div class="meta">${AS.esc(where)}${ev.stats ? ` · ${AS.esc(ev.stats)}` : ""}</div>`;
  }
  const box = $("log");
  box.prepend(el);
  while (box.children.length > 100) box.lastChild.remove();
}
$("clear-log").addEventListener("click", () => ($("log").innerHTML = ""));

// The bar between the preview and the live translations: drag it to give the log more room
// (remembered on this PC).
(function logSplit() {
  const wrap = $("log-wrap");
  const set = (h) => {
    const max = $("viewport").parentElement.clientHeight - 160; // leave the preview some room
    wrap.style.height = `${Math.round(Math.min(Math.max(h, 80), Math.max(max, 80)))}px`;
    fitFrame();
  };
  try { const h = Number(localStorage.getItem("logHeight")); if (h) set(h); } catch {}
  $("split").addEventListener("mousedown", (e) => {
    e.preventDefault();
    const start = { y: e.clientY, h: wrap.getBoundingClientRect().height };
    $("split").classList.add("dragging");
    const move = (ev) => set(start.h + (start.y - ev.clientY));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      $("split").classList.remove("dragging");
      try { localStorage.setItem("logHeight", String(wrap.getBoundingClientRect().height)); } catch {}
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
  // double-click: back to the default height
  $("split").addEventListener("dblclick", () => { set(150); try { localStorage.removeItem("logHeight"); } catch {} });
})();

// ------------------------------------------------------------------------------------- events + init

// A capture card's own YUY2 frames, drawn on the GPU the way OBS does: the frame goes up as a texture
// as it is (each texel = two pixels: Y0 U Y1 V) and a shader turns it into colour per pixel. No JPEG
// encoding or decoding on the way, full quality.
const YUV = { gl: null, prog: null, tex: null, w: 0, h: 0, u: {} };
function yuvInit() {
  const canvas = $("frame-gl");
  // desynchronized: the canvas goes to the screen without waiting for the rest of the page
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, desynchronized: true, preserveDrawingBuffer: false });
  if (!gl) return false;
  const sh = (type, src) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const prog = gl.createProgram();
  // one triangle that covers the canvas
  gl.attachShader(prog, sh(gl.VERTEX_SHADER, `#version 300 es
    void main() { vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2); gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }`));
  gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, `#version 300 es
    precision highp float;
    uniform highp sampler2D tex;
    uniform int height;
    uniform float yOff, yScale, cScale, kr, kgu, kgv, kb;
    out vec4 color;
    void main() {
      ivec2 p = ivec2(gl_FragCoord.xy);
      vec4 s = texelFetch(tex, ivec2(p.x >> 1, height - 1 - p.y), 0) * 255.0; // Y0 U Y1 V
      float y = ((p.x & 1) == 0 ? s.r : s.b);
      y = (y - yOff) * yScale;
      float u = (s.g - 128.0) * cScale, v = (s.a - 128.0) * cScale;
      color = vec4(clamp(vec3(y + kr * v, y - kgu * u - kgv * v, y + kb * u) / 255.0, 0.0, 1.0), 1.0);
    }`));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
  gl.useProgram(prog);
  for (const n of ["tex", "height", "yOff", "yScale", "cScale", "kr", "kgu", "kgv", "kb"]) YUV.u[n] = gl.getUniformLocation(prog, n);
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  for (const [k, v] of [[gl.TEXTURE_MIN_FILTER, gl.NEAREST], [gl.TEXTURE_MAG_FILTER, gl.NEAREST], [gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE], [gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE]]) gl.texParameteri(gl.TEXTURE_2D, k, v);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
  Object.assign(YUV, { gl, prog, tex, w: 0, h: 0 });
  return true;
}
/** Draw one YUY2 frame (`data`: w × h × 2 bytes). */
function yuvDraw(data, w, h, bt709, fullRange) {
  if (!YUV.gl && !yuvInit()) return false;
  const gl = YUV.gl;
  const canvas = $("frame-gl");
  if (YUV.w !== w || YUV.h !== h) {
    canvas.width = w;
    canvas.height = h;
    gl.viewport(0, 0, w, h);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w / 2, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
    Object.assign(YUV, { w, h });
  } else {
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w / 2, h, gl.RGBA, gl.UNSIGNED_BYTE, data);
  }
  const [kr, kgu, kgv, kb] = bt709 ? [1.5748, 0.1873, 0.4681, 1.8556] : [1.402, 0.3441, 0.7141, 1.772];
  const u = YUV.u;
  gl.uniform1i(u.tex, 0);
  gl.uniform1i(u.height, h);
  gl.uniform1f(u.yOff, fullRange ? 0 : 16);
  gl.uniform1f(u.yScale, fullRange ? 1 : 255 / 219);
  gl.uniform1f(u.cScale, fullRange ? 1 : 255 / 224);
  for (const [n, v] of [["kr", kr], ["kgu", kgu], ["kgv", kgv], ["kb", kb]]) gl.uniform1f(u[n], v);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  return true;
}

// The live picture: ask for the newest frame, show it, then ask for the next one. Never more than one
// frame is on the way, so the picture can't fall behind the game (a stream that keeps pushing frames
// piles them up whenever the window draws a little slower than they come). This event only says
// whether a picture is coming in, its size and the status, twice a second.
let streaming = "";
let pullRun = 0;

// The header's "Preview" switch (remembered on this PC). Off: the window stops asking for frames, so
// the app stops making them too (less CPU); reading and translating carry on as before.
let previewOn = true;
try { previewOn = localStorage.getItem("preview") !== "off"; } catch {}
function setPreview(on) {
  previewOn = on;
  $("preview-on").checked = on;
  document.body.classList.toggle("no-preview", !on);
  try { localStorage.setItem("preview", on ? "on" : "off"); } catch {}
  pullRun++; // stops the picture loop; when on, the next "frame" event (twice a second) starts it again
  streaming = "";
}
$("preview-on").addEventListener("change", (e) => setPreview(e.target.checked));
$("preview-on-2").addEventListener("click", () => setPreview(true));
setPreview(previewOn);

// Frames shown per second, in the corner of the preview (counted where frames are actually drawn).
let shownTimes = [];
function frameShown() {
  shownTimes.push(performance.now());
}
setInterval(() => {
  const now = performance.now();
  shownTimes = shownTimes.filter((t) => now - t <= 1000);
  const badge = $("fps");
  badge.hidden = !streaming || !shownTimes.length;
  badge.textContent = `${shownTimes.length} fps`;
}, 500);
async function pullPreview(url, run) {
  const img = $("frame");
  const canvas = $("frame-gl");
  let seq = 0;
  let shownUrl = null;
  let first = true;
  while (run === pullRun) {
    try {
      const r = await fetch(`${url}?after=${seq}`, { cache: "no-store" });
      if (run !== pullRun) break;
      // 204: no new frame for a second (still picture, or window hidden); anything else is wrong
      // (another program on the port?): wait a moment rather than asking again at once
      if (r.status !== 200) {
        if (r.status !== 204) await new Promise((res) => setTimeout(res, 1000));
        continue;
      }
      seq = Number(r.headers.get("X-Seq")) || 0;
      if (r.headers.get("Content-Type") === "application/x-yuy2") {
        const w = Number(r.headers.get("X-Width"));
        const h = Number(r.headers.get("X-Height"));
        const data = new Uint8Array(await r.arrayBuffer());
        if (run !== pullRun) break;
        if (yuvDraw(data, w, h, r.headers.get("X-Matrix") !== "601", r.headers.get("X-Range") === "full")) {
          frameShown();
          canvas.hidden = false;
          img.hidden = true;
          if (first) { first = false; fitFrame(); }
        }
        continue;
      }
      const blobUrl = URL.createObjectURL(await r.blob());
      const next = new Image();
      next.src = blobUrl;
      await next.decode().catch(() => {}); // decoded off-screen first: no flicker when it's swapped in
      if (run !== pullRun) { URL.revokeObjectURL(blobUrl); break; }
      img.src = blobUrl;
      frameShown();
      img.hidden = false;
      canvas.hidden = true;
      if (shownUrl) URL.revokeObjectURL(shownUrl);
      shownUrl = blobUrl;
      if (first) { first = false; fitFrame(); }
    } catch {
      await new Promise((res) => setTimeout(res, 500)); // the app's local server isn't up yet
    }
  }
}
listen("frame", (e) => {
  const f = { ...e.payload, status: I18N.tMsg(e.payload.status) };
  const pill = $("status");
  $("status-text").textContent = f.status || "…";
  pill.title = f.status || "";
  pill.className = `pill ${f.live ? "ok" : "warn"}`;
  $("source-status").textContent = f.status || "";
  $("ps-status").textContent = f.status || "";
  $("ps-status").className = `s ${f.live ? "ok" : "warn"}`;
  if (f.live) {
    const changed = f.w !== frameSize.w || f.h !== frameSize.h;
    frameSize = { w: f.w, h: f.h };
    if (previewOn && streaming !== f.preview_url) {
      streaming = f.preview_url;
      pullPreview(f.preview_url, ++pullRun);
    }
    $("empty").style.display = "none";
    if (changed) fitFrame();
  } else {
    streaming = "";
    pullRun++; // stops the picture loop
    frameSize = { w: 0, h: 0 };
    shown = { w: 0, h: 0 };
    $("frame").removeAttribute("src");
    $("frame").style.width = $("frame").style.height = "0";
    $("frame-gl").hidden = true; // no picture: hide the card's last frame too
    $("layer").innerHTML = "";
    $("empty").style.display = "";
    $("empty-msg").textContent = f.status || t("Pick the window to capture in Source.");
  }
});

listen("translation", (e) => {
  if (!cfg) return; // (before the settings are loaded: right after a reload)
  const ev = e.payload;
  if (ev.error) return log(ev);
  if (ev.area === "__screen_reason") {
    screen.reason = ev.segments;
  } else {
    lastSegs[ev.area] = ev.segments;
    lastCached[ev.area] = !!ev.cached;
    if (ev.line_h) lastLineH[ev.area] = ev.line_h;
    if (ev.text_box) lastTextBox[ev.area] = ev.text_box;
  }
  // the list keeps filling while "Live translations" is on, whether or not the overlays are
  if (ev.segments.length && cfg.live_translations !== false) log(ev);
  drawLayer();
});
listen("status-msg", (e) => log({ info: I18N.tMsg(e.payload) }));
// A new pages script was saved (a layout update, test versions turned on or off): Game pages asks again.
listen("pages", () => {
  for (const k of Object.keys(pageScripts)) delete pageScripts[k];
  if (cfg) renderPages();
});
// The app changed the built-in profiles (a layout update, a game the server added): take those
// (the rest of the settings are the window's own, being edited here), and ask the pages again.
listen("config", (e) => {
  if (!cfg) return;
  const now = e.payload;
  // (every save comes back here too: only a real change does anything)
  const newLayout = JSON.stringify(now.layout_versions) !== JSON.stringify(cfg.layout_versions);
  if (newLayout) for (const k of Object.keys(pageScripts)) delete pageScripts[k];
  const changed = JSON.stringify(now.builtins) !== JSON.stringify(cfg.builtins);
  // (a switch the app made by itself is a newer profile_rev; any other differing profile is an older
  // event, from before a change made here, and the window's own choice stays)
  const appSwitched = now.profile_rev !== cfg.profile_rev;
  // (only these: the event carries no secrets — the keys are blank in it — and the window's own
  // settings stay as the window has them; taking the whole payload would wipe the keys at the next save)
  Object.assign(cfg, { builtins: now.builtins, server_games: now.server_games, games_version: now.games_version, layout_versions: now.layout_versions, game_profiles: now.game_profiles, profile_rev: now.profile_rev });
  // the app switched games (the picture showed another one)
  if (appSwitched && now.profile !== cfg.profile) {
    syncProfile();
    Object.assign(cfg, { profile: now.profile, areas: now.areas, style: now.style, profile_opts: now.profile_opts });
    selected = cfg.areas[0]?.id ?? null;
    for (const k of Object.keys(lastSegs)) delete lastSegs[k];
    renderAll();
    return;
  }
  if (!changed) {
    if (newLayout) renderPages(); // (a layout update can bring a new pages script)
    return;
  }
  if (locked()) {
    const p = activeProfile();
    Object.assign(cfg, { areas: clone(p.areas), style: clone(p.style), profile_opts: clone(p.pages) });
    renderAll();
  } else {
    renderProfile();
  }
});
listen("screen", (e) => {
  const was = !!screen.layout;
  screen.layout = e.payload;
  if (!screen.layout) screen.reason = [];
  if (!!screen.layout !== was) log({ info: screen.layout ? t("Game page detected") : t("Game page closed") });
  drawLayer();
});

// ------------------------------------------------------------------------------------- game profile

// A game profile holds the read areas, the text style and the game pages. Built-in ones (GTA VI, and any game the server adds)
// can't be changed: "Make a copy" gives the player one of their own. The settings the rest of the app
// reads (cfg.areas, cfg.style, cfg.profile_opts) are the profile in use; with one of the player's
// profiles, every save writes them back into it (see syncProfile).
// (the games: the app's own and any the server added, by their built-in profile)
const gameName = (g) => (cfg.builtins || []).find((b) => b.id === g)?.name || g;
const allProfiles = () => [...(cfg.builtins || []), ...(cfg.profiles || [])];
const activeProfile = () => allProfiles().find((p) => p.id === cfg.profile) || (cfg.builtins || [])[0];
const isBuiltin = (p) => (cfg.builtins || []).some((b) => b.id === p?.id);
const locked = () => isBuiltin(activeProfile());
const profileGame = () => activeProfile()?.game || "gta6";
const clone = (v) => JSON.parse(JSON.stringify(v));

/** One of the player's profiles is in use: keep it equal to what is being edited. */
function syncProfile() {
  const p = (cfg.profiles || []).find((x) => x.id === cfg.profile);
  if (p) Object.assign(p, { areas: clone(cfg.areas), style: clone(cfg.style), pages: clone(cfg.profile_opts) });
}

/** Use another profile: its areas, style and pages become the ones in use. */
function useProfile(id) {
  syncProfile();
  const p = allProfiles().find((x) => x.id === id);
  if (!p) return;
  cfg.profile = p.id;
  cfg.areas = clone(p.areas);
  cfg.style = clone(p.style);
  cfg.profile_opts = clone(p.pages);
  selected = cfg.areas[0]?.id ?? null;
  for (const k of Object.keys(lastSegs)) delete lastSegs[k];
  renderAll();
  save();
}

/** A copy of a profile (built-in or not) that the player can change. */
function copyProfile(from) {
  syncProfile();
  const id = `custom-${Date.now().toString(36)}`;
  cfg.profiles = cfg.profiles || [];
  cfg.profiles.push({ id, name: t("{name} (copy)", { name: from.name }), game: from.game, areas: clone(from.areas), style: clone(from.style), pages: clone(from.pages), timing: clone(from.timing || {}) });
  useProfile(id);
  setTimeout(() => $("pf-name")?.select(), 0);
}

async function deleteProfile(p) {
  if (!(await ask(t("Delete the profile “{name}”? This can't be undone.", { name: p.name }), t("Delete")))) return;
  cfg.profiles = cfg.profiles.filter((x) => x.id !== p.id);
  useProfile((cfg.builtins || []).some((b) => b.id === p.game) ? p.game : "gta6");
}

function renderAll() {
  applyLock();
  renderProfile();
  renderPages();
  renderAreas();
  renderStyleForm();
  drawLayer();
  updateGlance();
}

/** Built-in profile in use: its parts can be looked at, not changed (a note offers a copy). */
function applyLock() {
  const on = locked();
  document.body.classList.toggle("locked", on);
  const p = activeProfile();
  document.querySelectorAll("[data-lock]").forEach((el) => {
    el.innerHTML = on
      ? `<svg viewBox="0 0 24 24"><rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></svg>
         <span>${AS.esc(t("“{name}” is built in and can't be changed.", { name: p.name }))}</span>
         <button class="btn" data-copy>${t("Make a copy")}</button>`
      : "";
    el.querySelector("[data-copy]")?.addEventListener("click", () => copyProfile(activeProfile()));
  });
  lockControls();
}

function renderProfile() {
  const f = $("profile-form");
  const p = activeProfile();
  const opt = (x) => `<option value="${AS.esc(x.id)}" ${x.id === cfg.profile ? "selected" : ""}>${AS.esc(x.name)}</option>`;
  const mine = cfg.profiles || [];
  f.innerHTML = `
    <div class="field"><label for="pf-select">${t("Profile in use")}</label>
      <select id="pf-select">
        <optgroup label="${AS.esc(t("Built-in"))}">${(cfg.builtins || []).map(opt).join("")}</optgroup>
        ${mine.length ? `<optgroup label="${AS.esc(t("Your profiles"))}">${mine.map(opt).join("")}</optgroup>` : ""}
      </select></div>
    <div class="card profile-card">
      <div class="row">
        ${isBuiltin(p)
          ? `<div class="grow"><b>${AS.esc(p.name)}</b></div><span class="badge lock-badge">${t("Built-in")}</span>`
          : `<input type="text" id="pf-name" class="grow" value="${AS.esc(p.name)}" title="${AS.esc(t("Rename"))}" /><span class="badge">${t("Yours")}</span>`}
      </div>
      <p class="muted small" style="margin:8px 0 0">${t("For {game} · {n} read area(s)", { game: AS.esc(gameName(p.game)), n: p.areas.length })}</p>
      <div class="row" style="margin-top:12px">
        <button id="pf-copy" class="btn">${t("Make a copy")}</button>
        ${isBuiltin(p) ? "" : `<button id="pf-delete" class="btn danger">${t("Delete")}</button>`}
      </div>
    </div>
    <p class="muted small">${isBuiltin(p)
      ? t("Built-in profiles get updates for the game automatically. A copy keeps your changes and stays yours.")
      : t("Your changes to read areas, text style and game pages are saved in this profile.")}</p>`;
  $("pf-select").addEventListener("change", (e) => useProfile(e.target.value));
  $("pf-copy").addEventListener("click", () => copyProfile(activeProfile()));
  $("pf-delete")?.addEventListener("click", () => deleteProfile(activeProfile()));
  $("pf-name")?.addEventListener("input", (e) => {
    const mineNow = cfg.profiles.find((x) => x.id === cfg.profile);
    if (mineNow) mineNow.name = e.target.value;
    save(false);
  });
  $("pf-name")?.addEventListener("change", () => renderProfile());
}

/** A script's text: plain, or by interface language (English the fallback). */
const scriptText = (v) => (typeof v === "string" ? v : v ? v[cfg.ui_lang] || v.en || Object.values(v)[0] || "" : "");

/** The pages a game's script has and the switches it declares, asked once per game. */
const pageScripts = {};
async function pageScript(game) {
  if (!pageScripts[game]) pageScripts[game] = invoke("page_options", { game }).catch(() => ({ pages: [], options: [] }));
  return pageScripts[game];
}

/** Game pages of the profile in use: the switches the game's pages script declares, and the fonts. */
async function renderPages() {
  const f = $("pages-form");
  const o = cfg.profile_opts;
  const game = profileGame();
  const script = await pageScript(game);
  if (profileGame() !== game) return; // (another profile meanwhile: its own render runs)
  if (!script.pages.length) {
    f.innerHTML = `<div class="callout">${t("{game}'s own pages aren't known yet, so none are translated for now. Subtitles and the read areas work as usual.", { game: AS.esc(gameName(game)) })}</div>`;
    return;
  }
  o.more = o.more || {};
  const isOn = (opt) => (opt.id in o.more ? o.more[opt.id] : opt.on);
  // the switches the game's script declares (its pages come with layout updates)
  const declared = script.options.map((opt) => `
    <div class="card">
      <label class="check" style="font-weight:600"><input type="checkbox" data-more="${AS.esc(opt.id)}" ${isOn(opt) ? "checked" : ""}/> ${AS.esc(scriptText(opt.name) || opt.id)}</label>
      ${scriptText(opt.about) ? `<p class="muted small" style="margin:6px 0 0">${AS.esc(scriptText(opt.about))}</p>` : ""}
    </div>`).join("");
  f.innerHTML = declared + `
    <div class="card">
      <div class="field"><label>${t("Title font")}</label><select id="pf-title">${fontOptions(o.title_font)}</select></div>
      <div class="field" style="margin-top:10px"><label>${t("Reason and button font")}</label><select id="pf-text">${fontOptions(o.text_font)}</select></div>
    </div>`;
  f.querySelectorAll("[data-more]").forEach((el) => el.addEventListener("change", () => { o.more[el.dataset.more] = el.checked; save(); }));
  for (const [id, key] of [["pf-title", "title_font"], ["pf-text", "text_font"]]) {
    const sel = $(id);
    paintFonts(sel);
    sel.style.fontFamily = fontCss(sel.value);
    sel.addEventListener("change", () => { o[key] = sel.value; sel.style.fontFamily = fontCss(sel.value); save(); });
  }
  lockControls();
}


function applyTheme() {
  const dark = cfg.theme === "dark";
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  // the icon shows where a click takes you (sun: to light, moon: to dark)
  $("theme").title = `${t("Switch between light and dark")} — ${dark ? t("☀ Light") : t("🌙 Dark")}`;
}

// A built-in profile in use: the controls of its parts are switched off (not just dimmed), so they
// can't be reached with the keyboard either.
function lockControls() {
  const on = locked();
  document.querySelectorAll("#tab-areas, #tab-style, #tab-pages").forEach((tab) =>
    tab.querySelectorAll("input, select, textarea, button:not([data-copy])").forEach((el) => { el.disabled = on; }));
  $("add-area").disabled = on;
  // (its boxes can't be moved or resized, so the hint about doing that is gone too)
  $("drag-hint").hidden = on;
}

(async function init() {
  cfg = await invoke("get_config");
  I18N.setLang(cfg.ui_lang || "en");  const uiSel = $("ui-lang");
  for (const l of I18N.UI_LANGS) uiSel.add(new Option(l.name, l.code));
  uiSel.value = I18N.lang;
  uiSel.addEventListener("change", async () => {
    cfg.ui_lang = uiSel.value;
    clearTimeout(saveTimer);
    try { await invoke("save_config", { config: cfg }); } catch {}
    location.reload(); // simplest way to redraw every panel in the new language and direction
  });
  AS.setLang(cfg.target_lang);
  LANGS = await invoke("list_langs").catch((e) => { log({ error: t("Languages: {e}", { e }) }); return []; });
  try { renderLang(); } catch (e) { log({ error: t("Languages: {e}", { e }) }); }
  selected = cfg.areas[0]?.id ?? null;
  applyTheme();
  $("theme").addEventListener("click", () => {
    cfg.theme = cfg.theme === "dark" ? "light" : "dark";
    applyTheme();
    save(false);
  });
  $("overlays_enabled").checked = cfg.overlays_enabled;
  const showLive = () => { $("log-paused").hidden = cfg.live_translations !== false; };
  $("live_translations").checked = cfg.live_translations !== false;
  showLive();
  $("live_translations").addEventListener("change", (e) => { cfg.live_translations = e.target.checked; showLive(); save(false); });
  // (save(): redraws the preview, which hides or shows the translations on it too)
  $("overlays_enabled").addEventListener("change", (e) => { cfg.overlays_enabled = e.target.checked; save(); });
  try {
    FONT_LIST = await invoke("list_fonts");
  } catch (e) {
    log({ error: t("Could not list fonts: {e}", { e }) });
  }
  renderEngines();
  applyLock();
  renderProfile();
  renderPages();
  renderAreas();
  renderStyleForm();
  renderTiming();
  renderPreviewForm();
  renderMemory();
  renderDebug();
  try {
    saver = await invoke("frame_saver_status");
    updateSaver();
  } catch {}
  try {
    const s = await invoke("get_screen");
    screen.layout = s.layout;
    screen.reason = s.reason || [];
  } catch {}
  document.fonts.ready.then(drawLayer); // re-measure once the bundled fonts have loaded
  renderObs();
  updateLan();
  invoke("app_version").then((v) => { $("app-version").textContent = "v" + v; document.title = "VI Translator " + v; }).catch(() => {});
  invoke("update_status").then(showUpdate).catch(() => {});
  $("log").dataset.empty = t("Translations appear here as the game shows text.");
  let mode = "setup";
  try { mode = localStorage.getItem("mode") || "setup"; } catch {}
  setMode(mode === "play" ? "play" : "setup");
  await loadWindows();
  updateGlance();
})();
