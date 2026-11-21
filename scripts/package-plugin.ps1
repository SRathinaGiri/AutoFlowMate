# Builds the agent plugin ZIP for the OpenAI plugin directory (ChatGPT / Codex) in .\dist:
#   powershell -ExecutionPolicy Bypass -File scripts\package-plugin.ps1
# The skill itself lives in skills\autoflowmate (also used by the Claude Code plugin in
# .claude-plugin\ and by the release zip), so all of them stay the same.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$name = "autoflowmate-plugin-v$version"
$stage = Join-Path $root "dist\$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null

# Manifest with the current version.
$manifest = Get-Content plugin\plugin.json -Raw | ConvertFrom-Json
$manifest.version = $version
# UTF-8 without a byte-order mark: strict JSON readers reject one.
[IO.File]::WriteAllText((Join-Path $stage 'plugin.json'), ($manifest | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding $false))

Copy-Item plugin\README.md, LICENSE $stage
New-Item -ItemType Directory -Force (Join-Path $stage 'skills') | Out-Null
Copy-Item skills\autoflowmate (Join-Path $stage 'skills\autoflowmate') -Recurse
New-Item -ItemType Directory -Force (Join-Path $stage 'assets') | Out-Null
Copy-Item assets\autoflowmate.png (Join-Path $stage 'assets\icon.png')
Copy-Item assets\autoflowmate.png (Join-Path $stage 'assets\logo.png')

# Zip with forward-slash entry names (Windows PowerShell's Compress-Archive writes backslashes).
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$zip = Join-Path $root "dist\$name.zip"
if (Test-Path $zip) { Remove-Item $zip }
$archive = [IO.Compression.ZipFile]::Open($zip, 'Create')
try {
    Get-ChildItem $stage -Recurse -File | ForEach-Object {
        $entry = $_.FullName.Substring($stage.Length + 1).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $_.FullName, $entry, 'Optimal') | Out-Null
    }
} finally {
    $archive.Dispose()
}
Write-Host "Created $zip"
