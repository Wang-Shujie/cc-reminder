<div align="center">

# CC Reminder

**Group-notification butler for Claude Code / Codex — the moment a task finishes, waits for confirmation, or needs authorization, your DingTalk / WeCom group knows.**

Approvals, stops and every other interaction stay exactly where they belong: inside the Agent's native flow. CC Reminder does one thing — deliver what's worth knowing, reliably.

[![Release](https://img.shields.io/github/v/release/Wang-Shujie/cc-reminder)](https://github.com/Wang-Shujie/cc-reminder/releases/latest)
[![CI](https://github.com/Wang-Shujie/cc-reminder/actions/workflows/ci.yml/badge.svg)](https://github.com/Wang-Shujie/cc-reminder/actions/workflows/ci.yml)
[![Release](https://github.com/Wang-Shujie/cc-reminder/actions/workflows/release.yml/badge.svg)](https://github.com/Wang-Shujie/cc-reminder/actions/workflows/release.yml)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)

English · [简体中文](./README.md)

<img src="docs/images/workbench.png" alt="Workbench: status overview and notification history" width="49%" /><img src="docs/images/integrations.png" alt="Integrations: notification sources and destinations" width="49%" />

<img src="docs/images/rules.png" alt="Hook rules: filtering, quiet hours, aggregation and delivery targets" width="70%" />

</div>

## ✨ Features

- **🔗 Lifecycle hook capture** — Events flow through the official Claude Code / Codex hook mechanisms into a separately signed helper process; approvals and stops are untouched (neutral output, constant exit code).
- **🎯 Rule engine** — Global rules with per-project field overrides; quiet hours, high-frequency aggregation, cooldown windows, TTL expiry. Permission requests are never aggregated.
- **🛡 Privacy first** — Sensitive fields are encrypted per-field with XChaCha20-Poly1305 before they touch disk; keys live in the system keychain / credential manager. Notification templates redact on demand; diagnostic bundles are forcibly sanitized.
- **📬 Reliable delivery** — Exponential-backoff retries (honoring `Retry-After`), at-least-once semantics, offline spool replay. Three consecutive credential failures pause a channel automatically; replacing the credential resumes delivery.
- **❤️‍🩹 Self-healing health** — Agent configs wiped or drifted externally are restored automatically; hook drift, paused channels and queue backlog all project into the workbench and tray menu.
- **🔔 Native tray** — Open main window, health at a glance, pause 15 min / 1 hour / today, resume, quit — localized with the app language.
- **🔌 Open channels** — DingTalk custom robots (signed / keyword) and WeCom group robots, official HTTPS webhook endpoints only.
- **🌐 Cross-platform** — macOS 12+ (Apple Silicon / Intel), Windows 10/11 x64, Ubuntu 22.04+ x64.

## 📦 Install

Grab the installer for your platform from [**Releases → Latest**](https://github.com/Wang-Shujie/cc-reminder/releases/latest); every artifact ships with a `.sha256` checksum:

| Platform | Artifact |
|---|---|
| macOS 12+ (Apple Silicon / Intel) | `.dmg` |
| Windows 10/11 x64 | `.msi` / NSIS installer |
| Ubuntu 22.04+ x64 | `.AppImage` / `.deb` |

> Binaries are provided from the first formal release tag onward. Updates go through the Tauri updater with minisign-verified manifests.

## 🚀 Quick start

1. **Launch** — the onboarding wizard walks you through detection and default rules.
2. **Connect agents** — Integrations page → **Detect agents** → **Install hooks**; for Codex, run `/hooks` once in its official UI to confirm trust.
3. **Create a robot** — add a DingTalk / WeCom group robot and copy the webhook (step-by-step guide in the in-app form and the [operations manual §5](docs/operations.md)), then add the channel in the integrations page and **send a test**.
4. **Configure rules** — filter by event / project / agent on the rules page, pick delivery channels, done.

## 🔧 How it works

```
Claude Code / Codex ──hook──▶ signed helper ──IPC──▶ rule match · privacy ──▶ queue ──▶ DingTalk / WeCom
                                    │                     │
                              offline spool        local SQLite (field-level encryption)
```

When the app is unavailable the helper spools to disk and replays within TTL after recovery. Every interactive operation — approvals, stops — bypasses CC Reminder entirely.

## 🛠 Build from source

```bash
# Prerequisites: Node 20+ / pnpm 10+ / Rust 1.80+ (Xcode CLT on macOS, webkit2gtk-4.1 on Linux)
pnpm install --frozen-lockfile

pnpm dev                     # run in development
pnpm verify                  # frontend tests + typecheck + build + Playwright
cargo test --manifest-path src-tauri/Cargo.toml   # full Rust test suite

pnpm tauri build             # package for the current platform
scripts/local-release-build.sh   # local release bundle (hash-verified helper + real manifest)
```

## 📚 Documentation

- [Operations manual (install / channels / rules / diagnostics / uninstall)](docs/operations.md)
- [Design specification and UI discipline](DESIGN.md)
- [Deferred issues and field-incident records](docs/v2-issues.md)
