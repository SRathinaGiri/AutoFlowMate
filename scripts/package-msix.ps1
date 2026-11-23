# Builds an MSIX package for the Microsoft Store from the release build in target\release.
#   ./scripts/package-msix.ps1                    # test identity, for local tries
#   ./scripts/package-msix.ps1 -IdentityName X -Publisher "CN=..." -PublisherDisplayName "..."
# The three identity values come from Partner Center (Product management > Product identity).
# The Store signs the package, so it is left unsigned here.
# -Layout only stages the files in dist\msix\layout (for `Add-AppxPackage -Register`, which needs
# Developer Mode) and skips makeappx.
param(
    [string]$IdentityName = 'AutoFlowMate.Test',
    [string]$Publisher = 'CN=AutoFlowMate Test',
    [string]$PublisherDisplayName = 'AutoFlowMate (test)',
    [switch]$Layout
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$v = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$version = "$v.0"   # the Store needs four parts, the last one 0
foreach ($exe in 'autoflow.exe', 'autoflow-studio.exe') {
    if (-not (Test-Path "target\release\$exe")) { throw "target\release\$exe missing: run cargo build --release" }
}

$out = Join-Path $root 'dist\msix'
$stage = Join-Path $out 'layout'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage, (Join-Path $stage 'Assets') | Out-Null

Copy-Item target\release\autoflow.exe, target\release\autoflow-studio.exe, README.md, LICENSE $stage
foreach ($dir in 'lang', 'templates', 'schema', 'skills') { Copy-Item $dir (Join-Path $stage $dir) -Recurse }
$ex = Join-Path $stage 'examples'
New-Item -ItemType Directory -Force $ex | Out-Null
Get-ChildItem examples -File -Filter *.json | Copy-Item -Destination $ex
foreach ($dir in 'images', 'web', 'data') { if (Test-Path "examples\$dir") { Copy-Item "examples\$dir" (Join-Path $ex $dir) -Recurse } }

# Logos, drawn from the 256x256 app icon on a transparent background.
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Image]::FromFile((Resolve-Path assets\autoflowmate.png))
function Save-Logo($name, $w, $h, $iconSize) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'; $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($icon, [int](($w - $iconSize) / 2), [int](($h - $iconSize) / 2), $iconSize, $iconSize)
    $g.Dispose()
    $bmp.Save((Join-Path $stage "Assets\$name"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
try {
    Save-Logo 'StoreLogo.png' 50 50 50
    Save-Logo 'Square44x44Logo.png' 44 44 44
    Save-Logo 'Square44x44Logo.targetsize-44_altform-unplated.png' 44 44 44
    Save-Logo 'Square150x150Logo.png' 150 150 120
    Save-Logo 'Wide310x150Logo.png' 310 150 120
} finally { $icon.Dispose() }

$m = Get-Content packaging\msix\AppxManifest.xml -Raw
$esc = { param($s) [Security.SecurityElement]::Escape($s) }
$m = $m.Replace('{{IDENTITY_NAME}}', (& $esc $IdentityName)).Replace('{{PUBLISHER}}', (& $esc $Publisher)).
    Replace('{{PUBLISHER_DISPLAY_NAME}}', (& $esc $PublisherDisplayName)).Replace('{{VERSION}}', $version)
if ($m -match '\{\{') { throw 'manifest still has a {{...}} placeholder' }
[IO.File]::WriteAllText((Join-Path $stage 'AppxManifest.xml'), $m, [Text.UTF8Encoding]::new($false))

if ($Layout) { Write-Host "Staged $stage"; return }

$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1
if (-not $makeappx) { throw 'makeappx.exe not found: install the Windows SDK' }
$msix = Join-Path $out "AutoFlowMate-v$v-x64.msix"
& $makeappx.FullName pack /d $stage /p $msix /o /h SHA256
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed' }
Write-Host "Created $msix ($version, $IdentityName)"
