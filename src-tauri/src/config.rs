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
    /// The list of live translations (in the app and on the home-network page) keeps filling,
    /// whether or not the overlays are on.
    pub live_translations: bool,
    /// The preview's "NN fps" badge: shown or not, and in which corner of the picture
    /// (top-left | top-right | bottom-left | bottom-right).
    pub fps_badge: bool,
    pub fps_corner: String,
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
    /// Get test versions of game pages before everyone (Advanced).
    pub early_pages: bool,
    /// Switch to the game the picture shows (recognised by the games' pages scripts).
    pub auto_game: bool,
    /// The profile last used for each game (where switching to a game goes).
    pub game_profiles: std::collections::BTreeMap<String, String>,
    /// Version of the last layout update applied (from before per-game versions; see `layout_versions`).
    pub layout_version: u32,
    /// Version of the last layout update applied, by game.
    pub layout_versions: std::collections::BTreeMap<String, u32>,
    /// Games the server added (built-in profiles beyond the app's own `BUILTINS`), and the version of
    /// the list they came in.
    pub server_games: Vec<String>,
    pub games_version: u32,
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
    /// The game profile in use: a built-in one ("gta5", "gta6") or one of `profiles` (a copy).
    pub profile: String,
    /// The player's own profiles (copies they can change); the built-in ones are in `builtins`.
    pub profiles: Vec<Profile>,
    /// The built-in profiles as they are now (the app's defaults, tuned by layout updates from the
    /// server). Never changed by the player: copied to make one of theirs.
    pub builtins: Vec<Profile>,
    /// 1 once the setup from before profiles has been moved into them.
    pub profiles_version: u32,
    /// The profile in use, as the rest of the app reads it (kept equal to that profile by `normalize`):
    /// its game pages, read areas and translation text style.
    pub profile_opts: ProfileOpts,
    pub areas: Vec<Area>,
    pub style: Style,
}

/// A game profile: where the text is read and where its translation goes (read areas), how the
/// translation looks (text style), and the game's own pages it translates (Mission Failed …).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// The game it is for ("gta5" | "gta6"): which game pages it can recognise, and which shared
    /// translations and layout updates it gets. A copy keeps the game of the profile it came from.
    pub game: String,
    pub areas: Vec<Area>,
    pub style: Style,
    pub pages: ProfileOpts,
    /// How the game's text comes and goes, if it needs other timing than the app's (0 = the app's).
    pub timing: Timing,
}

/// Reading timing a game can bring (see `Config::timing`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Timing {
    pub interval_ms: u64,
    pub stable_ms: u64,
    pub clear_after_ms: u64,
}

impl Default for Profile {
    fn default() -> Self {
        builtin_profile("gta5").expect("built-in")
    }
}

/// Equal when written out (for settings that don't compare directly).
fn same_json<T: Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

/// The app's own built-in profiles, in the order they are listed (the server can add more games:
/// `Config::server_games`).
pub const BUILTINS: [&str; 2] = ["gta6", "gta5"];

/// A built-in profile as the app ships it.
pub fn builtin_profile(id: &str) -> Option<Profile> {
    let gta_areas = || {
        vec![
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
        ]
    };
    match id {
        "gta5" => Some(Profile { id: "gta5".into(), name: "Grand Theft Auto V".into(), game: "gta5".into(), areas: gta_areas(), style: Style::default(), pages: ProfileOpts::default(), timing: Timing::default() }),
        // GTA VI starts from GTA V's areas; layout updates from the server tune it once the game is out.
        // Its own pages aren't known yet: none are recognised.
        "gta6" => Some(Profile {
            id: "gta6".into(),
            name: "Grand Theft Auto VI".into(),
            game: "gta6".into(),
            areas: gta_areas(),
            style: Style::default(),
            pages: ProfileOpts { mission_failed: false, pause_menu: false, captions: false, ..ProfileOpts::default() },
            timing: Timing::default(),
        }),
        _ => None,
    }
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
    /// Switches a game's pages script declares itself (`pages.option`), by name; one not set here
    /// takes the script's default.
    pub more: std::collections::BTreeMap<String, bool>,
    /// Font for big titles such as "فشلت المهمة".
    pub title_font: String,
    /// Font for the smaller screen text (reason, buttons).
    pub text_font: String,
}

impl Default for ProfileOpts {
    fn default() -> Self {
        Self { mission_failed: true, pause_menu: true, captions: true, more: Default::default(), title_font: "Lalezar".into(), text_font: "Tajawal".into() }
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
            live_translations: true,
            fps_badge: false,
            fps_corner: "top-right".into(),
            theme: "dark".into(),
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
            layout_versions: Default::default(),
            early_pages: false,
            auto_game: false,
            game_profiles: Default::default(),
            server_games: Vec::new(),
            games_version: 0,
            obs_port: 4455,
            obs_password: String::new(),
            obs_page_port: 8790,
            capture_fps: 8,
            preview_fps: 30,
            debug_save_frames: false,
            debug_frame_interval_ms: 1000,
            profile: "gta6".into(),
            profiles: Vec::new(),
            builtins: BUILTINS.iter().filter_map(|id| builtin_profile(id)).collect(),
            // 0 here too: a settings file from before profiles has no such field and takes this
            // value, which is what tells `normalize` to move its setup into a profile
            profiles_version: 0,
            profile_opts: builtin_profile("gta6").expect("built-in").pages,
            areas: builtin_profile("gta6").expect("built-in").areas,
            style: Style::default(),
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Self {
        let mut c: Self = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        c.normalize();
        c
    }

    /// Reading timing in use (interval, settle, clear after; ms): the player's own where they moved
    /// the slider (Advanced), else the profile's game timing if it has one, else the app's.
    pub fn timing(&self) -> (u64, u64, u64) {
        let d = Config::default();
        let t = self.builtins.iter().chain(self.profiles.iter()).find(|p| p.id == self.profile).map(|p| p.timing.clone()).unwrap_or_default();
        let pick = |own: u64, default: u64, game: u64| if own != default || game == 0 { own } else { game };
        (pick(self.interval_ms, d.interval_ms, t.interval_ms), pick(self.stable_ms, d.stable_ms, t.stable_ms), pick(self.clear_after_ms, d.clear_after_ms, t.clear_after_ms))
    }

    /// A built-in profile (the app's own, or a game the server added)?
    pub fn is_builtin(&self, id: &str) -> bool {
        BUILTINS.contains(&id) || self.server_games.iter().any(|g| g == id)
    }

    /// The game the profile in use is for ("gta5" | "gta6" | one the server added).
    pub fn game(&self) -> String {
        if self.is_builtin(&self.profile) {
            return self.profile.clone();
        }
        self.profiles.iter().find(|p| p.id == self.profile).map(|p| p.game.clone()).unwrap_or_else(|| "gta5".into())
    }

    /// Keep the profiles consistent (after loading, and after the window saves):
    ///  - every built-in profile is there;
    ///  - the setup from before profiles becomes the player's own profile "My GTA V" if it differs
    ///    from the built-in one (nothing they tuned is lost);
    ///  - the profile in use exists, and the settings the app reads (areas, style, game pages) are
    ///    that profile's: a built-in one exactly as it is (it can't be changed), one of the
    ///    player's as they last saved it.
    pub fn normalize(&mut self) {
        for id in BUILTINS {
            if !self.builtins.iter().any(|p| p.id == id) {
                self.builtins.push(builtin_profile(id).expect("built-in"));
            }
        }
        let server_games = std::mem::take(&mut self.server_games);
        self.server_games = server_games.into_iter().filter(|g| self.builtins.iter().any(|p| &p.id == g)).collect();
        let keep: Vec<String> = self.builtins.iter().map(|p| p.id.clone()).filter(|id| self.is_builtin(id)).collect();
        self.builtins.retain(|p| keep.contains(&p.id));
        // (one per id: the first)
        let mut seen = std::collections::HashSet::new();
        self.builtins.retain(|p| seen.insert(p.id.clone()));
        for p in self.builtins.iter_mut() {
            p.game = p.id.clone();
            if let Some(d) = builtin_profile(&p.id) {
                p.name = d.name; // (built-in names aren't the player's to change)
            }
        }
        // (only a file from before profiles: no profiles of the player's yet, a built-in or "none" in use)
        let before_profiles = self.profiles.is_empty() && (BUILTINS.contains(&self.profile.as_str()) || self.profile == "none");
        if self.profiles_version == 0 && !before_profiles {
            self.profiles_version = 1;
        }
        if self.profiles_version == 0 {
            self.profiles_version = 1;
            // compared with the built-in profile it was using (GTA V before profiles; a new install: GTA VI)
            let base_id = if BUILTINS.contains(&self.profile.as_str()) { self.profile.clone() } else { "gta5".to_string() };
            let base = self.builtins.iter().find(|p| p.id == base_id).cloned().expect("built-in");
            let mine = Profile {
                id: "custom-1".into(),
                name: if base.id == "gta6" { "My GTA VI".into() } else { "My GTA V".into() },
                game: base.id.clone(),
                areas: self.areas.clone(),
                style: self.style.clone(),
                // "none" (no game pages) before profiles: keep them off
                pages: if self.profile == "none" { ProfileOpts { mission_failed: false, pause_menu: false, captions: false, ..self.profile_opts.clone() } } else { self.profile_opts.clone() },
                timing: base.timing.clone(),
            };
            if self.profile == "none" || !(same_json(&mine.areas, &base.areas) && same_json(&mine.style, &base.style) && same_json(&mine.pages, &base.pages)) {
                self.profile = mine.id.clone();
                self.profiles.push(mine);
            } else {
                self.profile = base.id;
            }
        }
        // a single layout version from before per-game ones: it was the game in use's
        if self.layout_versions.is_empty() && self.layout_version > 0 {
            self.layout_versions.insert(self.game(), self.layout_version);
        }
        // ids are unique; names are never empty
        let mut seen = std::collections::HashSet::new();
        let builtin_ids: Vec<String> = self.builtins.iter().map(|p| p.id.clone()).collect();
        self.profiles.retain(|p| !builtin_ids.contains(&p.id) && !p.id.is_empty() && seen.insert(p.id.clone()));
        for p in self.profiles.iter_mut() {
            if p.name.trim().is_empty() {
                p.name = "My profile".into();
            }
            if !builtin_ids.contains(&p.game) {
                p.game = "gta5".into();
            }
        }
        // the profile each game was last played with (where switching to that game goes)
        let ids: Vec<String> = self.builtins.iter().chain(self.profiles.iter()).map(|p| p.id.clone()).collect();
        self.game_profiles.retain(|_, p| ids.contains(p));
        let active = self
            .builtins
            .iter()
            .chain(self.profiles.iter())
            .find(|p| p.id == self.profile)
            .cloned()
            .unwrap_or_else(|| self.builtins.iter().find(|p| p.id == "gta6").cloned().expect("built-in"));
        self.profile = active.id.clone();
        self.game_profiles.insert(active.game.clone(), active.id.clone());
        self.areas = active.areas;
        self.style = active.style;
        self.profile_opts = active.pages;
    }

    /// Switch to a game: the profile it was last played with, else its built-in one. False if it is
    /// the game in use already or not a game the app knows.
    pub fn switch_to_game(&mut self, game: &str) -> bool {
        if self.game() == game || !self.builtins.iter().any(|p| p.id == game) {
            return false;
        }
        let mine = self.game_profiles.get(game).filter(|id| self.profiles.iter().any(|p| &p.id == *id && p.game == game)).cloned();
        self.profile = mine.unwrap_or_else(|| game.to_string());
        self.normalize();
        true
    }

    /// The server's list of games (the layout update named "games"):
    /// `{"games": [{"id": "rdr2", "name": "Red Dead Redemption 2", "areas": [...], "style": {...}, "pages": {...}}]}`,
    /// each a built-in profile for a game the app doesn't carry itself. A newer list adds those games
    /// and replaces the ones it added before (the app's own games are never replaced; a game the list
    /// no longer has stays, so a profile in use never disappears). The games it changed, for their
    /// layout updates to be applied again.
    pub fn apply_games(&mut self, list: &serde_json::Value, version: u32) -> Vec<String> {
        if version <= self.games_version {
            return Vec::new();
        }
        let mut changed = Vec::new();
        for g in list["games"].as_array().into_iter().flatten() {
            let Ok(mut p) = serde_json::from_value::<Profile>(g.clone()) else { continue };
            let ok_id = !p.id.is_empty() && p.id.len() <= 16 && p.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
            if !ok_id || BUILTINS.contains(&p.id.as_str()) || p.name.trim().is_empty() {
                continue;
            }
            // a profile of the player's with that id (unlikely: theirs are "custom-…") gives way
            if self.profiles.iter().any(|x| x.id == p.id) {
                continue;
            }
            p.game = p.id.clone();
            self.builtins.retain(|x| x.id != p.id);
            self.builtins.push(p.clone());
            if !self.server_games.contains(&p.id) {
                self.server_games.push(p.id.clone());
            }
            // its layout update applies again over the new profile
            self.layout_versions.remove(&p.id);
            changed.push(p.id);
        }
        self.games_version = version;
        self.normalize();
        changed
    }

    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn a_games_timing_unless_the_player_set_their_own() {
        let mut c = Config::default();
        c.normalize();
        let d = Config::default();
        assert_eq!(c.timing(), (d.interval_ms, d.stable_ms, d.clear_after_ms));
        c.builtins.iter_mut().find(|p| p.id == c.profile).unwrap().timing = Timing { interval_ms: 0, stable_ms: 120, clear_after_ms: 1500 };
        assert_eq!(c.timing(), (d.interval_ms, 120, 1500));
        c.clear_after_ms = 900; // the player's own
        assert_eq!(c.timing(), (d.interval_ms, 120, 900));
    }

    #[test]
    fn switching_games_goes_to_the_profile_last_used_for_it() {
        let mut c = Config::default();
        c.normalize();
        c.profiles.push(Profile { id: "custom-5".into(), name: "Mine".into(), game: "gta5".into(), ..Profile::default() });
        c.profile = "custom-5".into();
        c.normalize();
        assert!(c.switch_to_game("gta6"));
        assert_eq!(c.profile, "gta6");
        assert!(!c.switch_to_game("gta6"), "already");
        assert!(!c.switch_to_game("nope"));
        assert!(c.switch_to_game("gta5"));
        assert_eq!(c.profile, "custom-5");
    }

    #[test]
    fn the_server_adds_games() {
        let mut c = Config::default();
        c.normalize();
        let list = serde_json::json!({"games": [
            {"id": "rdr2", "name": "Red Dead Redemption 2", "areas": [], "pages": {"more": {"camp": false}}},
            {"id": "gta5", "name": "Not this one"},
            {"id": "Bad Id", "name": "x"}
        ]});
        assert_eq!(c.apply_games(&list, 3), vec!["rdr2".to_string()]);
        assert!(c.is_builtin("rdr2") && !c.is_builtin("custom-1"));
        assert_eq!(c.builtins.iter().find(|p| p.id == "gta5").unwrap().name, "Grand Theft Auto V");
        assert!(c.apply_games(&list, 3).is_empty(), "same version: nothing new");
        c.profile = "rdr2".into();
        c.normalize();
        assert_eq!((c.game().as_str(), c.areas.len(), c.profile_opts.more.get("camp")), ("rdr2", 0, Some(&false)));
        // kept across a save and a load
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        let mut back = back;
        back.normalize();
        assert_eq!(back.profile, "rdr2");
        // a copy of it is for that game
        back.profiles.push(Profile { id: "custom-9".into(), name: "Mine".into(), game: "rdr2".into(), ..Profile::default() });
        back.normalize();
        assert_eq!(back.profiles[0].game, "rdr2");
        // a newer list that drops it: the game stays
        assert!(back.apply_games(&serde_json::json!({"games": []}), 4).is_empty());
        assert!(back.is_builtin("rdr2"));
    }

    /// A settings file from before profiles, with the given areas.
    fn old_settings(areas: Vec<Area>, profile: &str) -> Config {
        let mut c = Config::default();
        c.profiles_version = 0;
        c.profiles.clear();
        c.builtins.clear();
        c.profile = profile.into();
        c.areas = areas;
        c.profile_opts = ProfileOpts::default();
        c
    }

    #[test]
    fn a_new_install_starts_on_the_built_in_gta_vi() {
        let mut c = Config::default();
        c.normalize();
        assert_eq!(c.profile, "gta6");
        assert!(c.profiles.is_empty(), "no copy is made for a new install");
        // and loading a file saved by it doesn't make one later either
        let mut again: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        again.normalize();
        assert!(again.profiles.is_empty());
    }

    #[test]
    fn a_settings_file_from_before_profiles_is_moved_over() {
        // (no "profiles_version" in it: it must count as 0)
        let mut c: Config = serde_json::from_str(r#"{"profile":"gta5","areas":[{"id":"subtitles","name":"Subtitles","y":0.5}]}"#).unwrap();
        c.normalize();
        assert_eq!(c.profile, "custom-1");
        assert_eq!(c.areas[0].y, 0.5);
    }

    #[test]
    fn an_old_setup_that_was_changed_becomes_the_players_own_profile() {
        let mut moved = builtin_profile("gta5").unwrap().areas;
        moved[0].y = 0.5; // the player had moved the subtitles area
        let mut c = old_settings(moved, "gta5");
        c.normalize();
        assert_eq!(c.profile, "custom-1");
        assert_eq!(c.profiles[0].name, "My GTA V");
        assert_eq!(c.profiles[0].game, "gta5");
        assert_eq!(c.areas[0].y, 0.5, "nothing tuned is lost");
        assert_eq!(c.game(), "gta5", "a copy keeps its game (pages, shared lines)");
        assert_eq!(c.builtins.len(), BUILTINS.len());
    }

    #[test]
    fn an_old_setup_like_the_built_in_one_just_uses_it() {
        let mut c = old_settings(builtin_profile("gta5").unwrap().areas, "gta5");
        c.normalize();
        assert_eq!(c.profile, "gta5");
        assert!(c.profiles.is_empty());
    }

    #[test]
    fn a_built_in_profile_cant_be_changed() {
        let mut c = Config::default();
        c.profile = "gta5".into();
        c.normalize();
        // the window sends changed areas while a built-in profile is in use: they don't stick
        c.areas[0].x = 0.9;
        c.style.font_size = 99.0;
        c.normalize();
        assert_eq!(c.areas[0].x, builtin_profile("gta5").unwrap().areas[0].x);
        assert_eq!(c.style.font_size, Style::default().font_size);
        // the built-in itself can't be renamed or made another game's either
        c.builtins[0].name = "Mine".into();
        c.builtins[0].game = "x".into();
        c.normalize();
        assert!(c.builtins.iter().all(|p| p.game == p.id && builtin_profile(&p.id).unwrap().name == p.name));
    }

    #[test]
    fn the_players_profile_keeps_their_changes_and_a_missing_one_falls_back() {
        let mut c = Config::default();
        let mut mine = builtin_profile("gta5").unwrap();
        mine.id = "custom-x".into();
        mine.name = "Mine".into();
        mine.areas[0].h = 0.3;
        c.profiles.push(mine);
        c.profile = "custom-x".into();
        c.normalize();
        assert_eq!(c.areas[0].h, 0.3);
        c.profile = "gone".into();
        c.normalize();
        assert_eq!(c.profile, "gta6", "an unknown profile falls back to a built-in one");
    }
}
