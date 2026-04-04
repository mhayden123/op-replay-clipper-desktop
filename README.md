# OP Replay Clipper Desktop

Native desktop app for rendering openpilot driving clips. Launches a local server and opens the clipper UI in a native window. No Docker required.

## Download

Get the latest release for your platform from [Releases](https://github.com/mhayden123/op-replay-clipper-desktop/releases).

| Platform | Format | Notes |
|----------|--------|-------|
| Linux | `.AppImage` | Portable, run anywhere |
| Linux | `.deb` | Ubuntu, Debian, Mint, Pop!_OS |
| Linux | `.rpm` | Fedora, RHEL |
| macOS (Apple Silicon) | `.dmg` | M1/M2/M3/M4 |
| macOS (Intel) | `.dmg` | Intel Macs |
| Windows | `.exe` | NSIS installer (auto-bootstraps everything) |
| Windows | `.msi` | MSI installer |

## How It Works

The app is a lightweight native wrapper (~13 MB) around the [OP Replay Clipper](https://github.com/mhayden123/op-replay-clipper-native) web UI. On launch:

1. Checks that the clipper backend is installed
2. Starts the FastAPI server as a managed child process
3. Opens the web UI in a native window
4. Stops the server when you close the app

## First Run

### Windows

Everything is automatic. The installer:
- Installs Python 3.12, Git, uv, and FFmpeg
- Clones the clipper project
- Sets up dependencies

No terminal, no manual steps. First launch takes a few minutes for setup, then subsequent launches are instant.

For UI render types (ui, ui-alt, driver-debug), the app guides you through WSL installation with a step-by-step wizard.

### Linux

Install the clipper backend first:

```bash
git clone https://github.com/mhayden123/op-replay-clipper-native.git
cd op-replay-clipper-native
./install.sh
```

Then run the desktop app. It finds the clipper project automatically.

### macOS (beta)

Same as Linux -- install the backend first, then run the app.

## Clean Install

If something breaks:

- **Windows installer**: reinstall the app (automatically cleans and re-downloads)
- **App flag**: run `"OP Replay Clipper.exe" --clean` to reset
- **Manual**: delete `%LOCALAPPDATA%\op-replay-clipper\` and relaunch

## Building from Source

```bash
# Prerequisites
# Linux: sudo apt install libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
# All platforms: Rust toolchain + Node.js 22+

git clone https://github.com/mhayden123/op-replay-clipper-desktop.git
cd op-replay-clipper-desktop
npm install
npm run tauri dev      # Development mode
npm run tauri build    # Release build
```

## CLI Usage and Details

For command-line usage, render type documentation, and technical details, see the [native clipper repo](https://github.com/mhayden123/op-replay-clipper-native).

## Credits

- [nelsonjchen](https://github.com/nelsonjchen) -- original op-replay-clipper
- [commaai](https://github.com/commaai) -- openpilot

## License

See [LICENSE.md](https://github.com/mhayden123/op-replay-clipper/blob/main/LICENSE.md).
