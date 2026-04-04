// OP Replay Clipper — Tauri Desktop App (Native Edition)
//
// Manages a local FastAPI server (uvicorn) as a child process.
// No Docker dependency — the rendering pipeline runs natively.
//
// Platform support:
//   Linux:   Full support (all render types, NVIDIA GPU)
//   macOS:   Full support (all render types, VideoToolbox GPU)
//   Windows: Auto-bootstrap on first launch, non-UI renders native, UI via WSL

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;

const SERVER_URL: &str = "http://localhost:7860";
const HEALTH_URL: &str = "http://localhost:7860/api/health";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct AppState {
    server_process: Mutex<Option<Child>>,
}

// ---------------------------------------------------------------------------
// Path detection
// ---------------------------------------------------------------------------

/// Get the app data directory (~/.op-replay-clipper).
fn data_dir() -> PathBuf {
    let dir = dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".op-replay-clipper");
    fs::create_dir_all(&dir).ok();
    dir
}

/// Locate the clipper project directory (must contain clip.py).
fn find_clipper_project() -> Option<PathBuf> {
    // Explicit override
    if let Ok(dir) = std::env::var("CLIPPER_PROJECT_DIR") {
        let p = PathBuf::from(&dir);
        if p.join("clip.py").exists() {
            return Some(p);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Sibling of the running executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for ancestor in [parent, parent.parent().unwrap_or(parent)] {
                candidates.push(ancestor.join("op-replay-clipper-native"));
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
        candidates.push(cwd.join("op-replay-clipper-native"));
    }

    // Home directory
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("op-replay-clipper-native"));
    }

    // Windows: %LOCALAPPDATA%\op-replay-clipper\ (where NSIS bootstrap puts it)
    if cfg!(windows) {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("op-replay-clipper")
                    .join("op-replay-clipper-native"),
            );
        }
    }

    candidates.into_iter().find(|p| p.join("clip.py").exists())
}

/// Resolve the `uv` binary path.
fn resolve_uv() -> Option<String> {
    let uv_name = if cfg!(windows) { "uv.exe" } else { "uv" };

    // Try PATH first
    if Command::new(uv_name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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
        vec![home.join(".local/bin/uv"), home.join(".cargo/bin/uv")]
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

// ---------------------------------------------------------------------------
// Environment checks
// ---------------------------------------------------------------------------

fn check_nvidia() -> bool {
    let smi = if cfg!(windows) {
        "nvidia-smi.exe"
    } else {
        "nvidia-smi"
    };
    Command::new(smi)
        .arg("-L")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn check_wsl() -> bool {
    Command::new("wsl.exe")
        .args(["--list", "--verbose"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Running"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn check_wsl() -> bool {
    false
}

/// Check that the clipper environment is ready to launch the server.
/// Returns (project_dir, uv_path) on success.
fn check_environment() -> Result<(PathBuf, String), String> {
    let project_dir = find_clipper_project().ok_or("project_not_found")?;
    let uv_path = resolve_uv().ok_or("uv_not_found")?;

    // On Linux/macOS, also need openpilot for UI renders
    if !cfg!(windows) {
        let python_path = data_dir().join("openpilot/.venv/bin/python");
        if !python_path.exists() {
            return Err("openpilot_not_installed".into());
        }
    }

    Ok((project_dir, uv_path))
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

fn start_server(project_dir: &PathBuf, uv_path: &str) -> Result<Child, String> {
    let openpilot_dir = data_dir().join("openpilot");
    let output_dir = data_dir().join("output");
    let data_dir_path = data_dir().join("data");

    fs::create_dir_all(&output_dir).ok();
    fs::create_dir_all(&data_dir_path).ok();

    let child = Command::new(uv_path)
        .args([
            "run",
            "python",
            "-m",
            "uvicorn",
            "web.server:app",
            "--host",
            "0.0.0.0",
            "--port",
            "7860",
        ])
        .current_dir(project_dir)
        .env("CLIPPER_HOME", data_dir().to_string_lossy().as_ref())
        .env("OPENPILOT_ROOT", openpilot_dir.to_string_lossy().as_ref())
        .env("CLIPPER_OUTPUT_DIR", output_dir.to_string_lossy().as_ref())
        .env("CLIPPER_DATA_DIR", data_dir_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start server: {}", e))?;

    eprintln!("Server process started (pid {})", child.id());
    Ok(child)
}

fn wait_for_server() -> bool {
    eprintln!("Waiting for server...");
    let start = Instant::now();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    while start.elapsed() < STARTUP_TIMEOUT {
        if let Ok(resp) = client.get(HEALTH_URL).send() {
            if resp.status().is_success() {
                eprintln!("Server ready!");
                return true;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    eprintln!("Server did not start in time.");
    false
}

fn stop_server(process: &mut Option<Child>) {
    if let Some(mut child) = process.take() {
        eprintln!("Stopping server (pid {})...", child.id());
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("Server stopped.");
    }
}

// ---------------------------------------------------------------------------
// Windows bootstrap — runs automatically when project is missing
// ---------------------------------------------------------------------------

/// Find the bootstrap.ps1 script — checks many locations and logs each attempt.
#[cfg(target_os = "windows")]
fn find_bootstrap_script(resource_dir: &Option<PathBuf>) -> Option<PathBuf> {
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
        eprintln!("[bootstrap-find]   {:?} -> {}", path, if exists { "FOUND" } else { "not found" });
        if exists {
            return Some(path.clone());
        }
    }

    eprintln!("[bootstrap-find] Script not found in any candidate location");
    None
}

/// Download bootstrap.ps1 from GitHub as a last resort.
#[cfg(target_os = "windows")]
fn download_bootstrap_script() -> Option<PathBuf> {
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
                 Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/mhayden123/op-replay-clipper-desktop/main/src-tauri/resources/bootstrap.ps1' \
                 -OutFile '{}'",
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
            eprintln!("[bootstrap-download] Download failed (exit code {:?})", s.code());
            None
        }
        Err(e) => {
            eprintln!("[bootstrap-download] PowerShell launch failed: {}", e);
            None
        }
    }
}

/// Run the bootstrap.ps1 script with live progress updates to the window.
#[cfg(target_os = "windows")]
fn run_bootstrap(window: &tauri::WebviewWindow, script: &std::path::Path) -> bool {
    use std::io::BufRead;

    eprintln!("[bootstrap-run] Script: {:?}", script);
    eprintln!("[bootstrap-run] Script exists: {}", script.exists());
    eprintln!("[bootstrap-run] Script size: {:?}", fs::metadata(script).map(|m| m.len()));
    send_status(window, "Setting up OP Replay Clipper...");

    fs::create_dir_all(data_dir()).ok();

    // Use -Command with explicit script invocation to avoid path quoting issues
    let script_path = script.to_string_lossy().to_string();
    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path,
        ])
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

// ---------------------------------------------------------------------------
// Window communication
// ---------------------------------------------------------------------------

fn send_status(window: &tauri::WebviewWindow, msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = window.eval(&format!("updateStatus('{}')", escaped));
}

fn send_error(window: &tauri::WebviewWindow, msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = window.eval(&format!("showError('{}')", escaped));
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let state = AppState {
        server_process: Mutex::new(None),
    };

    tauri::Builder::default()
        .manage(state)
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                let mut proc = state.server_process.lock().unwrap();
                stop_server(&mut proc);
            }
        })
        .setup(|app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("OP Replay Clipper")
            .inner_size(820.0, 920.0)
            .min_inner_size(600.0, 700.0)
            .build()?;

            let handle = app.handle().clone();
            #[cfg(target_os = "windows")]
            let resource_path = app.path().resource_dir().ok();

            thread::spawn(move || {
                let win = window;

                // --- Phase 1: Ensure environment is ready ---
                send_status(&win, "Checking environment...");
                let env_result = check_environment();

                let (project_dir, uv_path) = match env_result {
                    Ok(env) => env,

                    // On Windows: auto-bootstrap if project/uv is missing
                    #[cfg(target_os = "windows")]
                    Err(ref reason)
                        if reason == "project_not_found" || reason == "uv_not_found" =>
                    {
                        eprintln!(
                            "Environment not ready ({}), starting bootstrap...",
                            reason
                        );
                        send_status(&win, "First-time setup — this takes a few minutes...");

                        // Find the bootstrap script: bundled resources → download from GitHub
                        let script = find_bootstrap_script(&resource_path)
                            .or_else(|| {
                                eprintln!("[startup] Bundled script not found, downloading...");
                                send_status(&win, "Downloading setup script...");
                                download_bootstrap_script()
                            });

                        if let Some(ref script_path) = script {
                            if !run_bootstrap(&win, script_path) {
                                send_error(
                                    &win,
                                    "Setup failed. Check the log at:\n%LOCALAPPDATA%\\op-replay-clipper\\bootstrap-app.log\n\nOr run debug_bootstrap.bat from the install directory.",
                                );
                                return;
                            }
                            send_status(&win, "Setup complete! Starting server...");
                        } else {
                            send_error(
                                &win,
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
                                    &win,
                                    "Setup completed but environment is still not ready.\nCheck: %LOCALAPPDATA%\\op-replay-clipper\\bootstrap-app.log",
                                );
                                return;
                            }
                        }
                    }

                    // On Linux/macOS or non-recoverable Windows errors: show platform-specific message
                    Err(reason) => {
                        let msg = match reason.as_str() {
                            "project_not_found" if cfg!(target_os = "macos") => {
                                "Clipper not found. Run: git clone https://github.com/mhayden123/op-replay-clipper-native && cd op-replay-clipper-native && ./install.sh"
                            }
                            "project_not_found" if cfg!(windows) => {
                                "Setup failed — clipper project not found after bootstrap."
                            }
                            "project_not_found" => {
                                "Clipper not found. Run: git clone https://github.com/mhayden123/op-replay-clipper-native && cd op-replay-clipper-native && ./install.sh"
                            }
                            "uv_not_found" if cfg!(windows) => {
                                "uv not found after setup. Try reinstalling the application."
                            }
                            "uv_not_found" => {
                                "uv not found. Run ./install.sh in the clipper project directory."
                            }
                            "openpilot_not_installed" => {
                                "openpilot not installed. Run ./install.sh in the clipper project directory."
                            }
                            other => other,
                        };
                        send_error(&win, msg);
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
                send_status(&win, "Starting server...");
                let child = match start_server(&project_dir, &uv_path) {
                    Ok(c) => c,
                    Err(msg) => {
                        send_error(&win, &msg);
                        return;
                    }
                };

                let state = handle.state::<AppState>();
                *state.server_process.lock().unwrap() = Some(child);

                // --- Phase 4: Wait for server ---
                send_status(&win, "Waiting for server...");
                if !wait_for_server() {
                    let mut proc = state.server_process.lock().unwrap();
                    stop_server(&mut proc);
                    send_error(
                        &win,
                        "Server failed to start. Check that the clipper is installed correctly.",
                    );
                    return;
                }

                // --- Phase 5: Redirect to web UI ---
                let _ = win.eval(&format!("window.location.href = '{}'", SERVER_URL));
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Error running Tauri application");
}
