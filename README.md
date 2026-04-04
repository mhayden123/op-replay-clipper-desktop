# OP Replay Clipper Desktop

One-click app for rendering openpilot driving clips. Download, install, paste a route, get a video.

On Windows, the installer sets up everything automatically — Python, Git, FFmpeg, the works. On Linux and macOS, install the [clipper backend](https://github.com/mhayden123/op-replay-clipper-native) first, then the desktop app finds it and handles the rest.

## Download

Grab the latest from [Releases](https://github.com/mhayden123/op-replay-clipper-desktop/releases):

| Platform | Format |
|----------|--------|
| Linux | `.AppImage` `.deb` `.rpm` |
| macOS | `.dmg` (Apple Silicon + Intel) |
| Windows | `.exe` `.msi` |

## What It Does

Launches a local render server in the background and opens the clipper UI in a native window. Pick a render type, paste a Comma Connect URL, click Clip. The app manages the server lifecycle — starts on open, stops on close.

## First Launch

**Windows** — fully automatic. The installer handles Python, Git, uv, FFmpeg, and project setup. First launch takes a few minutes while dependencies install, then it's instant after that. For UI render types (the ones with openpilot overlays), the app walks you through a one-time WSL setup.

**Linux** — run `git clone https://github.com/mhayden123/op-replay-clipper-native.git && cd op-replay-clipper-native && ./install.sh` first. The desktop app finds the project automatically after that.

**macOS** — same as Linux. Beta support.

## Fixing a Broken Install

- Reinstall the app (Windows auto-cleans on reinstall)
- Run with `--clean` flag: `"OP Replay Clipper.exe" --clean`
- Delete `%LOCALAPPDATA%\op-replay-clipper\` and relaunch

## Build from Source

Needs Rust and Node.js 22+. On Linux, also install `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`.

```bash
git clone https://github.com/mhayden123/op-replay-clipper-desktop.git
cd op-replay-clipper-desktop
npm install
npm run tauri dev      # dev mode
npm run tauri build    # release build
```

## More

CLI usage, render type details, and architecture docs are in the [native clipper repo](https://github.com/mhayden123/op-replay-clipper-native).

## Credits

Built on [nelsonjchen's](https://github.com/nelsonjchen) op-replay-clipper and [comma.ai's](https://github.com/commaai) openpilot.

## License

[LICENSE.md](https://github.com/mhayden123/op-replay-clipper/blob/main/LICENSE.md)
