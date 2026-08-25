#!/usr/bin/env bash
#
# check-sensitive-artifacts.sh (Task 22)
#
# Scans RUNTIME BUILD ARTIFACTS ONLY for planted plaintext markers, concrete
# credential values, PEM private-key blocks, and executable bypass arguments.
# Its argument is the repository root that contains the build outputs.
#
# Scanned surfaces (exactly four, per design 8 / Task 22):
#   1. dist/                              — built frontend output
#   2. src-tauri/resources/               — non-test packaged resources
#   3. src-tauri/target/release/bundle/   — release package staging
#   4. src-tauri/target/release/cc-reminder{,.exe}
#      src-tauri/target/release/cc-reminder-hook{,.exe}  — final binaries
#
# Source, tests, fixtures, scripts, and prose documentation are NEVER scanned,
# so security assertions and documented terms elsewhere in the repo cannot
# cause findings (fixture sanitization remains covered by Task 3 tests).
#
# On a match it prints the FILE and RULE NAME only — never the matched value —
# and exits 1. Missing artifact locations are skipped with a note (the scan
# runs in CI both before and after packaging).
#
# Usage: ./scripts/check-sensitive-artifacts.sh <repo-root>

set -u

if [ $# -ne 1 ]; then
    echo "usage: $0 <repo-root>" >&2
    exit 64
fi

ROOT=$1
if [ ! -d "$ROOT" ]; then
    echo "error: '$ROOT' is not a directory" >&2
    exit 64
fi
ROOT=$(cd "$ROOT" && pwd)

RELEASE_DIR="$ROOT/src-tauri/target/release"

# Scan targets: directories first, then explicit final binaries.
DIR_TARGETS=(
    "$ROOT/dist"
    "$ROOT/src-tauri/resources"
    "$RELEASE_DIR/bundle"
)
BIN_TARGETS=()
for bin_name in cc-reminder cc-reminder-hook; do
    for suffix in "" ".exe"; do
        candidate="$RELEASE_DIR/$bin_name$suffix"
        [ -f "$candidate" ] && BIN_TARGETS+=("$candidate")
    done
done

present_targets=()
missing_notes=""
for target in "${DIR_TARGETS[@]}"; do
    if [ -d "$target" ]; then
        present_targets+=("$target")
    else
        missing_notes+="  (dir absent, skipped) $target"$'\n'
    fi
done
for target in ${BIN_TARGETS[@]+"${BIN_TARGETS[@]}"}; do
    present_targets+=("$target")
done

if [ ${#BIN_TARGETS[@]} -eq 0 ]; then
    missing_notes+="  (no release binaries found, skipped) $RELEASE_DIR/{cc-reminder,cc-reminder-hook}"$'\n'
fi

echo "check-sensitive-artifacts: repository root $ROOT"

# ---------------------------------------------------------------------------
# Rules. Each entry is RULE_NAME<TAB>egrep-pattern.
#
# Deliberately narrower than the redactor's own patterns, which are embedded
# verbatim in the production binary (src-tauri/src/security/redact.rs): e.g.
# the redactor stores the literal text
#   -----BEGIN [^-\r\n]*PRIVATE KEY-----...
# and
#   (?i:[?&](?:access_token|key|secret)=[^&#\s]+)
# Neither matches the CONCRETE-shape rules below, because a real PEM header
# needs a known algorithm slot and a real credential query value needs a long
# unbroken secret-shaped token where the stored regex has metacharacters.
# ---------------------------------------------------------------------------
RULES=(
    "plaintext-test-marker	cc-reminder-e2e|secret-raw-value|VITE_CC_REMINDER_TEST_BACKEND|CC_REMINDER_TEST_DATA_DIR"
    "pem-private-key-block	-----BEGIN (RSA |EC |DSA |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"
    "concrete-credential-query-value	[?&](access_token|sign|secret|accessToken|apiKey|apikey|key)=[A-Za-z0-9+/_-]{24,}"
    "executable-bypass-argument	--[A-Za-z0-9][A-Za-z0-9_-]*(bypass|skip[_-]verif|insecure|unsafe|trust[_-]all)[A-Za-z0-9_-]*"
)

findings=0
if [ ${#present_targets[@]} -eq 0 ]; then
    echo "no artifact locations present; nothing to scan"
    echo "OK: 0 sensitive-artifact findings"
    exit 0
fi
for rule in "${RULES[@]}"; do
    name=${rule%%$'\t'*}
    pattern=${rule#*$'\t'}
    # shellcheck disable=SC2086
    matches=$(grep -RalaE -- "${pattern}" ${present_targets[@]+"${present_targets[@]}"} 2>/dev/null || true)
    if [ -n "$matches" ]; then
        while IFS= read -r file; do
            echo "SENSITIVE ARTIFACT MATCH"
            echo "  rule: $name"
            echo "  file: $file"
            echo "  (matched value intentionally not printed)"
            findings=$((findings + 1))
        done <<< "$matches"
    fi
done

if [ -n "$missing_notes" ]; then
    echo "skipped artifact locations:"
    printf '%s' "$missing_notes"
fi

echo "scanned ${#present_targets[@]} artifact location(s)"
if [ "$findings" -gt 0 ]; then
    echo "FAIL: $findings sensitive-artifact finding(s)" >&2
    exit 1
fi
echo "OK: 0 sensitive-artifact findings"
exit 0
