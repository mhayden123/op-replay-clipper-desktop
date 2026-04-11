#[cfg(not(target_os = "windows"))]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

use std::path::PathBuf;

use crate::paths::{find_glidekit_project, openpilot_root, resolve_uv};

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("GlideKit project not found")]
    ProjectNotFound,
    #[error("uv binary not found")]
    UvNotFound,
    #[error("openpilot is not installed")]
    OpenpilotNotInstalled,
}

/// Check that the GlideKit environment is ready to launch the server.
/// Returns (project_dir, uv_path) on success.
pub fn check_environment() -> Result<(PathBuf, String), EnvError> {
    let project_dir = find_glidekit_project().ok_or(EnvError::ProjectNotFound)?;
    let uv_path = resolve_uv().ok_or(EnvError::UvNotFound)?;

    // On Linux/macOS, also need openpilot for UI renders
    if !cfg!(windows) {
        let python_path = openpilot_root().join(".venv/bin/python");
        if !python_path.exists() {
            return Err(EnvError::OpenpilotNotInstalled);
        }
    }

    Ok((project_dir, uv_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_error_display_project_not_found() {
        let e = EnvError::ProjectNotFound;
        assert_eq!(e.to_string(), "GlideKit project not found");
    }
    #[test]
    fn env_error_display_uv_not_found() {
        let e = EnvError::UvNotFound;
        assert_eq!(e.to_string(), "uv binary not found");
    }
    #[test]
    fn env_error_display_openpilot_not_installed() {
        let e = EnvError::OpenpilotNotInstalled;
        assert_eq!(e.to_string(), "openpilot is not installed");
    }
}
