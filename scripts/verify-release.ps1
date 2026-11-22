# Checks that every version number agrees before testing or releasing:
# Cargo.toml, the built programs (target/release), the plugin manifests, the zip in dist/ and the
# git tag. Run it after `cargo build --release` and `scripts/package.ps1`:
#   ./scripts/verify-release.ps1
# Exits with 1 and lists the mismatches when anything disagrees.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$v = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
$tag = "v$v"
$bad = [System.Collections.Generic.List[string]]::new()
function Test-Same($what, $found) {
    if ($found -eq $v) { Write-Host "  ok   $what $found" }
    else { Write-Host "  BAD  $what $found (expected $v)" -ForegroundColor Red; $bad.Add($what) }
}

Write-Host "Cargo.toml version $v"
Test-Same 'gemini-extension.json' (Get-Content gemini-extension.json -Raw | ConvertFrom-Json).version
Test-Same 'Claude plugin.json' (Get-Content skills/autoflowmate/.claude-plugin/plugin.json -Raw | ConvertFrom-Json).version
Test-Same 'Antigravity plugin.json' (Get-Content .antigravity-plugin/plugin.json -Raw | ConvertFrom-Json).version

$cli = 'target/release/autoflow.exe'
if (Test-Path $cli) {
    $out = & $cli --version
    Test-Same 'autoflow --version' (($out -split '\s+')[-1])
    Test-Same 'autoflow.exe file version' ((Get-Item $cli).VersionInfo.ProductVersion)
} else { Write-Host "  BAD  $cli missing (cargo build --release)" -ForegroundColor Red; $bad.Add('cli') }

$studio = 'target/release/autoflow-studio.exe'
if (Test-Path $studio) { Test-Same 'autoflow-studio.exe file version' ((Get-Item $studio).VersionInfo.ProductVersion) }
else { Write-Host "  BAD  $studio missing (cargo build --release)" -ForegroundColor Red; $bad.Add('studio') }

if (Test-Path dist) {
    $zips = @(Get-ChildItem dist -Filter 'AutoFlowMate-v*-windows-x64.zip')
    foreach ($z in $zips) {
        if ($z.Name -eq "AutoFlowMate-$tag-windows-x64.zip") { Write-Host "  ok   dist $($z.Name)" }
        else { Write-Host "  --   dist $($z.Name) is an older package (not this version)" }
    }
    $pz = "dist/autoflowmate-plugin-$tag.zip"
    if (Test-Path $pz) {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $pz))
        try {
            $e = $zip.Entries | Where-Object FullName -eq 'plugin.json'
            $r = [IO.StreamReader]::new($e.Open()); $j = $r.ReadToEnd(); $r.Close()
            Test-Same 'plugin zip plugin.json' ($j | ConvertFrom-Json).version
        } finally { $zip.Dispose() }
    }
}

$head = git rev-parse HEAD
$tagged = git rev-parse -q --verify "refs/tags/$tag^{commit}" 2>$null
if ($tagged) {
    if ($tagged -eq $head) { Write-Host "  ok   tag $tag is this commit" }
    else { Write-Host "  --   tag $tag is already released (commit $($tagged.Substring(0,7))): bump the version before the next release" -ForegroundColor Yellow }
} else { Write-Host "  --   tag $tag not created yet" }

if ($bad.Count) { Write-Host "$($bad.Count) mismatch(es)" -ForegroundColor Red; exit 1 }
Write-Host "All versions agree: $v" -ForegroundColor Green
