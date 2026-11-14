//! Auto-update from GitHub Releases.
//!
//! On start-up a background thread asks GitHub for the latest release of
//! trilokmehta/vibe-craft. If its tag is newer than this build, the header
//! shows an "Update" button. Installing downloads the release's .exe next to
//! the running one, swaps them (Windows allows renaming a running exe), and
//! asks the user to restart.

use std::io::Read;
use std::sync::{Arc, Mutex};

const API: &str = "https://api.github.com/repos/trilokmehta/vibe-craft/releases/latest";
const AGENT: &str = concat!("vibe-craft-updater/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Checking,
    UpToDate,
    Available { tag: String, url: String },
    Downloading { tag: String, pct: f32 },
    Ready { tag: String },
    Failed(String),
}

pub type Shared = Arc<Mutex<State>>;

fn parse_ver(s: &str) -> Vec<u32> {
    s.trim_start_matches(|c| c == 'v' || c == 'V').split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    let (mut r, mut l) = (parse_ver(remote), parse_ver(local));
    let n = r.len().max(l.len());
    r.resize(n, 0);
    l.resize(n, 0);
    r > l
}

fn exe_paths() -> Option<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    Some((exe.clone(), exe.with_extension("exe.new"), exe.with_extension("exe.old")))
}

/// Remove the leftover from a previous update.
pub fn cleanup() {
    if let Some((_, new, old)) = exe_paths() {
        let _ = std::fs::remove_file(old);
        let _ = std::fs::remove_file(new);
    }
}

fn latest() -> Result<Option<(String, String)>, String> {
    let v: serde_json::Value = ureq::get(API).set("User-Agent", AGENT).set("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("no tag in release")?.to_string();
    if !is_newer(&tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets = v["assets"].as_array().ok_or("no assets in release")?;
    let url = assets
        .iter()
        .filter_map(|a| Some((a["name"].as_str()?, a["browser_download_url"].as_str()?)))
        .find(|(n, _)| n.to_ascii_lowercase().ends_with(".exe"))
        .map(|(_, u)| u.to_string())
        .ok_or("release has no .exe")?;
    Ok(Some((tag, url)))
}

pub fn check() -> Shared {
    let st: Shared = Arc::new(Mutex::new(State::Checking));
    let s2 = st.clone();
    std::thread::spawn(move || {
        let r = match latest() {
            Ok(Some((tag, url))) => State::Available { tag, url },
            Ok(None) => State::UpToDate,
            Err(e) => State::Failed(e),
        };
        eprintln!("update: {:?}", r);
        *s2.lock().unwrap() = r;
    });
    st
}

fn download(st: &Shared, tag: &str, url: &str) -> Result<(), String> {
    let (exe, new, old) = exe_paths().ok_or("can't find own exe")?;
    let resp = ureq::get(url).set("User-Agent", AGENT).call().map_err(|e| e.to_string())?;
    let total: f32 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let mut rd = resp.into_reader();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = rd.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if total > 0.0 {
            *st.lock().unwrap() = State::Downloading { tag: tag.to_string(), pct: buf.len() as f32 / total };
        }
    }
    if buf.len() < 1024 || &buf[..2] != b"MZ" {
        return Err("download is not a Windows exe".into());
    }
    std::fs::write(&new, &buf).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e.to_string());
    }
    Ok(())
}

pub fn install(st: &Shared) {
    let (tag, url) = match &*st.lock().unwrap() {
        State::Available { tag, url } => (tag.clone(), url.clone()),
        _ => return,
    };
    *st.lock().unwrap() = State::Downloading { tag: tag.clone(), pct: 0.0 };
    let s2 = st.clone();
    std::thread::spawn(move || {
        let r = match download(&s2, &tag, &url) {
            Ok(()) => State::Ready { tag },
            Err(e) => State::Failed(format!("install: {}", e)),
        };
        eprintln!("update: {:?}", r);
        *s2.lock().unwrap() = r;
    });
}

/// Start the freshly installed exe and quit this one.
pub fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).spawn();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::is_newer;
    #[test]
    fn versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2"));
    }
}
