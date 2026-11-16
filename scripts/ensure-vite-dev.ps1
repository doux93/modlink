$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$listeners = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue

foreach ($listener in $listeners) {
    $processInfo = Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)" -ErrorAction SilentlyContinue
    $commandLine = [string]$processInfo.CommandLine

    if ($commandLine.Contains($projectRoot) -and $commandLine.Contains("vite")) {
        Write-Host "Vite dev server already running on port 1420."
        exit 0
    }
}

if ($listeners) {
    $owners = ($listeners | Select-Object -ExpandProperty OwningProcess -Unique) -join ", "
    Write-Error "Port 1420 is already in use by process(es): $owners"
    exit 1
}

Push-Location $projectRoot
npm.cmd run dev
