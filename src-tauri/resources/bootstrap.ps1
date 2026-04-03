# OP Replay Clipper — Windows Bootstrap Script
# Called by the NSIS installer and by the app's first-run fallback.
# Installs Python, Git, uv, clones the clipper repo, and runs setup.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1 [-Silent]
#
# Exit codes:
#   0 = success
#   1 = failure (check output for details)

param(
    [switch]$Silent
)

$ErrorActionPreference = "Stop"
$ClipperHome = Join-Path $env:LOCALAPPDATA "op-replay-clipper"
$ProjectDir = Join-Path $ClipperHome "op-replay-clipper-native"
$ProgressFile = Join-Path $ClipperHome "bootstrap-progress.txt"

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message"
    # Write progress to a file so the Tauri app can read it
    if (Test-Path (Split-Path $ProgressFile)) {
        Set-Content -Path $ProgressFile -Value $Message -Force
    }
}

function Write-OK {
    param([string]$Message)
    Write-Host "  OK: $Message"
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  WARN: $Message"
}

function Test-Command {
    param([string]$Name)
    $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

# --- Create base directory ---
Write-Step "Creating directories"
New-Item -ItemType Directory -Force -Path $ClipperHome | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $ClipperHome "output") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $ClipperHome "data") | Out-Null
Write-OK "Base directory: $ClipperHome"

# --- Install Git ---
Write-Step "Checking Git"
if (Test-Command "git") {
    Write-OK "Git already installed: $(git --version)"
} else {
    Write-Step "Installing Git via winget"
    try {
        if (Test-Command "winget") {
            winget install Git.Git --silent --accept-package-agreements --accept-source-agreements 2>&1
            # Refresh PATH
            $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
            if (Test-Command "git") {
                Write-OK "Git installed"
            } else {
                # winget installed it but PATH hasn't refreshed — try common location
                $gitPath = "C:\Program Files\Git\cmd"
                if (Test-Path "$gitPath\git.exe") {
                    $env:PATH += ";$gitPath"
                    Write-OK "Git installed at $gitPath"
                } else {
                    Write-Warn "Git installed but not on PATH. Restart may be needed."
                }
            }
        } else {
            Write-Warn "winget not available. Please install Git manually from https://git-scm.com"
            exit 1
        }
    } catch {
        Write-Warn "Failed to install Git: $_"
        exit 1
    }
}

# --- Install Python 3.12+ ---
Write-Step "Checking Python"
$pythonCmd = $null
foreach ($cmd in @("python3", "python", "py")) {
    if (Test-Command $cmd) {
        $ver = & $cmd --version 2>&1 | Select-String -Pattern "(\d+)\.(\d+)"
        if ($ver -and [int]$ver.Matches[0].Groups[1].Value -ge 3 -and [int]$ver.Matches[0].Groups[2].Value -ge 12) {
            $pythonCmd = $cmd
            break
        }
    }
}

if ($pythonCmd) {
    Write-OK "Python already installed: $(& $pythonCmd --version)"
} else {
    Write-Step "Installing Python 3.12 via winget"
    try {
        if (Test-Command "winget") {
            winget install Python.Python.3.12 --silent --accept-package-agreements --accept-source-agreements 2>&1
            $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
            # Try to find python after install
            foreach ($cmd in @("python3", "python", "py")) {
                if (Test-Command $cmd) {
                    $pythonCmd = $cmd
                    break
                }
            }
            if (-not $pythonCmd) {
                # Check common install locations
                $pyPaths = @(
                    "$env:LOCALAPPDATA\Programs\Python\Python312\python.exe",
                    "C:\Python312\python.exe"
                )
                foreach ($p in $pyPaths) {
                    if (Test-Path $p) {
                        $env:PATH += ";$(Split-Path $p)"
                        $pythonCmd = $p
                        break
                    }
                }
            }
            if ($pythonCmd) {
                Write-OK "Python installed: $(& $pythonCmd --version)"
            } else {
                Write-Warn "Python installed but not found on PATH. Restart may be needed."
                exit 1
            }
        } else {
            Write-Warn "winget not available. Please install Python 3.12+ from https://www.python.org"
            exit 1
        }
    } catch {
        Write-Warn "Failed to install Python: $_"
        exit 1
    }
}

# --- Install uv ---
Write-Step "Checking uv"
if (Test-Command "uv") {
    Write-OK "uv already installed: $(uv --version)"
} else {
    Write-Step "Installing uv"
    try {
        & $pythonCmd -m pip install --user uv 2>&1
        # Also try the official installer
        if (-not (Test-Command "uv")) {
            Invoke-WebRequest -Uri "https://astral.sh/uv/install.ps1" -UseBasicParsing | Invoke-Expression 2>&1
        }
        $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
        if (Test-Command "uv") {
            Write-OK "uv installed: $(uv --version)"
        } else {
            # Check common location
            $uvPath = Join-Path $env:USERPROFILE ".local\bin"
            if (Test-Path "$uvPath\uv.exe") {
                $env:PATH += ";$uvPath"
                Write-OK "uv installed at $uvPath"
            } else {
                Write-Warn "uv installation may require PATH update. Restart may be needed."
            }
        }
    } catch {
        Write-Warn "Failed to install uv: $_"
    }
}

# --- Clone the clipper project ---
Write-Step "Setting up clipper project"
if (Test-Path (Join-Path $ProjectDir "clip.py")) {
    Write-OK "Project already cloned at $ProjectDir"
    # Pull latest
    try {
        Push-Location $ProjectDir
        git pull --ff-only 2>&1 | Out-Null
        Pop-Location
        Write-OK "Updated to latest"
    } catch {
        Write-Warn "Could not update project: $_"
    }
} else {
    Write-Step "Cloning op-replay-clipper-native"
    try {
        git clone "https://github.com/mhayden123/op-replay-clipper-native.git" $ProjectDir 2>&1
        Write-OK "Cloned to $ProjectDir"
    } catch {
        Write-Warn "Failed to clone project: $_"
        exit 1
    }
}

# --- Run the Python installer ---
Write-Step "Running clipper setup (install_windows.py)"
try {
    Push-Location $ProjectDir
    if (Test-Command "uv") {
        uv sync 2>&1
        Write-OK "Python dependencies installed"
    } elseif ($pythonCmd) {
        & $pythonCmd -m pip install -r requirements.txt 2>&1
    }
    Pop-Location
} catch {
    Write-Warn "Dependency install had issues: $_"
}

# --- Download FFmpeg if not on PATH ---
Write-Step "Checking FFmpeg"
if (Test-Command "ffmpeg") {
    Write-OK "FFmpeg already available"
} else {
    $ffmpegDir = Join-Path $ClipperHome "ffmpeg"
    if (Test-Path (Join-Path $ffmpegDir "ffmpeg.exe")) {
        Write-OK "FFmpeg already downloaded at $ffmpegDir"
    } else {
        Write-Step "Downloading FFmpeg (static build)"
        try {
            New-Item -ItemType Directory -Force -Path $ffmpegDir | Out-Null
            $ffmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
            $zipPath = Join-Path $env:TEMP "ffmpeg.zip"
            Invoke-WebRequest -Uri $ffmpegUrl -OutFile $zipPath -UseBasicParsing
            $extractDir = Join-Path $env:TEMP "ffmpeg-extract"
            Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force
            # Find the bin directory inside the extracted archive
            $binDir = Get-ChildItem -Path $extractDir -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
            if ($binDir) {
                Copy-Item -Path (Join-Path $binDir.DirectoryName "ffmpeg.exe") -Destination $ffmpegDir -Force
                Copy-Item -Path (Join-Path $binDir.DirectoryName "ffprobe.exe") -Destination $ffmpegDir -Force -ErrorAction SilentlyContinue
                Write-OK "FFmpeg downloaded to $ffmpegDir"
            }
            Remove-Item -Path $zipPath -Force -ErrorAction SilentlyContinue
            Remove-Item -Path $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        } catch {
            Write-Warn "FFmpeg download failed: $_"
        }
    }
}

# --- Check GPU ---
Write-Step "Checking GPU"
try {
    $nvsmi = nvidia-smi --query-gpu=name --format=csv,noheader 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-OK "NVIDIA GPU: $nvsmi"
    } else {
        Write-Warn "No NVIDIA GPU detected. CPU rendering will be used."
    }
} catch {
    Write-Warn "No NVIDIA GPU detected. CPU rendering will be used."
}

# --- Check WSL ---
Write-Step "Checking WSL"
try {
    $wslOut = wsl.exe --list --verbose 2>&1
    if ($LASTEXITCODE -eq 0 -and $wslOut -match "Running") {
        Write-OK "WSL available — UI render types supported"
    } else {
        Write-Warn "WSL not running. UI render types (ui, ui-alt, driver-debug) unavailable."
        Write-Warn "Install WSL: wsl --install"
    }
} catch {
    Write-Warn "WSL not detected."
}

# --- Write completion marker ---
$marker = Join-Path $ClipperHome "bootstrap-complete"
Set-Content -Path $marker -Value (Get-Date -Format o) -Force

Write-Step "Bootstrap complete!"
Write-Host ""
Write-Host "  Clipper home:  $ClipperHome"
Write-Host "  Project:       $ProjectDir"
Write-Host ""

# Clean up progress file
Remove-Item -Path $ProgressFile -Force -ErrorAction SilentlyContinue

exit 0
