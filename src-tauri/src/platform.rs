use std::process::{Command, Stdio};

use crate::env_sanitize::CommandExt;

pub fn check_nvidia() -> bool {
    let smi = if cfg!(windows) {
        "nvidia-smi.exe"
    } else {
        "nvidia-smi"
    };
    Command::new(smi)
        .arg("-L")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .sanitize_env()
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
pub fn check_wsl() -> bool {
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    // Quick check: if wsl.exe doesn't exist, skip the slow invocation
    let system32 = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let wsl_path = Path::new(&system32).join("System32").join("wsl.exe");
    if !wsl_path.exists() {
        eprintln!("[wsl] wsl.exe not found at {:?}", wsl_path);
        return false;
    }
    // Spawn with a timeout — wsl.exe can hang on machines without WSL
    match Command::new("wsl.exe")
        .args(["--list", "--verbose"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Wait up to 5 seconds
            for _ in 0..10 {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            return false;
                        }
                        // Read stdout after process exits
                        let mut output = String::new();
                        if let Some(mut stdout) = child.stdout.take() {
                            use std::io::Read;
                            let _ = stdout.read_to_string(&mut output);
                        }
                        return output.contains("Running");
                    }
                    Ok(None) => {
                        thread::sleep(Duration::from_millis(500));
                    }
                    Err(_) => return false,
                }
            }
            // Timed out — kill and move on
            eprintln!("[wsl] check timed out after 5s, killing");
            let _ = child.kill();
            let _ = child.wait();
            false
        }
        Err(e) => {
            eprintln!("[wsl] failed to spawn wsl.exe: {}", e);
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn check_wsl() -> bool {
    false
}
