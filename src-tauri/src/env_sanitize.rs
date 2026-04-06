use std::process::Command;

pub trait CommandExt {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
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
}
