#!/usr/bin/env bash
#
# verify-package.sh (Task 22)
#
# Verifies a CC Reminder release package BEFORE it is published. Unpacks the
# artifact (when an archive is given) into its own temporary directory and
# asserts, per design 8/9 and the Task 22 brief:
#   1. the final desktop and helper binaries exist,
#   2. the helper bytes hash-match the packaged helper-manifest.json entry,
#      and the manifest carries no unreplaced placeholder,
#   3. the release helper (and desktop binary) contain no
#      `CC_REMINDER_TEST_DATA_DIR` literal / test-support path,
#   4. no plaintext test marker or concrete credential query value occurs,
#   5. no forbidden bypass argument occurs,
#   6. every --published-file has a valid sibling `.sha256` checksum,
#   7. on a macOS host: the app bundle passes strict codesign verification and
#      Gatekeeper/notarization assessment (`spctl`). These FAIL LOUDLY on
#      unsigned or ad-hoc artifacts; they are skipped ONLY on non-macOS hosts.
#      (Windows Authenticode parity lives in verify-package.ps1.)
#
# The scripts take explicit artifact arguments and never delete anything
# outside their own temporary directory.
#
# Usage:
#   verify-package.sh --desktop-binary P --helper-binary P --manifest P
#                     [--archive A] [--macos-app-bundle B]
#                     [--published-file F]...
#
#   --archive A          unpack A first and resolve the three required paths
#                        inside it (.app.tar.gz/.tgz, .AppImage, .deb)
#   --macos-app-bundle B .app bundle for the codesign/notarization gate

set -u

PROGRAM=${0##*/}

fail() {
    echo "FAIL($PROGRAM): $*" >&2
    exit 1
}

usage() {
    sed -n '2,40p' "$0" | grep -E '^#' | sed 's/^# \{0,1\}//'
    exit 64
}

DESKTOP_BINARY=""
HELPER_BINARY=""
MANIFEST=""
ARCHIVE=""
MACOS_BUNDLE=""
PUBLISHED_FILES=()

while [ $# -gt 0 ]; do
    case "$1" in
        --desktop-binary) DESKTOP_BINARY=$2; shift 2 ;;
        --helper-binary) HELPER_BINARY=$2; shift 2 ;;
        --manifest) MANIFEST=$2; shift 2 ;;
        --archive) ARCHIVE=$2; shift 2 ;;
        --macos-app-bundle) MACOS_BUNDLE=$2; shift 2 ;;
        --published-file) PUBLISHED_FILES+=("$2"); shift 2 ;;
        *) usage ;;
    esac
done

WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/cc-reminder-verify.XXXXXX") || fail "cannot create temp dir"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

echo "verify-package: working in $WORKDIR"

# ---------------------------------------------------------------------------
# Optional unpack phase. Only ever writes INSIDE $WORKDIR.
# ---------------------------------------------------------------------------
if [ -n "$ARCHIVE" ]; then
    [ -f "$ARCHIVE" ] || fail "--archive '$ARCHIVE' does not exist"
    case "$ARCHIVE" in
        *.tar.gz|*.tgz)
            tar -xzf "$ARCHIVE" -C "$WORKDIR" || fail "cannot untar $ARCHIVE"
            ;;
        *.AppImage)
            chmod +x "$ARCHIVE"
            (cd "$WORKDIR" && "$ARCHIVE" --appimage-extract >/dev/null 2>&1) \
                || fail "AppImage extraction failed for $ARCHIVE"
            ;;
        *.deb)
            ar p "$ARCHIVE" data.tar.xz 2>/dev/null | tar -xJ -C "$WORKDIR" \
                || ar p "$ARCHIVE" data.tar.zst 2>/dev/null | tar --use-compress-program=unzstd -xf - -C "$WORKDIR" \
                || fail "cannot extract data.tar from $ARCHIVE"
            ;;
        *)
            fail "unsupported archive type: $ARCHIVE (supported: .app.tar.gz, .AppImage, .deb)"
            ;;
    esac

    locate() {
        local name=$1
        local hit
        hit=$(find "$WORKDIR" -type f \( -name "$name" \) | head -n 1)
        [ -n "$hit" ] || fail "could not locate '$name' inside $ARCHIVE"
        printf '%s' "$hit"
    }
    DESKTOP_BINARY=$(locate 'cc-reminder')
    # Prefer the exact name; fall back to the .exe-suffixed name.
    HELPER_BINARY=$(find "$WORKDIR" -type f -name 'cc-reminder-hook' | head -n 1)
    [ -n "$HELPER_BINARY" ] || HELPER_BINARY=$(find "$WORKDIR" -type f -name 'cc-reminder-hook.exe' | head -n 1)
    [ -n "$HELPER_BINARY" ] || fail "could not locate 'cc-reminder-hook[.exe]' inside $ARCHIVE"
    MANIFEST=$(find "$WORKDIR" -type f -name 'helper-manifest.json' | head -n 1)
    [ -n "$MANIFEST" ] || fail "could not locate 'helper-manifest.json' inside $ARCHIVE"
fi

for required in "$DESKTOP_BINARY" "$HELPER_BINARY" "$MANIFEST"; do
    [ -n "$required" ] || usage
done
[ -f "$DESKTOP_BINARY" ] || fail "desktop binary '$DESKTOP_BINARY' missing"
[ -f "$HELPER_BINARY" ] || fail "helper binary '$HELPER_BINARY' missing"
[ -f "$MANIFEST" ] || fail "helper manifest '$MANIFEST' missing"
[ -s "$DESKTOP_BINARY" ] || fail "desktop binary '$DESKTOP_BINARY' is empty"
[ -s "$HELPER_BINARY" ] || fail "helper binary '$HELPER_BINARY' is empty"

echo "  desktop binary : $DESKTOP_BINARY ($(wc -c <"$DESKTOP_BINARY" | tr -d ' ') bytes)"
echo "  helper binary  : $HELPER_BINARY ($(wc -c <"$HELPER_BINARY" | tr -d ' ') bytes)"
echo "  manifest       : $MANIFEST"

# ---------------------------------------------------------------------------
# Manifest integrity: parse JSON, select the entry for the shipped helper,
# reject placeholders, then length + SHA-256 match (same contract as
# installer::helper at runtime).
# ---------------------------------------------------------------------------
MANIFEST_CHECK=$(python3 - "$MANIFEST" "$HELPER_BINARY" 2>&1 <<'PYEOF'
import hashlib, json, os, sys

manifest_path, helper_path = sys.argv[1], sys.argv[2]
try:
    with open(manifest_path, encoding="utf-8") as fh:
        doc = json.load(fh)
except Exception as error:  # noqa: BLE001
    print(f"manifest is not valid JSON: {error}")
    sys.exit(1)

entries = doc.get("helpers") if isinstance(doc, dict) else None
if not isinstance(entries, list) or not entries:
    print("manifest has no helpers[] array")
    sys.exit(1)

filename = os.path.basename(helper_path)
matching = [entry for entry in entries if isinstance(entry, dict) and entry.get("filename") == filename]
if not matching:
    print(f"no manifest entry for filename '{filename}'")
    sys.exit(1)

data = open(helper_path, "rb").read()
actual_length = len(data)
actual_digest = hashlib.sha256(data).hexdigest()

# A universal/fat helper is legitimately described by SEVERAL entries (one
# per slice triple). EVERY entry for this filename must carry a real
# (non-placeholder) length + sha256 describing EXACTLY these bytes.
for index, entry in enumerate(matching):
    triple = entry.get("target_triple")
    digest = entry.get("sha256", "")
    length = entry.get("length", 0)
    if not isinstance(digest, str) or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest.lower()):
        print(f"manifest entry {index} ({triple}) still carries an unreplaced placeholder sha256")
        sys.exit(1)
    if not isinstance(length, int) or length <= 0:
        print(f"manifest entry {index} ({triple}) carries an unreplaced placeholder length")
        sys.exit(1)
    if actual_length != length:
        print(f"helper length mismatch for entry {index} ({triple}): manifest={length} actual={actual_length}")
        sys.exit(1)
    if actual_digest != digest.lower():
        print(f"helper sha-256 mismatch for entry {index} ({triple}): manifest={digest} actual={actual_digest}")
        sys.exit(1)
print("ok")
PYEOF
)
if [ "$MANIFEST_CHECK" != "ok" ]; then
    fail "helper manifest verification failed: ${MANIFEST_CHECK:-python3 produced no diagnostic}"
fi
echo "  manifest hash  : matches helper bytes"

# ---------------------------------------------------------------------------
# Payload scans. Rules mirror check-sensitive-artifacts.sh; every match fails
# with file + rule name and never prints the matched value.
# ---------------------------------------------------------------------------
SCAN_TARGETS=("$DESKTOP_BINARY" "$HELPER_BINARY")
if [ -n "$ARCHIVE" ]; then
    SCAN_TARGETS+=("$WORKDIR")
else
    SCAN_TARGETS+=("$MANIFEST")
fi

declare -a RULE_NAMES=(test-support-env-literal plaintext-test-marker concrete-credential-query-value executable-bypass-argument)
declare -a RULE_PATTERNS=(
    'CC_REMINDER_TEST_DATA_DIR'
    'cc-reminder-e2e|secret-raw-value|VITE_CC_REMINDER_TEST_BACKEND'
    '[?&](access_token|sign|secret|accessToken|apiKey|apikey|key)=[A-Za-z0-9+/_-]{24,}'
    '--[A-Za-z0-9][A-Za-z0-9_-]*(bypass|skip[_-]verif|insecure|unsafe|trust[_-]all)[A-Za-z0-9_-]*'
)

payload_failures=0
for index in "${!RULE_NAMES[@]}"; do
    name=${RULE_NAMES[$index]}
    pattern=${RULE_PATTERNS[$index]}
    # sort -u dedupes when a file matches both as an explicit target and via
    # its parent directory in the same scan.
    matches=$({ grep -RalaE -- "${pattern}" "${SCAN_TARGETS[@]}" 2>/dev/null || true; } | sort -u)
    if [ -n "$matches" ]; then
        while IFS= read -r file; do
            echo "FORBIDDEN CONTENT IN PACKAGE"
            echo "  rule: $name"
            echo "  file: $file"
            echo "  (matched value intentionally not printed)"
            payload_failures=$((payload_failures + 1))
        done <<< "$matches"
    fi
done
[ "$payload_failures" -eq 0 ] || fail "$payload_failures forbidden-content finding(s)"
echo "  payload scans  : clean (markers / credentials / bypass flags)"

# ---------------------------------------------------------------------------
# Published-file checksums (Linux release gate; usable anywhere).
# ---------------------------------------------------------------------------
checksum_missing=0
for file in ${PUBLISHED_FILES[@]+"${PUBLISHED_FILES[@]}"}; do
    sidecar="$file.sha256"
    if [ ! -f "$sidecar" ]; then
        echo "MISSING CHECKSUM: $sidecar (required for every published file)"
        checksum_missing=$((checksum_missing + 1))
        continue
    fi
    expected=$(cut -d' ' -f1 "$sidecar" | head -n 1)
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$file" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$file" | cut -d' ' -f1)
    fi
    if [ "$expected" != "$actual" ]; then
        echo "CHECKSUM MISMATCH: $(basename "$file") (recorded digest does not match artifact)"
        checksum_missing=$((checksum_missing + 1))
    fi
done
[ "$checksum_missing" -eq 0 ] || fail "$checksum_missing checksum finding(s)"
if [ ${#PUBLISHED_FILES[@]} -gt 0 ]; then
    echo "  checksums      : ${#PUBLISHED_FILES[@]} published file(s) verified"
fi

# ---------------------------------------------------------------------------
# macOS code-signing + notarization gate. Runs ONLY on a macOS host; fails
# LOUDLY there for unsigned/ad-hoc artifacts. Non-macOS hosts skip with a note.
# ---------------------------------------------------------------------------
HOST_OS=$(uname -s)
if [ -n "$MACOS_BUNDLE" ]; then
    [ -d "$MACOS_BUNDLE" ] || fail "--macos-app-bundle '$MACOS_BUNDLE' is not a directory"
    if [ "$HOST_OS" = "Darwin" ]; then
        if codesign --verify --deep --strict "$MACOS_BUNDLE" 2>"$WORKDIR/codesign.err"; then
            echo "  codesign       : strict verification passed"
        else
            echo "FAIL: app bundle did not pass strict codesign verification:" >&2
            sed 's/^/    /' "$WORKDIR/codesign.err" >&2
            echo "    (unsigned or ad-hoc artifacts must never reach release)" >&2
            exit 1
        fi
        if spctl -a -vv -t execute "$MACOS_BUNDLE" 2>&1 | grep -q "accepted"; then
            echo "  notarization   : Gatekeeper assessment accepted"
        else
            fail "notarization gate rejected the bundle (spctl did not accept it; is it notarized AND stapled?)"
        fi
    else
        echo "  codesign       : skipped (not a macOS host)"
    fi
fi

echo "OK: package verification passed"
