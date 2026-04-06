use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::env_sanitize::CommandExt;
#[cfg(target_os = "windows")]
use crate::constants::REGISTRY_KEY;

/// Get the app data directory (~/.glidekit).
pub fn data_dir() -> PathBuf {
    let dir = dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".glidekit");
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Warning: failed to create data directory {:?}: {}", dir, e);
    }
    dir
}

/// Resolve the openpilot root directory.
/// Honors the `OPENPILOT_ROOT` env var; falls back to `~/.glidekit/openpilot`.
/// Used by both `check_environment` and `start_server` so the check and the
/// server-side env agree on the path.
pub fn openpilot_root() -> PathBuf {
    std::env::var("OPENPILOT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_dir().join("openpilot"))
}

/// Read a string value from the app's registry key (Windows only).
/// Key: HKCU\Software\GlideKit\{name}
#[cfg(target_os = "windows")]
pub fn read_registry_string(name: &str) -> Option<String> {
    let output = Command::new("reg.exe")
        .args([
            "query",
            REGISTRY_KEY,
            "/v",
            name,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // reg.exe output: "    ValueName    REG_SZ    ValueData"
    for line in text.lines() {
        if line.contains("REG_SZ") {
            if let Some(pos) = line.find("REG_SZ") {
                let value = line[pos + 6..].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Locate the GlideKit project directory (must contain clip.py).
pub fn find_glidekit_project() -> Option<PathBuf> {
    // Explicit override
    if let Ok(dir) = std::env::var("GLIDEKIT_PROJECT_DIR") {
        let p = PathBuf::from(&dir);
        if p.join("clip.py").exists() {
            return Some(p);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Windows: check registry first (most reliable — set by bootstrap.ps1)
    #[cfg(target_os = "windows")]
    {
        if let Some(reg_path) = read_registry_string("ProjectDir") {
            let p = PathBuf::from(&reg_path);
            eprintln!("[find-project] Registry ProjectDir: {:?} exists={}", p, p.join("clip.py").exists());
            if p.join("clip.py").exists() {
                return Some(p);
            }
        }
    }

    // Sibling of the running executable (prefer glidekit-native over glidekit)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for ancestor in [parent, parent.parent().unwrap_or(parent)] {
                candidates.push(ancestor.join("glidekit-native"));
                candidates.push(ancestor.join("glidekit"));
                if ancestor.join("clip.py").exists() {
                    return Some(ancestor.to_path_buf());
                }
            }
        }
    }

    // Current working directory
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("clip.py").exists() {
            return Some(cwd);
        }
        candidates.push(cwd.join("glidekit-native"));
        candidates.push(cwd.join("glidekit"));
    }

    // Home directory (prefer glidekit-native over glidekit)
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("glidekit-native"));
        candidates.push(home.join("glidekit"));
    }

    // Windows: %LOCALAPPDATA%\glidekit\ (default bootstrap location)
    if cfg!(windows) {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("glidekit")
                    .join("glidekit"),
            );
        }
    }

    candidates.into_iter().find(|p| p.join("clip.py").exists())
}

/// Resolve the `uv` binary path.
pub fn resolve_uv() -> Option<String> {
    let uv_name = if cfg!(windows) { "uv.exe" } else { "uv" };

    // Try PATH first
    if Command::new(uv_name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .sanitize_env()
        .status()
        .is_ok()
    {
        eprintln!("[resolve-uv] Found on PATH: {}", uv_name);
        return Some(uv_name.to_string());
    }

    let home = dirs::home_dir()?;
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        // Search all known Windows install locations for uv.exe
        let mut paths = vec![
            home.join("AppData").join("Roaming").join("Python").join("Python312").join("Scripts").join("uv.exe"),
            home.join("AppData").join("Roaming").join("Python").join("Python313").join("Scripts").join("uv.exe"),
            home.join("AppData").join("Roaming").join("Python").join("Scripts").join("uv.exe"),
            home.join(".local").join("bin").join("uv.exe"),
            home.join(".cargo").join("bin").join("uv.exe"),
        ];
        // Also check LOCALAPPDATA Python paths
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            paths.push(local.join("Programs").join("Python").join("Python312").join("Scripts").join("uv.exe"));
            paths.push(local.join("Programs").join("Python").join("Python313").join("Scripts").join("uv.exe"));
        }
        paths
    } else {
        // Linux + macOS: check common install locations
        vec![
            home.join(".local/bin/uv"),
            home.join(".cargo/bin/uv"),
            home.join(".local/share/uv/bin/uv"),
            PathBuf::from("/usr/local/bin/uv"),                 // macOS Intel Homebrew + manual installs
            PathBuf::from("/usr/bin/uv"),                       // system package manager
            PathBuf::from("/opt/homebrew/bin/uv"),              // macOS Apple Silicon Homebrew
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin/uv"), // Linux Homebrew
        ]
    };

    for path in &candidates {
        let exists = path.exists();
        eprintln!("[resolve-uv]   {:?} -> {}", path, if exists { "FOUND" } else { "no" });
        if exists {
            let resolved = path.to_string_lossy().to_string();
            eprintln!("[resolve-uv] Using: {}", resolved);
            return Some(resolved);
        }
    }

    eprintln!("[resolve-uv] uv not found in any location");
    None
}
