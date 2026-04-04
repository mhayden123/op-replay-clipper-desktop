# OP Replay Clipper — Windows Bootstrap Script
# Works on PowerShell 5.1+ (ships with Windows 10/11).
# No PS7-only syntax: no &&, no ternary, no null-coalescing.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1 -Silent
#
# Logs everything to %LOCALAPPDATA%\op-replay-clipper\bootstrap-app.log

param(
    [switch]$Silent
)

# --- Guarantee logging from the very first line ---
$ClipperHome = Join-Path $env:LOCALAPPDATA "op-replay-clipper"
if (-not (Test-Path $ClipperHome)) {
    New-Item -ItemType Directory -Force -Path $ClipperHome | Out-Null
}
$LogFile = Join-Path $ClipperHome "bootstrap-app.log"
Start-Transcript -Path $LogFile -Force

$ErrorActionPreference = "Continue"

Write-Host "========================================"
Write-Host "  OP Replay Clipper — Windows Bootstrap"
Write-Host "========================================"
Write-Host ""
Write-Host "PowerShell version: $($PSVersionTable.PSVersion)"
Write-Host "OS: $([System.Environment]::OSVersion.VersionString)"
Write-Host "User: $env:USERNAME"
Write-Host "LOCALAPPDATA: $env:LOCALAPPDATA"
Write-Host "Script path: $PSCommandPath"
Write-Host "Working dir: $(Get-Location)"
Write-Host "Date: $(Get-Date -Format o)"
Write-Host ""

$ProjectDir = Join-Path $ClipperHome "op-replay-clipper-native"
$CheckpointFile = Join-Path $ClipperHome "bootstrap-checkpoints.txt"

function Write-Step {
    param([string]$Message)
    Write-Host ""
    Write-Host "==> $Message"
}

function Write-OK {
    param([string]$Message)
    $line = "[OK] $Message"
    Write-Host "  $line"
    Add-Content -Path $CheckpointFile -Value $line
}

function Write-Fail {
    param([string]$Message)
    $line = "[FAIL] $Message"
    Write-Host "  $line"
    Add-Content -Path $CheckpointFile -Value $line
}

function Write-Warn {
    param([string]$Message)
    Write-Host "  [WARN] $Message"
}

function Test-CommandExists {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    return ($null -ne $cmd)
}

function Refresh-Path {
    # Reload PATH from registry so newly installed tools are found
    $machinePath = [System.Environment]::GetEnvironmentVariable("PATH", "Machine")
    $userPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")
    $env:PATH = "$machinePath;$userPath"
}

# Reset checkpoints
if (Test-Path $CheckpointFile) { Remove-Item $CheckpointFile -Force }
New-Item -ItemType File -Force -Path $CheckpointFile | Out-Null

# --- Network check ---
Write-Step "Checking internet connection"
$netOk = $false
try {
    $response = Invoke-WebRequest -Uri "https://github.com" -UseBasicParsing -TimeoutSec 10 -Method Head
    if ($response.StatusCode -eq 200) {
        $netOk = $true
        Write-OK "Internet: connected"
    }
} catch {
    # Invoke-WebRequest may throw on redirect but that still means we have internet
    if ($_.Exception.Response) {
        $netOk = $true
        Write-OK "Internet: connected (via redirect)"
    }
}
if (-not $netOk) {
    Write-Fail "No internet connection"
    Write-Host "  Bootstrap requires internet to download Python, Git, and the clipper project."
    Write-Host "  Check your network settings and try again."
    Stop-Transcript
    exit 1
}

# --- Create directories ---
Write-Step "Creating directories"
New-Item -ItemType Directory -Force -Path $ClipperHome | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $ClipperHome "output") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $ClipperHome "data") | Out-Null
Write-OK "Directories created: $ClipperHome"

# --- Install Git ---
Write-Step "Checking Git"
Refresh-Path
if (Test-CommandExists "git") {
    $gitVer = git --version 2>&1
    Write-OK "Git already installed: $gitVer"
} else {
    Write-Step "Installing Git"
    $gitInstalled = $false

    # Try winget first
    if (Test-CommandExists "winget") {
        Write-Host "  Trying winget..."
        try {
            $out = winget install Git.Git --silent --accept-package-agreements --accept-source-agreements 2>&1
            Write-Host "  winget output: $out"
            Refresh-Path
            # Also check common install location
            if (-not (Test-CommandExists "git")) {
                $gitCmd = "C:\Program Files\Git\cmd"
                if (Test-Path "$gitCmd\git.exe") {
                    $env:PATH = "$env:PATH;$gitCmd"
                }
            }
            if (Test-CommandExists "git") {
                $gitInstalled = $true
                Write-OK "Git installed via winget"
            }
        } catch {
            Write-Warn "winget failed: $_"
        }
    }

    # Fallback: download Git installer directly
    if (-not $gitInstalled) {
        Write-Host "  Downloading Git installer from github.com..."
        try {
            $gitUrl = "https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.2/Git-2.47.1.2-64-bit.exe"
            $gitInstaller = Join-Path $env:TEMP "git-installer.exe"
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $gitUrl -OutFile $gitInstaller -UseBasicParsing
            Write-Host "  Running Git installer silently..."
            $proc = Start-Process -FilePath $gitInstaller -ArgumentList "/VERYSILENT /NORESTART /NOCANCEL /SP- /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS" -Wait -PassThru
            Write-Host "  Git installer exit code: $($proc.ExitCode)"
            Remove-Item $gitInstaller -Force -ErrorAction SilentlyContinue
            Refresh-Path
            $gitCmd = "C:\Program Files\Git\cmd"
            if (Test-Path "$gitCmd\git.exe") {
                $env:PATH = "$env:PATH;$gitCmd"
            }
            if (Test-CommandExists "git") {
                $gitInstalled = $true
                Write-OK "Git installed via direct download"
            }
        } catch {
            Write-Fail "Git download failed: $_"
        }
    }

    if (-not $gitInstalled) {
        Write-Fail "Could not install Git"
        Write-Host "  Please install Git manually from https://git-scm.com"
        Stop-Transcript
        exit 1
    }
}

# --- Install Python 3.12+ ---
Write-Step "Checking Python"
Refresh-Path
$pythonCmd = $null
foreach ($cmd in @("python", "python3", "py")) {
    if (Test-CommandExists $cmd) {
        try {
            $verOut = & $cmd --version 2>&1
            $verStr = "$verOut"
            if ($verStr -match "(\d+)\.(\d+)") {
                $major = [int]$Matches[1]
                $minor = [int]$Matches[2]
                if ($major -ge 3 -and $minor -ge 12) {
                    $pythonCmd = $cmd
                    break
                }
            }
        } catch {}
    }
}

if ($pythonCmd) {
    $pyVer = & $pythonCmd --version 2>&1
    Write-OK "Python already installed: $pyVer"
} else {
    Write-Step "Installing Python 3.12"
    $pyInstalled = $false

    # Try winget first
    if (Test-CommandExists "winget") {
        Write-Host "  Trying winget..."
        try {
            $out = winget install Python.Python.3.12 --silent --accept-package-agreements --accept-source-agreements 2>&1
            Write-Host "  winget output: $out"
            Refresh-Path
            foreach ($cmd in @("python", "python3", "py")) {
                if (Test-CommandExists $cmd) {
                    $pythonCmd = $cmd
                    $pyInstalled = $true
                    break
                }
            }
            # Check common install location
            if (-not $pyInstalled) {
                $pyPath = Join-Path $env:LOCALAPPDATA "Programs\Python\Python312"
                if (Test-Path "$pyPath\python.exe") {
                    $env:PATH = "$env:PATH;$pyPath;$pyPath\Scripts"
                    $pythonCmd = "$pyPath\python.exe"
                    $pyInstalled = $true
                }
            }
            if ($pyInstalled) {
                Write-OK "Python installed via winget"
            }
        } catch {
            Write-Warn "winget failed: $_"
        }
    }

    # Fallback: download from python.org
    if (-not $pyInstalled) {
        Write-Host "  Downloading Python from python.org..."
        try {
            $pyUrl = "https://www.python.org/ftp/python/3.12.8/python-3.12.8-amd64.exe"
            $pyInstaller = Join-Path $env:TEMP "python-installer.exe"
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $pyUrl -OutFile $pyInstaller -UseBasicParsing
            Write-Host "  Running Python installer silently..."
            $proc = Start-Process -FilePath $pyInstaller -ArgumentList "/quiet InstallAllUsers=0 PrependPath=1 Include_test=0" -Wait -PassThru
            Write-Host "  Python installer exit code: $($proc.ExitCode)"
            Remove-Item $pyInstaller -Force -ErrorAction SilentlyContinue
            Refresh-Path
            # Check common location after install
            $pyPath = Join-Path $env:LOCALAPPDATA "Programs\Python\Python312"
            if (Test-Path "$pyPath\python.exe") {
                $env:PATH = "$env:PATH;$pyPath;$pyPath\Scripts"
                $pythonCmd = "$pyPath\python.exe"
                $pyInstalled = $true
            }
            foreach ($cmd in @("python", "python3")) {
                if (Test-CommandExists $cmd) {
                    $pythonCmd = $cmd
                    $pyInstalled = $true
                    break
                }
            }
            if ($pyInstalled) {
                Write-OK "Python installed via direct download"
            }
        } catch {
            Write-Fail "Python download failed: $_"
        }
    }

    if (-not $pyInstalled) {
        Write-Fail "Could not install Python 3.12"
        Write-Host "  Please install Python manually from https://www.python.org/downloads/"
        Stop-Transcript
        exit 1
    }
}

# --- Install uv ---
Write-Step "Checking uv"
Refresh-Path
if (Test-CommandExists "uv") {
    $uvVer = uv --version 2>&1
    Write-OK "uv already installed: $uvVer"
} else {
    Write-Step "Installing uv"
    $uvInstalled = $false

    # Try the official installer first (most reliable)
    try {
        Write-Host "  Running uv installer..."
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $uvScript = Invoke-WebRequest -Uri "https://astral.sh/uv/install.ps1" -UseBasicParsing
        Invoke-Expression $uvScript.Content
        Refresh-Path
        # Check common location
        $uvHome = Join-Path $env:USERPROFILE ".local\bin"
        if (Test-Path "$uvHome\uv.exe") {
            $env:PATH = "$env:PATH;$uvHome"
        }
        $uvCargo = Join-Path $env:USERPROFILE ".cargo\bin"
        if (Test-Path "$uvCargo\uv.exe") {
            $env:PATH = "$env:PATH;$uvCargo"
        }
        if (Test-CommandExists "uv") {
            $uvInstalled = $true
            Write-OK "uv installed via official installer"
        }
    } catch {
        Write-Warn "uv official installer failed: $_"
    }

    # Fallback: pip install
    if (-not $uvInstalled -and $pythonCmd) {
        try {
            Write-Host "  Installing uv via pip..."
            & $pythonCmd -m pip install --user uv 2>&1
            Refresh-Path
            if (Test-CommandExists "uv") {
                $uvInstalled = $true
                Write-OK "uv installed via pip"
            }
        } catch {
            Write-Warn "pip install uv failed: $_"
        }
    }

    if (-not $uvInstalled) {
        Write-Fail "Could not install uv"
        Stop-Transcript
        exit 1
    }
}

# --- Clone the clipper project ---
Write-Step "Setting up clipper project"
if (Test-Path (Join-Path $ProjectDir "clip.py")) {
    Write-OK "Project already exists at $ProjectDir"
    try {
        Push-Location $ProjectDir
        $pullOut = git pull --ff-only 2>&1
        Write-Host "  git pull: $pullOut"
        Pop-Location
        Write-OK "Updated to latest"
    } catch {
        Write-Warn "Could not update: $_"
        if (Test-Path (Get-Location).Path) { Pop-Location }
    }
} else {
    Write-Step "Cloning op-replay-clipper-native"
    try {
        $cloneOut = git clone "https://github.com/mhayden123/op-replay-clipper-native.git" $ProjectDir 2>&1
        Write-Host "  $cloneOut"
        if (Test-Path (Join-Path $ProjectDir "clip.py")) {
            Write-OK "Cloned to $ProjectDir"
        } else {
            Write-Fail "Clone completed but clip.py not found"
            Stop-Transcript
            exit 1
        }
    } catch {
        Write-Fail "git clone failed: $_"
        Stop-Transcript
        exit 1
    }
}

# --- Install Python dependencies ---
Write-Step "Installing Python dependencies"
try {
    Push-Location $ProjectDir
    $syncOut = uv sync 2>&1
    Write-Host "  $syncOut"
    Pop-Location
    Write-OK "Python dependencies installed"
} catch {
    Write-Warn "uv sync had issues: $_"
    if (Test-Path (Get-Location).Path) { Pop-Location }
}

# --- Download FFmpeg ---
Write-Step "Checking FFmpeg"
Refresh-Path
if (Test-CommandExists "ffmpeg") {
    Write-OK "FFmpeg already on PATH"
} else {
    $ffmpegDir = Join-Path $ClipperHome "ffmpeg"
    $ffmpegExe = Join-Path $ffmpegDir "ffmpeg.exe"
    if (Test-Path $ffmpegExe) {
        $env:PATH = "$env:PATH;$ffmpegDir"
        Write-OK "FFmpeg already downloaded at $ffmpegDir"
    } else {
        Write-Step "Downloading FFmpeg"
        try {
            New-Item -ItemType Directory -Force -Path $ffmpegDir | Out-Null
            $ffmpegUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
            $zipPath = Join-Path $env:TEMP "ffmpeg-download.zip"
            Write-Host "  Downloading from: $ffmpegUrl"
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $ffmpegUrl -OutFile $zipPath -UseBasicParsing
            Write-Host "  Extracting..."
            $extractDir = Join-Path $env:TEMP "ffmpeg-extract"
            if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
            Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force
            # Find ffmpeg.exe inside the extracted tree
            $found = Get-ChildItem -Path $extractDir -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
            if ($found) {
                Copy-Item -Path $found.FullName -Destination $ffmpegDir -Force
                $ffprobe = Join-Path $found.DirectoryName "ffprobe.exe"
                if (Test-Path $ffprobe) {
                    Copy-Item -Path $ffprobe -Destination $ffmpegDir -Force
                }
                $env:PATH = "$env:PATH;$ffmpegDir"
                Write-OK "FFmpeg downloaded to $ffmpegDir"
            } else {
                Write-Fail "Could not find ffmpeg.exe in downloaded archive"
            }
            Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
            Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        } catch {
            Write-Fail "FFmpeg download failed: $_"
        }
    }
}

# --- GPU check (non-fatal) ---
Write-Step "Checking GPU"
try {
    $nvsmi = nvidia-smi --query-gpu=name --format=csv,noheader 2>&1
    if ($LASTEXITCODE -eq 0 -and $nvsmi) {
        Write-OK "NVIDIA GPU: $nvsmi"
    } else {
        Write-Warn "No NVIDIA GPU. CPU rendering will be used (slower but works fine)."
    }
} catch {
    Write-Warn "No NVIDIA GPU detected."
}

# --- WSL check (non-fatal) ---
Write-Step "Checking WSL"
try {
    $wslOut = wsl.exe --list --verbose 2>&1
    $wslStr = "$wslOut"
    if ($LASTEXITCODE -eq 0 -and $wslStr -match "Running") {
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
Write-OK "Bootstrap complete marker written"

# --- Summary ---
Write-Host ""
Write-Host "========================================"
Write-Host "  Bootstrap complete!"
Write-Host "========================================"
Write-Host ""
Write-Host "  Clipper home:  $ClipperHome"
Write-Host "  Project:       $ProjectDir"
Write-Host ""

Stop-Transcript
exit 0
