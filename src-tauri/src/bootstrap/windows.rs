use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::constants::BOOTSTRAP_SCRIPT_URL;
use crate::ipc::send_status;
use crate::paths::data_dir;

/// Find the bootstrap.ps1 script — checks many locations and logs each attempt.
pub fn find_bootstrap_script(resource_dir: &Option<PathBuf>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Tauri resource dir (primary)
    if let Some(ref res_dir) = resource_dir {
        eprintln!("[bootstrap-find] Tauri resource_dir: {:?}", res_dir);
        candidates.push(res_dir.join("resources").join("bootstrap.ps1"));
        candidates.push(res_dir.join("bootstrap.ps1"));
        if let Some(parent) = res_dir.parent() {
            candidates.push(parent.join("resources").join("bootstrap.ps1"));
        }
    } else {
        eprintln!("[bootstrap-find] Tauri resource_dir: None");
    }

    // Next to the executable (NSIS installs here)
    if let Ok(exe) = std::env::current_exe() {
        eprintln!("[bootstrap-find] Executable: {:?}", exe);
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("resources").join("bootstrap.ps1"));
            candidates.push(exe_dir.join("bootstrap.ps1"));
            // NSIS $INSTDIR is typically the exe's directory
            if let Some(grandparent) = exe_dir.parent() {
                candidates.push(grandparent.join("resources").join("bootstrap.ps1"));
            }
        }
    }

    // Check all candidates
    for path in &candidates {
        let exists = path.exists();
        eprintln!(
            "[bootstrap-find]   {:?} -> {}",
            path,
            if exists { "FOUND" } else { "not found" }
        );
        if exists {
            return Some(path.clone());
        }
    }

    eprintln!("[bootstrap-find] Script not found in any candidate location");
    None
}

/// Download bootstrap.ps1 from GitHub as a last resort.
pub fn download_bootstrap_script() -> Option<PathBuf> {
    let target = data_dir().join("bootstrap.ps1");
    eprintln!("[bootstrap-download] Downloading bootstrap.ps1 from GitHub...");

    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
                 Invoke-WebRequest -Uri '{}' \
                 -OutFile '{}'",
                BOOTSTRAP_SCRIPT_URL,
                target.to_string_lossy()
            ),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status();

    match result {
        Ok(s) if s.success() && target.exists() => {
            eprintln!("[bootstrap-download] Downloaded to {:?}", target);
            Some(target)
        }
        Ok(s) => {
            eprintln!(
                "[bootstrap-download] Download failed (exit code {:?})",
                s.code()
            );
            None
        }
        Err(e) => {
            eprintln!("[bootstrap-download] PowerShell launch failed: {}", e);
            None
        }
    }
}

/// Run the bootstrap.ps1 script with live progress updates to the window.
pub fn run_bootstrap(window: &tauri::WebviewWindow, script: &std::path::Path, clean: bool) -> bool {
    use std::io::BufRead;

    eprintln!("[bootstrap-run] Script: {:?}", script);
    eprintln!("[bootstrap-run] Script exists: {}", script.exists());
    eprintln!(
        "[bootstrap-run] Script size: {:?}",
        fs::metadata(script).map(|m| m.len())
    );
    eprintln!("[bootstrap-run] Clean: {}", clean);
    send_status(
        window,
        if clean {
            "Clean install - re-downloading all files..."
        } else {
            "Setting up GlideKit..."
        },
    );

    fs::create_dir_all(data_dir()).ok();

    let script_path = script.to_string_lossy().to_string();
    let mut args = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script_path,
    ];
    if clean {
        args.push("-Clean".to_string());
    }
    let result = Command::new("powershell.exe")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    match result {
        Ok(mut child) => {
            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        // Show step markers and checkpoints in the UI
                        if line.starts_with("==>") {
                            let msg = line.trim_start_matches("==>").trim();
                            send_status(window, msg);
                        } else if line.contains("[OK]") {
                            let msg = line.trim();
                            send_status(window, msg);
                        } else if line.contains("[FAIL]") {
                            let msg = line.trim();
                            send_status(window, msg);
                        }
                        eprintln!("[bootstrap] {}", line);
                    }
                }
            }

            let status = child.wait();
            match status {
                Ok(s) if s.success() => {
                    eprintln!("[bootstrap-run] Completed successfully");
                    true
                }
                Ok(s) => {
                    eprintln!("[bootstrap-run] Failed with exit code: {:?}", s.code());
                    false
                }
                Err(e) => {
                    eprintln!("[bootstrap-run] Wait error: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("[bootstrap-run] Failed to launch PowerShell: {}", e);
            false
        }
    }
}
