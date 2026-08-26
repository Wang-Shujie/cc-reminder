#!/usr/bin/env bash
# Stage a locally built helper binary + a REAL helper manifest into the dev
# resource directory so Install/Repair work under `tauri dev`.
#
# Why this exists: committed resources carry the PLACEHOLDER manifest by
# design (design §9.1 — dev builds must never silently install unsigned
# bytes), so every Install/Repair in a dev build fails with
# `configuration.helper_unavailable`. This script is the opt-in dev escape
# hatch: it builds the host-target helper and writes a matching manifest into
# `target/<profile>/resources/`.
#
# Caveat: cargo/tauri-build re-copies the committed placeholder resources on
# rebuild, clobbering the staged files — re-run this script after rebuilding.

set -euo pipefail

outdir="${1:-debug}"
[[ "$outdir" == "debug" ]] && profile="dev" || profile="$outdir"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/src-tauri"

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) triple="aarch64-apple-darwin" ;;
  Darwin/x86_64) triple="x86_64-apple-darwin" ;;
  Linux/x86_64) triple="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64) triple="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported host for dev staging" >&2; exit 1 ;;
esac

cargo build --profile "$profile" --bin cc-reminder-hook

built="target/$outdir/cc-reminder-hook"
resources="target/$outdir/resources"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
length=$(wc -c < "$built" | tr -d ' ')
sha=$(shasum -a 256 "$built" | awk '{print $1}')

cp "$built" "$resources/bin/cc-reminder-hook"
cat > "$resources/helper-manifest.json" <<EOF
{
  "helpers": [
    {
      "target_triple": "$triple",
      "helper_version": "$version",
      "filename": "cc-reminder-hook",
      "length": $length,
      "sha256": "$sha"
    }
  ]
}
EOF

echo "staged $triple helper $version ($length bytes) into $resources"
