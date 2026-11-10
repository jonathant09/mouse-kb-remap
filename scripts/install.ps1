# Builds the app and installs it, or updates it in place, as
#   Documents\mouse-kb-remap\mouse-kb-remap.exe
#
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 [-NoBuild] [-Dir <folder>]
#   (or: start.bat install)
#
# A running exe can't be overwritten, but it can be renamed: the running copy
# is moved aside to mouse-kb-remap.old.exe, the new exe is copied in, the old
# copy is asked to quit (`--quit`), and the app is started again the way it
# runs at sign-in (the scheduled task for "as administrator", so no admin
# prompt). The app deletes the .old.exe when it next starts.

param(
    [switch]$NoBuild,
    [string]$Dir = (Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'mouse-kb-remap')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$built = Join-Path $root 'target\release\mouse-kb-remap.exe'
$exe = Join-Path $Dir 'mouse-kb-remap.exe'
$old = Join-Path $Dir 'mouse-kb-remap.old.exe'

if (-not $NoBuild) {
    Write-Host 'Building (release)...'
    Push-Location $root
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { Write-Error 'Build failed.' }
}
if (-not (Test-Path $built)) { Write-Error "No build at $built. Run without -NoBuild." }

New-Item -ItemType Directory -Force -Path $Dir | Out-Null
$wasRunning = [bool](Get-Process mouse-kb-remap -ErrorAction SilentlyContinue)
$firstInstall = -not (Test-Path $exe)

# Swap the exe in place.
if (Test-Path $old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }
if (Test-Path $exe) {
    if (Test-Path $old) {
        # An even older copy is still running from .old.exe: keep its name free.
        $old = Join-Path $Dir ("mouse-kb-remap.old-{0}.exe" -f [DateTime]::Now.Ticks)
    }
    Move-Item -LiteralPath $exe -Destination $old -Force
}
Copy-Item -LiteralPath $built -Destination $exe -Force
$version = (& $exe --version | Out-String).Trim()
Write-Host "Installed $version to $exe"

# Stop the running copy (wherever it was started from).
if ($wasRunning) {
    $p = Start-Process -FilePath $exe -ArgumentList '--quit' -Wait -PassThru -WindowStyle Hidden
    if ($p.ExitCode -ne 0) {
        Write-Warning ('The running copy did not quit (an older version running as administrator can''t be asked to). ' +
            'Quit it from its tray icon, then run this again.')
        exit 1
    }
}
Get-ChildItem -LiteralPath $Dir -Filter 'mouse-kb-remap.old*.exe' -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue

# Start it again the way it starts at sign-in.
if ($wasRunning -or $firstInstall) {
    $taskCommand = $null
    $xml = schtasks /Query /TN mouse-kb-remap /XML 2>$null
    if ($LASTEXITCODE -eq 0 -and ($xml -join "`n") -match '<Command>([^<]+)</Command>') {
        $taskCommand = $Matches[1].Trim().Replace('&amp;', '&')
    }
    $sameExe = $taskCommand -and ((Resolve-Path -LiteralPath $taskCommand -ErrorAction SilentlyContinue).Path -eq (Resolve-Path -LiteralPath $exe).Path)
    if ($sameExe) {
        schtasks /Run /TN mouse-kb-remap | Out-Null
        Write-Host 'Restarted as administrator (startup task).'
    } else {
        if ($firstInstall) { Start-Process -FilePath $exe } else { Start-Process -FilePath $exe -ArgumentList '--tray' }
        Write-Host ($(if ($firstInstall) { 'Started.' } else { 'Restarted (in the tray).' }))
        if ($taskCommand) {
            Write-Host "Start with Windows still opens $taskCommand."
            Write-Host 'Open Settings in the app and click "Use this copy instead" to point it here (one admin prompt).'
        }
    }
}
