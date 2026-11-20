# Builds AutoFlowMate in release mode and packages a portable zip in .\dist
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Building AutoFlowMate $version (release)..."
cargo build --release --workspace
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$name = "AutoFlowMate-v$version-windows-x64"
$stage = Join-Path $root "dist\$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item target\release\autoflow.exe, target\release\autoflow-studio.exe $stage
Copy-Item README.md $stage
if (Test-Path LICENSE) { Copy-Item LICENSE $stage }
Copy-Item assets\autoflowmate.png $stage
# Logo images the README shows.
New-Item -ItemType Directory -Force (Join-Path $stage 'assets') | Out-Null
Copy-Item assets\wordmark.png, assets\wordmark-dark.png (Join-Path $stage 'assets')
foreach ($dir in 'skills', 'schema', 'docs') { Copy-Item $dir (Join-Path $stage $dir) -Recurse }

# Examples without run output or logs.
$ex = Join-Path $stage 'examples'
New-Item -ItemType Directory -Force $ex | Out-Null
Get-ChildItem examples -File -Filter *.json | Copy-Item -Destination $ex
foreach ($dir in 'images', 'web', 'data') { if (Test-Path "examples\$dir") { Copy-Item "examples\$dir" (Join-Path $ex $dir) -Recurse } }

@"
AutoFlowMate $version - portable Windows build

  autoflow-studio.exe   visual editor (flowchart, run, schedule)
  autoflow.exe          command line: run / validate / schedule / inspect ...

Quick start:
  autoflow-studio.exe examples\file_organizer.json
  autoflow.exe run examples\web_form_demo.json
  autoflow.exe run examples\documents_demo.json   (offline: PDF table, Word letter, zip)
  autoflow.exe record my_flow.json                (record clicks and typing; Ctrl+Alt+R stops)

Keep autoflow.exe next to autoflow-studio.exe (scheduled tasks run it).
Abort a running flow: move the mouse into a screen corner or press Ctrl+Alt+Q.
AI agents: copy skills\autoflowmate to your agent's skills folder.
https://github.com/SRathinaGiri/AutoFlowMate
"@ | Set-Content (Join-Path $stage 'QUICKSTART.txt')

$zip = Join-Path $root "dist\$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash
"$hash  $name.zip" | Set-Content "$zip.sha256"
Write-Host "Created $zip"
Write-Host "SHA256 $hash"
