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
    if Command::new(uv_name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some(uv_name.to_string());
    }

    let home = dirs::home_dir()?;
    if cfg!(windows) {
        let candidates = [
            home.join(".local\\bin\\uv.exe"),
            home.join("AppData\\Roaming\\Python\\Scripts\\uv.exe"),
            home.join(".cargo\\bin\\uv.exe"),
        ];
        for path in &candidates {
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    } else {
        let candidates = [home.join(".local/bin/uv"), home.join(".cargo/bin/uv")];
        for path in &candidates {
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
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

/// Find the bootstrap.ps1 script in the app's resources.
#[cfg(target_os = "windows")]
fn find_bootstrap_script(resource_dir: &Option<PathBuf>) -> Option<PathBuf> {
    if let Some(ref res_dir) = resource_dir {
        // Tauri bundles resources in different locations depending on dev vs release
        let candidates = [
            res_dir.join("resources").join("bootstrap.ps1"),
            res_dir.join("bootstrap.ps1"),
            res_dir
                .parent()
                .map(|p| p.join("resources").join("bootstrap.ps1"))
                .unwrap_or_default(),
        ];
        for path in &candidates {
            if path.exists() {
                return Some(path.clone());
            }
        }
    }

    // Also check next to the executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let path = exe_dir.join("resources").join("bootstrap.ps1");
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

/// Run the bootstrap.ps1 script with live progress updates to the window.
/// Returns true if bootstrap succeeded.
#[cfg(target_os = "windows")]
fn run_bootstrap(window: &tauri::WebviewWindow, script: &std::path::Path) -> bool {
    use std::io::BufRead;

    let log_path = data_dir().join("bootstrap-app.log");
    eprintln!("Running bootstrap: {:?}", script);
    eprintln!("Log: {:?}", log_path);
    send_status(window, "Setting up OP Replay Clipper...");

    // Ensure the data dir exists for the log
    fs::create_dir_all(data_dir()).ok();

    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script.to_string_lossy(),
            "-Silent",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    match result {
        Ok(mut child) => {
            let mut log_lines: Vec<String> = Vec::new();

            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        // Show step headers in the UI
                        if line.starts_with("==>") {
                            let msg = line.trim_start_matches("==>").trim();
                            send_status(window, msg);
                        }
                        eprintln!("[bootstrap] {}", line);
                        log_lines.push(line);
                    }
                }
            }

            // Write log file
            let _ = fs::write(&log_path, log_lines.join("\n"));

            let status = child.wait();
            match status {
                Ok(s) if s.success() => {
                    eprintln!("Bootstrap completed successfully");
                    true
                }
                Ok(s) => {
                    eprintln!("Bootstrap failed with exit code: {:?}", s.code());
                    false
                }
                Err(e) => {
                    eprintln!("Bootstrap wait error: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to launch PowerShell: {}", e);
            let _ = fs::write(&log_path, format!("Failed to launch PowerShell: {}", e));
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

                        // Find and run the bootstrap script
                        let script = find_bootstrap_script(&resource_path);
                        if let Some(ref script_path) = script {
                            send_status(&win, "First-time setup — this takes a few minutes...");
                            if !run_bootstrap(&win, script_path) {
                                send_error(
                                    &win,
                                    "Setup failed. Check the log at %LOCALAPPDATA%\\op-replay-clipper\\bootstrap-app.log",
                                );
                                return;
                            }
                            send_status(&win, "Setup complete! Starting server...");
                        } else {
                            eprintln!("Bootstrap script not found in resources");
                            send_error(
                                &win,
                                "Setup files not found. Please reinstall the application.",
                            );
                            return;
                        }

                        // Re-check environment after bootstrap
                        match check_environment() {
                            Ok(env) => env,
                            Err(_) => {
                                send_error(
                                    &win,
                                    "Setup completed but environment still not ready. Check the log at %LOCALAPPDATA%\\op-replay-clipper\\bootstrap-app.log",
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
