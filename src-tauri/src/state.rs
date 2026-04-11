use std::process::Child;
use std::sync::Mutex;

pub struct AppState {
    pub server_process: Mutex<Option<Child>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            server_process: Mutex::new(None),
        }
    }
}
