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
  if (redraw) drawLayer();
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

document.querySelectorAll(".tabs button").forEach((b) =>
  b.addEventListener("click", () => {
    document.querySelectorAll(".tabs button").forEach((x) => x.classList.toggle("active", x === b));
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("hidden", t.id !== `tab-${b.dataset.tab}`));
  }),
);

// ------------------------------------------------------------------------------------- preview

function fitFrame() {
  const img = $("frame");
  if (!frameSize.w) return;
  const vp = $("viewport").getBoundingClientRect();
  const s = Math.min((vp.width - 16) / frameSize.w, (vp.height - 16) / frameSize.h);
  shown = { w: Math.floor(frameSize.w * s), h: Math.floor(frameSize.h * s) };
  img.style.width = `${shown.w}px`;
  img.style.height = `${shown.h}px`;
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

// Profile page currently on screen (e.g. GTA V "Mission Failed") and its Arabic reason line.
const screen = { layout: null, reason: [] };

function drawLayer() {
  const layer = $("layer");
  layer.innerHTML = "";
  const sl = $("screen-layer");
  if (!cfg || !shown.w) { sl.innerHTML = ""; return; }
  // The profile page replaces the normal subtitles while it is up, exactly like the real overlay.
  AS.renderScreen(sl, cfg.profile === "gta5" ? screen.layout : null, screen.reason, cfg.profile_opts);
  if (screen.layout && cfg.profile === "gta5") {
    for (const a of cfg.areas) layer.appendChild(areaBox(a));
    return;
  }
  const k = shown.h / 1080;
  for (const a of cfg.areas) {
    const px = { x: a.x * shown.w, y: a.y * shown.h, w: a.w * shown.w, h: a.h * shown.h };
    // Arabic region (same placement maths as the real overlay). Drag it to place the Arabic anywhere.
    if (a.enabled) {
      const segs = lastSegs[a.id]?.length ? lastSegs[a.id] : $("show-sample").checked ? sampleSegments() : [];
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
  selected = area.id;
  if (kind === "out") toCustom(area);
  renderAreas();
  const K = kind === "out" ? { x: "out_x", y: "out_y", w: "out_w", h: "out_h" } : { x: "x", y: "y", w: "w", h: "h" };
  const start = { mx: e.clientX, my: e.clientY, x: area[K.x], y: area[K.y], w: area[K.w], h: area[K.h] };
  const min = 0.02;
  let raf = 0;
  const move = (ev) => {
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
    save();
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}
const round = (v) => Math.round(v * 10000) / 10000;

$("frame").addEventListener("load", fitFrame);
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
  const add = (g, text, value) => { g.appendChild(new Option(text, value)); values.push(value); };
  if (devices.length) {
    const g = group(t("Capture cards and cameras"));
    for (const d of devices) add(g, `🎥 ${d}`, `device:${d}`);
  }
  const st = $("obs-status");
  st.textContent = obs.ok ? t("Connected to OBS ✓ — {n} source(s). Pick one marked 📺 above.", { n: obs.sources.length }) : t("Not connected: {error}", { error: obs.error });
  st.style.color = obs.ok ? "var(--ok, #1a7f37)" : "var(--warn, #b45309)";
  const og = group(obs.ok ? t("OBS sources") : t("OBS sources (not connected)"));
  if (obs.ok) for (const s of obs.sources) add(og, `📺 ${s.name}  —  ${s.kind}`, `obs:${s.name}`);
  else og.appendChild(Object.assign(new Option(obs.error, ""), { disabled: true }));
  const wg = group(t("Windows"));
  for (const w of windows) add(wg, w.process ? `${w.title}  —  ${w.process}` : w.title, w.title);
  if (cfg.window_title && !values.includes(cfg.window_title)) {
    sel.insertBefore(new Option(t("{title}  (not available now)", { title: cfg.window_title }), cfg.window_title), sel.firstChild);
  }
  sel.value = cfg.window_title;
}
function renderObs() {
  $("obs_port").value = cfg.obs_port;
  $("obs_password").value = cfg.obs_password;
  $("obs-url").value = `http://127.0.0.1:${cfg.obs_page_port}/obs`;
}
$("obs_port").addEventListener("input", (e) => { cfg.obs_port = parseInt(e.target.value, 10) || 4455; save(false); });
$("obs_password").addEventListener("input", (e) => { cfg.obs_password = e.target.value; save(false); });
$("obs-connect").addEventListener("click", () => { $("obs-status").textContent = t("Connecting ..."); $("refresh-windows").click(); });
// Enter in the password box = connect
$("obs_password").addEventListener("keydown", (e) => { if (e.key === "Enter") $("refresh-windows").click(); });
$("obs-url-copy").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("obs-url").value); $("obs-url-copy").textContent = t("Copied ✓"); }
  catch { $("obs-url").select(); document.execCommand("copy"); $("obs-url-copy").textContent = t("Copied ✓"); }
  setTimeout(() => ($("obs-url-copy").textContent = t("Copy")), 1500);
});
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
        <button class="btn danger del" title="${t("Delete")}">✕</button>
      </div>
      <div class="row">
        <select class="pl grow">${PLACEMENTS.map(([v, txt]) => `<option value="${v}" ${a.placement === v ? "selected" : ""}>${t("Translation: {where}", { where: t(txt) })}</option>`).join("")}</select>
      </div>
      <div class="row field" style="margin-top:8px">
        <label style="margin:0;min-width:70px">${t("Font")}</label>
        <select class="ff grow" style="font-family:'${AS.esc(a.font_family || cfg.style.font_family)}'">${fontOptions(a.font_family, t("Main font ({font})", { font: cfg.style.font_family }))}</select>
      </div>
      <div class="row field" style="margin-top:8px">
        <label style="margin:0;min-width:70px">${t("Background")}</label>
        <select class="bgm grow">${[["", "Main style (Translation text tab)"], ["none", "None — outlined text, like game subtitles"], ["game", "Game-style dark box"]]
          .map(([v, txt]) => `<option value="${v}" ${(a.background || "") === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
      </div>
      <label class="check" style="margin-top:8px;color:var(--text)"><input type="checkbox" class="ms" ${a.match_size !== false ? "checked" : ""}/> ${t("Match the game's text size")}</label>
      <label class="check" style="margin-top:4px;color:var(--text)"><input type="checkbox" class="co" ${a.centered_only ? "checked" : ""}/> ${t("Only text centred in the picture (subtitles)")}</label>
      <div class="row field" style="margin-top:8px">
        <label style="margin:0;min-width:70px">${a.match_size !== false ? t("Fine-tune") : t("Text size")}</label>
        <input type="range" class="fs" min="0.5" max="2" step="0.05" value="${a.font_scale}" />
        <span class="val fsv">${Math.round(a.font_scale * 100)}%</span>
      </div>
      <div class="sub">${t("Read area")}</div>
      ${posGrid(a, { x: "x", y: "y", w: "w", h: "h" }, "n")}
      ${a.placement !== "custom" ? `
        <div class="row field" style="margin-top:8px">
          <label style="margin:0;min-width:70px">${t("Text sits")}</label>
          <select class="tp grow">${TEXT_POS.map(([v, txt]) => `<option value="${v}" ${(a.text_pos || "") === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select>
        </div>` : ""}
      ${a.placement === "custom" ? `
        <div class="sub">${t("Translation region")}</div>
        ${posGrid(a, OUT_KEYS, "o")}
        <div class="row" style="margin-top:8px">
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
    c.querySelector(".ff").addEventListener("change", (e) => {
      a.font_family = e.target.value;
      e.target.style.fontFamily = `'${a.font_family || cfg.style.font_family}'`;
      save();
    });
    c.querySelector(".fs").addEventListener("input", (e) => {
      a.font_scale = Number(e.target.value);
      c.querySelector(".fsv").textContent = `${Math.round(a.font_scale * 100)}%`;
      save();
    });
    bindPosGrid(c, a, { x: "x", y: "y", w: "w", h: "h" }, "n");
    c.querySelector(".del").addEventListener("click", () => {
      if (!confirm(t('Delete the read area "{name}"?', { name: a.name }))) return;
      cfg.areas = cfg.areas.filter((x) => x !== a);
      delete lastSegs[a.id];
      renderAreas();
      save();
    });
    list.appendChild(c);
  }
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

// <option>s for a font <select>; each name is drawn in its own font. `defaultLabel` adds a "use main font" choice.
function fontOptions(current, defaultLabel) {
  const opt = (f) =>
    `<option value="${AS.esc(f.name)}" style="font-family:'${AS.esc(f.name)}'" ${f.name === current ? "selected" : ""}>${AS.esc(f.name)}${f.note ? ` — ${AS.esc(t(f.note))}` : ""}</option>`;
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
      const showFont = () => (sel.style.fontFamily = `'${sel.value}', 'Segoe UI'`);
      showFont();
      sel.addEventListener("change", () => { showFont(); setStyle(key, sel.value); renderAreas(); });
    } else if (type === "size") {
      // slider + exact number, kept in sync
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="range" min="${min}" max="${max}" step="${step}" value="${s[key]}" />
        <input type="number" min="${min}" max="${max}" step="${step}" value="${s[key]}" style="width:72px" /><span class="val" style="min-width:18px">px</span></div>`;
      const [range, num] = f.querySelectorAll("input");
      range.addEventListener("input", () => { num.value = range.value; setStyle(key, Number(range.value)); });
      num.addEventListener("change", () => {
        const v = Math.min(Math.max(Number(num.value) || s[key], min), max);
        num.value = range.value = v;
        setStyle(key, v);
      });
    } else if (type === "color") {
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="color" value="${s[key]}" /><span class="val">${s[key]}</span></div>`;
      f.querySelector("input").addEventListener("input", (e) => { f.querySelector(".val").textContent = e.target.value; setStyle(key, e.target.value); });
    } else {
      const fmt = (v) => (step < 1 ? Number(v).toFixed(2) : String(v));
      f.innerHTML = `<label>${t(label)}</label><div class="inline"><input type="range" min="${min}" max="${max}" step="${step}" value="${s[key]}" /><span class="val">${fmt(s[key])}</span></div>`;
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
    <div class="hint">${t("Shares the English lines you see in the game (subtitles, instructions and captions; never menus) with the GTA 6 Translator server every few minutes, so they can be translated better for everyone. Anonymous: a random id, no account, no screenshots.")}</div>
    <label class="check" style="color:var(--text)"><input type="checkbox" id="layout-auto" ${cfg.auto_layout !== false ? "checked" : ""}/> ${t("Automatic layout updates")}</label>
    <div class="hint">${t("Keeps the game profile's read areas and fonts up to date.")}</div>`;
  $("size-reset").addEventListener("click", () => invoke("reset_text_sizes"));
  $("mem-reload").addEventListener("click", async () => { await invoke("memory_reload"); updateMemory(); });
  $("mem-on").addEventListener("change", (e) => { cfg.memory_enabled = e.target.checked; save(false); updateMemory(); });
  $("mem-tint").addEventListener("change", (e) => { cfg.memory_tint = e.target.checked; save(); });
  $("share-on").addEventListener("change", (e) => { cfg.share_lines = e.target.checked; save(false); });
  $("layout-auto").addEventListener("change", (e) => { cfg.auto_layout = e.target.checked; save(false); });
  $("mem-open").addEventListener("click", () => invoke("memory_open_folder").catch((e) => log({ error: String(e) })));
  $("mem-clear").addEventListener("click", async () => {
    if (!confirm(t("Forget all remembered translations?"))) return;
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
    <div class="muted small" style="margin:-6px 0 0">${AS.esc(t("Saves full-size PNG files of the captured window into a new folder (in Pictures\\GTA 6 Translator\\debug-frames) each time you turn it on. Turning it off opens the folder. 4K frames are a few MB each."))}</div>
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
    const where = area?.name ?? (ev.area === "__screen_reason" ? t("Mission Failed reason") : ev.area);
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

// The live picture streams as MJPEG from the app's local server (smooth, no per-frame events);
// this event only says whether a picture is coming in, its size and the status, twice a second.
let streaming = "";
listen("frame", (e) => {
  const f = e.payload;
  const pill = $("status");
  pill.textContent = f.status || "…";
  pill.className = `pill ${f.live ? "ok" : "warn"}`;
  $("source-status").textContent = f.status || "";
  if (f.live) {
    const changed = f.w !== frameSize.w || f.h !== frameSize.h;
    frameSize = { w: f.w, h: f.h };
    if (streaming !== f.preview_url) {
      streaming = f.preview_url;
      $("frame").src = `${f.preview_url}?t=${Date.now()}`;
    }
    $("empty").style.display = "none";
    if (changed) fitFrame();
  } else {
    streaming = "";
    frameSize = { w: 0, h: 0 };
    shown = { w: 0, h: 0 };
    $("frame").removeAttribute("src");
    $("frame").style.width = $("frame").style.height = "0";
    $("layer").innerHTML = "";
    $("empty").style.display = "";
    $("empty-msg").textContent = f.status || t("Pick the window to capture in Source.");
  }
});

listen("translation", (e) => {
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
  if (ev.segments.length) log(ev);
  drawLayer();
});
listen("status-msg", (e) => log({ info: e.payload }));
listen("screen", (e) => {
  const was = !!screen.layout;
  screen.layout = e.payload;
  if (!screen.layout) screen.reason = [];
  if (!!screen.layout !== was) log({ info: screen.layout ? t("GTA V: game page detected") : t("GTA V: game page closed") });
  updateProfileStatus();
  drawLayer();
});

// ------------------------------------------------------------------------------------- game profile

const PROFILES = [["none", "None — subtitles and read areas only"], ["gta5", "Grand Theft Auto V"]];
const GTA5_AREAS = () => [
  { id: "subtitles", name: t("Subtitles"), enabled: true, centered_only: true, x: 0.1, y: 0.78, w: 0.8, h: 0.17, placement: "above", text_pos: "bottom-center", background: "none", font_scale: 1.04, match_size: true, font_family: "Tajawal",
    out_x: 0.1, out_y: 0.6, out_w: 0.8, out_h: 0.15, out_valign: "bottom", out_halign: "center" },
  { id: "help", name: t("Instructions (top left)"), enabled: true, x: 0.01, y: 0.02, w: 0.3, h: 0.22, placement: "under_text", text_pos: "top-left", background: "game", font_scale: 0.92, match_size: true, font_family: "Tajawal",
    out_x: 0.31, out_y: 0.02, out_w: 0.35, out_h: 0.3, out_valign: "top", out_halign: "left" },
];

function renderProfile() {
  const f = $("profile-form");
  const o = cfg.profile_opts;
  f.innerHTML = `
    <div class="field"><label>${t("Profile")}</label>
      <select id="pf-profile">${PROFILES.map(([v, txt]) => `<option value="${v}" ${cfg.profile === v ? "selected" : ""}>${t(txt)}</option>`).join("")}</select></div>
    ${cfg.profile === "gta5" ? `
    <div class="card" style="margin-top:12px">
      <label class="check" style="font-weight:600"><input type="checkbox" id="pf-mf" ${o.mission_failed ? "checked" : ""}/> ${t("“Mission Failed” page")}</label>
      <p class="muted small" style="margin:6px 0 10px">${t("Adds the title in the same colour and size above the red title, the reason under the English reason, and translated labels with the game's own button icons above Retry / Restart / Skip.")}</p>
      <div id="pf-status" class="callout" style="margin:0 0 10px"></div>
      <div class="field"><label>${t("Title font")}</label><select id="pf-title">${fontOptions(o.title_font)}</select></div>
      <div class="field" style="margin-top:10px"><label>${t("Reason and button font")}</label><select id="pf-text">${fontOptions(o.text_font)}</select></div>
    </div>
    <div class="card" style="margin-top:12px">
      <label class="check" style="font-weight:600"><input type="checkbox" id="pf-pause" ${o.pause_menu !== false ? "checked" : ""}/> ${t("Pause menu")}</label>
      <p class="muted small" style="margin:6px 0 0">${t("Translates the tabs (Map, Brief, Stats …), the menus and the buttons at the bottom right, in the game's own colours. Subtitle reading pauses while the menu is open.")}</p>
    </div>
    <div class="card" style="margin-top:12px">
      <label class="check" style="font-weight:600"><input type="checkbox" id="pf-cap" ${o.captions !== false ? "checked" : ""}/> ${t("Captions in the middle of a black screen")}</label>
      <p class="muted small" style="margin:6px 0 0">${t("Place and time cards such as “Ludendorff, North Yankton, nine years ago.”: the translation appears centred just below the English, for as long as it is on screen.")}</p>
    </div>
    <button id="pf-areas" class="btn wide">${t("Use GTA V read areas (subtitles + top-left instructions)")}</button>` : ""}`;
  $("pf-profile").addEventListener("change", (e) => { cfg.profile = e.target.value; renderProfile(); save(); });
  if (cfg.profile !== "gta5") return;
  $("pf-mf").addEventListener("change", (e) => { o.mission_failed = e.target.checked; save(); });
  $("pf-pause").addEventListener("change", (e) => { o.pause_menu = e.target.checked; save(); });
  $("pf-cap").addEventListener("change", (e) => { o.captions = e.target.checked; save(); });
  for (const [id, key] of [["pf-title", "title_font"], ["pf-text", "text_font"]]) {
    const sel = $(id);
    sel.style.fontFamily = `'${sel.value}'`;
    sel.addEventListener("change", () => { o[key] = sel.value; sel.style.fontFamily = `'${sel.value}'`; renderProfile(); save(); });
  }
  $("pf-areas").addEventListener("click", () => {
    if (!confirm(t("Replace your read areas with the GTA V ones?"))) return;
    cfg.areas = GTA5_AREAS();
    selected = cfg.areas[0].id;
    renderAreas();
    save();
  });
  updateProfileStatus();
}
function updateProfileStatus() {
  const s = $("pf-status");
  if (!s) return;
  s.textContent = screen.layout
    ? t("On screen now ✓ — {n} button(s) found", { n: screen.layout.buttons.length }) + (screen.layout.reason_en ? t(", reason: “{reason}”", { reason: screen.layout.reason_en }) : "")
    : t("Waiting for the Mission Failed page…");
}

function applyTheme() {
  const dark = cfg.theme === "dark";
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  $("theme").textContent = dark ? t("☀ Light") : t("🌙 Dark");
}

(async function init() {
  cfg = await invoke("get_config");
  I18N.setLang(cfg.ui_lang || "en");
  const uiSel = $("ui-lang");
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
  $("overlays_enabled").addEventListener("change", (e) => { cfg.overlays_enabled = e.target.checked; save(false); });
  try {
    FONT_LIST = await invoke("list_fonts");
  } catch (e) {
    log({ error: t("Could not list fonts: {e}", { e }) });
  }
  renderEngines();
  renderProfile();
  renderAreas();
  renderStyleForm();
  renderTiming();
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
    updateProfileStatus();
  } catch {}
  document.fonts.ready.then(drawLayer); // re-measure once the bundled fonts have loaded
  renderObs();
  await loadWindows();
})();
