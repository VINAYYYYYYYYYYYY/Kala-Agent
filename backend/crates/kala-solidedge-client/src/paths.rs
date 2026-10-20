//! Platform path selection for the Windows-local client.
//!
//! The target runtime is Windows. Linux is a development and test host only,
//! so a Linux default is deliberately absent: without an explicit token file
//! or `KALA_SOLIDEDGE_TOKEN_FILE` there is no token path to guess, and no
//! `~/.local` runtime directory is created.
//!
//! The Windows defaults are the same ones the bridge uses, so the client
//! resolves exactly the token file the service wrote.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The default bridge base URL.
pub const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:8797";
/// The environment variable that overrides the bridge base URL.
pub const BRIDGE_URL_ENVIRONMENT_KEY: &str = "KALA_SOLIDEDGE_BRIDGE_URL";
/// The environment variable that overrides the token file path.
pub const TOKEN_FILE_ENVIRONMENT_KEY: &str = "KALA_SOLIDEDGE_TOKEN_FILE";
/// The bridge data directory, relative to `%LOCALAPPDATA%`.
pub const DATA_DIR_PARTS: [&str; 2] = ["Kala", "SolidEdgeBridge"];
/// The default run directory, relative to `%LOCALAPPDATA%`.
pub const RUNS_DIR_PARTS: [&str; 3] = ["Kala", "SolidEdge", "runs"];
/// The token file name shared with the bridge.
pub const TOKEN_FILENAME: &str = "bridge.token";

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A snapshot of the process environment.
pub type Environment = BTreeMap<String, String>;

/// Take a snapshot of the process environment.
pub fn environment_snapshot() -> Environment {
    std::env::vars().collect()
}

/// Return whether the given platform is Windows.
///
/// Both the Python platform name `win32` and the Rust target name `windows`
/// are accepted; anything else is not Windows.
pub fn is_windows(platform: Option<&str>) -> bool {
    match platform {
        Some(name) => name == "win32" || name == "windows",
        None => cfg!(target_os = "windows"),
    }
}

fn environment_value(environment: Option<&Environment>, key: &str) -> Option<String> {
    match environment {
        Some(map) => map.get(key).cloned(),
        None => std::env::var(key).ok(),
    }
}

/// Return `%LOCALAPPDATA%`, or `None` when this is not a Windows host.
pub fn local_app_data(
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if let Some(base) = environment_value(environment, "LOCALAPPDATA") {
        if !base.is_empty() {
            return Some(PathBuf::from(base));
        }
    }
    if !is_windows(platform) {
        return None;
    }
    home_directory(environment, platform).map(|home| home.join("AppData").join("Local"))
}

/// Return the user's home directory, the way `Path.home()` would.
///
/// Windows uses `%USERPROFILE%`, falling back to `%HOMEDRIVE%%HOMEPATH%`. A
/// POSIX host uses `$HOME`; the value is never used to invent a Windows
/// runtime default.
pub fn home_directory(
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if is_windows(platform) {
        if let Some(profile) = environment_value(environment, "USERPROFILE") {
            if !profile.is_empty() {
                return Some(PathBuf::from(profile));
            }
        }
        let drive = environment_value(environment, "HOMEDRIVE").unwrap_or_default();
        let path = environment_value(environment, "HOMEPATH").unwrap_or_default();
        if !drive.is_empty() || !path.is_empty() {
            return Some(PathBuf::from(format!("{drive}{path}")));
        }
        return None;
    }
    environment_value(environment, "HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Return the Windows token path shared with the bridge, or `None`.
///
/// The bridge writes `bridge.token` under its data directory. The client
/// resolves the same name and directory; it never creates the file.
pub fn default_token_file(
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if !is_windows(platform) {
        return None;
    }
    let base = local_app_data(environment, platform)?;
    let mut path = base;
    for part in DATA_DIR_PARTS {
        path = path.join(part);
    }
    Some(path.join(TOKEN_FILENAME))
}

/// Return the Windows default run directory, or `None` off Windows.
pub fn default_output_dir(
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if !is_windows(platform) {
        return None;
    }
    let base = local_app_data(environment, platform)?;
    let mut path = base;
    for part in RUNS_DIR_PARTS {
        path = path.join(part);
    }
    Some(path)
}

/// Resolve the token file: flag, then environment, then Windows default.
pub fn resolve_token_file(
    explicit: Option<&str>,
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    if let Some(from_environment) = environment_value(environment, TOKEN_FILE_ENVIRONMENT_KEY) {
        if !from_environment.is_empty() {
            return Some(PathBuf::from(from_environment));
        }
    }
    default_token_file(environment, platform)
}

/// Resolve the bridge URL: flag, then environment, then loopback default.
pub fn resolve_bridge_url(explicit: Option<&str>, environment: Option<&Environment>) -> String {
    if let Some(explicit) = explicit {
        if !explicit.is_empty() {
            return explicit.to_owned();
        }
    }
    match environment_value(environment, BRIDGE_URL_ENVIRONMENT_KEY) {
        Some(value) if !value.is_empty() => value,
        _ => DEFAULT_BRIDGE_URL.to_owned(),
    }
}

/// Resolve the artifact directory: flag, then Windows default.
pub fn resolve_output_dir(
    explicit: Option<&str>,
    environment: Option<&Environment>,
    platform: Option<&str>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    default_output_dir(environment, platform)
}

/// Return a build id usable as one Windows directory name.
///
/// The bridge already constrains ids, but the client writes the value to
/// disk, so it re-validates: no separators, no drive or ADS colon, no
/// dot-only name, no reserved device name, and no trailing dot or space
/// (Windows strips them, which would silently collide two builds).
pub fn safe_build_id(build_id: &str) -> Result<String, String> {
    let mut characters = build_id.chars();
    let Some(first) = characters.next() else {
        return Err("the build id is not a safe directory name".to_owned());
    };
    if !first.is_ascii_alphanumeric() {
        return Err("the build id is not a safe directory name".to_owned());
    }
    if build_id.len() > 64 {
        return Err("the build id is not a safe directory name".to_owned());
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err("the build id is not a safe directory name".to_owned());
    }
    if build_id == "." || build_id == ".." {
        return Err("the build id is not a safe directory name".to_owned());
    }
    let stem = build_id
        .split_once('.')
        .map_or(build_id, |(head, _)| head)
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err("the build id is a reserved Windows device name".to_owned());
    }
    if build_id.ends_with('.') || build_id.ends_with(' ') {
        return Err("the build id must not end with a dot or space".to_owned());
    }
    Ok(build_id.to_owned())
}

/// Return the build-specific artifact directory under the caller's root.
///
/// Each build gets its own directory so two builds that both produce
/// `plate.step` never overwrite each other, and a re-download of one build is
/// idempotent without touching another.
pub fn build_output_dir(output_root: &Path, build_id: &str) -> Result<PathBuf, String> {
    Ok(output_root.join("builds").join(safe_build_id(build_id)?))
}

#[cfg(test)]
mod tests {
    use super::{
        BRIDGE_URL_ENVIRONMENT_KEY, DATA_DIR_PARTS, DEFAULT_BRIDGE_URL, Environment,
        TOKEN_FILE_ENVIRONMENT_KEY, TOKEN_FILENAME, build_output_dir, default_output_dir,
        default_token_file, resolve_bridge_url, resolve_output_dir, resolve_token_file,
        safe_build_id,
    };
    use std::path::PathBuf;

    fn environment(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn windows_defaults_use_local_app_data() {
        let env = environment(&[("LOCALAPPDATA", r"C:\Users\kala\AppData\Local")]);
        let token = default_token_file(Some(&env), Some("win32")).expect("token");
        let runs = default_output_dir(Some(&env), Some("win32")).expect("runs");
        let token = token.to_string_lossy().replace('\\', "/");
        let runs = runs.to_string_lossy().replace('\\', "/");
        assert!(token.ends_with("AppData/Local/Kala/SolidEdgeBridge/bridge.token"));
        assert!(runs.ends_with("AppData/Local/Kala/SolidEdge/runs"));
    }

    #[test]
    fn no_linux_token_or_run_default_exists() {
        assert!(default_token_file(Some(&environment(&[])), Some("linux")).is_none());
        assert!(default_output_dir(Some(&environment(&[])), Some("linux")).is_none());
    }

    #[test]
    fn a_linux_home_is_never_used_as_a_runtime_default() {
        let env = environment(&[("HOME", "/home/kala")]);
        assert!(resolve_token_file(None, Some(&env), Some("linux")).is_none());
        assert!(resolve_output_dir(None, Some(&env), Some("linux")).is_none());
    }

    #[test]
    fn explicit_and_environment_values_win() {
        let env = environment(&[
            (TOKEN_FILE_ENVIRONMENT_KEY, "/tmp/env.token"),
            (BRIDGE_URL_ENVIRONMENT_KEY, "http://127.0.0.1:9000"),
            ("LOCALAPPDATA", "/tmp/local"),
        ]);
        assert_eq!(
            resolve_token_file(Some("/tmp/flag.token"), Some(&env), Some("linux")),
            Some(PathBuf::from("/tmp/flag.token"))
        );
        assert_eq!(
            resolve_token_file(None, Some(&env), Some("linux")),
            Some(PathBuf::from("/tmp/env.token"))
        );
        assert_eq!(
            resolve_bridge_url(None, Some(&env)),
            "http://127.0.0.1:9000"
        );
        assert_eq!(
            resolve_bridge_url(None, Some(&environment(&[]))),
            DEFAULT_BRIDGE_URL
        );
        assert_eq!(
            resolve_output_dir(Some("/tmp/out"), Some(&env), Some("linux")),
            Some(PathBuf::from("/tmp/out"))
        );
    }

    #[test]
    fn the_token_filename_matches_the_bridge_contract() {
        assert_eq!(TOKEN_FILENAME, "bridge.token");
        assert_eq!(DATA_DIR_PARTS, ["Kala", "SolidEdgeBridge"]);
    }

    #[test]
    fn windows_without_local_app_data_falls_back_to_the_user_profile() {
        let env = environment(&[("USERPROFILE", r"C:\Users\kala")]);
        let token = default_token_file(Some(&env), Some("win32")).expect("token");
        let token = token.to_string_lossy().replace('\\', "/");
        assert!(token.ends_with("AppData/Local/Kala/SolidEdgeBridge/bridge.token"));
    }

    #[test]
    fn unsafe_build_ids_are_refused() {
        assert!(safe_build_id("build-1").is_ok());
        assert!(safe_build_id("../etc").is_err());
        assert!(safe_build_id("a/b").is_err());
        assert!(safe_build_id("CON").is_err());
        assert!(safe_build_id("con.txt").is_err());
        assert!(safe_build_id("build.").is_err());
        assert!(safe_build_id("").is_err());
    }

    #[test]
    fn a_build_gets_its_own_directory() {
        let directory =
            build_output_dir(std::path::Path::new("/tmp/runs"), "build-1").expect("dir");
        assert_eq!(directory, PathBuf::from("/tmp/runs/builds/build-1"));
    }
}
