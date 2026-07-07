# Prefetch Tauri NSIS toolchain into %LOCALAPPDATA%\tauri\NSIS
# Matches tauri-bundler requirements for NSIS 3.11 + nsis_tauri_utils v0.5.3

$ErrorActionPreference = 'Stop'

$cacheRoot = Join-Path $env:LOCALAPPDATA 'tauri'
$nsisRoot = Join-Path $cacheRoot 'NSIS'
$pluginDir = Join-Path $nsisRoot 'Plugins\x86-unicode\additional'
$utilsDll = Join-Path $pluginDir 'nsis_tauri_utils.dll'

$nsisZipUrl = 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip'
$utilsUrl = 'https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll'

function Test-NsisReady {
    $required = @(
        (Join-Path $nsisRoot 'makensis.exe'),
        (Join-Path $nsisRoot 'Bin\makensis.exe'),
        (Join-Path $nsisRoot 'Include\MUI2.nsh'),
        $utilsDll
    )
    foreach ($path in $required) {
        if (-not (Test-Path $path)) { return $false }
    }
    return $true
}

if (Test-NsisReady) {
    Write-Host "Tauri NSIS toolchain already present at $nsisRoot"
    exit 0
}

New-Item -ItemType Directory -Force -Path $cacheRoot | Out-Null
$tempDir = Join-Path $env:TEMP ("tauri-nsis-" + [guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

try {
    $nsisZip = Join-Path $tempDir 'nsis-3.11.zip'
    Write-Host "Downloading NSIS 3.11..."
    Invoke-WebRequest -Uri $nsisZipUrl -OutFile $nsisZip -UseBasicParsing

    if (Test-Path $nsisRoot) {
        Remove-Item -Recurse -Force $nsisRoot
    }

    Write-Host "Extracting NSIS..."
    Expand-Archive -Path $nsisZip -DestinationPath $tempDir -Force
    $extracted = Join-Path $tempDir 'nsis-3.11'
    if (-not (Test-Path $extracted)) {
        throw "Expected folder not found after extract: $extracted"
    }
    Move-Item -Path $extracted -Destination $nsisRoot

    New-Item -ItemType Directory -Force -Path $pluginDir | Out-Null
    Write-Host "Downloading nsis_tauri_utils.dll..."
    Invoke-WebRequest -Uri $utilsUrl -OutFile $utilsDll -UseBasicParsing

    if (-not (Test-NsisReady)) {
        throw "NSIS setup incomplete; required files are still missing under $nsisRoot"
    }

    Write-Host "Ready: $nsisRoot"
}
finally {
    if (Test-Path $tempDir) {
        Remove-Item -Recurse -Force $tempDir
    }
}
