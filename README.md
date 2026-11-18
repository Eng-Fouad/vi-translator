<p align="center">
  <img src="src-tauri/icons/icon.png" alt="VI Translator logo" width="160" />
</p>

<h1 align="center">VI Translator - Desktop</h1>

<p align="center">Live subtitles in your language for Grand Theft Auto VI, read straight from the screen.</p>

---

## What it does

- Reads the game's **subtitles and instructions** and shows them translated, in an overlay or in OBS.
- Translates **game screens** too (menus, mission results, captions), as support for each one arrives
  through automatic updates.
- Works with a **game window**, a **capture card**, or an **OBS source**. It never touches the game.
- Keeps highlighted words in their **colours** and button **icons** in place.
- Shows the live translations on a **phone or tablet** on your home network (Advanced tab, off by default).
- **Updates itself**: when a new version is out it asks first, then checks its signature and restarts.

## Languages

- **Translation:** Arabic, Hindi, Bengali, Urdu, Indonesian, Turkish, Vietnamese, Persian, Thai,
  Tamil, Telugu, Marathi, Ukrainian, Dutch, Filipino, Malay, Swahili, Romanian, Greek, Czech.
- **Translators:** free offline model (some languages), DeepL, Azure, Google, LibreTranslate or Claude.
- **App interface:** 33 languages, including right-to-left layouts.

## Download

Get the executable for your system from the [latest release](https://github.com/Eng-Fouad/vi-translator-desktop/releases/latest).
It's portable, so there's nothing to install:

| System | File |
|---|---|
| Windows 10/11 (x64, ARM) | `vi-translator-windows-x64.exe`, `vi-translator-windows-arm64.exe` |
| Linux (x64, ARM) | `vi-translator-linux-x64`, `vi-translator-linux-arm64` |
| macOS (Apple silicon) | `vi-translator-macos-arm64` |

- **Linux** needs WebKitGTK 4.1 and Tesseract with English, e.g. on Ubuntu/Debian
  `sudo apt install libwebkit2gtk-4.1-0 tesseract-ocr tesseract-ocr-eng`. Window capture works on X11
  (on Wayland, use a capture card or OBS).
- **macOS**: run `chmod +x` on the file and allow it in *System Settings > Privacy & Security*. Window
  capture needs the *Screen Recording* permission. The macOS version hasn't been tested on a Mac yet.

## Getting started

1. Run the app.
2. **Source** tab: pick the game window, capture card or OBS source.
3. **Translator** tab: pick your language and a translation service.
4. **Game profile** tab: choose *Grand Theft Auto VI*.

## Privacy

*Help improve translations* (Advanced tab) is **on by default**. Every few minutes it sends the
English lines you saw (subtitles, instructions, captions) to the project server, so they can be
translated better for everyone. It's anonymous: a random install ID, no account, no screenshots,
and never menus (that's where player names appear). Each batch says which game the lines are from
and whether one of that game's own screens was seen while playing. Lines seen while the picture
looks like a different game the app knows are not sent. Turn it off and nothing more is sent.

The app also checks GitHub every hour for a new version. The
phone page is only served on your home network, and only while you have it turned on; open it by
the address the app shows (the PC's IP address), not a name.

## Build

- **Windows:** Rust (MSVC) and the Visual Studio C++ build tools.
- **Linux:** Rust and the packages listed in [`.github/workflows/release.yml`](.github/workflows/release.yml)
  (WebKitGTK, Tesseract, PipeWire, V4L2 …).
- **macOS:** Rust and the Xcode command-line tools.

```sh
cd src-tauri
cargo run                  # run (debug)
cargo build --release      # target/release/vi-translator(.exe)
cargo test
```

Releases are built by GitHub Actions when a `vX.Y.Z` tag is pushed (the version must be newer than
the latest release). Each executable is signed for the auto-update with the `UPDATE_SIGNING_KEY_PEM`
secret.

## Credits

- Included fonts (Lalezar, Changa, Cairo, Tajawal, Noto Sans Arabic) are under the SIL Open Font License.
- The offline translator uses OPUS-MT models by Helsinki-NLP (ONNX exports by Xenova).

---

<sub>VI Translator is an unofficial fan project, not affiliated with or endorsed by Rockstar Games
or Take-Two Interactive. "Grand Theft Auto" and "GTA" are trademarks of Take-Two Interactive Software, Inc.</sub>
