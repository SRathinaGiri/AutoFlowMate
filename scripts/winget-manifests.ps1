# Writes the winget manifests for a published release into .\dist\winget\<version>\, ready for a
# pull request to https://github.com/microsoft/winget-pkgs (folder
# manifests\s\SRathinaGiri\AutoFlowMate\<version>\), and validates them with `winget validate`.
#   powershell -ExecutionPolicy Bypass -File scripts\winget-manifests.ps1            (version from Cargo.toml)
#   powershell -ExecutionPolicy Bypass -File scripts\winget-manifests.ps1 -Version 0.4.2
# The release zip must already be on GitHub: its SHA-256 is taken from the downloaded file.
param([string]$Version)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue' # the progress bar slows Invoke-WebRequest down a lot
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not $Version) {
    $Version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
}

$id = 'SRathinaGiri.AutoFlowMate'
$repo = 'https://github.com/SRathinaGiri/AutoFlowMate'
$url = "$repo/releases/download/v$Version/AutoFlowMate-v$Version-windows-x64.zip"
$out = Join-Path $root "dist\winget\$Version"
New-Item -ItemType Directory -Force $out | Out-Null

$zip = Join-Path $env:TEMP "AutoFlowMate-v$Version-windows-x64.zip"
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
$sha = (Get-FileHash $zip -Algorithm SHA256).Hash
Remove-Item $zip

$utf8 = New-Object Text.UTF8Encoding $false
function Write-Manifest($name, $text) {
    [IO.File]::WriteAllText((Join-Path $out $name), $text.Replace("`r`n", "`n"), $utf8)
}

Write-Manifest "$id.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.28.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.28.0
"@

Write-Manifest "$id.installer.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.28.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
MinimumOSVersion: 10.0.17763.0
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: autoflow.exe
  PortableCommandAlias: autoflow
- RelativeFilePath: autoflow-studio.exe
  PortableCommandAlias: autoflow-studio
Installers:
- Architecture: x64
  InstallerUrl: $url
  InstallerSha256: $sha
ManifestType: installer
ManifestVersion: 1.28.0
"@

Write-Manifest "$id.locale.en-US.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.28.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
PackageLocale: en-US
Publisher: SRathinaGiri
PublisherUrl: https://github.com/SRathinaGiri
PublisherSupportUrl: $repo/issues
PrivacyUrl: $repo/blob/main/PRIVACY.md
PackageName: AutoFlowMate
PackageUrl: $repo
License: MIT
LicenseUrl: $repo/blob/main/LICENSE
ShortDescription: Free, open-source Windows automation (RPA) with a visual flowchart editor, a recorder and plain JSON flows.
Description: |-
  AutoFlowMate automates repetitive work on a Windows PC: files and folders, Excel, CSV, PDF and Word,
  e-mail, web pages, any Windows program (UI Automation, keyboard, mouse, images, OCR), databases and
  AI models. Build flows in AutoFlowMate Studio's flowchart editor or record them like a macro; run them
  on demand, on a schedule or when a file or e-mail arrives. Runs locally with no account or cloud;
  includes offline redaction of personal data and reversible pseudonymisation (beta).
  Commands: autoflow-studio (visual editor), autoflow (command line).
Moniker: autoflowmate
Tags:
- automation
- rpa
- workflow
- macro
- ocr
- excel
- pdf
- tally
- redaction
ReleaseNotesUrl: $repo/releases/tag/v$Version
ManifestType: defaultLocale
ManifestVersion: 1.28.0
"@

Write-Host "Manifests in $out (SHA256 $sha)"
winget validate --manifest $out
if ($LASTEXITCODE -ne 0) { throw "winget validate failed" }
