//! The snapshot's keyboard shortcut: pressed anywhere (in the game too), the app comes to the front
//! and translates the whole picture; closing the picture goes back to the window that was in front.

use std::sync::atomic::{AtomicIsize, Ordering};

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// The window in front when the shortcut was pressed (the game's): where closing the picture returns.
static BEFORE: AtomicIsize = AtomicIsize::new(0);

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                pressed(app);
            }
        })
        .build()
}

fn pressed<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window("main") else { return };
    // (pressed again with the app in front: the window closes the picture if one is up)
    let ours = w.is_focused().unwrap_or(false);
    if !ours {
        BEFORE.store(crate::capture::foreground_window(), Ordering::Relaxed);
    }
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit_to("main", "snap-hotkey", ours);
}

/// Use `keys` ("Control+Shift+KeyS", "F8", "" = none) as the shortcut, instead of any other.
pub fn set<R: Runtime>(app: &AppHandle<R>, keys: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if keys.is_empty() {
        return Ok(());
    }
    let sc: Shortcut = keys.parse().map_err(|e| format!("{e}"))?;
    gs.register(sc).map_err(|e| format!("{e} (another app may use these keys)"))
}

/// Back to the window that was in front when the shortcut was pressed.
pub fn back() {
    let h = BEFORE.swap(0, Ordering::Relaxed);
    if h == 0 {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
        let hwnd = HWND(h as *mut _);
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use tauri_plugin_global_shortcut::Shortcut;

    #[test]
    fn the_windows_key_names_are_shortcuts() {
        // as the settings window writes them (modifiers, then the key's code)
        for keys in ["F8", "Control+Shift+KeyS", "Alt+Digit1", "Control+Alt+F12", "Shift+Space", "Super+KeyT", "Control+Numpad5"] {
            assert!(keys.parse::<Shortcut>().is_ok(), "{keys}");
        }
    }
}
