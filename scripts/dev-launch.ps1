$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$targetExe = Join-Path $repoRoot "target\debug\smart-shift.exe"
$devRoot = Join-Path $env:TEMP "smart-shift-dev"
$devExe = Join-Path $devRoot "smart-shift.exe"

cargo build

New-Item -ItemType Directory -Force -Path $devRoot | Out-Null
Copy-Item -LiteralPath $targetExe -Destination $devExe -Force

Start-Process -FilePath $devExe
