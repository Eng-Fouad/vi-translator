# GTA 6 Translator (Rust + Tauri)

Live subtitles in your language for GTA games, read from the screen: Arabic, Hindi, Bengali,
Urdu, Indonesian, Turkish, Vietnamese, Persian, Thai, Tamil, Telugu, Marathi, Ukrainian, Dutch,
Filipino, Malay, Swahili, Romanian, Greek and Czech. These are widely spoken languages that the
games themselves don't offer. The free offline translator covers Arabic, Hindi, Indonesian,
Vietnamese, Ukrainian, Dutch, Romanian and Czech. Every language works with DeepL, Azure,
Google, LibreTranslate or Claude. The hand-made GTA V word lists for menus, buttons and Mission
Failed reasons are Arabic; in other languages those words go through the translator. GTA V is supported today; GTA VI
support is planned for its release. It translates subtitles, instructions, the Mission Failed
page, the pause menu and centre captions, shown in an overlay, the app's preview, or an OBS page.

> GTA 6 Translator is an unofficial fan project. It is not affiliated with, endorsed or sponsored by
> Rockstar Games or Take-Two Interactive. "Grand Theft Auto" and "GTA" are trademarks of Take-Two
> Interactive Software, Inc. The app does not modify the game or include any of its files: it only
> reads the picture on screen.

Desktop app that reads English text from a game window (e.g. **PS Remote Play**), translates it to
Arabic, and shows the Arabic in click-through overlays on top of the game.

- **Window picker:** capture any open window. Only that window is captured (Windows Graphics
  Capture), even while other windows cover it.
- **Live preview:** see the captured frames, and drag or resize the read areas on top of them.
  Exact positions are also editable in percent.
- **Translators:** Local (offline, free), DeepL, Microsoft Azure, Google Cloud, LibreTranslate, Claude.
- **Arabic text style:** any installed font (fonts with Arabic letters listed first), exact size,
  colours, box opacity, outline, shadow, padding, corners, line spacing. Each area can use its own font
  and size. Highlighted (coloured) game words keep their colour.
- **Placement:** each area's Arabic can go above, below, left of, right of, or over that area, or
  anywhere you like; *Text sits* chooses where the Arabic box lines up in that space (e.g. top-left).
  GTA V default: subtitles → above, centred; top-left instructions → below, top-left. Drag the blue Arabic region in the preview, or type its position in percent, and
  choose where the text sits inside it.
- **Light or dark:** light by default; the button in the header switches to dark and is remembered.
- **Game profiles** (Game profile tab), starting with **GTA V**:
  - **Mission Failed page:** detected automatically as soon as the red title fades in (also over
    the bright game scene), confirmed by OCR. Adds "فشلت المهمة" above the title in the same colour and size, the reason in Arabic under
    the English reason (common GTA phrases such as "X died." use built-in exact translations), and an
    Arabic Retry / Restart / Skip row that reuses the game's own button icons cut from the frame.
    While the page is up, the normal subtitle areas pause.
  - **GTA V read areas:** one click sets the subtitle and top-left instructions areas.
  - **Instruction phrasebook** (`src-tauri/src/phrasebook.rs`): exact Arabic for ~65 common GTA V
    instructions ("Press ✕ to take cover." → "اضغط ✕ للاحتماء."), with each button icon placed in the
    Arabic sentence. Matching ignores case/punctuation and small OCR slips but needs the same number of
    icons. Lines not in the book go to the chosen translator. Add lines by appending to `PHRASES`.
- **Controller button icons in text:** in lines like "Press ✕ to enter the vehicle" or "Aim using L2",
  the icon (✕ △ □ ◯, L1/L2/R1/R2, sticks R/L/R3/L3) is found — a solid, near-white shape with a dark
  symbol between the words, or a "word" OCR made of it ("@", "O", "R2") — cut out of the frame,
  and put back into the Arabic: Claude places it itself; other translators put it after the Arabic
  for the English word before it ("Press ✕" → after "اضغط"), or at the start/end of the line.
- **Included Arabic fonts** (`ui/fonts`, SIL Open Font License): Lalezar, Changa, Cairo, Tajawal.

## Layout

```
ui/                 HTML/CSS/JS (no build step)
  index.html app.js style.css   main window
  overlay.html                  one transparent overlay window per read area
  common.js                     placement + text rendering shared by both
src-tauri/src/
  capture.rs   window list + Windows Graphics Capture
  ocr.rs       Windows OCR, colour-highlight detection
  translate.rs local ONNX model (OPUS-MT en→ar) + DeepL/Azure/Google/Libre/Claude
  overlay.rs   overlay windows (topmost, click-through, never focused)
  main.rs      threads: capture → OCR → translate → overlays/preview
```

## Build and run

Requirements: Rust (MSVC toolchain), Visual Studio C++ build tools, WebView2 (built into Windows 11).

Keep the project in a **short path** such as `C:\dev\gta6-translator`. The MSVC linker can't handle
very long paths.

```powershell
cd src-tauri
cargo run                 # debug
cargo build --release     # optimised exe in target\release\gta6-translator.exe
```

For an installer, install the Tauri CLI once (`cargo install tauri-cli --version "^2"`), then run
`cargo tauri build`.

Tools for building and testing profiles (no UI):

```powershell
target\debug\gta6-translator.exe --grab "PS Remote Play" frame.png              # save one frame of a window
target\debug\gta6-translator.exe --test-image frame.png --area subtitles        # OCR + translate one read area
target\debug\gta6-translator.exe --test-image frame.png --region 0.1,0.78,0.8,0.17  # ...a custom region (fractions)
target\debug\gta6-translator.exe --test-screen frame.png                        # run the GTA V page detector
```

Set the environment variable `GTA6_TRANSLATOR_DEBUG=1` to print what the page detector sees.

**Save frames (debugging):** Advanced tab → Debugging → *Save frames to disk*. While it's on, a red
"● Saving frames" badge shows in the header. Frames are saved as full-size PNG files (one every 1 s
by default, adjustable) into `Pictures\GTA 6 Translator\debug-frames\<date_time>\`, named
`frame_00012_0011500ms.png` (12th frame, 11.5 s into the recording). Turning it off, from the switch
or the badge, opens that folder. It always starts off when the app launches. Saved frames work
directly with `--test-screen` and `--test-image`.

## GTA V pause menu

Game profile → **Pause menu** (on by default). The menu is recognised by its tab bar: a white
selected tab about 11 % of the width wide near the top. That check uses colour only and runs every
frame; OCR then confirms at least 3 known tab names. Each tab (Map, Brief, Stats, Settings, Game,
Online …) is covered by an Arabic replica with the same size and background colour and a matching
text colour. The bottom-right buttons ("Browse L1 R1 · Back ◯ · Select ✕") get Arabic labels with the
game's own icons, like on the Mission Failed page. The page goes the moment the tab bar does, and
the read areas pause while it is up.

The tab content is translated too (all tabs but Map). Twice a second the list on the left and the
panel on the right are read separately, enlarged ×2. Lines of a wrapped sentence are joined. Each
item gets an Arabic replica with the same place, background and text colour, aligned to the same
edge as the English and in one size for the whole menu. About 100 common menu words come with
built-in Arabic (Settings options, controller labels, Stats and Game items), matched even with
small OCR slips. Anything else goes to the translator once per session; its Arabic appears on the
next read and also goes into the translation memory.

Subtitle areas have **Only text centred in the picture** (on for GTA's subtitles): a line counts
only if its middle is within 4 % of the picture's centre. A menu word such as "Browse" at the bottom
right is therefore no longer read as a subtitle.

## Capture card and OBS

The **Source** tab lists three kinds of source:

- **Windows**, as before (e.g. PS Remote Play).
- **Capture cards and cameras** (🎥), read directly through Media Foundation, preferring 1920×1080.
  This works only while no other app holds the card: OBS's "Video Capture Device" source opens it
  exclusively, and the app then shows *"Hardware MFT failed to start streaming"*.
- **OBS sources** (📺). OBS keeps the card, and the app asks OBS for a snapshot of the source about 8
  times a second. In OBS, turn on *Tools → WebSocket Server Settings → Enable WebSocket server*, then
  enter its port (4455) and password in the Source tab and press ↻.

**The Arabic in OBS:** add a **Browser Source** above the capture-card source with the URL shown in
the Source tab (`http://127.0.0.1:8790/obs`). Set its width and height to the canvas size. The page
is transparent and draws the subtitles, instructions and Mission Failed page at the same places and
sizes as the overlays. It only listens on 127.0.0.1.

With a card or OBS source there is no game window on this PC, so the overlay windows stay hidden;
the Arabic shows in OBS and in the app's preview.

Test from the command line: `--list-devices`, `--grab-device "fifine Video Capture" out.png`,
`--grab-obs "Video Capture Device" out.png` (uses the saved OBS port and password).

## Frame rates

Two separate rates, in Advanced → Timing:

- **Frames captured for reading text** (default 8/s): full-size frames for OCR, game pages and
  saved frames.
- **Live preview frame rate** (default 30/s): small frames (≤ 1280 wide), made only while the app's
  preview is on screen. They stream to the preview as MJPEG from `http://127.0.0.1:8790/preview.mjpg`.

Measured on a moving 1280×720 window: preview 30.0 fps while text frames stayed at ~8/s.
`/state` on the same port also shows the capture status, for troubleshooting.

## Translation memory (experiment)

Advanced → Translation memory → **Use translation memory** (off by default). While it is on:

- Every translated line is remembered in `memory.jsonl`. Next time the same line shows up (any
  punctuation, small OCR slips, new icon images or highlight colours), its Arabic appears right away:
  it skips the translator *and* the `stable_ms` settle wait.
- The Mission Failed reason is remembered too. Built-in reasons ("X died.") are stored with the
  source `phrasebook`.
- The panel shows hits/misses and the average time from the English appearing to the Arabic being
  ready, for memory hits and translator lines, plus the time saved.

The memory is in `Documents\GTA 6 Translator	ranslation-db\`.

## Help improve translations (sharing)

Advanced → **Help improve translations** is **on by default**. While it is on, the app sends the
English lines you see to the GTA 6 Translator server about every 10 minutes, in batches. It sends
subtitles, instructions and screen captions, together with where and when each appeared. That lets
lines be translated better for everyone.

- **Never sent:** menus (that's where player names appear, e.g. the friends list), screenshots,
  your settings or API keys, or anything that identifies you.
- **Identification:** a random install id made on first start, with no account.
- **Turning it off:** turn the setting off and nothing more is sent. Lines seen while it was off
  are never sent later.
- **Local log:** each line's time on screen is also logged locally in
  `translation-db\lines-log.jsonl`.

**Automatic layout updates** (on by default) keeps the game profile's read areas, fonts and menu
words up to date. Changes you make yourself stay until the next update arrives.

## Where things are stored

- Settings, including any API keys you enter: `%APPDATA%\com.gta6translator.app\config.json`
- Layout updates and sync state: `%APPDATA%\com.gta6translator.app\`
- Free local model, downloaded once (~110 MB): `%APPDATA%\com.gta6translator.app\models\`

## Notes

- The Arabic overlays only appear while the captured window is the active window.
- When the app is started from inside another packaged Windows app (as when Claude launched it for
  testing), Windows redirects its `AppData` writes into that app's private folder. Run it yourself from
  RustRover or a terminal and it uses the normal `%APPDATA%\com.gta6translator.app`.
- The local model is the open OPUS-MT English→Arabic model (Helsinki-NLP), in the ONNX export
  by Xenova, run with ONNX Runtime.
