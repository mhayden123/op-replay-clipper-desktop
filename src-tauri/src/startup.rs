use std::fs;
use std::path::PathBuf;

use crate::bootstrap::{self, check_environment, EnvError};
use crate::constants::*;
use crate::ipc::{send_error, send_status};
use crate::paths::{data_dir, find_glidekit_project, resolve_uv};
use crate::platform::{check_nvidia, check_wsl};
use crate::server::{start_server, stop_server, wait_for_server};
use crate::state::AppState;
use tauri::Manager;

pub fn startup_sequence(
    win: &tauri::WebviewWindow,
    handle: &tauri::AppHandle,
    #[allow(unused_variables)] clean_install: bool,
    #[allow(unused_variables)] resource_path: &Option<PathBuf>,
) {
    // --- Phase 0: Clean install if requested via --clean ---
    #[cfg(target_os = "windows")]
    if clean_install {
        eprintln!("[startup] --clean: deleting project directory");
        send_status(win, "Clean install - removing old files...");
        let project = data_dir().join("glidekit");
        if project.exists() {
            let _ = fs::remove_dir_all(&project);
        }
        let marker = data_dir().join("bootstrap-complete");
        let _ = fs::remove_file(&marker);
    }

    // --- Phase 1: Ensure environment is ready ---
    send_status(win, "Checking environment...");
    let env_result = check_environment();

    let (project_dir, uv_path) = match env_result {
        Ok(env) => env,

        // On Windows: auto-bootstrap if project/uv is missing
        #[cfg(target_os = "windows")]
        Err(EnvError::ProjectNotFound) | Err(EnvError::UvNotFound) => {
            eprintln!("Environment not ready, starting bootstrap...");
            send_status(
                win,
                if clean_install {
                    "Clean install - re-downloading all files..."
                } else {
                    "First-time setup - this takes a few minutes..."
                },
            );

            // Find the bootstrap script: bundled resources -> download from GitHub
            let script = bootstrap::windows::find_bootstrap_script(resource_path).or_else(|| {
                eprintln!("[startup] Bundled script not found, downloading...");
                send_status(win, "Downloading setup script...");
                bootstrap::windows::download_bootstrap_script()
            });

            if let Some(ref script_path) = script {
                if !bootstrap::windows::run_bootstrap(win, script_path, clean_install) {
                    send_error(
                        win,
                        "Setup failed. Check the log at:\n%LOCALAPPDATA%\\glidekit\\bootstrap-app.log\n\nOr run debug_bootstrap.bat from the install directory.",
                    );
                    return;
                }
                send_status(win, "Setup complete! Starting server...");
            } else {
                send_error(
                    win,
                    "Could not find or download the setup script. Check your internet connection and try reinstalling.",
                );
                return;
            }

            // Re-check environment after bootstrap
            match check_environment() {
                Ok(env) => env,
                Err(retry_reason) => {
                    eprintln!("[startup] Post-bootstrap check failed: {}", retry_reason);
                    send_error(
                        win,
                        "Setup completed but environment is still not ready.\nCheck: %LOCALAPPDATA%\\glidekit\\bootstrap-app.log",
                    );
                    return;
                }
            }
        }

        // On Linux/macOS: auto-bootstrap if dependencies are missing
        #[cfg(not(target_os = "windows"))]
        Err(EnvError::ProjectNotFound)
        | Err(EnvError::UvNotFound)
        | Err(EnvError::OpenpilotNotInstalled) => {
            eprintln!("Environment not ready, starting bootstrap...");
            send_status(win, "First-time setup — this may take a while...");

            // Step 1: Install uv if missing
            if resolve_uv().is_none() && !bootstrap::linux::install_uv(win) {
                send_error(
                    win,
                    "Failed to install uv. Check your internet connection and try again.",
                );
                return;
            }

            // Step 2: Clone project if missing
            let project = if find_glidekit_project().is_none() {
                match bootstrap::linux::clone_project(win) {
                    Some(p) => p,
                    None => {
                        send_error(win, "Failed to download GlideKit. Check your internet connection and try again.");
                        return;
                    }
                }
            } else {
                find_glidekit_project().unwrap()
            };

            // Step 3: Run install.sh if openpilot isn't set up
            let openpilot_root = std::env::var("OPENPILOT_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| data_dir().join("openpilot"));

            // Clean up partial openpilot clone from a previous failed attempt.
            // If the directory exists but .git doesn't, git clone will refuse
            // to clone into a non-empty directory (exit 128).
            if openpilot_root.exists() && !openpilot_root.join(".git").exists() {
                eprintln!(
                    "[bootstrap] Removing partial openpilot clone at {:?}",
                    openpilot_root
                );
                let _ = fs::remove_dir_all(&openpilot_root);
            }

            if !openpilot_root.join(".venv/bin/python").exists() {
                // openpilot's installer calls `sudo apt-get` on its own.
                // Without cached credentials, the prompt is invisible in the
                // subprocess and the install hangs silently.
                if !bootstrap::linux::check_sudo_available() {
                    send_error(
                        win,
                        "Installing openpilot requires sudo access.\n\nOpen a terminal and run:\n  sudo -v\n\nThen re-launch GlideKit. Credentials will be cached for ~15 minutes, long enough for the install to finish.",
                    );
                    return;
                }
                send_status(win, "Installing dependencies — this takes 10-20 minutes...");
                if !bootstrap::linux::run_install_script(win, &project) {
                    send_error(
                        win,
                        "Install script failed.\n\nCheck the log for the actual error:\n  ~/.glidekit/install.log\n\nCommon causes:\n  - Missing system packages — open a terminal and run:\n      sudo apt-get install -y build-essential cmake ffmpeg git curl git-lfs\n  - Sudo credentials not cached — run `sudo -v` in a terminal first\n  - Then re-launch GlideKit",
                    );
                    return;
                }
            }

            send_status(win, "Setup complete! Starting server...");

            // Re-check environment after bootstrap
            match check_environment() {
                Ok(env) => env,
                Err(retry_reason) => {
                    eprintln!("[startup] Post-bootstrap check failed: {}", retry_reason);
                    send_error(
                        win,
                        &format!(
                            "Setup completed but environment is still not ready ({}).\nTry running: cd ~/glidekit-native && ./install.sh",
                            retry_reason
                        ),
                    );
                    return;
                }
            }
        }

        // Non-recoverable errors (Windows post-bootstrap failures)
        #[cfg(target_os = "windows")]
        Err(err) => {
            let msg = match err {
                EnvError::ProjectNotFound => {
                    "Setup failed — GlideKit project not found after bootstrap."
                }
                EnvError::UvNotFound => {
                    "uv not found after setup. Try reinstalling the application."
                }
                EnvError::OpenpilotNotInstalled => {
                    "openpilot not installed. Run ./install.sh in the GlideKit project directory."
                }
            };
            send_error(win, msg);
            return;
        }
    };

    // --- Phase 2: Report platform capabilities ---
    if cfg!(target_os = "macos") {
        eprintln!("macOS — VideoToolbox hardware acceleration available.");
    } else if check_nvidia() {
        eprintln!("NVIDIA GPU detected.");
    } else {
        eprintln!("No NVIDIA GPU detected. CPU rendering will be used.");
    }

    if cfg!(windows) {
        if check_wsl() {
            eprintln!("WSL detected — UI render types available.");
        } else {
            eprintln!("WSL not detected — only non-UI render types available.");
        }
    }

    // --- Phase 3: Start server ---
    send_status(win, "Starting server...");
    let child = match start_server(&project_dir, &uv_path) {
        Ok(c) => c,
        Err(msg) => {
            send_error(win, &msg);
            return;
        }
    };

    let state = handle.state::<AppState>();
    match state.server_process.lock() {
        Ok(mut proc) => *proc = Some(child),
        Err(_) => {
            eprintln!("Failed to store server process handle");
            send_error(win, "Internal error: could not track server process.");
            return;
        }
    }

    // --- Phase 4: Wait for server ---
    send_status(win, "Waiting for server...");
    if !wait_for_server() {
        if let Ok(mut proc) = state.server_process.lock() {
            stop_server(&mut proc);
        }
        send_error(
            win,
            "Server failed to start.\n\nCheck the log for details:\n  ~/.glidekit/server.log\n\nCommon fixes:\n  - Port 7860 already in use (close other GlideKit instances)\n  - Missing dependencies: cd ~/glidekit-native && ./install.sh",
        );
        return;
    }

    // --- Phase 5: Redirect to web UI ---
    let _ = win.eval(format!("window.location.href = '{}'", SERVER_URL));

    // --- Phase 6 (Windows): Check if WSL setup can continue ---
    #[cfg(target_os = "windows")]
    {
        use std::thread;
        use std::time::Duration;

        // Wait for the page to load, then check WSL status
        thread::sleep(Duration::from_secs(2));
        if check_wsl() {
            // WSL exists — check if GlideKit setup is incomplete
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
            {
                if let Ok(resp) = client.get(&format!("{}/api/wsl/status", SERVER_URL)).send() {
                    if let Ok(body) = resp.text() {
                        let needs_setup = serde_json::from_str::<serde_json::Value>(&body)
                            .map(|v| {
                                v.get("glidekit_installed") == Some(&serde_json::Value::Bool(false))
                                    || v.get("openpilot_installed")
                                        == Some(&serde_json::Value::Bool(false))
                            })
                            .unwrap_or(false);
                        if needs_setup {
                            eprintln!("[startup] WSL detected but GlideKit setup incomplete — prompting user");
                            let _ = win.eval("setTimeout(function(){ showWslDialog(); }, 1000)");
                        }
                    }
                }
            }
        }
    }
}
