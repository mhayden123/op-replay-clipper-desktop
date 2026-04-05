# GlideKit - Windows Bootstrap Script
# Compatible with PowerShell 5.1 (ships with Windows 10/11).
# No PS7 syntax. No && operator. No ternary. No null-coalescing.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1 -Silent
#   powershell -ExecutionPolicy Bypass -File bootstrap.ps1 -Clean

param(
    [switch]$Silent,
    [switch]$Clean
)

# Registry key for tracking install locations
$RegKey = 'HKCU:\Software\GlideKit'

# Determine GlideKitHome - check registry first, then default
$GlideKitHome = $null
try {
    $regVal = Get-ItemProperty -Path $RegKey -Name 'GlideKitHome' -ErrorAction SilentlyContinue
    if ($regVal -and $regVal.GlideKitHome -and (Test-Path $regVal.GlideKitHome)) {
        $GlideKitHome = $regVal.GlideKitHome
    }
} catch {}
if (-not $GlideKitHome) {
    $GlideKitHome = Join-Path $env:LOCALAPPDATA 'glidekit'
}

# Guarantee logging from the very first line
if (-not (Test-Path $GlideKitHome)) {
    New-Item -ItemType Directory -Force -Path $GlideKitHome | Out-Null
}
$LogFile = Join-Path $GlideKitHome 'bootstrap-app.log'
Start-Transcript -Path $LogFile -Force

$ErrorActionPreference = 'Continue'

Write-Host '========================================'
Write-Host '  GlideKit - Windows Bootstrap'
Write-Host '========================================'
Write-Host ''

$psVer = $PSVersionTable.PSVersion
Write-Host ('PowerShell version: ' + $psVer.ToString())
$osVer = [System.Environment]::OSVersion.VersionString
Write-Host ('OS: ' + $osVer)
Write-Host ('User: ' + $env:USERNAME)
Write-Host ('LOCALAPPDATA: ' + $env:LOCALAPPDATA)
Write-Host ('Script path: ' + $PSCommandPath)
Write-Host ('Working dir: ' + (Get-Location).Path)
Write-Host ('Date: ' + (Get-Date -Format o))
Write-Host ''

$ProjectDir = Join-Path $GlideKitHome 'glidekit'
$CheckpointFile = Join-Path $GlideKitHome 'bootstrap-checkpoints.txt'

function Write-Step {
    param([string]$Message)
    Write-Host ''
    Write-Host ('==> ' + $Message)
}

function Write-OK {
    param([string]$Message)
    $line = '[OK] ' + $Message
    Write-Host ('  ' + $line)
    Add-Content -Path $CheckpointFile -Value $line
}

function Write-Fail {
    param([string]$Message)
    $line = '[FAIL] ' + $Message
    Write-Host ('  ' + $line)
    Add-Content -Path $CheckpointFile -Value $line
}

function Write-Warn {
    param([string]$Message)
    Write-Host ('  [WARN] ' + $Message)
}

function Test-CommandExists {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    return ($null -ne $cmd)
}

function Refresh-Path {
    $machinePath = [System.Environment]::GetEnvironmentVariable('PATH', 'Machine')
    $userPath = [System.Environment]::GetEnvironmentVariable('PATH', 'User')
    $env:PATH = $machinePath + ';' + $userPath
}

function Add-ToPath {
    param([string]$Dir)
    if (Test-Path $Dir) {
        # Add to current session
        $env:PATH = $env:PATH + ';' + $Dir
        # Persist to user PATH so new processes (Tauri app, terminals) can find it
        $userPath = [System.Environment]::GetEnvironmentVariable('PATH', 'User')
        if ($userPath -and ($userPath -notlike ('*' + $Dir + '*'))) {
            [System.Environment]::SetEnvironmentVariable('PATH', $userPath + ';' + $Dir, 'User')
            Write-Host ('  [PATH] Permanently added: ' + $Dir)
        }
    }
}

function Invoke-Native {
    # Run an external command, capture stdout, ignore stderr noise.
    # PowerShell 5.1 treats ANY stderr output as a NativeCommandError
    # when using 2>&1, so we avoid that entirely.
    param(
        [string]$Command,
        [string[]]$Arguments
    )
    $pinfo = New-Object System.Diagnostics.ProcessStartInfo
    $pinfo.FileName = $Command
    $pinfo.Arguments = $Arguments -join ' '
    $pinfo.RedirectStandardOutput = $true
    $pinfo.RedirectStandardError = $true
    $pinfo.UseShellExecute = $false
    $pinfo.CreateNoWindow = $true
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $pinfo
    $p.Start() | Out-Null
    $stdout = $p.StandardOutput.ReadToEnd()
    $stderr = $p.StandardError.ReadToEnd()
    $p.WaitForExit()
    return @{
        Output = $stdout.Trim()
        Error = $stderr.Trim()
        ExitCode = $p.ExitCode
    }
}

# Reset checkpoints
if (Test-Path $CheckpointFile) { Remove-Item $CheckpointFile -Force }
New-Item -ItemType File -Force -Path $CheckpointFile | Out-Null

# --- Network check ---
Write-Step 'Checking internet connection'
$netOk = $false
try {
    $response = Invoke-WebRequest -Uri 'https://github.com' -UseBasicParsing -TimeoutSec 10 -Method Head
    if ($response.StatusCode -eq 200) {
        $netOk = $true
        Write-OK 'Internet: connected'
    }
} catch {
    if ($_.Exception.Response) {
        $netOk = $true
        Write-OK 'Internet: connected (via redirect)'
    }
}
if (-not $netOk) {
    Write-Fail 'No internet connection'
    Write-Host '  Bootstrap requires internet to download Python, Git, and the GlideKit project.'
    Write-Host '  Check your network settings and try again.'
    Stop-Transcript
    exit 1
}

# --- Clean install ---
if ($Clean) {
    Write-Step 'Clean install requested'
    # Read existing location from registry before deleting
    $cleanDir = $GlideKitHome
    try {
        $regVal = Get-ItemProperty -Path $RegKey -Name 'GlideKitHome' -ErrorAction SilentlyContinue
        if ($regVal -and $regVal.GlideKitHome) {
            $cleanDir = $regVal.GlideKitHome
        }
    } catch {}
    # Delete the entire data directory
    if (Test-Path $cleanDir) {
        Write-Host ('  Removing entire data directory: ' + $cleanDir)
        # Stop transcript before deleting the dir that contains the log
        Stop-Transcript
        Remove-Item -Path $cleanDir -Recurse -Force -ErrorAction SilentlyContinue
        # Recreate and restart transcript
        New-Item -ItemType Directory -Force -Path $GlideKitHome | Out-Null
        $LogFile = Join-Path $GlideKitHome 'bootstrap-app.log'
        Start-Transcript -Path $LogFile -Force
    }
    # Clean registry
    if (Test-Path $RegKey) {
        Remove-Item -Path $RegKey -Force -ErrorAction SilentlyContinue
    }
    Write-OK 'Clean slate prepared'
}

# --- Create directories ---
Write-Step 'Creating directories'
New-Item -ItemType Directory -Force -Path $GlideKitHome | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $GlideKitHome 'output') | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $GlideKitHome 'data') | Out-Null
Write-OK ('Directories created: ' + $GlideKitHome)

# --- Install Git ---
Write-Step 'Checking Git'
Refresh-Path
if (Test-CommandExists 'git') {
    $r = Invoke-Native 'git' @('--version')
    Write-OK ('Git already installed: ' + $r.Output)
} else {
    Write-Step 'Installing Git'
    $gitInstalled = $false

    # Try winget first
    if (Test-CommandExists 'winget') {
        Write-Host '  Trying winget...'
        try {
            $r = Invoke-Native 'winget' @('install', 'Git.Git', '--silent', '--accept-package-agreements', '--accept-source-agreements')
            Write-Host ('  winget exit code: ' + $r.ExitCode)
            Refresh-Path
            if (-not (Test-CommandExists 'git')) {
                $gitCmdDir = 'C:\Program Files\Git\cmd'
                $gitExe = Join-Path $gitCmdDir 'git.exe'
                if (Test-Path $gitExe) {
                    Add-ToPath $gitCmdDir
                }
            }
            if (Test-CommandExists 'git') {
                $gitInstalled = $true
                Write-OK 'Git installed via winget'
            }
        } catch {
            Write-Warn ('winget failed: ' + $_)
        }
    }

    # Fallback: download Git installer directly
    if (-not $gitInstalled) {
        Write-Host '  Downloading Git installer from github.com...'
        try {
            $gitUrl = 'https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.2/Git-2.47.1.2-64-bit.exe'
            $gitInstaller = Join-Path $env:TEMP 'git-installer.exe'
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $gitUrl -OutFile $gitInstaller -UseBasicParsing
            Write-Host '  Running Git installer silently...'
            $proc = Start-Process -FilePath $gitInstaller -ArgumentList '/VERYSILENT /NORESTART /NOCANCEL /SP- /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS' -Wait -PassThru
            Write-Host ('  Git installer exit code: ' + $proc.ExitCode)
            Remove-Item $gitInstaller -Force -ErrorAction SilentlyContinue
            Refresh-Path
            $gitCmdDir = 'C:\Program Files\Git\cmd'
            $gitExe = Join-Path $gitCmdDir 'git.exe'
            if (Test-Path $gitExe) {
                Add-ToPath $gitCmdDir
            }
            if (Test-CommandExists 'git') {
                $gitInstalled = $true
                Write-OK 'Git installed via direct download'
            }
        } catch {
            Write-Fail ('Git download failed: ' + $_)
        }
    }

    if (-not $gitInstalled) {
        Write-Fail 'Could not install Git'
        Write-Host '  Please install Git manually from https://git-scm.com'
        Stop-Transcript
        exit 1
    }
}

# --- Install Python 3.12+ ---
Write-Step 'Checking Python'
Refresh-Path
$pythonCmd = $null
foreach ($cmd in @('python', 'python3', 'py')) {
    if (Test-CommandExists $cmd) {
        try {
            $r = Invoke-Native $cmd @('--version')
            $verStr = $r.Output
            if ($verStr -match '(\d+)\.(\d+)') {
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
    $r = Invoke-Native $pythonCmd @('--version')
    Write-OK ('Python already installed: ' + $r.Output)
} else {
    Write-Step 'Installing Python 3.12'
    $pyInstalled = $false

    # Try winget first
    if (Test-CommandExists 'winget') {
        Write-Host '  Trying winget...'
        try {
            $r = Invoke-Native 'winget' @('install', 'Python.Python.3.12', '--silent', '--accept-package-agreements', '--accept-source-agreements')
            Write-Host ('  winget exit code: ' + $r.ExitCode)
            Refresh-Path
            foreach ($cmd in @('python', 'python3', 'py')) {
                if (Test-CommandExists $cmd) {
                    $pythonCmd = $cmd
                    $pyInstalled = $true
                    break
                }
            }
            if (-not $pyInstalled) {
                $pyPath = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312'
                $pyExe = Join-Path $pyPath 'python.exe'
                if (Test-Path $pyExe) {
                    Add-ToPath $pyPath
                    Add-ToPath (Join-Path $pyPath 'Scripts')
                    $pythonCmd = $pyExe
                    $pyInstalled = $true
                }
            }
            if ($pyInstalled) {
                Write-OK 'Python installed via winget'
            }
        } catch {
            Write-Warn ('winget failed: ' + $_)
        }
    }

    # Fallback: download from python.org
    if (-not $pyInstalled) {
        Write-Host '  Downloading Python from python.org...'
        try {
            $pyUrl = 'https://www.python.org/ftp/python/3.12.8/python-3.12.8-amd64.exe'
            $pyInstaller = Join-Path $env:TEMP 'python-installer.exe'
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $pyUrl -OutFile $pyInstaller -UseBasicParsing
            Write-Host '  Running Python installer silently...'
            $proc = Start-Process -FilePath $pyInstaller -ArgumentList '/quiet InstallAllUsers=0 PrependPath=1 Include_test=0' -Wait -PassThru
            Write-Host ('  Python installer exit code: ' + $proc.ExitCode)
            Remove-Item $pyInstaller -Force -ErrorAction SilentlyContinue
            Refresh-Path
            $pyPath = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312'
            $pyExe = Join-Path $pyPath 'python.exe'
            if (Test-Path $pyExe) {
                Add-ToPath $pyPath
                Add-ToPath (Join-Path $pyPath 'Scripts')
                $pythonCmd = $pyExe
                $pyInstalled = $true
            }
            foreach ($cmd in @('python', 'python3')) {
                if ((-not $pyInstalled) -and (Test-CommandExists $cmd)) {
                    $pythonCmd = $cmd
                    $pyInstalled = $true
                    break
                }
            }
            if ($pyInstalled) {
                Write-OK 'Python installed via direct download'
            }
        } catch {
            Write-Fail ('Python download failed: ' + $_)
        }
    }

    if (-not $pyInstalled) {
        Write-Fail 'Could not install Python 3.12'
        Write-Host '  Please install Python manually from https://www.python.org/downloads/'
        Stop-Transcript
        exit 1
    }
}

# --- Install uv ---
Write-Step 'Checking uv'
Refresh-Path

# Helper: search common uv install locations and add to PATH if found
function Find-UvOnDisk {
    $searchPaths = @(
        (Join-Path $env:USERPROFILE '.local\bin'),
        (Join-Path $env:USERPROFILE '.cargo\bin'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\Scripts'),
        (Join-Path $env:APPDATA 'Python\Python312\Scripts'),
        (Join-Path $env:APPDATA 'Python\Scripts'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Python\Python313\Scripts')
    )
    # Also check pip --user site scripts
    if ($pythonCmd) {
        try {
            $r = Invoke-Native $pythonCmd @('-m', 'site', '--user-base')
            if ($r.ExitCode -eq 0 -and $r.Output) {
                $searchPaths += (Join-Path $r.Output 'Scripts')
            }
        } catch {}
    }
    foreach ($dir in $searchPaths) {
        $uvExe = Join-Path $dir 'uv.exe'
        Write-Host ('  Checking: ' + $uvExe)
        if (Test-Path $uvExe) {
            Write-Host ('  FOUND uv at: ' + $dir)
            Add-ToPath $dir
            return $true
        }
    }

    # Last resort: scan all Python Scripts directories under AppData
    Write-Host '  Scanning AppData for uv.exe...'
    $appDataDirs = @($env:APPDATA, $env:LOCALAPPDATA)
    foreach ($base in $appDataDirs) {
        if (-not $base) { continue }
        $pyDir = Join-Path $base 'Python'
        if (Test-Path $pyDir) {
            $found = Get-ChildItem -Path $pyDir -Recurse -Filter 'uv.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($found) {
                $foundDir = $found.DirectoryName
                Write-Host ('  FOUND uv via scan: ' + $foundDir)
                Add-ToPath $foundDir
                return $true
            }
        }
    }

    return $false
}

if (Test-CommandExists 'uv') {
    $r = Invoke-Native 'uv' @('--version')
    Write-OK ('uv already installed: ' + $r.Output)
} else {
    Write-Step 'Installing uv'
    $uvInstalled = $false

    # Try the official installer first
    try {
        Write-Host '  Running uv installer...'
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $uvScript = Invoke-WebRequest -Uri 'https://astral.sh/uv/install.ps1' -UseBasicParsing
        Invoke-Expression $uvScript.Content
        Refresh-Path
        if (Test-CommandExists 'uv') {
            $uvInstalled = $true
        } else {
            $uvInstalled = Find-UvOnDisk
        }
        if ($uvInstalled) {
            Write-OK 'uv installed via official installer'
        }
    } catch {
        Write-Warn ('uv official installer failed: ' + $_)
    }

    # Fallback: pip install --user
    if ((-not $uvInstalled) -and $pythonCmd) {
        try {
            Write-Host '  Installing uv via pip...'
            $r = Invoke-Native $pythonCmd @('-m', 'pip', 'install', '--user', 'uv')
            Write-Host ('  pip exit code: ' + $r.ExitCode)
            if ($r.Output) { Write-Host ('  pip output: ' + $r.Output) }

            # Ask Python where pip --user scripts actually go
            $r2 = Invoke-Native $pythonCmd @('-c', 'import sysconfig; print(sysconfig.get_path("scripts", "nt_user"))')
            if ($r2.ExitCode -eq 0 -and $r2.Output) {
                $pipScriptsDir = $r2.Output.Trim()
                Write-Host ('  pip --user scripts dir: ' + $pipScriptsDir)
                if (Test-Path (Join-Path $pipScriptsDir 'uv.exe')) {
                    Add-ToPath $pipScriptsDir
                    $uvInstalled = $true
                }
            }

            # Also try the site module approach
            if (-not $uvInstalled) {
                $r3 = Invoke-Native $pythonCmd @('-m', 'site', '--user-base')
                if ($r3.ExitCode -eq 0 -and $r3.Output) {
                    $userScripts = Join-Path $r3.Output.Trim() 'Scripts'
                    Write-Host ('  site --user-base scripts: ' + $userScripts)
                    if (Test-Path (Join-Path $userScripts 'uv.exe')) {
                        Add-ToPath $userScripts
                        $uvInstalled = $true
                    }
                }
            }

            # Broad search if still not found
            if (-not $uvInstalled) {
                Refresh-Path
                if (Test-CommandExists 'uv') {
                    $uvInstalled = $true
                } else {
                    $uvInstalled = Find-UvOnDisk
                }
            }
            if ($uvInstalled) {
                Write-OK 'uv installed via pip'
            }
        } catch {
            Write-Warn ('pip install uv failed: ' + $_)
        }
    }

    if (-not $uvInstalled) {
        Write-Fail 'Could not install uv (searched all known locations)'
        Stop-Transcript
        exit 1
    }
}

# --- Clone the GlideKit project ---
Write-Step 'Setting up GlideKit project'
$clipPy = Join-Path $ProjectDir 'clip.py'
if (Test-Path $clipPy) {
    Write-OK ('Project already exists at ' + $ProjectDir)
    try {
        Push-Location $ProjectDir
        $r = Invoke-Native 'git' @('pull', '--ff-only')
        Write-Host ('  git pull: ' + $r.Output)
        Pop-Location
        Write-OK 'Updated to latest'
    } catch {
        Write-Warn ('Could not update: ' + $_)
        try { Pop-Location } catch {}
    }
} else {
    Write-Step 'Cloning glidekit'
    try {
        $r = Invoke-Native 'git' @('clone', 'https://github.com/mhayden123/glidekit-native.git', $ProjectDir)
        Write-Host ('  git clone exit code: ' + $r.ExitCode)
        if ($r.Output) { Write-Host ('  ' + $r.Output) }
        if (Test-Path (Join-Path $ProjectDir 'clip.py')) {
            Write-OK ('Cloned to ' + $ProjectDir)
        } else {
            Write-Fail 'Clone completed but clip.py not found'
            Stop-Transcript
            exit 1
        }
    } catch {
        Write-Fail ('git clone failed: ' + $_)
        Stop-Transcript
        exit 1
    }
}

# --- Install Python dependencies ---
Write-Step 'Installing Python dependencies'
try {
    Push-Location $ProjectDir
    # First try with locked versions
    $r = Invoke-Native 'uv' @('sync')
    Write-Host ('  uv sync exit code: ' + $r.ExitCode)
    if ($r.Output) { Write-Host ('  ' + $r.Output) }
    if ($r.Error) { Write-Host ('  uv stderr: ' + $r.Error) }
    if ($r.ExitCode -ne 0) {
        # Exit code 2 = lock file mismatch (common cross-platform).
        # Retry without --frozen to let uv re-resolve for this platform.
        Write-Host '  Retrying with --no-frozen...'
        $r2 = Invoke-Native 'uv' @('sync', '--no-frozen')
        Write-Host ('  uv sync --no-frozen exit code: ' + $r2.ExitCode)
        if ($r2.Output) { Write-Host ('  ' + $r2.Output) }
    }
    Pop-Location
    Write-OK 'Python dependencies installed'
} catch {
    Write-Warn ('uv sync had issues: ' + $_)
    try { Pop-Location } catch {}
}

# --- Download FFmpeg ---
Write-Step 'Checking FFmpeg'
Refresh-Path
if (Test-CommandExists 'ffmpeg') {
    Write-OK 'FFmpeg already on PATH'
} else {
    $ffmpegDir = Join-Path $GlideKitHome 'ffmpeg'
    $ffmpegExe = Join-Path $ffmpegDir 'ffmpeg.exe'
    if (Test-Path $ffmpegExe) {
        Add-ToPath $ffmpegDir
        Write-OK ('FFmpeg already downloaded at ' + $ffmpegDir)
    } else {
        Write-Step 'Downloading FFmpeg'
        try {
            New-Item -ItemType Directory -Force -Path $ffmpegDir | Out-Null
            $ffmpegUrl = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip'
            $zipPath = Join-Path $env:TEMP 'ffmpeg-download.zip'
            Write-Host ('  Downloading from: ' + $ffmpegUrl)
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            Invoke-WebRequest -Uri $ffmpegUrl -OutFile $zipPath -UseBasicParsing
            Write-Host '  Extracting...'
            $extractDir = Join-Path $env:TEMP 'ffmpeg-extract'
            if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
            Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force
            $found = Get-ChildItem -Path $extractDir -Recurse -Filter 'ffmpeg.exe' | Select-Object -First 1
            if ($found) {
                Copy-Item -Path $found.FullName -Destination $ffmpegDir -Force
                $ffprobeFile = Join-Path $found.DirectoryName 'ffprobe.exe'
                if (Test-Path $ffprobeFile) {
                    Copy-Item -Path $ffprobeFile -Destination $ffmpegDir -Force
                }
                Add-ToPath $ffmpegDir
                Write-OK ('FFmpeg downloaded to ' + $ffmpegDir)
            } else {
                Write-Fail 'Could not find ffmpeg.exe in downloaded archive'
            }
            Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
            Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue
        } catch {
            Write-Fail ('FFmpeg download failed: ' + $_)
        }
    }
}

# --- GPU check (non-fatal) ---
Write-Step 'Checking GPU'
try {
    $r = Invoke-Native 'nvidia-smi' @('--query-gpu=name', '--format=csv,noheader')
    if ($r.ExitCode -eq 0) {
        Write-OK ('NVIDIA GPU: ' + $r.Output)
    } else {
        Write-Warn 'No NVIDIA GPU. CPU rendering will be used (slower but works fine).'
    }
} catch {
    Write-Warn 'No NVIDIA GPU detected.'
}

# --- WSL check (non-fatal, with timeout) ---
Write-Step 'Checking WSL'
try {
    $wslExe = Get-Command 'wsl.exe' -ErrorAction SilentlyContinue
    if (-not $wslExe) {
        Write-Warn 'wsl.exe not found. WSL is not installed.'
        Write-Warn 'Install WSL: wsl --install'
    } else {
        $pinfo = New-Object System.Diagnostics.ProcessStartInfo
        $pinfo.FileName = 'wsl.exe'
        $pinfo.Arguments = '--list --verbose'
        $pinfo.RedirectStandardOutput = $true
        $pinfo.RedirectStandardError = $true
        $pinfo.UseShellExecute = $false
        $pinfo.CreateNoWindow = $true
        $p = New-Object System.Diagnostics.Process
        $p.StartInfo = $pinfo
        $p.Start() | Out-Null
        $finished = $p.WaitForExit(5000)
        if (-not $finished) {
            $p.Kill()
            Write-Warn 'WSL check timed out (5s). Assuming WSL is not available.'
        } else {
            $wslOut = $p.StandardOutput.ReadToEnd()
            if (($p.ExitCode -eq 0) -and ($wslOut -match 'Running')) {
                Write-OK 'WSL available - UI render types supported'
            } else {
                Write-Warn 'WSL not running. UI render types (ui, ui-alt, driver-debug) unavailable.'
                Write-Warn 'Install WSL: wsl --install'
            }
        }
    }
} catch {
    Write-Warn 'WSL check failed. Skipping.'
}

# --- Write completion marker ---
$marker = Join-Path $GlideKitHome 'bootstrap-complete'
Set-Content -Path $marker -Value (Get-Date -Format o) -Force
Write-OK 'Bootstrap complete marker written'

# --- Write registry keys ---
Write-Step 'Writing registry keys'
try {
    if (-not (Test-Path $RegKey)) {
        New-Item -Path $RegKey -Force | Out-Null
    }
    Set-ItemProperty -Path $RegKey -Name 'GlideKitHome' -Value $GlideKitHome
    Set-ItemProperty -Path $RegKey -Name 'ProjectDir' -Value $ProjectDir
    Set-ItemProperty -Path $RegKey -Name 'InstalledDate' -Value (Get-Date -Format o)
    Write-OK ('Registry: ' + $RegKey)
} catch {
    Write-Warn ('Registry write failed: ' + $_)
}

# --- Summary ---
Write-Host ''
Write-Host '========================================'
Write-Host '  Bootstrap complete!'
Write-Host '========================================'
Write-Host ''
Write-Host ('  GlideKit home: ' + $GlideKitHome)
Write-Host ('  Project:       ' + $ProjectDir)
Write-Host ''

Stop-Transcript
exit 0
