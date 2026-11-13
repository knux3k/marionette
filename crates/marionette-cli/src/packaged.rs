//! Packaged games (decision 0140): a binary started with no arguments that finds
//! `game.package.json` beside itself opens a window on the package's entry scene. Kept here,
//! without the packaging crate, so lean game binaries carry only these few lines.

use marionette_agent::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// File name and format tag, equal to `marionette_package::{MANIFEST, FORMAT}` (tested).
pub(crate) const MANIFEST: &str = "game.package.json";
pub(crate) const FORMAT: &str = "marionette.package";

pub(crate) struct Packaged {
    dir: PathBuf,
    title: String,
    entry: String,
    window: String,
    /// The game has an `XrRig`: try the headset first (decision 0190).
    vr: bool,
}

/// The manifest beside the running executable, if any.
pub(crate) fn find() -> Option<Packaged> {
    let exe = std::env::current_exe().ok()?;
    read(exe.parent()?)
}

pub(crate) fn read(dir: &Path) -> Option<Packaged> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if v["format"] != FORMAT {
        return None;
    }
    Some(Packaged {
        dir: dir.to_path_buf(),
        title: v["title"].as_str().unwrap_or("Marionette").to_string(),
        entry: v["entry"].as_str()?.to_string(),
        window: v["window"].as_str().unwrap_or("1280x720").to_string(),
        vr: v["vr"] == true,
    })
}

/// Plays the packaged game in a window until it is closed.
pub(crate) fn play(engine: &Engine, game: &Packaged) -> ExitCode {
    detach_console();
    marionette_host::init();
    let _ = marionette_host::run_dir();
    // Scene paths are relative to the package folder.
    let _ = std::env::set_current_dir(&game.dir);
    let scene = game.dir.join(&game.entry).display().to_string();
    if let Some(code) = play_vr(engine, game, &scene) {
        marionette_host::shutdown();
        return code;
    }
    let result = crate::render::windowed(crate::render::WindowedRun {
        engine,
        path: &scene,
        ticks: None,
        seed: 0,
        camera: None,
        size: &game.window,
        input: None,
        record: None,
        mute: false,
        title: Some(&game.title),
        window: crate::render::WindowFlags {
            present: "vsync".into(),
            frame_latency: 2,
            ..Default::default()
        },
    });
    marionette_host::shutdown();
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            report(&game.title, &e.to_string());
            ExitCode::FAILURE
        }
    }
}

/// A VR game plays in the headset when an OpenXR runtime and headset are there; `None` means
/// no headset (or not a VR game), so the caller opens a window instead.
#[cfg(feature = "xr")]
fn play_vr(engine: &Engine, game: &Packaged, scene: &str) -> Option<ExitCode> {
    if !game.vr {
        return None;
    }
    let result = crate::xr::run(crate::xr::XrRun {
        engine,
        path: scene,
        seed: 0,
        camera: None,
        msaa: 4,
        frames: None,
        ticks: None,
        scale: 1.0,
        input: None,
        record: None,
        mute: false,
    });
    match result {
        Ok(_) => Some(ExitCode::SUCCESS),
        Err(e) if e.0.iter().any(|d| d.code.starts_with("xr.no_")) => None,
        Err(e) => {
            report(&game.title, &e.to_string());
            Some(ExitCode::FAILURE)
        }
    }
}

#[cfg(not(feature = "xr"))]
fn play_vr(_: &Engine, game: &Packaged, _: &str) -> Option<ExitCode> {
    // Built without OpenXR: a VR game opens in a window.
    let _ = game.vr;
    None
}

/// On Windows, a game started by double-click gets a console window of its own (the binary
/// is a console program so the CLI works); let it go. A console the player started the game
/// from stays.
#[cfg(windows)]
fn detach_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 2];
    // SAFETY: the buffer is valid for two process ids.
    let attached = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    if attached == 1 {
        // SAFETY: plain Win32 call with no arguments.
        unsafe { FreeConsole() };
    }
}

#[cfg(not(windows))]
fn detach_console() {}

/// Tells the player why the game did not start.
fn report(title: &str, message: &str) {
    eprintln!("{title}: {message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (t, m) = (wide(title), wide(message));
        // SAFETY: both strings are NUL-terminated UTF-16 that live across the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONERROR) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_is_read_and_other_files_are_ignored() {
        let tmp = marionette_host::temp_dir("packaged-test").unwrap();
        assert!(read(tmp.path()).is_none());
        std::fs::write(tmp.path().join(MANIFEST), r#"{"format":"other","entry":"a"}"#).unwrap();
        assert!(read(tmp.path()).is_none());
        std::fs::write(
            tmp.path().join(MANIFEST),
            r#"{"format":"marionette.package","title":"Cloudstep","entry":"scenes/main.scene.json","window":"1280x800"}"#,
        )
        .unwrap();
        let p = read(tmp.path()).unwrap();
        assert_eq!(
            (p.title.as_str(), p.entry.as_str(), p.window.as_str()),
            ("Cloudstep", "scenes/main.scene.json", "1280x800")
        );
    }

    #[cfg(feature = "package")]
    #[test]
    fn names_match_the_packaging_crate() {
        assert_eq!(MANIFEST, marionette_package::MANIFEST);
        assert_eq!(FORMAT, marionette_package::FORMAT);
    }
}
