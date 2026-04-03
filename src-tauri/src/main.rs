// OP Replay Clipper — Tauri Desktop App (Native Edition)
//
// Manages a local FastAPI server (uvicorn) as a child process.
// No Docker dependency — the rendering pipeline runs natively.

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
            // Binary might be in src-tauri/target/debug or next to the app
            for ancestor in [parent, parent.parent().unwrap_or(parent)] {
                candidates.push(ancestor.join("op-replay-clipper-native"));
                // Also check if we're running from inside the project
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

    candidates.into_iter().find(|p| p.join("clip.py").exists())
}

/// Resolve the `uv` binary path. Checks PATH, then common install locations.
fn resolve_uv() -> Option<String> {
    if Command::new("uv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("uv".to_string());
    }

    let home = dirs::home_dir()?;
    let candidates = [home.join(".local/bin/uv"), home.join(".cargo/bin/uv")];
    for path in &candidates {
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Environment checks
// ---------------------------------------------------------------------------

/// Check if NVIDIA GPU is available.
fn check_nvidia() -> bool {
    Command::new("nvidia-smi")
        .arg("-L")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check that the native clipper environment is set up.
fn check_environment() -> Result<(PathBuf, String), String> {
    // Find clipper project
    let project_dir = find_clipper_project()
        .ok_or("Clipper project not found. Clone op-replay-clipper-native and run ./install.sh")?;

    // Check uv
    let uv_path = resolve_uv().ok_or("uv not found. Run the install script: ./install.sh")?;

    // Check openpilot installation
    let openpilot_dir = data_dir().join("openpilot");
    if !openpilot_dir.join(".venv/bin/python").exists() {
        return Err("openpilot not installed. Run ./install.sh in the clipper project.".into());
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

    // Ensure output/data dirs exist
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
            thread::spawn(move || {
                let win = window;

                // Check native environment
                send_status(&win, "Checking environment...");
                let (project_dir, uv_path) = match check_environment() {
                    Ok(env) => env,
                    Err(msg) => {
                        send_error(&win, &msg);
                        return;
                    }
                };

                let has_gpu = check_nvidia();
                if has_gpu {
                    eprintln!("NVIDIA GPU detected.");
                } else {
                    eprintln!("WARNING: No NVIDIA GPU detected. Rendering will use CPU (slower).");
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
                    // Server failed to start — kill it and show error
                    let mut proc = state.server_process.lock().unwrap();
                    stop_server(&mut proc);
                    send_error(
                        &win,
                        "Server failed to start. Run ./install.sh to set up the environment.",
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
