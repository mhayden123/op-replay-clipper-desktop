# GlideKit Desktop

Tauri v2 native shell (Rust + single HTML page) that manages the GlideKit Python/FastAPI
render server as a child process and displays its UI in a webview window.

## Stack

- **Rust 2021** / **Tauri 2** (single-crate desktop app, `src-tauri/`)
- **Frontend**: one static `src/index.html` loading screen; webview navigates to
  `http://localhost:7860` once the bundled uvicorn server is healthy
- **Runtime deps**: `reqwest` (blocking health checks), `serde`, `dirs`
- **Node**: only for `@tauri-apps/cli` (via `npm run tauri`)
- **Bundling**: NSIS (Windows) with custom `nsis/installer-hooks.nsi`, plus AppImage/deb/dmg

## Architecture

Desktop wrapper, not a UI. It:
1. Locates the sibling `glidekit-native` (or `glidekit`) Python project via registry
   (Windows), env var `GLIDEKIT_PROJECT_DIR`, exe-sibling search, or `~/.glidekit/`.
2. Resolves `uv` (PATH, then known install locations).
3. On Windows, auto-bootstraps Python/Git/uv/FFmpeg via bundled
   `resources/bootstrap.ps1` (falls back to downloading it from GitHub).
4. On Linux/macOS, requires `~/.glidekit/openpilot/.venv` (openpilot install).
5. `uv sync` then spawns `uv run python -m uvicorn web.server:app --port 7860`.
6. Polls `/api/health` (30s timeout), then loads `http://localhost:7860` in webview.
7. On window `Destroyed`, kills the child server.

All server env is sanitized (`LD_LIBRARY_PATH`, `LD_PRELOAD`, `PYTHONHOME`, `PYTHONPATH`
removed) to avoid AppImage-bundled-lib pollution.

## Key Files

| File | Purpose |
|------|---------|
| `src-tauri/src/main.rs` | Entry point + Tauri builder (~90 lines) |
| `src-tauri/src/constants.rs` | URLs, timeouts, ports |
| `src-tauri/src/env_sanitize.rs` | `CommandExt` trait (strips LD_LIBRARY_PATH etc.) |
| `src-tauri/src/ipc.rs` | `send_status` / `send_error` to webview |
| `src-tauri/src/state.rs` | `AppState` struct |
| `src-tauri/src/paths.rs` | `data_dir`, `openpilot_root`, `find_glidekit_project`, `resolve_uv` |
| `src-tauri/src/platform.rs` | `check_nvidia`, `check_wsl` |
| `src-tauri/src/server.rs` | `kill_stale_server`, `start_server`, `wait_for_server`, `stop_server` |
| `src-tauri/src/startup.rs` | `startup_sequence` (background thread orchestrator) |
| `src-tauri/src/bootstrap/mod.rs` | `EnvError`, `check_environment` |
| `src-tauri/src/bootstrap/linux.rs` | Linux/macOS: `install_uv`, `clone_project`, `run_install_script` |
| `src-tauri/src/bootstrap/windows.rs` | Windows: `find_bootstrap_script`, `download_bootstrap_script`, `run_bootstrap` |
| `src-tauri/tauri.conf.json` | Tauri config (identifier `com.glidekit.desktop`) |
| `src-tauri/nsis/installer-hooks.nsi` | Windows installer hooks (runs bootstrap) |
| `src-tauri/resources/bootstrap.ps1` | Windows first-launch Python/uv/Git installer |
| `src/index.html` | Loading spinner UI; status/error events from Rust |
| `.github/workflows/release.yml` | Tag-triggered 4-platform release build |
| `install.sh` | Linux/macOS dev bootstrap |

## Commands

```bash
npm install
npm run tauri dev         # dev mode
npm run tauri build       # release build
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

Run with `--clean` to force wipe `~/.glidekit/glidekit/` and re-bootstrap (Windows).

## Conventions

- **Modular Rust**: code split into focused modules under `src-tauri/src/`, no file over 320 lines.
- **Error style**: `Result<_, String>` at boundaries, `eprintln!` with `[tag]` prefixes
  for logs (e.g. `[startup]`, `[resolve-uv]`, `[server]`).
- **`#[cfg(target_os = "windows")]`** for platform branches; non-Windows is the default path.
- **Status/error events**: `send_status(win, ...)` / `send_error(win, ...)` → frontend
  via Tauri events.
- **Commits**: Conventional commits (`feat:`, `fix:`, `chore:`) — attribution disabled.
- **Tags**: `vMAJOR.MINOR.PATCH` (optionally `-beta`/`-alpha`); release workflow strips
  suffix and injects version into `tauri.conf.json` at build time.

## Version Injection

`tauri.conf.json` ships with `"version": "0.0.0"`. CI extracts version from the git tag
(`v0.6.1` → `0.6.1`) and rewrites the file before building — never hand-edit version.

## Constants

- Server port: `7860` (hardcoded)
- Data dir: `~/.glidekit/` (`data_dir()`)
- Startup timeout: 30s

## Where to Look

| Task | Location |
|------|----------|
| Add startup step | `src-tauri/src/startup.rs` |
| Change server launch | `src-tauri/src/server.rs` |
| Adjust project discovery | `src-tauri/src/paths.rs` |
| Linux/macOS bootstrap | `src-tauri/src/bootstrap/linux.rs` |
| Windows bootstrap | `src-tauri/src/bootstrap/windows.rs` |
| Edit loading UI | `src/index.html` |
| Windows bootstrap script | `src-tauri/resources/bootstrap.ps1` |
| Installer UX | `src-tauri/nsis/installer-hooks.nsi` |
