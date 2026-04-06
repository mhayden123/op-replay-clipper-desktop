use std::time::Duration;

pub const SERVER_PORT: u16 = 7860;
pub const SERVER_HOST: &str = "127.0.0.1";
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "windows")]
pub const BOOTSTRAP_SCRIPT_URL: &str =
    "https://raw.githubusercontent.com/mhayden123/glidekit-desktop/main/src-tauri/resources/bootstrap.ps1";
#[cfg(not(target_os = "windows"))]
pub const GLIDEKIT_NATIVE_REPO: &str = "https://github.com/mhayden123/glidekit-native.git";
#[cfg(target_os = "windows")]
pub const REGISTRY_KEY: &str = r"HKCU\Software\GlideKit";

pub const SERVER_URL: &str = "http://localhost:7860";
pub const HEALTH_URL: &str = "http://localhost:7860/api/health";
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
