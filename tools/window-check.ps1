# Real-screen check of windowed play on Windows (decision 0173, docs/ai/proofs/window.md).
# Runs short windowed sessions with each present mode, drives them with synthetic input
# (key presses, a held key across a focus loss, resize, minimize) and writes one JSON file
# per run plus a summary. Opens windows: the owner's OK is needed before running it on a
# user's machine (RULES.md rule 3). Muted; every window closes itself (--seconds).
#
#   powershell -ExecutionPolicy Bypass -File tools\window-check.ps1 -Exe target\release\marionette.exe -Out <dir>
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Scene = "examples\playground3d.scene.json",
    [int]$Seconds = 10
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hgt, bool repaint);
  [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
  public static void Down(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); }
  public static void Up(byte vk) { keybd_event(vk, 0, 2, UIntPtr.Zero); }
}
"@

function Start-Run([string]$name, [string[]]$extra) {
    $stats = Join-Path $Out "$name.frames.json"
    $rec = Join-Path $Out "$name.inputs.json"
    $log = Join-Path $Out "$name.out.json"
    $argv = @("run", $Scene, "--mute", "--seconds", "$Seconds", "--frame-stats", $stats, "--record", $rec) + $extra
    $p = Start-Process -FilePath $Exe -ArgumentList $argv -PassThru -RedirectStandardOutput $log -WindowStyle Normal
    $null = $p.Handle  # keeps the exit code readable in Windows PowerShell 5.1
    # Wait for the window.
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    return $p
}

function Drive($p, [bool]$disturb) {
    $h = $p.MainWindowHandle
    [W]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Seconds 2
    # Taps of W (0x57) and D (0x44) for input latency samples.
    for ($i = 0; $i -lt 20; $i++) {
        $vk = if ($i % 2) { 0x44 } else { 0x57 }
        [W]::Down($vk); Start-Sleep -Milliseconds 60; [W]::Up($vk); Start-Sleep -Milliseconds 90
    }
    if (-not $disturb) { return }
    # Hold W, then move focus away: the release goes elsewhere; the engine must release W.
    [W]::Down(0x57); Start-Sleep -Milliseconds 200
    [W]::SetForegroundWindow([W]::GetShellWindow()) | Out-Null
    Start-Sleep -Milliseconds 300
    [W]::Up(0x57)
    Start-Sleep -Milliseconds 500
    [W]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 500
    # Resize twice, minimize for 2 s (CPU time while minimized is measured), restore.
    [W]::MoveWindow($h, 100, 100, 1600, 900, $true) | Out-Null; Start-Sleep -Milliseconds 500
    [W]::MoveWindow($h, 100, 100, 960, 540, $true) | Out-Null; Start-Sleep -Milliseconds 500
    $p.Refresh(); $cpu0 = $p.TotalProcessorTime
    [W]::ShowWindow($h, 6) | Out-Null; Start-Sleep -Seconds 2
    $p.Refresh(); $cpu1 = $p.TotalProcessorTime
    [W]::ShowWindow($h, 9) | Out-Null
    [W]::SetForegroundWindow($h) | Out-Null
    return ($cpu1 - $cpu0).TotalMilliseconds
}

$runs = @(
    @{ name = "vsync"; extra = @(); disturb = $true },
    @{ name = "vsync-latency1"; extra = @("--frame-latency", "1"); disturb = $false },
    @{ name = "mailbox"; extra = @("--present", "mailbox"); disturb = $false },
    @{ name = "immediate"; extra = @("--present", "immediate"); disturb = $false },
    @{ name = "immediate-cap144"; extra = @("--present", "immediate", "--fps-cap", "144"); disturb = $false },
    @{ name = "fullscreen"; extra = @("--fullscreen"); disturb = $false }
)
$summary = @{}
foreach ($r in $runs) {
    $p = Start-Run $r.name $r.extra
    $minCpu = Drive $p $r.disturb
    if (-not $p.WaitForExit(($Seconds + 15) * 1000)) { $p.Kill(); $summary[$r.name] = @{ error = "did not close" }; continue }
    $frames = Get-Content (Join-Path $Out "$($r.name).frames.json") -Raw | ConvertFrom-Json
    $summary[$r.name] = @{ exit = $p.ExitCode; minimized_cpu_ms_per_2s = $minCpu; frames = $frames }
    Start-Sleep -Seconds 1
}
$summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $Out "summary.json")
Write-Output (Join-Path $Out "summary.json")
