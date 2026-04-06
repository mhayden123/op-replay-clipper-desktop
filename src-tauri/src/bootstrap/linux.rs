use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::constants::GLIDEKIT_NATIVE_REPO;
use crate::env_sanitize::CommandExt;
use crate::ipc::send_status;
use crate::paths::data_dir;

/// Check whether the user has cached sudo credentials.
///
/// install.sh runs `sudo apt-get install` for build dependencies, and
/// openpilot's own installer also calls `sudo apt-get`. If credentials
/// aren't cached, the prompt appears in the subprocess's stdin — which is
/// invisible from the Tauri UI — and the install hangs silently. This
/// check surfaces the requirement up-front with a clear actionable message.
///
/// Returns true if `sudo -n true` succeeds (passwordless sudo OR cached
/// credentials). Returns false if sudo isn't installed or a password is
/// required.
pub fn check_sudo_available() -> bool {
    match Command::new("sudo")
        .args(["-n", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// Install uv via the official installer script and verify it exists.
pub fn install_uv(window: &tauri::WebviewWindow) -> bool {
    eprintln!("[bootstrap] Installing uv...");
    send_status(window, "Installing uv package manager...");

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("[bootstrap] Cannot determine home directory");
            return false;
        }
    };

    // Run installer with HOME explicitly set and AppImage's LD_LIBRARY_PATH
    // cleared so curl uses the system's libcurl instead of the bundled one.
    let result = Command::new("sh")
        .args(["-c", "curl -LsSf https://astral.sh/uv/install.sh | sh"])
        .env("HOME", &home)
        .sanitize_env()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(s) if !s.success() => {
            eprintln!("[bootstrap] uv install script failed (exit code {:?})", s.code());
            return false;
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to run uv installer: {}", e);
            return false;
        }
        _ => {}
    }

    // Verify uv actually landed on disk
    let uv_path = home.join(".local/bin/uv");
    if uv_path.exists() {
        eprintln!("[bootstrap] uv installed at {:?}", uv_path);
        true
    } else {
        let cargo_path = home.join(".cargo/bin/uv");
        if cargo_path.exists() {
            eprintln!("[bootstrap] uv installed at {:?}", cargo_path);
            true
        } else {
            eprintln!("[bootstrap] uv installer ran but binary not found");
            false
        }
    }
}

/// Clone the glidekit-native project to ~/glidekit-native.
pub fn clone_project(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let target = home.join("glidekit-native");

    if target.join("clip.py").exists() {
        eprintln!("[bootstrap] Project already exists at {:?}", target);
        return Some(target);
    }

    eprintln!("[bootstrap] Cloning glidekit-native...");
    send_status(window, "Downloading GlideKit...");

    let result = Command::new("git")
        .args([
            "clone",
            "--depth", "1",
            GLIDEKIT_NATIVE_REPO,
            &target.to_string_lossy(),
        ])
        .sanitize_env()
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(s) if s.success() && target.join("clip.py").exists() => {
            eprintln!("[bootstrap] Cloned to {:?}", target);
            Some(target)
        }
        Ok(s) => {
            eprintln!("[bootstrap] git clone failed (exit code {:?})", s.code());
            None
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to run git: {}", e);
            None
        }
    }
}

/// Strip ANSI escape codes from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit a letter (end of ANSI sequence)
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Run install.sh from the project directory with live progress to the UI.
pub fn run_install_script(window: &tauri::WebviewWindow, project_dir: &std::path::Path) -> bool {
    use std::io::{BufRead, Write};

    let script = project_dir.join("install.sh");
    if !script.exists() {
        eprintln!("[bootstrap] install.sh not found at {:?}", script);
        return false;
    }

    // Open the install log (truncate each run) so users can diagnose failures.
    // install.sh produces a lot of output; the UI only shows progress markers,
    // so without this file the full build output is unrecoverable.
    let install_log_path = data_dir().join("install.log");
    let mut install_log = match fs::File::create(&install_log_path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("[bootstrap] Warning: could not open install log: {}", e);
            None
        }
    };
    if let Some(ref mut log) = install_log {
        let _ = writeln!(log, "=== GlideKit install.sh log ===");
        let _ = writeln!(log, "project_dir: {:?}", project_dir);
        let _ = writeln!(log, "script: {:?}", script);
        let _ = writeln!(log, "---");
        let _ = log.flush();
    }
    eprintln!("[bootstrap] install.sh log: {:?}", install_log_path);

    eprintln!("[bootstrap] Running install.sh in {:?}", project_dir);
    send_status(window, "Running install script — this may take a while...");

    // Ensure uv is findable by augmenting PATH with common install locations.
    // Sudo credentials are pre-checked in the startup_sequence so apt is safe
    // to run here — it'll use cached credentials silently.
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("[bootstrap] Cannot determine home directory — install.sh cannot run");
            send_status(window, "Error: cannot find home directory");
            return false;
        }
    };
    let extra_paths = format!(
        "{}:{}",
        home.join(".local/bin").display(),
        home.join(".cargo/bin").display(),
    );
    let path = match std::env::var("PATH") {
        Ok(p) => format!("{}:{}", extra_paths, p),
        Err(_) => extra_paths,
    };

    // Run install.sh with stderr merged into stdout (2>&1) so git errors
    // and build failures are visible in the progress stream instead of lost.
    let cmd = format!("exec bash {} 2>&1", script.to_string_lossy());
    let result = Command::new("bash")
        .args(["-c", &cmd])
        .current_dir(project_dir)
        .env("PATH", &path)
        .env("HOME", &home)
        // Bypass user's global git config entirely — all repos cloned by
        // install.sh are public, no credentials or user prefs are needed.
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        // Clear AppImage's LD_LIBRARY_PATH — the bundled libs (libcurl,
        // libgnutls, etc.) conflict with system binaries like git-remote-https.
        // Child processes (git, cmake, scons) must use the system's own libs.
        // Clear AppImage's PYTHONHOME/PYTHONPATH — the bundled Python stdlib
        // breaks uv's build isolation, causing "No module named 'encodings'"
        // when hatchling tries to build openpilot as an editable install.
        .sanitize_env()
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    match result {
        Ok(mut child) => {
            // Stream stdout for progress updates
            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for raw_line in reader.lines().map_while(Result::ok) {
                    let clean = strip_ansi(&raw_line);
                    let trimmed = clean.trim();

                    // Persist every line to install.log for post-mortem diagnosis.
                    if let Some(ref mut log) = install_log {
                        let _ = writeln!(log, "{}", clean);
                    }

                    // Match install.sh output patterns:
                    //   "==> Step description"   — major step header
                    //   "  OK: detail"            — success substep
                    //   "  WARN: detail"          — warning
                    //   "  ERROR: detail"         — failure
                    //   "Installation complete!"  — final banner
                    if trimmed.starts_with("==>") {
                        let msg = trimmed.trim_start_matches("==>").trim();
                        if !msg.is_empty() {
                            send_status(window, msg);
                        }
                    } else if trimmed.starts_with("OK:") {
                        let msg = trimmed.trim_start_matches("OK:").trim();
                        if !msg.is_empty() {
                            send_status(window, msg);
                        }
                    } else if trimmed.starts_with("WARN:") || trimmed.starts_with("ERROR:") {
                        send_status(window, trimmed);
                    } else if trimmed.contains("Installation complete!") {
                        send_status(window, "Installation complete!");
                    }

                    eprintln!("[install.sh] {}", trimmed);
                }
            }

            match child.wait() {
                Ok(s) if s.success() => {
                    eprintln!("[bootstrap] install.sh completed successfully");
                    true
                }
                Ok(s) => {
                    eprintln!("[bootstrap] install.sh failed (exit code {:?})", s.code());
                    false
                }
                Err(e) => {
                    eprintln!("[bootstrap] install.sh wait error: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to launch install.sh: {}", e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_color_codes() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_removes_bold() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m text"), "bold text");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }

    #[test]
    fn strip_ansi_handles_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strip_ansi_handles_multiple_sequences() {
        assert_eq!(
            strip_ansi("\x1b[32m[OK]\x1b[0m: \x1b[1mbuild\x1b[0m done"),
            "[OK]: build done"
        );
    }
}
