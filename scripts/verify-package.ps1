<#
.SYNOPSIS
    verify-package.ps1 (Task 22) - Windows twin of scripts/verify-package.sh.

.DESCRIPTION
    Verifies a CC Reminder Windows release package BEFORE it is published.
    Mirrors verify-package.sh assertion-for-assertion (keep both readable
    side-by-side):

      1. the final desktop and helper binaries exist,
      2. the helper bytes hash-match the packaged helper-manifest.json entry,
         and the manifest carries no unreplaced placeholder,
      3. the release helper (and desktop binary) contain no
         CC_REMINDER_TEST_DATA_DIR literal / test-support path,
      4. no plaintext test marker or concrete credential query value occurs,
      5. no forbidden bypass argument occurs,
      6. every -PublishedFile has a valid sibling ".sha256" checksum,
      7. on a Windows host: the Authenticode signature status of the desktop
         binary (and -Installer, when given) must be Valid. This fails LOUDLY
         on unsigned artifacts; it is skipped ONLY on non-Windows hosts
         (pwsh on macOS/Linux). (macOS codesign/notarization parity lives in
         verify-package.sh.)

    The scripts take explicit artifact arguments and never delete anything
    outside their own temporary directory.

.PARAMETER DesktopBinary
    Path to the final desktop executable (cc-reminder.exe).
.PARAMETER HelperBinary
    Path to the final helper executable (cc-reminder-hook.exe).
.PARAMETER Manifest
    Path to helper-manifest.json packaged next to the binaries.
.PARAMETER Archive
    Optional artifact to unpack first into the script's own temp directory
    before resolving the three required paths inside it. Supported:
    .msi (administrative install via msiexec /a), .zip.
.PARAMETER Installer
    Optional installer artifact (.msi/.exe) whose Authenticode status is also
    asserted on a Windows host.
.PARAMETER PublishedFile
    Repeatable list of published artifacts that must each have a valid sibling
    ".sha256" checksum.

.EXAMPLE
    ./scripts/verify-package.ps1 -DesktopBinary .\target\release\cc-reminder.exe `
        -HelperBinary .\target\release\cc-reminder-hook.exe `
        -Manifest .\resources\helper-manifest.json
#>
param(
    [Parameter(Mandatory = $true)] [string] $DesktopBinary,
    [Parameter(Mandatory = $true)] [string] $HelperBinary,
    [Parameter(Mandatory = $true)] [string] $Manifest,
    [string] $Archive = "",
    [string] $Installer = "",
    [string[]] $PublishedFile = @()
)

$ErrorActionPreference = "Stop"

function Fail([string] $message) {
    Write-Error "FAIL(verify-package.ps1): $message"
    exit 1
}

function FirstToken([string] $text) {
    return ($text.Trim() -split "\s+")[0]
}

$WorkDir = Join-Path ([IO.Path]::GetTempPath()) ("cc-reminder-verify-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $WorkDir | Out-Null
try {

    Write-Host "verify-package: working in $WorkDir"

    # -------------------------------------------------------------------------
    # Optional unpack phase. Only ever writes INSIDE $WorkDir.
    # -------------------------------------------------------------------------
    if ($Archive -ne "") {
        if (-not (Test-Path -LiteralPath $Archive -PathType Leaf)) {
            Fail "-Archive '$Archive' does not exist"
        }
        switch -Regex ($Archive) {
            "\.msi$" {
                $target = Join-Path $WorkDir "msi-extract"
                $p = Start-Process -FilePath "msiexec.exe" `
                    -ArgumentList "/a", "`"$Archive`"", "/qn", "TARGETDIR=`"$target`"" `
                    -Wait -PassThru
                if ($p.ExitCode -ne 0) { Fail "msiexec administrative install failed for $Archive" }
            }
            "\.zip$" {
                Expand-Archive -LiteralPath $Archive -DestinationPath $WorkDir
            }
            default {
                Fail "unsupported archive type: $Archive (supported: .msi, .zip)"
            }
        }
        $found = Get-ChildItem -Path $WorkDir -Recurse -File -Filter "cc-reminder.exe" |
            Select-Object -First 1
        if (-not $found) { Fail "could not locate 'cc-reminder.exe' inside $Archive" }
        $DesktopBinary = $found.FullName
        $found = Get-ChildItem -Path $WorkDir -Recurse -File -Filter "cc-reminder-hook.exe" |
            Select-Object -First 1
        if (-not $found) { Fail "could not locate 'cc-reminder-hook.exe' inside $Archive" }
        $HelperBinary = $found.FullName
        $found = Get-ChildItem -Path $WorkDir -Recurse -File -Filter "helper-manifest.json" |
            Select-Object -First 1
        if (-not $found) { Fail "could not locate 'helper-manifest.json' inside $Archive" }
        $Manifest = $found.FullName
    }

    foreach ($required in @($DesktopBinary, $HelperBinary, $Manifest)) {
        if ([string]::IsNullOrWhiteSpace($required)) {
            Fail "-DesktopBinary, -HelperBinary and -Manifest are required"
        }
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
            Fail "required artifact '$required' missing"
        }
        if ((Get-Item -LiteralPath $required).Length -le 0) {
            Fail "required artifact '$required' is empty"
        }
    }

    Write-Host ("  desktop binary : {0} ({1} bytes)" -f $DesktopBinary, (Get-Item -LiteralPath $DesktopBinary).Length)
    Write-Host ("  helper binary  : {0} ({1} bytes)" -f $HelperBinary, (Get-Item -LiteralPath $HelperBinary).Length)
    Write-Host ("  manifest       : {0}" -f $Manifest)

    # -------------------------------------------------------------------------
    # Manifest integrity: parse JSON, select the entry for the shipped helper,
    # reject placeholders, then length + SHA-256 match (same contract as
    # installer::helper at runtime).
    # -------------------------------------------------------------------------
    try {
        $doc = Get-Content -LiteralPath $Manifest -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        Fail "manifest is not valid JSON: $_"
    }
    $entries = @($doc.helpers | Where-Object { $_.filename -eq (Split-Path $HelperBinary -Leaf) })
    if ($entries.Count -ne 1) {
        Fail ("expected exactly one manifest entry for filename '{0}', found {1}" -f (Split-Path $HelperBinary -Leaf), $entries.Count)
    }
    $entry = $entries[0]
    if ($entry.sha256 -notmatch "^[0-9a-fA-F]{64}$") {
        Fail ("manifest entry for '{0}' still carries an unreplaced placeholder sha256" -f $entry.filename)
    }
    if (-not $entry.length -or [int64]$entry.length -le 0) {
        Fail ("manifest entry for '{0}' carries an unreplaced placeholder length" -f $entry.filename)
    }
    $helperBytes = [IO.File]::ReadAllBytes($HelperBinary)
    if ([int64]$helperBytes.Length -ne [int64]$entry.length) {
        Fail ("helper length mismatch: manifest={0} actual={1}" -f $entry.length, $helperBytes.Length)
    }
    $sha256Provider = [System.Security.Cryptography.SHA256]::Create()
    try {
        $actualDigest = ([BitConverter]::ToString($sha256Provider.ComputeHash($helperBytes)) -replace "-", "").ToLowerInvariant()
    } finally {
        $sha256Provider.Dispose()
    }
    if ($actualDigest -ne $entry.sha256.ToLowerInvariant()) {
        Fail ("helper sha-256 mismatch: manifest={0} actual={1}" -f $entry.sha256, $actualDigest)
    }
    Write-Host "  manifest hash  : matches helper bytes"

    # -------------------------------------------------------------------------
    # Payload scans. Rules mirror check-sensitive-artifacts.sh; every match
    # fails with file + rule name and never prints the matched value.
    # -------------------------------------------------------------------------
    $scanTargets = @($DesktopBinary, $HelperBinary, $Manifest)
    $ruleNames = @(
        "test-support-env-literal",
        "plaintext-test-marker",
        "concrete-credential-query-value",
        "executable-bypass-argument"
    )
    $rulePatterns = @(
        "CC_REMINDER_TEST_DATA_DIR",
        "cc-reminder-e2e|secret-raw-value|VITE_CC_REMINDER_TEST_BACKEND",
        "[?&](access_token|sign|secret|accessToken|apiKey|apikey|key)=[A-Za-z0-9+/_-]{24,}",
        "--[A-Za-z0-9][A-Za-z0-9_-]*(bypass|skip[_-]verif|insecure|unsafe|trust[_-]all)[A-Za-z0-9_-]*"
    )

    # ISO-8859-1 maps bytes 1:1 so byte-oriented scanning is safe.
    $latin1 = [System.Text.Encoding]::GetEncoding(28591)
    $payloadFailures = 0
    foreach ($target in $scanTargets) {
        $text = $latin1.GetString([IO.File]::ReadAllBytes($target))
        for ($i = 0; $i -lt $ruleNames.Count; $i++) {
            $match = [Regex]::Match($text, $rulePatterns[$i])
            if ($match.Success) {
                Write-Host "FORBIDDEN CONTENT IN PACKAGE"
                Write-Host ("  rule: {0}" -f $ruleNames[$i])
                Write-Host ("  file: {0}" -f $target)
                Write-Host "  (matched value intentionally not printed)"
                $payloadFailures++
            }
        }
    }
    if ($payloadFailures -gt 0) {
        Fail "$payloadFailures forbidden-content finding(s)"
    }
    Write-Host "  payload scans  : clean (markers / credentials / bypass flags)"

    # -------------------------------------------------------------------------
    # Published-file checksums.
    # -------------------------------------------------------------------------
    $checksumFindings = 0
    foreach ($file in $PublishedFile) {
        $sidecar = "$file.sha256"
        if (-not (Test-Path -LiteralPath $sidecar -PathType Leaf)) {
            Write-Host "MISSING CHECKSUM: $sidecar (required for every published file)"
            $checksumFindings++
            continue
        }
        $expected = FirstToken (Get-Content -LiteralPath $sidecar -TotalCount 1)
        $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected.ToLowerInvariant() -ne $actual) {
            Write-Host "CHECKSUM MISMATCH: $(Split-Path $file -Leaf) (recorded digest does not match artifact)"
            $checksumFindings++
        }
    }
    if ($checksumFindings -gt 0) {
        Fail "$checksumFindings checksum finding(s)"
    }
    if ($PublishedFile.Count -gt 0) {
        Write-Host ("  checksums      : {0} published file(s) verified" -f $PublishedFile.Count)
    }

    # -------------------------------------------------------------------------
    # Windows Authenticode gate. Runs ONLY on a Windows host; fails LOUDLY
    # there for unsigned or untrusted artifacts. Non-Windows hosts skip with a
    # note (PowerShell 5.1 has no $IsWindows; absence means Windows).
    # ---------------------------------------------------------------------------
    $onWindows = (-not $PSVersionTable.ContainsKey("Platform")) -or ($PSVersionTable.Platform -eq "Win32NT")
    if ($onWindows) {
        foreach ($signedArtifact in (@($DesktopBinary, $Installer) | Where-Object { $_ -ne "" })) {
            $signature = Get-AuthenticodeSignature -FilePath $signedArtifact
            if ($signature.Status -ne "Valid") {
                Fail ("Authenticode status of '{0}' is '{1}' (must be Valid; detail: {2})" -f `
                    $signedArtifact, $signature.Status, $signature.StatusMessage)
            }
            Write-Host ("  authenticode   : {0} is Valid" -f (Split-Path $signedArtifact -Leaf))
        }
    } else {
        Write-Host "  authenticode   : skipped (not a Windows host)"
    }

    Write-Host "OK: package verification passed"

} finally {
    # Only ever remove OUR OWN temporary directory, never caller paths.
    if (Test-Path -LiteralPath $WorkDir) {
        Remove-Item -LiteralPath $WorkDir -Recurse -Force
    }
}
