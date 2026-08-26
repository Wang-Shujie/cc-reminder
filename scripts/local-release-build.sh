#!/usr/bin/env bash
# Build a LOCAL UNSIGNED release bundle with a real, hash-verified helper.
#
# Why this exists: the committed resources/bin + helper-manifest.json are
# PLACEHOLDERS by design (anti-forgery: dev builds must never install
# unsigned bytes). A locally usable bundle must stage the real host-target
# helper into the tracked resource paths for the duration of `tauri build`,
# then restore the placeholders so nothing real ever gets committed.
#
# Usage: scripts/local-release-build.sh   (from the repo root)
# Produces: src-tauri/target/release/bundle/macos/CC Reminder.app (+ .dmg)

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

resources_dir="src-tauri/resources"
bin="$resources_dir/bin/cc-reminder-hook"
manifest="$resources_dir/helper-manifest.json"

restore() { git checkout -- "$bin" "$manifest"; }
trap restore EXIT

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)  triple="aarch64-apple-darwin" ;;
  Darwin/x86_64) triple="x86_64-apple-darwin" ;;
  *) echo "unsupported host for local release staging" >&2; exit 1 ;;
esac

echo "[1/4] building release helper"
cargo build --manifest-path src-tauri/Cargo.toml --release --bin cc-reminder-hook

echo "[2/4] staging helper + real manifest ($triple)"
cp src-tauri/target/release/cc-reminder-hook "$bin"
length=$(stat -f%z "$bin")
sha=$(shasum -a 256 "$bin" | awk '{print $1}')
version=$(sed -n 's/^version = "\(.*\)"/\1/p' src-tauri/Cargo.toml | head -1)
python3 - "$triple" "$version" "$length" "$sha" > "$manifest" <<'EOF'
import json, sys
print(json.dumps({
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "CC Reminder signed helper manifest",
  "description": "LOCAL UNSIGNED BUILD staging. NOT FOR COMMIT — release packaging regenerates this from signed bytes.",
  "helpers": [{
    "target_triple": sys.argv[1],
    "helper_version": sys.argv[2],
    "filename": "cc-reminder-hook",
    "length": int(sys.argv[3]),
    "sha256": sys.argv[4],
  }],
}, indent=2, ensure_ascii=False))
EOF

echo "[3/4] building the app bundle (updater artifacts off: no signing key locally)"
# v2-issues: 按平台注入资源表,异平台 0 字节占位 helper 不再进包。
case "$triple" in
  *-windows-*) host_bin="resources/bin/cc-reminder-hook.exe" ;;
  *) host_bin="resources/bin/cc-reminder-hook" ;;
esac
pnpm tauri build --config "$(printf '{"bundle":{"createUpdaterArtifacts":false,"resources":["resources/capabilities/claude-code-2.1.218.json","resources/capabilities/codex-0.145.0.json","resources/helper-manifest.json","%s"]}}' "$host_bin")"

echo "[4/4] verifying the bundle actually embeds the staged manifest"
bundled="src-tauri/target/release/bundle/macos/CC Reminder.app/Contents/Resources/resources/helper-manifest.json"
python3 - "$bundled" "$triple" <<'EOF'
import json, sys
m = json.load(open(sys.argv[1]))
entry = next((h for h in m["helpers"] if h["target_triple"] == sys.argv[2]), None)
assert entry and entry["length"] > 0 and not entry["sha256"].startswith("REPLACE"), \
    f"bundle still carries the placeholder manifest: {m}"
print(f"verified: bundle carries a real {sys.argv[2]} helper entry "
      f"(v{entry['helper_version']}, {entry['length']} bytes)")
EOF

echo "done — bundle at src-tauri/target/release/bundle/macos/CC Reminder.app"
echo "tracked placeholders restored automatically."
