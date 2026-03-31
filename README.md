# OP Replay Clipper — Desktop App

A native desktop application for rendering openpilot replay clips locally. Built with [Tauri](https://tauri.app/) for a lightweight ~13 MB binary.

## Download

Go to [Releases](https://github.com/mhayden123/op-replay-clipper-desktop/releases) and download the installer for your platform:

| Platform | File | Notes |
|----------|------|-------|
| **Linux** | `.AppImage` | Portable, no install needed |
| **Linux** | `.deb` | Debian/Ubuntu package |
| **macOS** | `.dmg` | Drag to Applications |
| **Windows** | `.msi` | Standard installer |

## Prerequisites

- **[Docker Desktop](https://www.docker.com/products/docker-desktop/)** must be installed and running
- **Linux / Windows**: NVIDIA GPU + [NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html) for GPU-accelerated rendering
- **macOS**: Renders using CPU only (no NVIDIA GPU required, but slower)

## How it works

1. **First launch**: The app pulls the required Docker images from GitHub Container Registry (~10 GB download, one-time)
2. **Starts the web server**: Runs a local Docker container on port 7860
3. **Opens the UI**: A native app window with the full rendering interface
4. **On close**: Docker containers are automatically stopped and cleaned up

No source code checkout or manual Docker setup required.

## Render types

| Type | Description |
|------|-------------|
| UI | openpilot UI replay with path, lanes, and metadata |
| UI Alt | UI with steering wheel and confidence rail |
| Driver Debug | Driver camera with DM telemetry |
| Forward / Wide / Driver | Raw camera transcodes |
| 360 | Spherical video from wide + driver cameras |
| Fwd/Wide | Forward projected onto wide using camera calibration |
| 360 Fwd/Wide | 8K 360 with forward projected onto wide |

## Download sources

- **Comma Connect**: Download route data from comma's cloud servers (default)
- **Local SSH**: Download directly from your comma device on the local network

## Building from source

```bash
# Install dependencies
npm install

# Development mode (hot reload)
npm run tauri dev

# Release build
npm run tauri build
```

### Build dependencies

- Rust (via [rustup](https://rustup.rs/))
- Node.js 18+
- **Linux**: `libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
- **macOS**: Xcode Command Line Tools
- **Windows**: Visual Studio Build Tools with C++ workload
