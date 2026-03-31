// OP Replay Clipper — Tauri Desktop App
//
// Self-contained desktop app that manages Docker images and containers
// directly. No local repo checkout required — pulls pre-built images
// from GHCR on first launch.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;

const SERVER_URL: &str = "http://localhost:7860";
const HEALTH_URL: &str = "http://localhost:7860/api/health";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);

const WEB_IMAGE: &str = "ghcr.io/mhayden123/op-replay-clipper-web:latest";
const RENDER_IMAGE: &str = "ghcr.io/mhayden123/op-replay-clipper-render:latest";

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

/// Check if Docker is available.
fn check_docker() -> Result<(), String> {
    let output = Command::new("docker")
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

/// Check if a Docker image exists locally.
fn image_exists(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Pull a Docker image with progress output.
fn pull_image(image: &str) -> Result<(), String> {
    eprintln!("Pulling {}...", image);
    let status = Command::new("docker")
        .args(["pull", image])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to run docker pull: {}", e))?;
    if status.success() {
        eprintln!("Pulled {} successfully.", image);
        Ok(())
    } else {
        Err(format!("Failed to pull {}. Check your internet connection.", image))
    }
}

/// Ensure both Docker images are available, pulling if needed.
fn ensure_images() -> Result<(), String> {
    if !image_exists(WEB_IMAGE) {
        pull_image(WEB_IMAGE)?;
    }
    if !image_exists(RENDER_IMAGE) {
        pull_image(RENDER_IMAGE)?;
    }
    Ok(())
}

/// Start the web server container directly (no docker-compose needed).
fn start_web_container() -> Result<(Child, String), String> {
    let shared = shared_dir();
    let home = dirs::home_dir().unwrap_or_default();
    let ssh_dir = home.join(".ssh");

    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--name".to_string(), "op-replay-clipper-web".to_string(),
        "-p".to_string(), "7860:7860".to_string(),
        "-v".to_string(), format!("{}:/app/shared", shared.display()),
        "-v".to_string(), "/var/run/docker.sock:/var/run/docker.sock".to_string(),
        "-e".to_string(), format!("CLIPPER_IMAGE={}", RENDER_IMAGE),
        "-e".to_string(), format!("SHARED_HOST_DIR={}", shared.display()),
        "-e".to_string(), "SHARED_LOCAL_DIR=/app/shared".to_string(),
        "-e".to_string(), format!("HOST_HOME_DIR={}", home.display()),
    ];

    // Mount SSH keys if available (for Local SSH download mode)
    if ssh_dir.exists() {
        args.extend([
            "-e".to_string(),
            format!("HOST_SSH_DIR={}", ssh_dir.display()),
        ]);
    }

    args.push(WEB_IMAGE.to_string());

    eprintln!("Starting web server container...");
    let child = Command::new("docker")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start web container: {}", e))?;

    Ok((child, "op-replay-clipper-web".to_string()))
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
        let _ = Command::new("docker")
            .args(["stop", &id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn main() {
    // Pre-flight checks
    if let Err(msg) = check_docker() {
        eprintln!("ERROR: {}", msg);
        std::process::exit(1);
    }

    let has_gpu = check_nvidia();
    if !has_gpu {
        eprintln!("WARNING: No NVIDIA GPU detected. Rendering will use CPU (slower).");
    }

    // Ensure Docker images are available
    if let Err(msg) = ensure_images() {
        eprintln!("ERROR: {}", msg);
        std::process::exit(1);
    }

    // Stop any leftover container from a previous crash
    let _ = Command::new("docker")
        .args(["rm", "-f", "op-replay-clipper-web"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // Start the web server
    let (mut _child, container_name) = start_web_container()
        .expect("Failed to start web container");

    if !wait_for_server() {
        eprintln!("Failed to start server. Check Docker logs.");
        let _ = Command::new("docker")
            .args(["stop", &container_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::process::exit(1);
    }

    let state = AppState {
        web_container_id: Mutex::new(Some(container_name)),
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
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(SERVER_URL.parse().unwrap()),
            )
            .title("OP Replay Clipper")
            .inner_size(820.0, 920.0)
            .min_inner_size(600.0, 700.0)
            .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Error running Tauri application");
}
