#!/usr/bin/env bash
set -euo pipefail

# OP Replay Clipper Desktop -- Build from source
# Sets up Tauri build dependencies and builds the desktop app.

info()  { printf '\033[1;34m[INFO]\033[0m  %s\n' "$*"; }
ok()    { printf '\033[1;32m[OK]\033[0m    %s\n' "$*"; }
fail()  { printf '\033[1;31m[FAIL]\033[0m  %s\n' "$*"; exit 1; }

APP_DIR="$(cd "$(dirname "$0")" && pwd)"

# 1. Check Rust
if ! command -v cargo &>/dev/null; then
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
ok "Rust $(rustc --version | cut -d' ' -f2) installed"

# 2. Check Node.js
command -v node &>/dev/null || fail "Node.js is required. Install it from https://nodejs.org/"
ok "Node.js $(node --version) installed"

# 3. Install Tauri system dependencies (Linux)
if [[ "$(uname)" == "Linux" ]]; then
    info "Checking Tauri system dependencies..."
    if [ -f /run/ostree-booted ]; then
        if pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
            ok "Tauri system dependencies found (immutable OS)"
        else
            fail "WebKitGTK 4.1 not found. Install with: rpm-ostree install webkit2gtk4.1-devel openssl-devel libxdo-devel librsvg2-devel && systemctl reboot"
        fi
    elif command -v apt-get &>/dev/null; then
        sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
            libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
        ok "Tauri system dependencies installed"
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y webkit2gtk4.1-devel openssl-devel curl wget file \
            libxdo-devel librsvg2-devel
        ok "Tauri system dependencies installed"
    fi
fi

# 4. Install npm dependencies
info "Installing npm dependencies..."
cd "$APP_DIR"
npm install
ok "npm dependencies installed"

# 5. Build the Tauri app
info "Building Tauri desktop app..."
npm run tauri build

ok "Build complete!"
BINARY=$(find "$APP_DIR/src-tauri/target/release/bundle" -type f -name "*.deb" -o -name "*.AppImage" -o -name "*.rpm" 2>/dev/null | head -1)
if [ -n "$BINARY" ]; then
    echo ""
    echo "  Binary: $BINARY"
fi
echo ""
echo "  Or run in dev mode: npm run tauri dev"
echo ""
