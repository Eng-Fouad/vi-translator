//! User settings, saved as JSON in the app's config folder.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A rectangle of the captured window that is read with OCR. Coordinates are fractions (0..1)
/// of the window's client area, so they survive window resizes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Area {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Where the Arabic box goes relative to the area: above | below | left | right | over | custom
    pub placement: String,
    /// With match_size: fine-tune on top of the matched size. Without: multiplies the global font size.
    pub font_scale: f64,
    /// Size the Arabic from the English text's measured height, so both look the same size.
    pub match_size: bool,
    /// Font for this area's Arabic; empty = use the main font from Style.
    pub font_family: String,
    /// Where the Arabic box sits in its region for the preset placements: "" (automatic) or
    /// "<top|middle|bottom>-<left|center|right>", e.g. "top-left". Custom placement uses out_*align.
    pub text_pos: String,
    /// Box behind this area's Arabic: "" = main style, "none" = no box, outlined text like game
    /// subtitles, "game" = GTA-style dark see-through box with square corners.
    pub background: String,
    /// placement = "custom": the region the Arabic is drawn in, as fractions of the window.
    pub out_x: f64,
    pub out_y: f64,
    pub out_w: f64,
    pub out_h: f64,
    /// placement = "custom": where the text box sits inside that region.
    pub out_valign: String, // top | middle | bottom
    pub out_halign: String, // right | center | left
    /// Only read text centred in the picture (game subtitles are): a word elsewhere in the area — a
    /// menu's "Browse" at the bottom right — is not a subtitle.
    pub centered_only: bool,
}

impl Default for Area {
    fn default() -> Self {
        Self {
            id: "area".into(),
            name: "Area".into(),
            enabled: true,
            x: 0.1,
            y: 0.1,
            w: 0.3,
            h: 0.1,
            placement: "above".into(),
            font_scale: 1.0,
            match_size: true,
            centered_only: false,
            font_family: String::new(),
            text_pos: String::new(),
            background: String::new(),
            out_x: 0.1,
            out_y: 0.6,
            out_w: 0.8,
            out_h: 0.15,
            out_valign: "bottom".into(),
            out_halign: "center".into(),
        }
    }
}

/// How the Arabic text looks. Sizes are in pixels for a 1080p window and scale with the window.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub font_family: String,
    pub font_size: f64,
    pub bold: bool,
    pub text_color: String,
    pub bg_color: String,
    pub bg_opacity: f64,
    pub outline_color: String,
    pub outline_width: f64,
    pub shadow: bool,
    pub padding: f64,
    pub radius: f64,
    pub line_height: f64,
    /// Colour Arabic words the same as highlighted English words.
    pub keep_colors: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            font_family: "Segoe UI".into(),
            font_size: 26.0,
            bold: true,
            text_color: "#ffffff".into(),
            bg_color: "#000000".into(),
            bg_opacity: 0.65,
            outline_color: "#000000".into(),
            outline_width: 0.0,
            shadow: true,
            padding: 10.0,
            radius: 10.0,
            line_height: 1.5,
            keep_colors: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Exact title of the window to capture.
    pub window_title: String,
    /// local | deepl | azure | google | libre | claude
    pub engine: String,
    /// Language to translate into (see `langs::LANGS`): "ar", "hi", "tr" …
    pub target_lang: String,
    /// Look for a newer version of the app (on start and daily) and offer it.
    pub update_check: bool,
    /// Live translations page for other devices on the home network (off by default).
    pub lan_page: bool,
    pub lan_port: u16,
    /// Language of the app's own interface: "en" or "ar" (right to left).
    pub ui_lang: String,
    pub deepl_key: String,
    pub azure_key: String,
    pub azure_region: String,
    pub google_key: String,
    pub libre_url: String,
    pub libre_key: String,
    pub claude_key: String,
    pub claude_model: String,
    /// Claude only: msa | egyptian | gulf | levantine
    pub dialect: String,
    pub interval_ms: u64,
    pub stable_ms: u64,
    pub clear_after_ms: u64,
    pub overlays_enabled: bool,
    /// App window theme: light | dark
    pub theme: String,
    /// Experiment: remember translated lines and reuse them instantly (translation memory).
    pub memory_enabled: bool,
    /// Show Arabic that came from the memory in green (to see what was reused).
    pub memory_tint: bool,
    /// Help improve translations: share the English lines seen (subtitles, instructions, captions)
    /// with the project server, anonymously, in batches. On by default; off in the Advanced tab.
    pub share_lines: bool,
    /// Random id of this install (no account), made on first start.
    pub install_id: String,
    /// Project server; empty = the built-in one.
    pub server_url: String,
    /// Apply read-area and font updates for the game profile automatically.
    pub auto_layout: bool,
    /// Version of the last layout update applied.
    pub layout_version: u32,
    /// OBS on this PC: its WebSocket server (Tools → WebSocket Server Settings), used to read an OBS
    /// source ("obs:<name>" as the source) while OBS owns the capture card.
    pub obs_port: u16,
    pub obs_password: String,
    /// Port of the Arabic-only page for an OBS Browser Source (http://127.0.0.1:<port>/obs).
    pub obs_page_port: u16,
    /// Full frames per second read for text (OCR, profiles, saved frames).
    pub capture_fps: u32,
    /// Frames per second of the live preview in the app (small frames, only while it's visible).
    pub preview_fps: u32,
    /// Debugging: save captured frames as PNG files while on.
    pub debug_save_frames: bool,
    /// Debugging: minimum time between saved frames.
    pub debug_frame_interval_ms: u64,
    /// Game profile with hand-made screen layouts: none | gta5
    pub profile: String,
    pub profile_opts: ProfileOpts,
    pub areas: Vec<Area>,
    pub style: Style,
}

/// Options for the game profile's special screens.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileOpts {
    /// GTA V: translate the "MISSION FAILED" page (title, reason, buttons).
    pub mission_failed: bool,
    /// GTA V: translate the pause menu (tabs, buttons).
    pub pause_menu: bool,
    /// GTA V: translate captions shown in the middle of a black screen (place / time cards).
    pub captions: bool,
    /// Font for big titles such as "فشلت المهمة".
    pub title_font: String,
    /// Font for the smaller screen text (reason, buttons).
    pub text_font: String,
}

impl Default for ProfileOpts {
    fn default() -> Self {
        Self { mission_failed: true, pause_menu: true, captions: true, title_font: "Lalezar".into(), text_font: "Tajawal".into() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            window_title: "PS Remote Play".into(),
            engine: "local".into(),
            deepl_key: String::new(),
            azure_key: String::new(),
            azure_region: String::new(),
            google_key: String::new(),
            libre_url: "http://localhost:5000".into(),
            libre_key: String::new(),
            claude_key: String::new(),
            claude_model: "claude-haiku-5-5".into(),
            dialect: "msa".into(),
            interval_ms: 250,
            stable_ms: 300,
            clear_after_ms: 800,
            overlays_enabled: true,
            theme: "light".into(),
            memory_enabled: false,
            memory_tint: true,
            target_lang: "ar".into(),
            ui_lang: "en".into(),
            update_check: true,
            lan_page: false,
            lan_port: 8791,
            share_lines: true,
            install_id: String::new(),
            server_url: String::new(),
            auto_layout: true,
            layout_version: 0,
            obs_port: 4455,
            obs_password: String::new(),
            obs_page_port: 8790,
            capture_fps: 8,
            preview_fps: 30,
            debug_save_frames: false,
            debug_frame_interval_ms: 1000,
            profile: "gta5".into(),
            profile_opts: ProfileOpts::default(),
            areas: vec![
                Area {
                    id: "subtitles".into(),
                    name: "Subtitles".into(),
                    centered_only: true,
                    x: 0.10,
                    y: 0.78,
                    w: 0.80,
                    h: 0.17,
                    placement: "above".into(),
                    text_pos: "bottom-center".into(),
                    background: "none".into(),
                    font_family: "Tajawal".into(),
                    // matched size × 104% looks the same as GTA's subtitles (calibrated on real frames)
                    font_scale: 1.04,
                    ..Area::default()
                },
                Area {
                    id: "help".into(),
                    name: "Instructions (top left)".into(),
                    x: 0.01,
                    y: 0.02,
                    w: 0.30,
                    h: 0.22,
                    // right under GTA's own box: same left edge, at most its width, growing downward
                    placement: "under_text".into(),
                    text_pos: "top-left".into(),
                    font_family: "Tajawal".into(),
                    // square, dark see-through box like GTA's own instructions box
                    background: "game".into(),
                    // matched size × 92% looks the same as GTA's instruction text
                    font_scale: 0.92,
                    ..Area::default()
                },
            ],
            style: Style::default(),
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
