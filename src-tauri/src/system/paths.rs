//! Paths for bundled and user-configured executables.

use std::path::{Path, PathBuf};

/// Resolves the directory containing the running executable.
pub fn executable_dir() -> std::io::Result<PathBuf> {
    std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| std::io::Error::other("executable has no parent directory"))
}

/// Data root: `<exe dir>\data`.
///
/// Runtime data sits beside the executable so a portable copy of the folder
/// migrates everything. Falls back to the working directory, then the temp
/// directory, rather than silently writing into `%APPDATA%` where the user
/// cannot find it.
pub fn data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(std::env::temp_dir)
        .join("data")
}

/// Legacy-compatible log path: `<exe dir>\data\logs\autofox.log`.
pub fn log_file() -> PathBuf {
    data_dir().join("logs").join("autofox.log")
}

/// Daily log path: `<exe dir>\data\logs\YYYY-MM-DD.log`.
pub fn daily_log_file(date: &str) -> PathBuf {
    data_dir().join("logs").join(format!("{date}.log"))
}

fn resource_path(name: &str) -> std::io::Result<PathBuf> {
    let bundled = executable_dir()?.join(name);
    if bundled.is_file() {
        return Ok(bundled);
    }

    #[cfg(debug_assertions)]
    if let Some(project_root) = option_env!("CARGO_MANIFEST_DIR") {
        let development = PathBuf::from(project_root).join("..").join(name);
        if development.is_file() {
            return Ok(development);
        }
    }

    Ok(bundled)
}

/// Resolves a user-configured executable when it exists, otherwise returns the bundled path.
pub fn configured_or_bundled(configured: Option<&str>, bundled: PathBuf) -> PathBuf {
    configured
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or(bundled)
}

/// Returns the bundled yt-dlp path.
pub fn bundled_ytdlp() -> std::io::Result<PathBuf> {
    resource_path("yt-dlp.exe")
}

/// Returns the bundled ffmpeg path.
pub fn bundled_ffmpeg() -> std::io::Result<PathBuf> {
    resource_path("lib/ffmpeg.exe")
}

/// Returns the bundled ffprobe path.
pub fn bundled_ffprobe() -> std::io::Result<PathBuf> {
    resource_path("lib/ffprobe.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_paths_follow_executable_directory() {
        let exe = std::env::current_exe().expect("current exe");
        let exe_dir = exe.parent().expect("exe parent");
        assert_eq!(data_dir(), exe_dir.join("data"));
        assert_eq!(
            log_file(),
            exe_dir.join("data").join("logs").join("autofox.log")
        );
        assert_eq!(
            daily_log_file("2026-09-26"),
            exe_dir.join("data").join("logs").join("2026-09-26.log")
        );
    }
}
