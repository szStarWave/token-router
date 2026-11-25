# 开发期启动桌面壳（不弹控制台窗口）。
#
# 背景：`src-tauri` 的 debug 构建是 **console 子系统**程序（release 构建才是 GUI 子系统，
# 见 `src/main.rs` 的 `windows_subsystem = "windows"`）。用 `Start-Process` 或资源管理器
# 双击启动 debug 二进制时，Windows 会给它分配一个控制台（Win11 默认落在 Windows Terminal），
# 于是出现一个只显示工作目录的 cmd/Terminal 窗口。
#
# `Start-Process -WindowStyle Hidden` 在 Windows Terminal 下会被忽略（终端接管控制台），
# 所以这里用 .NET ProcessStartInfo 的 `CreateNoWindow`（即 CREATE_NO_WINDOW），
# 这样控制台不会创建，Tauri 自己的窗口不受影响。
#
# 用法（在任意目录）：
#   powershell -NoProfile -ExecutionPolicy Bypass -File v2/apps/desktop/scripts/launch-debug.ps1
# 可选参数：-Configuration debug|release（release 直接双击 exe 即可，不需要本脚本）。

param(
    [ValidateSet('debug', 'release')]
    [string]$Configuration = 'debug'
)

$ErrorActionPreference = 'Stop'
$srcTauri = Join-Path $PSScriptRoot '..\src-tauri'
$exe = Join-Path $srcTauri "target\$Configuration\tr-desktop.exe"
if (-not (Test-Path $exe)) {
    throw "找不到 $exe；先运行 cargo build --features custom-protocol"
}

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = (Resolve-Path $exe).Path
$psi.WorkingDirectory = (Resolve-Path $srcTauri).Path
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true

# 同一数据目录只允许一个壳：已运行时先退出，避免第二个实例抢端口。
$running = Get-Process tr-desktop -ErrorAction SilentlyContinue
if ($running) {
    Write-Host "tr-desktop 已在运行（PID $($running[0].Id)），先结束再启动"
    $running | ForEach-Object { Stop-Process -Id $_.Id -Force }
    Start-Sleep -Seconds 1
    Get-Process token-router-v2 -ErrorAction SilentlyContinue |
        ForEach-Object { Stop-Process -Id $_.Id -Force }
}

[System.Diagnostics.Process]::Start($psi) | Out-Null
Write-Host "已启动 $exe（无控制台窗口）"
