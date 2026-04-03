// OP Replay Clipper — Tauri Desktop App (Native Edition)
//
// Manages a local FastAPI server (uvicorn) as a child process.
// No Docker dependency — the rendering pipeline runs natively.
//
// Platform support:
//   Linux:   Full support (all render types, NVIDIA GPU)
//   macOS:   Full support (all render types, VideoToolbox GPU)
//   Windows: Non-UI renders native, UI renders via WSL

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
// App state — holds the managed server child process
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

/// Locate the clipper project directory.
///
/// Search order:
/// 1. `CLIPPER_PROJECT_DIR` env var (explicit override)
/// 2. Sibling directory `op-replay-clipper-native` next to the executable
/// 3. `~/op-replay-clipper-native` (common clone location)
///
/// Returns `None` if clip.py is not found at any candidate.
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

    // Windows: check LOCALAPPDATA (where the NSIS installer puts it)
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

/// Resolve the `uv` binary path. Checks PATH, then common install locations.
fn resolve_uv() -> Option<String> {
    // Try PATH first
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
        // Windows: uv installs to %USERPROFILE%\.local\bin or via pip
        let candidates = [
            home.join(".local\\bin\\uv.exe"),
            home.join("AppData\\Roaming\\Python\\Scripts\\uv.exe"),
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

/// Check if NVIDIA GPU is available (Linux/Windows).
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

/// Check if WSL is available and has a running distribution (Windows only).
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

/// Check that the native clipper environment is set up.
fn check_environment() -> Result<(PathBuf, String), String> {
    // Find clipper project
    let project_dir = find_clipper_project().ok_or_else(|| {
        if cfg!(windows) {
            "Clipper project not found. Clone op-replay-clipper-native and run install_windows.py"
                .to_string()
        } else {
            "Clipper project not found. Clone op-replay-clipper-native and run ./install.sh"
                .to_string()
        }
    })?;

    // Check uv
    let uv_path = resolve_uv().ok_or_else(|| {
        if cfg!(windows) {
            "uv not found. Run: pip install uv".to_string()
        } else {
            "uv not found. Run the install script: ./install.sh".to_string()
        }
    })?;

    // On Linux/macOS, check for openpilot installation (needed for UI renders).
    // On Windows, openpilot is optional (only non-UI renders run natively).
    if !cfg!(windows) {
        let python_path = if cfg!(target_os = "macos") {
            data_dir().join("openpilot/.venv/bin/python")
        } else {
            data_dir().join("openpilot/.venv/bin/python")
        };
        if !python_path.exists() {
            return Err(
                "openpilot not installed. Run ./install.sh in the clipper project.".into(),
            );
        }
    }

    Ok((project_dir, uv_path))
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// Start the uvicorn server as a child process.
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

/// Wait for the server health endpoint to respond.
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

/// Stop the server child process.
fn stop_server(process: &mut Option<Child>) {
    if let Some(mut child) = process.take() {
        eprintln!("Stopping server (pid {})...", child.id());
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("Server stopped.");
    }
}

// ---------------------------------------------------------------------------
// Windows first-run bootstrap
// ---------------------------------------------------------------------------

/// Check if the Windows bootstrap has completed (marker file exists).
#[cfg(target_os = "windows")]
fn bootstrap_completed() -> bool {
    data_dir().join("bootstrap-complete").exists()
}

/// Run the bootstrap.ps1 script bundled as a resource.
/// Returns true if bootstrap succeeded, false otherwise.
/// Reads progress from the progress file and sends status updates to the window.
#[cfg(target_os = "windows")]
fn run_bootstrap(
    window: &tauri::WebviewWindow,
    resource_dir: &std::path::Path,
) -> bool {
    let script = resource_dir.join("resources").join("bootstrap.ps1");
    if !script.exists() {
        // Try alternative resource path (Tauri bundles resources differently)
        let alt = resource_dir.join("bootstrap.ps1");
        if !alt.exists() {
            eprintln!("Bootstrap script not found at {:?} or {:?}", script, alt);
            return false;
        }
        return run_bootstrap_script(window, &alt);
    }
    run_bootstrap_script(window, &script)
}

#[cfg(target_os = "windows")]
fn run_bootstrap_script(
    window: &tauri::WebviewWindow,
    script: &std::path::Path,
) -> bool {
    use std::io::BufRead;

    eprintln!("Running bootstrap: {:?}", script);
    send_status(window, "Setting up (this may take a few minutes)...");

    let result = Command::new("powershell.exe")
        .args([
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
            // Read stdout for progress updates
            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if line.starts_with("==>") {
                            let msg = line.trim_start_matches("==>").trim();
                            send_status(window, msg);
                        }
                        eprintln!("[bootstrap] {}", line);
                    }
                }
            }
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
            eprintln!("Failed to launch bootstrap: {}", e);
            false
        }
    }
}

// Stub for non-Windows platforms
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn bootstrap_completed() -> bool {
    true // No bootstrap needed on Linux/macOS
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

                // Windows: run first-run bootstrap if not completed
                #[cfg(target_os = "windows")]
                {
                    if !bootstrap_completed() {
                        send_status(&win, "First-time setup — installing dependencies...");
                        if let Some(ref res_dir) = resource_path {
                            if run_bootstrap(&win, res_dir) {
                                send_status(&win, "Setup complete!");
                            } else {
                                send_status(&win, "Setup had issues — trying to continue...");
                            }
                        } else {
                            eprintln!("WARNING: Could not determine resource directory for bootstrap");
                        }
                    }
                }

                // Check native environment
                send_status(&win, "Checking environment...");
                let (project_dir, uv_path) = match check_environment() {
                    Ok(env) => env,
                    Err(msg) => {
                        send_error(&win, &msg);
                        return;
                    }
                };

                // Platform-specific GPU detection
                if cfg!(target_os = "macos") {
                    eprintln!("macOS detected — VideoToolbox hardware acceleration available.");
                } else if check_nvidia() {
                    eprintln!("NVIDIA GPU detected.");
                } else {
                    eprintln!(
                        "WARNING: No NVIDIA GPU detected. Rendering will use CPU (slower)."
                    );
                }

                // Windows: report WSL status
                if cfg!(windows) {
                    if check_wsl() {
                        eprintln!("WSL detected — UI render types available.");
                    } else {
                        eprintln!("WSL not detected — only non-UI render types available.");
                    }
                }

                // Start the server
                send_status(&win, "Starting server...");
                let child = match start_server(&project_dir, &uv_path) {
                    Ok(c) => c,
                    Err(msg) => {
                        send_error(&win, &msg);
                        return;
                    }
                };

                // Store the child process for cleanup on window close
                let state = handle.state::<AppState>();
                *state.server_process.lock().unwrap() = Some(child);

                // Wait for the server to be ready
                send_status(&win, "Waiting for server...");
                if !wait_for_server() {
                    let mut proc = state.server_process.lock().unwrap();
                    stop_server(&mut proc);
                    let install_cmd = if cfg!(windows) {
                        "install_windows.py"
                    } else {
                        "./install.sh"
                    };
                    send_error(
                        &win,
                        &format!(
                            "Server failed to start. Run {} to set up the environment.",
                            install_cmd
                        ),
                    );
                    return;
                }

                // Redirect the window to the running server
                let _ = win.eval(&format!("window.location.href = '{}'", SERVER_URL));
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Error running Tauri application");
}
