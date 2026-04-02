// OP Replay Clipper — Tauri Desktop App
//
// Self-contained desktop app that manages Docker images and containers
// directly. No local repo checkout required — pulls pre-built images
// from GHCR on first launch.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;

const SERVER_URL: &str = "http://localhost:7860";
const HEALTH_URL: &str = "http://localhost:7860/api/health";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);

const WEB_IMAGE: &str = "ghcr.io/mhayden123/op-replay-clipper-web:latest";
const RENDER_IMAGE: &str = "ghcr.io/mhayden123/op-replay-clipper-render:latest";

/// Cached path to the docker binary, resolved once.
static DOCKER_PATH: OnceLock<String> = OnceLock::new();

/// Resolve the docker binary path. On macOS, GUI-launched apps have a
/// minimal PATH that doesn't include /usr/local/bin, so we check common
/// Docker Desktop install locations as a fallback.
fn resolve_docker() -> String {
    if Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return "docker".to_string();
    }

    let candidates = [
        // macOS
        "/usr/local/bin/docker",
        "/opt/homebrew/bin/docker",
        "/Applications/Docker.app/Contents/Resources/bin/docker",
        // Windows
        "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe",
    ];
    for path in candidates {
        if Path::new(path).exists() {
            return path.to_string();
        }
    }

    "docker".to_string()
}

/// Return a Command pre-configured with the resolved docker binary.
fn docker_cmd() -> Command {
    let path = DOCKER_PATH.get_or_init(resolve_docker);
    Command::new(path)
}

/// Find the Docker socket path. Checks platform-specific locations.
fn docker_socket_path() -> String {
    let default = "/var/run/docker.sock";
    if Path::new(default).exists() {
        return default.to_string();
    }
    if let Some(home) = dirs::home_dir() {
        // Newer Docker Desktop on macOS
        let candidates = [
            home.join(".docker/run/docker.sock"),
            home.join("Library/Containers/com.docker.docker/Data/docker.sock"),
        ];
        for socket in &candidates {
            if socket.exists() {
                return socket.to_string_lossy().to_string();
            }
        }
    }
    default.to_string()
}

struct AppState {
    web_container_id: Mutex<Option<String>>,
}

/// Get the app data directory (~/.op-replay-clipper).
fn data_dir() -> PathBuf {
    let dir = dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".op-replay-clipper");
    fs::create_dir_all(&dir).ok();
    dir
}

/// Get the shared volume directory for render I/O.
fn shared_dir() -> PathBuf {
    let dir = data_dir().join("shared");
    fs::create_dir_all(&dir).ok();
    dir
}

/// Detect the host's LAN IP address for network scanning inside containers.
fn detect_host_lan_ip() -> Option<String> {
    // Try connecting to a public DNS to determine which interface is used for LAN
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:53").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                let ip = addr.ip().to_string();
                if !ip.starts_with("127.") {
                    return Some(ip);
                }
            }
        }
    }
    None
}

/// Check if Docker is available.
fn check_docker() -> Result<(), String> {
    let output = docker_cmd()
        .args(["info"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match output {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err("Docker is installed but not running. Please start Docker Desktop.".into()),
        Err(_) => Err("Docker is not installed. Please install Docker Desktop from https://www.docker.com/products/docker-desktop/".into()),
    }
}

/// Check if NVIDIA GPU is available (Linux/Windows only).
fn check_nvidia() -> bool {
    Command::new("nvidia-smi")
        .arg("-L")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pull a Docker image (no-op if already up to date).
fn pull_image(image: &str) -> Result<(), String> {
    eprintln!("Pulling {}...", image);
    let status = docker_cmd()
        .args(["pull", image])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to run docker pull: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Failed to pull {}. Check your internet connection.", image))
    }
}

/// Pull latest Docker images, downloading or updating as needed.
fn ensure_images(status_fn: &dyn Fn(&str)) -> Result<(), String> {
    status_fn("Checking for updates...");
    pull_image(WEB_IMAGE)?;
    pull_image(RENDER_IMAGE)?;
    Ok(())
}

/// Start the web server container directly (no docker-compose needed).
fn start_web_container(has_gpu: bool) -> Result<String, String> {
    let shared = shared_dir();
    let home = dirs::home_dir().unwrap_or_default();
    let ssh_dir = home.join(".ssh");

    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "-d".to_string(),
        "--name".to_string(), "op-replay-clipper-web".to_string(),
        "-p".to_string(), "7860:7860".to_string(),
        "-v".to_string(), format!("{}:/app/shared", shared.display()),
        "-v".to_string(), format!("{}:/var/run/docker.sock", docker_socket_path()),
        "-e".to_string(), format!("CLIPPER_IMAGE={}", RENDER_IMAGE),
        "-e".to_string(), format!("SHARED_HOST_DIR={}", shared.display()),
        "-e".to_string(), "SHARED_LOCAL_DIR=/app/shared".to_string(),
        "-e".to_string(), format!("HOST_HOME_DIR={}", home.display()),
    ];

    // Pass the host LAN IP so the container knows which subnet to scan
    if let Some(lan_ip) = detect_host_lan_ip() {
        args.extend(["-e".to_string(), format!("HOST_LAN_IP={}", lan_ip)]);
    }

    // Tell the web server whether GPU is available for render containers
    if !has_gpu {
        args.extend(["-e".to_string(), "HAS_GPU=false".to_string()]);
    }

    // Mount SSH keys if available (for Local SSH download mode)
    if ssh_dir.exists() {
        args.extend([
            "-e".to_string(),
            format!("HOST_SSH_DIR={}", ssh_dir.display()),
        ]);
    }

    args.push(WEB_IMAGE.to_string());

    eprintln!("Starting web server container...");
    let output = docker_cmd()
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to start web container: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to start web container: {}", stderr.trim()));
    }

    Ok("op-replay-clipper-web".to_string())
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
        thread::sleep(Duration::from_secs(1));
    }
    eprintln!("Server did not start in time.");
    false
}

/// Stop the web server container.
fn stop_web_container(container_id: &mut Option<String>) {
    if let Some(id) = container_id.take() {
        eprintln!("Stopping web container {}...", id);
        let _ = docker_cmd()
            .args(["stop", &id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Send a JS call to the window to update the loading status text.
fn send_status(window: &tauri::WebviewWindow, msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = window.eval(&format!("updateStatus('{}')", escaped));
}

/// Send a JS call to the window to show an error message.
fn send_error(window: &tauri::WebviewWindow, msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = window.eval(&format!("showError('{}')", escaped));
}

fn main() {
    let state = AppState {
        web_container_id: Mutex::new(None),
    };

    tauri::Builder::default()
        .manage(state)
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                let mut id = state.web_container_id.lock().unwrap();
                stop_web_container(&mut id);
            }
        })
        .setup(|app| {
            // Create the window immediately so the user sees the loading screen
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("OP Replay Clipper")
            .inner_size(820.0, 920.0)
            .min_inner_size(600.0, 700.0)
            .build()?;

            // Run all Docker startup in a background thread
            let handle = app.handle().clone();
            thread::spawn(move || {
                let win = window;

                // Check Docker
                send_status(&win, "Checking Docker...");
                if let Err(msg) = check_docker() {
                    send_error(&win, &msg);
                    return;
                }

                let has_gpu = check_nvidia();
                if !has_gpu {
                    eprintln!("WARNING: No NVIDIA GPU detected. Rendering will use CPU (slower).");
                }

                // Ensure Docker images are available
                send_status(&win, "Checking Docker images...");
                let win_ref = &win;
                if let Err(msg) = ensure_images(&|status| send_status(win_ref, status)) {
                    send_error(&win, &msg);
                    return;
                }

                // Stop any leftover container from a previous crash
                send_status(&win, "Starting server...");
                let _ = docker_cmd()
                    .args(["rm", "-f", "op-replay-clipper-web"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                // Start the web server
                let container_name = match start_web_container(has_gpu) {
                    Ok(name) => name,
                    Err(msg) => {
                        send_error(&win, &msg);
                        return;
                    }
                };

                // Store container name for cleanup on window close
                let state = handle.state::<AppState>();
                *state.web_container_id.lock().unwrap() = Some(container_name.clone());

                // Wait for the server to be ready
                send_status(&win, "Waiting for server to start...");
                if !wait_for_server() {
                    let _ = docker_cmd()
                        .args(["stop", &container_name])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    *state.web_container_id.lock().unwrap() = None;
                    send_error(&win, "Server failed to start in time. Check that Docker has enough resources allocated.");
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
