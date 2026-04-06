// GlideKit -- Tauri Desktop App
//
// Manages a local FastAPI server (uvicorn) as a child process.
// On Windows, auto-bootstraps Python/Git/uv/FFmpeg on first launch.
//
// Platform support:
//   Linux:   Full support (all render types, NVIDIA GPU)
//   macOS:   Full support (all render types, VideoToolbox GPU)
//   Windows: Auto-bootstrap on first launch, non-UI renders native, UI via WSL

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bootstrap;
mod constants;
mod env_sanitize;
mod ipc;
mod paths;
mod platform;
mod server;
mod startup;
mod state;

use ipc::send_error;
use server::stop_server;
use startup::startup_sequence;
use state::AppState;

use std::sync::Mutex;
use std::thread;
use tauri::Manager;

fn main() {
    // Parse --clean flag before Tauri consumes args
    let clean_install = std::env::args().any(|a| a == "--clean");
    if clean_install {
        eprintln!("[main] --clean flag detected, will force clean bootstrap");
    }

    let state = AppState {
        server_process: Mutex::new(None),
    };

    tauri::Builder::default()
        .manage(state)
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                let mut guard = match state.server_process.lock() {
                    Ok(g) => g,
                    Err(_) => {
                        eprintln!("Warning: could not acquire lock to stop server");
                        return;
                    }
                };
                stop_server(&mut guard);
            }
        })
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("GlideKit")
            .inner_size(820.0, 920.0)
            .min_inner_size(600.0, 700.0)
            .build()?;

            let handle = app.handle().clone();
            let resource_path = app.path().resource_dir().ok();

            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    startup_sequence(
                        &window,
                        &handle,
                        clean_install,
                        &resource_path,
                    );
                }));
                if result.is_err() {
                    eprintln!("[startup] Initialization thread panicked");
                    send_error(&window, "An unexpected internal error occurred. Please restart the app.");
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Error running Tauri application");
}
