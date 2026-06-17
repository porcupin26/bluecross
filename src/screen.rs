//! Screen-size detection for Wayland compositors.
//!
//! BlueCross is Wayland-only. Detection tries, in order: GNOME/Mutter (D-Bus),
//! wlroots (`wlr-randr`), and KDE Plasma (`kscreen-doctor`). If all fail, the
//! caller should set `screen_width`/`screen_height` explicitly in the config.

use std::process::Command;

use regex::Regex;

pub fn detect_screen_size() -> (i32, i32) {
    if let Some(size) = detect_gnome_mutter() {
        return size;
    }
    if let Some(size) = detect_wlr_randr() {
        return size;
    }
    if let Some(size) = detect_kde_wayland() {
        return size;
    }
    log::warn!("Could not detect screen size on Wayland, using default 1920x1080");
    (1920, 1080)
}

/// Match the first occurrence of `pattern` (two integer capture groups) in `text`.
fn match_dims(pattern: &str, text: &str) -> Option<(i32, i32)> {
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    let w = caps.get(1)?.as_str().parse().ok()?;
    let h = caps.get(2)?.as_str().parse().ok()?;
    Some((w, h))
}

fn detect_gnome_mutter() -> Option<(i32, i32)> {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.gnome.Mutter.DisplayConfig",
            "--object-path",
            "/org/gnome/Mutter/DisplayConfig",
            "--method",
            "org.gnome.Mutter.DisplayConfig.GetCurrentState",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // The current mode is the resolution tuple flagged with 'is-current': <true>.
    match_dims(r"\('(\d+)x(\d+)@[\d.]+',[^)]*'is-current':\s*<true>", &text)
}

fn detect_wlr_randr() -> Option<(i32, i32)> {
    let output = Command::new("wlr-randr").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains("current") {
            if let Some(size) = match_dims(r"(\d+)x(\d+)\s+px", line) {
                return Some(size);
            }
        }
    }
    None
}

fn detect_kde_wayland() -> Option<(i32, i32)> {
    let output = Command::new("kscreen-doctor")
        .arg("--outputs")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains("Geometry:") {
            if let Some(size) = match_dims(r"Geometry:\s*\d+,\d+\s+(\d+)x(\d+)", line) {
                return Some(size);
            }
        }
    }
    None
}
