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

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;

trait CommandExt {
    fn sanitize_env(&mut self) -> &mut Self;
}
impl CommandExt for Command {
    fn sanitize_env(&mut self) -> &mut Self {
        self.env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
    }
}

const SERVER_PORT: u16 = 7860;
const SERVER_HOST: &str = "127.0.0.1";
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "windows")]
const BOOTSTRAP_SCRIPT_URL: &str =
    "https://raw.githubusercontent.com/mhayden123/glidekit-desktop/main/src-tauri/resources/bootstrap.ps1";
#[cfg(not(target_os = "windows"))]
const GLIDEKIT_NATIVE_REPO: &str = "https://github.com/mhayden123/glidekit-native.git";
#[cfg(target_os = "windows")]
const REGISTRY_KEY: &str = r"HKCU\Software\GlideKit";

const SERVER_URL: &str = "http://localhost:7860";
const HEALTH_URL: &str = "http://localhost:7860/api/health";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct AppState {
    server_process: Mutex<Option<Child>>,
}

// ---------------------------------------------------------------------------
// Path detection
// ---------------------------------------------------------------------------

/// Get the app data directory (~/.glidekit).
fn data_dir() -> PathBuf {
    let dir = dirs::home_dir()
        .expect("Cannot determine home directory")
        .join(".glidekit");
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Warning: failed to create data directory {:?}: {}", dir, e);
    }
    dir
}

/// Resolve the openpilot root directory.
/// Honors the `OPENPILOT_ROOT` env var; falls back to `~/.glidekit/openpilot`.
/// Used by both `check_environment` and `start_server` so the check and the
/// server-side env agree on the path.
fn openpilot_root() -> PathBuf {
    std::env::var("OPENPILOT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_dir().join("openpilot"))
}

/// Read a string value from the app's registry key (Windows only).
/// Key: HKCU\Software\GlideKit\{name}
#[cfg(target_os = "windows")]
fn read_registry_string(name: &str) -> Option<String> {
    let output = Command::new("reg.exe")
        .args([
            "query",
            REGISTRY_KEY,
            "/v",
            name,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // reg.exe output: "    ValueName    REG_SZ    ValueData"
    for line in text.lines() {
        if line.contains("REG_SZ") {
            if let Some(pos) = line.find("REG_SZ") {
                let value = line[pos + 6..].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Locate the GlideKit project directory (must contain clip.py).
fn find_glidekit_project() -> Option<PathBuf> {
    // Explicit override
    if let Ok(dir) = std::env::var("GLIDEKIT_PROJECT_DIR") {
        let p = PathBuf::from(&dir);
        if p.join("clip.py").exists() {
            return Some(p);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Windows: check registry first (most reliable — set by bootstrap.ps1)
    #[cfg(target_os = "windows")]
    {
        if let Some(reg_path) = read_registry_string("ProjectDir") {
            let p = PathBuf::from(&reg_path);
            eprintln!("[find-project] Registry ProjectDir: {:?} exists={}", p, p.join("clip.py").exists());
            if p.join("clip.py").exists() {
                return Some(p);
            }
        }
    }

    // Sibling of the running executable (prefer glidekit-native over glidekit)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for ancestor in [parent, parent.parent().unwrap_or(parent)] {
                candidates.push(ancestor.join("glidekit-native"));
                candidates.push(ancestor.join("glidekit"));
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
        candidates.push(cwd.join("glidekit-native"));
        candidates.push(cwd.join("glidekit"));
    }

    // Home directory (prefer glidekit-native over glidekit)
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("glidekit-native"));
        candidates.push(home.join("glidekit"));
    }

    // Windows: %LOCALAPPDATA%\glidekit\ (default bootstrap location)
    if cfg!(windows) {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("glidekit")
                    .join("glidekit"),
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
        .sanitize_env()
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
        // Linux + macOS: check common install locations
        vec![
            home.join(".local/bin/uv"),
            home.join(".cargo/bin/uv"),
            home.join(".local/share/uv/bin/uv"),
            PathBuf::from("/usr/local/bin/uv"),                 // macOS Intel Homebrew + manual installs
            PathBuf::from("/usr/bin/uv"),                       // system package manager
            PathBuf::from("/opt/homebrew/bin/uv"),              // macOS Apple Silicon Homebrew
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin/uv"), // Linux Homebrew
        ]
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
        .sanitize_env()
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn check_wsl() -> bool {
    use std::path::Path;
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
fn check_wsl() -> bool {
    false
}

/// Check that the GlideKit environment is ready to launch the server.
/// Returns (project_dir, uv_path) on success.
fn check_environment() -> Result<(PathBuf, String), String> {
    let project_dir = find_glidekit_project().ok_or("project_not_found")?;
    let uv_path = resolve_uv().ok_or("uv_not_found")?;

    // On Linux/macOS, also need openpilot for UI renders
    if !cfg!(windows) {
        let python_path = openpilot_root().join(".venv/bin/python");
        if !python_path.exists() {
            return Err("openpilot_not_installed".into());
        }
    }

    Ok((project_dir, uv_path))
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// Kill any stale server process that is still holding port 7860.
///
/// This can happen when a previous GlideKit session crashed or the window was
/// closed without the child-process cleanup running (e.g. SIGKILL, power loss,
/// or the old process bound to 0.0.0.0 before the 127.0.0.1 fix).
///
/// On Unix, we use `lsof` to find the PID; on Windows, `netstat` + `taskkill`.
fn kill_stale_server() {
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

fn start_server(project_dir: &PathBuf, uv_path: &str) -> Result<Child, String> {
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

fn wait_for_server() -> bool {
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

fn stop_server(process: &mut Option<Child>) {
    if let Some(mut child) = process.take() {
        eprintln!("Stopping server (pid {})...", child.id());
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("Server stopped.");
    }
}

// ---------------------------------------------------------------------------
// Linux/macOS bootstrap — runs automatically when dependencies are missing
// ---------------------------------------------------------------------------

/// Check whether the user has cached sudo credentials.
///
/// install.sh runs `sudo apt-get install` for build dependencies, and
/// openpilot's own installer also calls `sudo apt-get`. If credentials
/// aren't cached, the prompt appears in the subprocess's stdin — which is
/// invisible from the Tauri UI — and the install hangs silently. This
/// check surfaces the requirement up-front with a clear actionable message.
///
/// Returns true if `sudo -n true` succeeds (passwordless sudo OR cached
/// credentials). Returns false if sudo isn't installed or a password is
/// required.
#[cfg(not(target_os = "windows"))]
fn check_sudo_available() -> bool {
    match Command::new("sudo")
        .args(["-n", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// Install uv via the official installer script and verify it exists.
#[cfg(not(target_os = "windows"))]
fn install_uv(window: &tauri::WebviewWindow) -> bool {
    eprintln!("[bootstrap] Installing uv...");
    send_status(window, "Installing uv package manager...");

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("[bootstrap] Cannot determine home directory");
            return false;
        }
    };

    // Run installer with HOME explicitly set and AppImage's LD_LIBRARY_PATH
    // cleared so curl uses the system's libcurl instead of the bundled one.
    let result = Command::new("sh")
        .args(["-c", "curl -LsSf https://astral.sh/uv/install.sh | sh"])
        .env("HOME", &home)
        .sanitize_env()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(s) if !s.success() => {
            eprintln!("[bootstrap] uv install script failed (exit code {:?})", s.code());
            return false;
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to run uv installer: {}", e);
            return false;
        }
        _ => {}
    }

    // Verify uv actually landed on disk
    let uv_path = home.join(".local/bin/uv");
    if uv_path.exists() {
        eprintln!("[bootstrap] uv installed at {:?}", uv_path);
        true
    } else {
        let cargo_path = home.join(".cargo/bin/uv");
        if cargo_path.exists() {
            eprintln!("[bootstrap] uv installed at {:?}", cargo_path);
            true
        } else {
            eprintln!("[bootstrap] uv installer ran but binary not found");
            false
        }
    }
}

/// Clone the glidekit-native project to ~/glidekit-native.
#[cfg(not(target_os = "windows"))]
fn clone_project(window: &tauri::WebviewWindow) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let target = home.join("glidekit-native");

    if target.join("clip.py").exists() {
        eprintln!("[bootstrap] Project already exists at {:?}", target);
        return Some(target);
    }

    eprintln!("[bootstrap] Cloning glidekit-native...");
    send_status(window, "Downloading GlideKit...");

    let result = Command::new("git")
        .args([
            "clone",
            "--depth", "1",
            GLIDEKIT_NATIVE_REPO,
            &target.to_string_lossy(),
        ])
        .sanitize_env()
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match result {
        Ok(s) if s.success() && target.join("clip.py").exists() => {
            eprintln!("[bootstrap] Cloned to {:?}", target);
            Some(target)
        }
        Ok(s) => {
            eprintln!("[bootstrap] git clone failed (exit code {:?})", s.code());
            None
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to run git: {}", e);
            None
        }
    }
}

/// Strip ANSI escape codes from a string.
#[cfg(not(target_os = "windows"))]
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit a letter (end of ANSI sequence)
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Run install.sh from the project directory with live progress to the UI.
#[cfg(not(target_os = "windows"))]
fn run_install_script(window: &tauri::WebviewWindow, project_dir: &std::path::Path) -> bool {
    use std::io::{BufRead, Write};

    let script = project_dir.join("install.sh");
    if !script.exists() {
        eprintln!("[bootstrap] install.sh not found at {:?}", script);
        return false;
    }

    // Open the install log (truncate each run) so users can diagnose failures.
    // install.sh produces a lot of output; the UI only shows progress markers,
    // so without this file the full build output is unrecoverable.
    let install_log_path = data_dir().join("install.log");
    let mut install_log = match fs::File::create(&install_log_path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!("[bootstrap] Warning: could not open install log: {}", e);
            None
        }
    };
    if let Some(ref mut log) = install_log {
        let _ = writeln!(log, "=== GlideKit install.sh log ===");
        let _ = writeln!(log, "project_dir: {:?}", project_dir);
        let _ = writeln!(log, "script: {:?}", script);
        let _ = writeln!(log, "---");
        let _ = log.flush();
    }
    eprintln!("[bootstrap] install.sh log: {:?}", install_log_path);

    eprintln!("[bootstrap] Running install.sh in {:?}", project_dir);
    send_status(window, "Running install script — this may take a while...");

    // Ensure uv is findable by augmenting PATH with common install locations.
    // Sudo credentials are pre-checked in the startup_sequence so apt is safe
    // to run here — it'll use cached credentials silently.
    let home = dirs::home_dir().unwrap_or_default();
    let extra_paths = format!(
        "{}:{}",
        home.join(".local/bin").display(),
        home.join(".cargo/bin").display(),
    );
    let path = match std::env::var("PATH") {
        Ok(p) => format!("{}:{}", extra_paths, p),
        Err(_) => extra_paths,
    };

    // Run install.sh with stderr merged into stdout (2>&1) so git errors
    // and build failures are visible in the progress stream instead of lost.
    let cmd = format!("exec bash {} 2>&1", script.to_string_lossy());
    let result = Command::new("bash")
        .args(["-c", &cmd])
        .current_dir(project_dir)
        .env("PATH", &path)
        .env("HOME", &home)
        // Bypass user's global git config entirely — all repos cloned by
        // install.sh are public, no credentials or user prefs are needed.
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        // Clear AppImage's LD_LIBRARY_PATH — the bundled libs (libcurl,
        // libgnutls, etc.) conflict with system binaries like git-remote-https.
        // Child processes (git, cmake, scons) must use the system's own libs.
        // Clear AppImage's PYTHONHOME/PYTHONPATH — the bundled Python stdlib
        // breaks uv's build isolation, causing "No module named 'encodings'"
        // when hatchling tries to build openpilot as an editable install.
        .sanitize_env()
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();

    match result {
        Ok(mut child) => {
            // Stream stdout for progress updates
            if let Some(stdout) = child.stdout.take() {
                let reader = std::io::BufReader::new(stdout);
                for raw_line in reader.lines().map_while(Result::ok) {
                    let clean = strip_ansi(&raw_line);
                    let trimmed = clean.trim();

                    // Persist every line to install.log for post-mortem diagnosis.
                    if let Some(ref mut log) = install_log {
                        let _ = writeln!(log, "{}", clean);
                    }

                    // Match install.sh output patterns:
                    //   "==> Step description"   — major step header
                    //   "  OK: detail"            — success substep
                    //   "  WARN: detail"          — warning
                    //   "  ERROR: detail"         — failure
                    //   "Installation complete!"  — final banner
                    if trimmed.starts_with("==>") {
                        let msg = trimmed.trim_start_matches("==>").trim();
                        if !msg.is_empty() {
                            send_status(window, msg);
                        }
                    } else if trimmed.starts_with("OK:") {
                        let msg = trimmed.trim_start_matches("OK:").trim();
                        if !msg.is_empty() {
                            send_status(window, msg);
                        }
                    } else if trimmed.starts_with("WARN:") || trimmed.starts_with("ERROR:") {
                        send_status(window, trimmed);
                    } else if trimmed.contains("Installation complete!") {
                        send_status(window, "Installation complete!");
                    }

                    eprintln!("[install.sh] {}", trimmed);
                }
            }

            match child.wait() {
                Ok(s) if s.success() => {
                    eprintln!("[bootstrap] install.sh completed successfully");
                    true
                }
                Ok(s) => {
                    eprintln!("[bootstrap] install.sh failed (exit code {:?})", s.code());
                    false
                }
                Err(e) => {
                    eprintln!("[bootstrap] install.sh wait error: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("[bootstrap] Failed to launch install.sh: {}", e);
            false
        }
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
                 Invoke-WebRequest -Uri '{}' \
                 -OutFile '{}'",
                BOOTSTRAP_SCRIPT_URL,
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
fn run_bootstrap(window: &tauri::WebviewWindow, script: &std::path::Path, clean: bool) -> bool {
    use std::io::BufRead;

    eprintln!("[bootstrap-run] Script: {:?}", script);
    eprintln!("[bootstrap-run] Script exists: {}", script.exists());
    eprintln!("[bootstrap-run] Script size: {:?}", fs::metadata(script).map(|m| m.len()));
    eprintln!("[bootstrap-run] Clean: {}", clean);
    send_status(window, if clean { "Clean install - re-downloading all files..." } else { "Setting up GlideKit..." });

    fs::create_dir_all(data_dir()).ok();

    let script_path = script.to_string_lossy().to_string();
    let mut args = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script_path,
    ];
    if clean {
        args.push("-Clean".to_string());
    }
    let result = Command::new("powershell.exe")
        .args(&args)
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
    let _ = window.eval(format!("updateStatus('{}')", escaped));
}

fn send_error(window: &tauri::WebviewWindow, msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = window.eval(format!("showError('{}')", escaped));
}

// ---------------------------------------------------------------------------
// Startup sequence (runs in background thread)
// ---------------------------------------------------------------------------

fn startup_sequence(
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
        Err(ref reason)
            if reason == "project_not_found" || reason == "uv_not_found" =>
        {
            eprintln!(
                "Environment not ready ({}), starting bootstrap...",
                reason
            );
            send_status(win, if clean_install {
                "Clean install - re-downloading all files..."
            } else {
                "First-time setup - this takes a few minutes..."
            });

            // Find the bootstrap script: bundled resources -> download from GitHub
            let script = find_bootstrap_script(resource_path)
                .or_else(|| {
                    eprintln!("[startup] Bundled script not found, downloading...");
                    send_status(win, "Downloading setup script...");
                    download_bootstrap_script()
                });

            if let Some(ref script_path) = script {
                if !run_bootstrap(win, script_path, clean_install) {
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
        Err(ref reason)
            if reason == "project_not_found"
                || reason == "uv_not_found"
                || reason == "openpilot_not_installed" =>
        {
            eprintln!("Environment not ready ({}), starting bootstrap...", reason);
            send_status(win, "First-time setup — this may take a while...");

            // Step 1: Install uv if missing
            if resolve_uv().is_none() && !install_uv(win) {
                send_error(win, "Failed to install uv. Check your internet connection and try again.");
                return;
            }

            // Step 2: Clone project if missing
            let project = if find_glidekit_project().is_none() {
                match clone_project(win) {
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
                eprintln!("[bootstrap] Removing partial openpilot clone at {:?}", openpilot_root);
                let _ = fs::remove_dir_all(&openpilot_root);
            }

            if !openpilot_root.join(".venv/bin/python").exists() {
                // openpilot's installer calls `sudo apt-get` on its own.
                // Without cached credentials, the prompt is invisible in the
                // subprocess and the install hangs silently.
                if !check_sudo_available() {
                    send_error(
                        win,
                        "Installing openpilot requires sudo access.\n\nOpen a terminal and run:\n  sudo -v\n\nThen re-launch GlideKit. Credentials will be cached for ~15 minutes, long enough for the install to finish.",
                    );
                    return;
                }
                send_status(win, "Installing dependencies — this takes 10-20 minutes...");
                if !run_install_script(win, &project) {
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
        Err(reason) => {
            let msg = match reason.as_str() {
                "project_not_found" => {
                    "Setup failed — GlideKit project not found after bootstrap."
                }
                "uv_not_found" => {
                    "uv not found after setup. Try reinstalling the application."
                }
                "openpilot_not_installed" => {
                    "openpilot not installed. Run ./install.sh in the GlideKit project directory."
                }
                other => other,
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
                                    || v.get("openpilot_installed") == Some(&serde_json::Value::Bool(false))
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

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

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

#[cfg(test)]
#[cfg(not(target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn sanitize_env_removes_pollution_vars() {
        let mut cmd = Command::new("env");
        cmd.env("LD_LIBRARY_PATH", "/fake")
           .env("LD_PRELOAD", "/fake/lib")
           .env("PYTHONHOME", "/fake/python")
           .env("PYTHONPATH", "/fake/path")
           .env("KEEP_ME", "value");
        cmd.sanitize_env();
        let output = cmd.output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("LD_LIBRARY_PATH"));
        assert!(!text.contains("LD_PRELOAD"));
        assert!(!text.contains("PYTHONHOME"));
        assert!(!text.contains("PYTHONPATH"));
        assert!(text.contains("KEEP_ME=value"));
    }

    #[test]
    fn strip_ansi_removes_color_codes() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_removes_bold() {
        assert_eq!(strip_ansi("\x1b[1mbold\x1b[0m text"), "bold text");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }

    #[test]
    fn strip_ansi_handles_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strip_ansi_handles_multiple_sequences() {
        assert_eq!(
            strip_ansi("\x1b[32m[OK]\x1b[0m: \x1b[1mbuild\x1b[0m done"),
            "[OK]: build done"
        );
    }
}
