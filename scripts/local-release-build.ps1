# Build a LOCAL UNSIGNED Windows release bundle with a real, hash-verified
# helper. Windows twin of scripts/local-release-build.sh.
#
# Why this exists: the committed resources/bin + helper-manifest.json are
# PLACEHOLDERS by design (anti-forgery: dev builds must never install
# unsigned bytes). A locally usable bundle must stage the real host-target
# helper into the tracked resource paths for the duration of `tauri build`,
# then restore the placeholders so nothing real ever gets committed.
#
# Manifest alignment (Windows nuance, differs from macOS b399ccb): Tauri
# bundles EVERY cargo bin in the install root AND the explicitly listed
# resources/bin copy, and every link stamps a fresh PE timestamp — so the
# in-tree rebuild bytes NEVER match the staged bytes and a re-package can
# never converge. The runtime contract (installer::helper BIN_RESOURCE_DIR)
# reads ONLY <install>\resources\bin\cc-reminder-hook.exe, i.e. the STAGED
# bytes, so the manifest records the staged bytes and verify-package.ps1
# -Archive proves the bytes INSIDE the .msi match it end-to-end.
#
# Usage:  powershell -File scripts/local-release-build.ps1   (from repo root)
# Needs:  cargo (MSVC), pnpm, git on PATH; VS Build Tools + Windows SDK.
# Output: release-v<version>\ with the NSIS setup.exe + .msi + .sha256 sides.
# The Authenticode gate is skipped (-SkipAuthenticode): local builds carry no
# certificate. CI (release.yml) signs everything and never skips it.

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath (Join-Path $PSScriptRoot "..")

function Fail([string] $message) {
    Write-Host "FAIL(local-release-build): $message" -ForegroundColor Red
    exit 1
}

# --- 0. Preflight ------------------------------------------------------------
foreach ($tool in @("cargo", "pnpm", "git")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Fail "$tool not found on PATH (see the script header for prerequisites)"
    }
}

$conf = Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
$stageDir = "release-v$version"
Write-Host "[0/6] preflight ok — packaging CC Reminder $version (local UNSIGNED build)"

# Everything below runs inside try/finally so Ctrl-C or a failure still
# restores the tracked placeholder bytes (local-release-build.sh trap restore).
$bin = "src-tauri\resources\bin\cc-reminder-hook.exe"
$manifest = "src-tauri\resources\helper-manifest.json"
$sha256Provider = [System.Security.Cryptography.SHA256]::Create()
function Get-Sha256([string] $path) {
    # Provider lives for the whole script; disposed
    # in the outer finally.
    ([BitConverter]::ToString(
        $sha256Provider.ComputeHash([IO.File]::ReadAllBytes($path))) -replace "-", "").ToLowerInvariant()
}
# Write UTF-8 WITHOUT a BOM: PS 5.1's Set-Content -Encoding UTF8 always emits
# one, and the runtime manifest parser (serde_json) rejects BOM-prefixed JSON
# as malformed — which bricks hook installation in the packaged app.
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
function Write-JsonNoBom([string] $path, $value) {
    # [IO.File] uses the PROCESS working directory, which does not follow
    # PowerShell's Set-Location — resolve against the provider location.
    $full = if ([IO.Path]::IsPathRooted($path)) { $path } else {
        Join-Path (Get-Location -PSProvider FileSystem).ProviderPath $path
    }
    [IO.File]::WriteAllText($full, ($value | ConvertTo-Json -Depth 5) + "`n", $utf8NoBom)
}
try {

    # --- 1. Build the release helper ----------------------------------------
    Write-Host "[1/6] building release helper"
    cargo build --manifest-path src-tauri/Cargo.toml --release --bin cc-reminder-hook
    if ($LASTEXITCODE -ne 0) { Fail "cargo build of the helper failed" }

    # --- 2. Stage helper + real manifest (x86_64-pc-windows-msvc) ------------
    Write-Host "[2/6] staging helper + real manifest"
    Copy-Item "src-tauri\target\release\cc-reminder-hook.exe" $bin -Force
    $stagedSha = Get-Sha256 $bin
    $stagedLength = (Get-Item $bin).Length
    $manifestDoc = [ordered]@{
        "`$schema"    = "https://json-schema.org/draft/2020-12/schema"
        title         = "CC Reminder signed helper manifest"
        description   = "LOCAL UNSIGNED BUILD staging. NOT FOR COMMIT — release packaging regenerates this from signed bytes."
        helpers       = @(
            [ordered]@{
                target_triple  = "x86_64-pc-windows-msvc"
                helper_version = $version
                filename       = "cc-reminder-hook.exe"
                length         = $stagedLength
                sha256         = $stagedSha
            }
        )
    }
    $manifestDoc | ConvertTo-Json -Depth 5 > $null  # fail fast if not serializable
    Write-JsonNoBom $manifest $manifestDoc

    # Tauri reads the resource map through a merge-config FILE (not inline
    # JSON) to stay out of shell-quoting hell; updater artifacts stay off —
    # no minisign key exists for a local build.
    $buildConfig = Join-Path $env:TEMP ("cc-reminder-local-build-" + [Guid]::NewGuid().ToString("N") + ".json")
    Write-JsonNoBom $buildConfig ([ordered]@{
        bundle = [ordered]@{
            createUpdaterArtifacts = $false
            resources              = @(
                "resources/capabilities/claude-code-2.1.218.json",
                "resources/capabilities/codex-0.145.0.json",
                "resources/helper-manifest.json",
                "resources/bin/cc-reminder-hook.exe"
            )
        }
    })

    # --- 3. Build the installers (NSIS + MSI) --------------------------------
    # tauri build re-runs cargo across ALL bins, so the in-tree rebuild's
    # helper bytes (install-root copy) will NOT match the staged bytes — that
    # is expected and irrelevant: the runtime reads only the resources/bin
    # copy, which tauri-build copies from the staged files above.
    Write-Host "[3/6] building the app installers (updater artifacts off: no signing key locally)"
    pnpm tauri build --config $buildConfig
    if ($LASTEXITCODE -ne 0) { Fail "tauri build failed" }

    # --- 4. verify-package gate (structure, hashes, scans; no Authenticode) --
    # -Archive unpacks the .msi via msiexec /a into the script's own temp dir
    # and validates the helper bytes INSIDE the installer (the resources\bin
    # copy the runtime reads) against the manifest — the same end-to-end check
    # the release pipeline runs.
    $nsis = Get-ChildItem "src-tauri\target\release\bundle\nsis" -Filter "*-setup.exe" |
        Select-Object -First 1
    if (-not $nsis) { Fail "no NSIS installer produced" }
    $msi = Get-ChildItem "src-tauri\target\release\bundle\msi" -Filter "*.msi" |
        Select-Object -First 1
    if (-not $msi) { Fail "no MSI produced" }
    Write-Host "[4/6] verify-package gate (MSI byte-level + payload scans; Authenticode skipped: unsigned local build)"
    powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\verify-package.ps1" `
        -Archive $msi.FullName `
        -DesktopBinary "src-tauri\target\release\cc-reminder.exe" `
        -HelperBinary $bin `
        -Manifest $manifest `
        -SkipAuthenticode
    if ($LASTEXITCODE -ne 0) { Fail "verify-package.ps1 rejected the package" }

    # --- 6. Checksums ---------------------------------------------------------
    # Sidecars record "<sha256>  <basename>" so they stay valid wherever the
    # artifacts are later staged/published (standard sha256sum -c layout).
    Write-Host "[5/6] generating .sha256 sidecars"
    foreach ($artifact in @($nsis.FullName, $msi.FullName)) {
        $hash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
        Set-Content -LiteralPath "$artifact.sha256" -Value "$hash  $(Split-Path $artifact -Leaf)"
    }

    # --- 6. Stage into the gitignored release directory ----------------------
    # CI's publish job flattens every artifact beside latest.json; the local
    # staging dir mirrors that final layout for a manual upload.
    Write-Host "[6/6] staging artifacts into $stageDir\"
    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
    foreach ($file in @($nsis.FullName, $msi.FullName)) {
        foreach ($suffix in @("", ".sha256")) {
            Copy-Item "$file$suffix" (Join-Path $stageDir ((Split-Path $file -Leaf) + $suffix)) -Force
        }
    }

    Write-Host ""
    Write-Host "done — unsigned installers staged in $stageDir\ :"
    Get-ChildItem $stageDir | ForEach-Object { Write-Host ("  {0}  ({1:N0} bytes)" -f $_.Name, $_.Length) }
    Write-Host "tracked placeholders restored automatically."

} finally {
    if (Test-Path $buildConfig) { Remove-Item -LiteralPath $buildConfig -Force }
    if ($sha256Provider) { $sha256Provider.Dispose() }
    git checkout -- $bin $manifest 2>$null
}
