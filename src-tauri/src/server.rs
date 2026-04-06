use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::constants::*;
use crate::env_sanitize::CommandExt;
use crate::paths::{data_dir, openpilot_root};

/// Kill any stale server process that is still holding port 7860.
///
/// This can happen when a previous GlideKit session crashed or the window was
/// closed without the child-process cleanup running (e.g. SIGKILL, power loss,
/// or the old process bound to 0.0.0.0 before the 127.0.0.1 fix).
///
/// On Unix, we use `lsof` to find the PID; on Windows, `netstat` + `taskkill`.
pub fn kill_stale_server() {
    use std::net::TcpStream;

    // Quick check: if nothing is listening, skip the heavier lsof/netstat call.
    if TcpStream::connect((SERVER_HOST, SERVER_PORT)).is_err() {
        // Also check 0.0.0.0 binding (legacy pre-fix servers).
        if TcpStream::connect(("0.0.0.0", SERVER_PORT)).is_err() {
            return;
        }
    }

    eprintln!("[server] Port {} is in use — killing stale server...", SERVER_PORT);

    #[cfg(unix)]
    {
        // `lsof -ti :<port>` returns PIDs of processes listening on the port.
        let lsof_port = format!(":{}", SERVER_PORT);
        if let Ok(output) = Command::new("lsof")
            .args(["-ti", &lsof_port])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            let pids = String::from_utf8_lossy(&output.stdout);
            for pid_str in pids.split_whitespace() {
                if let Ok(pid) = pid_str.parse::<i32>() {
                    eprintln!("[server] Killing stale process pid {}", pid);
                    unsafe { libc::kill(pid, libc::SIGTERM); }
                }
            }
            // Give processes a moment to exit.
            thread::sleep(Duration::from_millis(500));
        }
    }

    #[cfg(windows)]
    {
        // Find PID via netstat, then taskkill it.
        let netstat_cmd = format!("netstat -ano | findstr :{} | findstr LISTENING", SERVER_PORT);
        if let Ok(output) = Command::new("cmd")
            .args(["/C", &netstat_cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(pid_str) = line.split_whitespace().last() {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        eprintln!("[server] Killing stale process pid {}", pid);
                        let _ = Command::new("taskkill")
                            .args(["/F", "/PID", &pid.to_string()])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
}

pub fn start_server(project_dir: &Path, uv_path: &str) -> Result<Child, String> {
    use std::io::Write;

    // Kill any leftover server from a previous session that didn't clean up.
    kill_stale_server();

    let openpilot_dir = openpilot_root();
    let output_dir = data_dir().join("output");
    let data_dir_path = data_dir().join("data");

    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!("Warning: failed to create output directory {:?}: {}", output_dir, e);
    }
    if let Err(e) = fs::create_dir_all(&data_dir_path) {
        eprintln!("Warning: failed to create data directory {:?}: {}", data_dir_path, e);
    }

    // Open the server log file (truncate on each launch). stdout/stderr from
    // `uv sync` and `uvicorn` are redirected here so Linux users can diagnose
    // startup failures — otherwise the output is lost and "Server failed to
    // start" gives the user nothing to work with.
    let log_path = data_dir().join("server.log");
    let mut log = fs::File::create(&log_path)
        .map_err(|e| format!("Failed to open server log at {:?}: {}", log_path, e))?;
    let _ = writeln!(log, "=== GlideKit server log ===");
    let _ = writeln!(log, "project_dir: {:?}", project_dir);
    let _ = writeln!(log, "uv_path: {}", uv_path);
    let _ = writeln!(log, "openpilot_root: {:?}", openpilot_dir);
    let _ = writeln!(log, "--- uv sync ---");
    let _ = log.flush();
    eprintln!("[server] Server log: {:?}", log_path);

    // Ensure Python dependencies are installed before starting the server.
    // Without this, `uv run` may fail if .venv/ doesn't exist yet.
    eprintln!("[server] Running uv sync in {:?}", project_dir);
    let sync_stdout = log
        .try_clone()
        .map_err(|e| format!("Failed to clone log handle: {}", e))?;
    let sync_stderr = log
        .try_clone()
        .map_err(|e| format!("Failed to clone log handle: {}", e))?;
    let sync_status = Command::new(uv_path)
        .args(["sync"])
        .current_dir(project_dir)
        .sanitize_env()
        .stdout(Stdio::from(sync_stdout))
        .stderr(Stdio::from(sync_stderr))
        .status();
    match sync_status {
        Ok(s) if s.success() => eprintln!("[server] uv sync completed"),
        Ok(s) => eprintln!("[server] uv sync exited with {:?} (continuing anyway)", s.code()),
        Err(e) => eprintln!("[server] uv sync failed to run: {} (continuing anyway)", e),
    }

    let _ = writeln!(log, "--- uvicorn ---");
    let _ = log.flush();

    let server_stdout = log
        .try_clone()
        .map_err(|e| format!("Failed to clone log handle: {}", e))?;
    let server_stderr = log
        .try_clone()
        .map_err(|e| format!("Failed to clone log handle: {}", e))?;
    let child = Command::new(uv_path)
        .args([
            "run",
            "python",
            "-m",
            "uvicorn",
            "web.server:app",
            "--host",
            SERVER_HOST,
            "--port",
            &SERVER_PORT.to_string(),
        ])
        .current_dir(project_dir)
        .env("GLIDEKIT_HOME", data_dir().to_string_lossy().as_ref())
        .env("OPENPILOT_ROOT", openpilot_dir.to_string_lossy().as_ref())
        .env("GLIDEKIT_OUTPUT_DIR", output_dir.to_string_lossy().as_ref())
        .env("GLIDEKIT_DATA_DIR", data_dir_path.to_string_lossy().as_ref())
        .sanitize_env()
        .stdout(Stdio::from(server_stdout))
        .stderr(Stdio::from(server_stderr))
        .spawn()
        .map_err(|e| format!("Failed to start server: {}", e))?;

    eprintln!("Server process started (pid {})", child.id());
    Ok(child)
}

pub fn wait_for_server() -> bool {
    eprintln!("Waiting for server...");
    let start = Instant::now();
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create HTTP client: {}", e);
            return false;
        }
    };

    while start.elapsed() < STARTUP_TIMEOUT {
        if let Ok(resp) = client.get(HEALTH_URL).send() {
            if resp.status().is_success() {
                eprintln!("Server ready!");
                return true;
            }
        }
        thread::sleep(HEALTH_POLL_INTERVAL);
    }
    eprintln!("Server did not start in time.");
    false
}

pub fn stop_server(process: &mut Option<Child>) {
    let Some(mut child) = process.take() else { return };
    let pid = child.id();
    eprintln!("Stopping server (pid {})...", pid);

    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("Server exited gracefully with {:?}", status.code());
                    return;
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        eprintln!("Graceful shutdown timed out, sending SIGKILL");
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    eprintln!("Error waiting for server: {}", e);
                    break;
                }
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    eprintln!("Server stopped.");
}
