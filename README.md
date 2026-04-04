# GlideKit Desktop

Native desktop wrapper for [GlideKit](https://github.com/mhayden123/glidekit). One-click app for rendering openpilot driving clips. Download, install, paste a route, get a video.

On Windows, the installer sets up everything automatically — Python, Git, FFmpeg, the works. On Linux and macOS, install the [GlideKit backend](https://github.com/mhayden123/glidekit) first, then the desktop app finds it and handles the rest.

## Download

Grab the latest from [Releases](https://github.com/mhayden123/glidekit-desktop/releases):

| Platform | Format |
|----------|--------|
| Windows | `.exe` `.msi` |
| Linux | `.AppImage` `.deb` |
| macOS | `.dmg` (Apple Silicon + Intel) |

## What It Does

Launches a local render server in the background and opens the GlideKit UI in a native window. Pick a render type, paste a Comma Connect URL, click Clip. The app manages the server lifecycle — starts on open, stops on close.

## First Launch

**Windows** — fully automatic. The installer handles Python, Git, uv, FFmpeg, and project setup. First launch takes a few minutes while dependencies install, then it's instant after that. For UI render types (the ones with openpilot overlays), the app walks you through a one-time WSL setup.

**Linux** — run `git clone https://github.com/mhayden123/glidekit.git && cd glidekit && ./install.sh` first. The desktop app finds the project automatically after that.

**macOS** — same as Linux. Beta support.

## Troubleshooting

- Reinstall the app (Windows auto-cleans on reinstall)
- Run with `--clean` flag: `"GlideKit.exe" --clean`
- Delete `%LOCALAPPDATA%\glidekit\` and relaunch

## Build from Source

Needs Rust and Node.js 22+. On Linux, also install `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`.

```bash
git clone https://github.com/mhayden123/glidekit-desktop.git
cd glidekit-desktop
npm install
npm run tauri dev      # dev mode
npm run tauri build    # release build
```

## More

CLI usage, render type details, and architecture docs are in the [GlideKit repo](https://github.com/mhayden123/glidekit).

## Credits

Built on [nelsonjchen's](https://github.com/nelsonjchen) op-replay-clipper. Uses [openpilot](https://github.com/commaai/openpilot) by [comma.ai](https://github.com/commaai), replay tooling by [deanlee](https://github.com/deanlee), and headless rendering patches by [ntegan1](https://github.com/ntegan1).

## License

[LICENSE.md](https://github.com/mhayden123/glidekit/blob/main/LICENSE.md)
