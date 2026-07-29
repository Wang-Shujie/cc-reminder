# CC Reminder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a signed cross-platform tray application that captures Claude Code and Codex lifecycle Hooks, applies user-configured privacy and delivery rules, and reliably sends one-way DingTalk or WeCom notifications.

**Architecture:** A single Tauri 2 Rust package builds the desktop application and the short-lived `cc-reminder-hook` binary. The helper accepts bounded Hook JSON and hands a white-listed event to the running application over user-only local IPC, or persists a safe metadata envelope to SQLite/spool when the application is unavailable; the desktop core resolves rules, redacts content, renders a platform-neutral document, queues one job per target, and sends through fixed DingTalk/WeCom adapters. React renders a dense, accessible management UI and calls only explicit typed Tauri commands; no local HTTP server, daemon, dynamic plugin runtime, or inbound chat transport exists in v1.

**Tech Stack:** Tauri 2, Rust 2024, Tokio, rusqlite/SQLite WAL, reqwest/rustls, keyring, XChaCha20-Poly1305, React 19, TypeScript 5, Vite, Vitest/Testing Library, Playwright, pnpm, Lucide React.

## Global Constraints

- First release is outbound notifications only; reserve `correlation_id`, `action_id`, `action_capabilities`, and the action module boundary without starting inbound transports or executing remote actions.
- Support Claude Code and Codex only; show every catalogued Hook in the GUI, but install only events enabled globally or by at least one project override.
- Support DingTalk custom robot and WeCom group robot official HTTPS Webhooks only; personal WeChat/iLink, arbitrary Webhooks, scripts, and executable templates are out of scope.
- Supported desktop baselines are macOS 12+ on Apple Silicon and Intel, Windows 10/11 x64, and Ubuntu 22.04+ x64 or compatible distributions.
- Use OS secure storage with no plaintext fallback: macOS Keychain, Windows Credential Manager, Linux Secret Service. Without Secret Service, Linux may read non-sensitive configuration but must reject credential persistence.
- Never persist raw Hook JSON, plaintext channel credentials, original session/turn IDs, transcript content, environment variables, unregistered/raw full paths received from Hook payloads, or unencrypted opt-in sensitive fields. User-selected project roots/aliases and fixed Agent configuration paths remain local configuration data and are never included in notifications or diagnostics.
- The helper must return neutral stdout and exit code `0` for valid invocations even when IPC, SQLite, or spool delivery fails; notification failures must never change Agent behavior.
- Hook normal-path performance target is p95 under 100 ms; application rule resolution plus enqueue target is p95 under 50 ms; normal-network event-to-send target is usually under 5 seconds; idle resident memory target is under 100 MiB.
- SQLite uses WAL and versioned migrations. External delivery is at least once, with a local unique idempotency key, expiring leases, default five-attempt jittered exponential retry, and server `Retry-After` precedence.
- Permission notifications expire after 10 minutes by default; ordinary notifications expire after 30 minutes by default. Event/delivery retention defaults to 30 days and diagnostic log retention defaults to 7 days.
- HTTP connect timeout is 5 seconds and total request timeout is 10 seconds. TLS verification cannot be disabled, and only built-in official hosts are accepted.
- Hook input, templates, rules, platform responses, and user redaction patterns are untrusted. Bound input size/depth/field count, compile regex with limits, redact before logging, and expose only authorized template fields.
- Claude Code installation modifies only user settings through a JSONC syntax tree while preserving comments, formatting, unknown fields, and unrelated Hooks. Codex installation maintains `~/.codex/hooks.json`, never writes `--dangerously-bypass-hook-trust`, and reports official `/hooks` trust as a user action.
- The desktop app is single-instance, does not install a daemon, does not open a TCP/HTTP listener, and must not be launched by the helper after the user exits it.
- The GUI defaults to Chinese with English support, follows system light/dark theme, has a 960 x 640 minimum window, uses Lucide icons, uses no card nesting, and provides keyboard navigation, visible focus, screen-reader labels, and system zoom support.
- No default telemetry, remote logging, model-generated summaries, transcript parsing, cloud sync, media sending, Agent process hosting, or cc-connect scheduling/relay behavior.

## Repository Map

The implementation creates the following focused files. Keep responsibilities at these boundaries; add a new file only when a listed file would otherwise mix unrelated behavior.

```text
.
├── .github/workflows/ci.yml                 # lint, unit, integration, UI, and OS build checks
├── .github/workflows/release.yml            # signed/certified release matrix and checksums
├── .gitignore
├── assets/app-icon.svg                      # single source for generated native icons
├── docs/
│   ├── images/hook-rules.png                # reviewed product screenshot for README
│   ├── operations.md                        # install, trust, diagnostics, uninstall, recovery
│   └── superpowers/{specs,plans}/            # approved design and this implementation plan
├── index.html
├── migrations/0001_initial.sql              # complete v1 SQLite schema
├── package.json
├── playwright.config.ts
├── pnpm-lock.yaml
├── README.md
├── tsconfig.json
├── vite.config.ts
├── src/
│   ├── main.tsx                             # React entry
│   ├── App.tsx                              # app routing and backend subscription
│   ├── App.test.tsx                         # bootstrap and route behavior
│   ├── app.css                              # tokens, layout, themes, focus, responsive rules
│   ├── lib/backend.ts                       # typed invoke/listen boundary and browser test fake
│   ├── lib/contracts.ts                     # frontend representations of command DTOs
│   ├── lib/i18n.ts                          # Chinese/English dictionaries and locale selection
│   ├── test/setup.ts                        # DOM and Tauri mocks
│   ├── shell/AppShell.tsx                   # rail navigation, scope switcher, status header
│   ├── shell/AppShell.test.tsx
│   ├── onboarding/Onboarding.tsx            # five-step first configuration flow
│   ├── onboarding/Onboarding.test.tsx
│   ├── overview/OverviewPage.tsx             # health and recent-failure dashboard
│   ├── overview/OverviewPage.test.tsx
│   ├── agents/AgentsPage.tsx                 # detection, install, drift, repair, trust, uninstall
│   ├── agents/AgentsPage.test.tsx
│   ├── hooks/HookRulesPage.tsx               # complete capability table and filters
│   ├── hooks/HookRuleDrawer.tsx              # inherited field editor and safe preview/test
│   ├── hooks/HookRulesPage.test.tsx
│   ├── channels/ChannelsPage.tsx             # instances, credential replace/delete, test send
│   ├── channels/ChannelsPage.test.tsx
│   ├── projects/ProjectsPage.tsx             # roots, aliases, worktree choice, override counts
│   ├── projects/ProjectsPage.test.tsx
│   ├── history/HistoryPage.tsx               # redacted event/jobs/attempts and manual retry
│   ├── history/HistoryPage.test.tsx
│   ├── settings/SettingsPage.tsx             # startup, close, locale, theme, retention, diagnostics
│   └── settings/SettingsPage.test.tsx
├── tests/
│   ├── e2e/app.spec.ts                       # browser-level workflows against typed fake backend
│   └── fixtures/
│       ├── claude-code/2.1.218/*.json        # sanitized Hook contract fixtures
│       ├── codex/0.145.0/*.json              # sanitized Hook contract fixtures
│       ├── configs/claude-settings.jsonc     # comments, foreign Hooks, unknown fields
│       └── configs/codex-hooks.json          # trusted/foreign Hook coexistence
├── src-tauri/
│   ├── build.rs
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── capabilities/default.json             # minimum Tauri permissions
│   ├── icons/*                               # generated platform icons
│   ├── resources/capabilities/
│   │   ├── claude-code-2.1.218.json           # verified Claude capability catalog
│   │   └── codex-0.145.0.json                 # verified Codex capability catalog
│   ├── resources/helper-manifest.json         # packaged helper target/version/hash manifest
│   ├── tauri.conf.json
│   ├── src/main.rs                            # desktop binary entry
│   ├── src/lib.rs                             # single-instance app initialization
│   ├── src/actions.rs                         # v1 action correlation contract only
│   ├── src/error.rs                           # stable redacted errors and suggested actions
│   ├── src/model.rs                           # shared IDs, event, rule, channel, and DTO types
│   ├── src/paths.rs                           # shared desktop/helper data paths
│   ├── src/hook_command.rs                    # shared cross-platform command quoting/fingerprint
│   ├── src/bin/cc-reminder-hook.rs            # bounded one-shot helper CLI
│   ├── src/events/{mod.rs,catalog.rs,normalize.rs}
│   ├── src/projects/{mod.rs,resolver.rs}
│   ├── src/rules/{mod.rs,resolve.rs,policy.rs,template.rs}
│   ├── src/security/{mod.rs,credentials.rs,crypto.rs,redact.rs,permissions.rs}
│   ├── src/storage/{mod.rs,db.rs,events.rs,config.rs,integrations.rs,queue.rs,spool.rs,retention.rs}
│   ├── src/ipc/{mod.rs,protocol.rs,server.rs,client.rs}
│   ├── src/agents/{mod.rs,claude.rs,codex.rs,detect.rs}
│   ├── src/installer/{mod.rs,jsonc.rs,atomic.rs,helper.rs}
│   ├── src/channels/{mod.rs,dingtalk.rs,wecom.rs,http.rs}
│   ├── src/pipeline.rs                        # normalize through durable enqueue
│   ├── src/worker.rs                          # lease, send, classify, retry loop
│   ├── src/health.rs                          # common overview/tray/page health projection
│   ├── src/diagnostics.rs                     # redacted rotating logs and diagnostic archive
│   ├── src/commands/{mod.rs,agents.rs,rules.rs,channels.rs,projects.rs,history.rs,settings.rs}
│   └── tests/{hook_contract.rs,installer_roundtrip.rs,storage_recovery.rs,channel_contract.rs,pipeline.rs}
├── scripts/check-sensitive-artifacts.sh      # runtime/artifact secret-pattern scan
├── scripts/verify-package.sh                 # POSIX package verification
└── scripts/verify-package.ps1                # Windows package verification
```

## Dependency Budget

After the Task 1 base manifest, run each command in `src-tauri/` during the named task and commit `Cargo.lock`. These are the remaining complete v1 direct Rust additions; use standard library/Tauri facilities outside this list and require a plan revision before adding another production crate.

| Task | Command |
|---|---|
| 2 | `cargo add async-trait chrono --features chrono/serde && cargo add semver --features serde && cargo add uuid --features serde,v7 && cargo add thiserror` |
| 3 | `cargo add hmac sha2 hex` |
| 5 | `cargo add regex` |
| 6 | `cargo add rusqlite --features bundled,chrono,uuid && cargo add --dev tempfile && cargo add --target 'cfg(windows)' windows-sys --features Win32_Foundation,Win32_Security,Win32_Security_Authorization,Win32_Storage_FileSystem,Win32_System_Pipes,Win32_System_Threading` |
| 7 | `cargo add chacha20poly1305 keyring secrecy zeroize rand` |
| 8 | `cargo add tokio --features rt-multi-thread,macros,net,io-util,sync,time,process && cargo add directories` |
| 9 | `cargo add wait-timeout` |
| 10 | `cargo add jsonc-parser fs2` |
| 12 | `cargo add rand` (no-op if Task 7 already locked it) |
| 13 | `cargo add reqwest --no-default-features --features json,rustls-tls-native-roots,system-proxy && cargo add url base64 percent-encoding httpdate && cargo add --dev wiremock` |
| 14 | `cargo add tokio-util --features rt && cargo add futures-util` |
| 15 | `cargo add tauri-plugin-updater` |
| 18 | `cargo add tauri-plugin-dialog` |
| 20 | `cargo add tracing && cargo add tracing-subscriber --features json,env-filter && cargo add zip` |

Frontend additions are similarly bounded: Task 1 installs React/Tauri/Vite/Vitest/Testing Library/Lucide, Task 15 adds `@tauri-apps/plugin-updater`, Task 18 adds `@tauri-apps/plugin-dialog`, and Task 21 adds Playwright plus axe. pnpm resolves exact versions into `pnpm-lock.yaml`; all CI/release installs use `--frozen-lockfile`.

## Execution Prerequisites

- Node.js 22 LTS or newer, pnpm 10, and current stable Rust installed through rustup; verify `node --version`, `pnpm --version`, `rustc --version`, and `cargo --version` before Task 1.
- macOS: Xcode Command Line Tools plus Rust targets `aarch64-apple-darwin` and `x86_64-apple-darwin`; Tauri combines them through its `universal-apple-darwin` bundle target.
- Windows: Visual Studio 2022 Build Tools with Desktop C++, Windows 10/11 SDK, WebView2 runtime, and the `x86_64-pc-windows-msvc` Rust target.
- Ubuntu 22.04: `build-essential`, `curl`, `wget`, `file`, `libssl-dev`, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libxdo-dev`, plus the `x86_64-unknown-linux-gnu` Rust target.
- Signing/notarization credentials are needed only for Task 22 protected release jobs; unit, integration, and unsigned build-smoke tasks must run without them.

## Shared Contracts

These names are fixed across tasks. Rust owns serialization; TypeScript mirrors the command DTO shape in `src/lib/contracts.ts`. IDs use UUID v7 strings at the UI boundary and `uuid::Uuid` internally.

```rust
pub type ProjectId = uuid::Uuid;
pub type ChannelId = uuid::Uuid;
pub type RuleId = uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind { ClaudeCode, Codex }

impl AgentKind {
    pub const fn as_str(self) -> &'static str {
        match self { Self::ClaudeCode => "claude-code", Self::Codex => "codex" }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Session, Prompt, Tool, Permission, Compaction, Subagent,
    Task, Configuration, Worktree, Notification, Completion, Other,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Info, Warning, Error, Critical }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ScalarValue { String(String), Number(f64), Bool(bool), Null }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EncryptedBlobRef { pub blob_id: uuid::Uuid }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub id: uuid::Uuid,
    pub source: AgentKind,
    pub source_version: semver::Version,
    pub source_event: String,
    pub category: EventCategory,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub project_id: Option<ProjectId>,
    pub project_display_name: Option<String>,
    pub unmatched_cwd_fingerprint: Option<String>,
    pub session_ref: Option<String>,
    pub turn_ref: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub severity: Severity,
    pub public_fields: std::collections::BTreeMap<String, ScalarValue>,
    pub encrypted_sensitive_fields: Option<EncryptedBlobRef>,
    pub correlation_id: uuid::Uuid,
    pub action_id: Option<String>,
    pub action_capabilities: Vec<ActionCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCapability { Approve, Reject, Reply }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NotificationDocument {
    pub title: String,
    pub severity: Severity,
    pub facts: Vec<(String, String)>,
    pub body: String,
    pub footer: Option<String>,
}
```

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TargetConfig {
    pub channel_id: ChannelId,
    pub template: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FilterGroup {
    pub tool_names: Vec<String>,
    pub event_subtypes: Vec<String>,
    pub permission_modes: Vec<String>,
    pub models: Vec<String>,
    pub statuses: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrivacyPolicy {
    pub allowed_sensitive_fields: Vec<String>,
    pub max_body_chars: u32,
    pub summary_mode: SummaryMode,
    pub extra_redaction_patterns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryMode { MetadataOnly, NativeSummary }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DeliveryMode { Immediate, Aggregate { window_seconds: u32 } }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuietBehavior { Suppress, Defer }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeliveryPolicy {
    pub mode: DeliveryMode,
    pub cooldown_seconds: u32,
    pub max_per_window: u32,
    pub window_seconds: u32,
    pub quiet_behavior: QuietBehavior,
    pub ttl_seconds: u32,
    pub max_attempts: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QuietHours {
    pub start_local: String,
    pub end_local: String,
    pub weekdays: Vec<u8>,
    pub bypass_at_or_above: Option<Severity>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NotificationPause {
    pub started_at: DateTime<Utc>,
    pub until: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RuleConfig {
    pub enabled: bool,
    pub targets: Vec<TargetConfig>,
    pub filters: FilterGroup,
    pub privacy: PrivacyPolicy,
    pub delivery: DeliveryPolicy,
    pub quiet_hours: Option<QuietHours>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RulePatch {
    pub enabled: Option<bool>,
    pub targets: Option<Vec<TargetConfig>>,
    pub filters: Option<FilterGroup>,
    pub privacy: Option<PrivacyPolicy>,
    pub delivery: Option<DeliveryPolicy>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub quiet_hours: Option<Option<QuietHours>>,
}

fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
```

Error and sender interfaces are also fixed:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppError {
    pub domain: ErrorDomain,
    pub code: String,
    pub message: String,
    pub suggested_action: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorDomain { Integration, Configuration, SecretStore, Delivery, Storage, Update }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub http_status: u16,
    pub platform_code: Option<String>,
    pub sent_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryErrorKind {
    Network, Timeout, HttpStatus, TemporaryPlatform,
    Authentication, Signature, Permission, Format,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryError {
    pub kind: DeliveryErrorKind,
    pub code: String,
    pub redacted_message: String,
    pub http_status: Option<u16>,
    pub platform_code: Option<String>,
    pub retry_after_seconds: Option<u64>,
}

#[async_trait::async_trait]
pub trait ChannelSender: Send + Sync {
    async fn send(
        &self,
        document: &NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError>;
}
```

### Task 1: Bootstrap a Testable Tauri and React Shell

**Files:**
- Create: `package.json`
- Create: `pnpm-lock.yaml`
- Create: `tsconfig.json`
- Create: `vite.config.ts`
- Create: `index.html`
- Create: `src/main.tsx`
- Create: `src/App.tsx`
- Create: `src/app.css`
- Create: `src/test/setup.ts`
- Create: `src/App.test.tsx`
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/Cargo.lock`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Create: `src-tauri/src/lib.rs`
- Create: `.gitignore`

**Interfaces:**
- Consumes: none.
- Produces: `cc_reminder_lib::run()`, a Tauri application named `CC Reminder`, `pnpm test`, `pnpm build`, and `cargo test --manifest-path src-tauri/Cargo.toml` as stable verification entry points.

- [ ] **Step 1: Create the manifests and test harness**

Use pnpm to resolve and lock current stable releases within the required major versions:

```bash
pnpm init
pnpm add react@19 react-dom@19 @tauri-apps/api@2 @tauri-apps/plugin-autostart@2 lucide-react
pnpm add -D typescript@5 vite @vitejs/plugin-react vitest jsdom @testing-library/react @testing-library/jest-dom @testing-library/user-event @types/react@19 @types/react-dom@19 @tauri-apps/cli@2
```

Set exact scripts in `package.json`:

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "test": "vitest run",
    "test:watch": "vitest",
    "tauri": "tauri"
  }
}
```

Create `src-tauri/Cargo.toml` with this initial package shape; later tasks add only the dependencies they use:

```toml
[package]
name = "cc-reminder"
version = "0.1.0"
edition = "2024"
default-run = "cc-reminder"

[lib]
name = "cc_reminder_lib"
crate-type = ["lib", "cdylib", "staticlib"]

[[bin]]
name = "cc-reminder"
path = "src/main.rs"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-autostart = "2"
tauri-plugin-single-instance = "2"
```

Configure Vitest in `vite.config.ts` with `environment: "jsdom"`, `setupFiles: ["./src/test/setup.ts"]`, and `clearMocks: true`. Configure TypeScript with `strict`, `noUncheckedIndexedAccess`, and `jsx: "react-jsx"`.

- [ ] **Step 2: Write the failing shell tests**

```tsx
import { render, screen } from "@testing-library/react";
import App from "./App";

test("renders the product shell in Chinese", () => {
  render(<App />);
  expect(screen.getByRole("application", { name: "CC Reminder" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Hook 规则" })).toBeVisible();
});
```

- [ ] **Step 3: Run the frontend test and verify the intended failure**

Run: `pnpm test -- src/App.test.tsx`

Expected: FAIL because `src/App.tsx` does not yet export a rendered application shell.

- [ ] **Step 4: Implement the minimal React and Tauri entries**

Create an accessible application root; do not add marketing copy or decorative cards:

```tsx
export default function App() {
  return (
    <main role="application" aria-label="CC Reminder">
      <h1>Hook 规则</h1>
    </main>
  );
}
```

The Rust entry remains a thin call:

```rust
fn main() {
    cc_reminder_lib::run();
}
```

Set the Tauri identifier to `com.ccreminder.app`. `run()` builds a Tauri app with single-instance and autostart plugins registered, a minimum window size of 960 x 640 in `tauri.conf.json`, and no remote URL or network capability granted to the WebView. Set production CSP to `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost; style-src 'self'; font-src 'self'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'` with no inline/remote script, frame, object, form, or base destinations; a separate debug-only `devCsp` adds only the exact Vite localhost HTTP/WebSocket origin while retaining those explicit `none` directives. Keep the native window visible during this bootstrap task; tray/minimize behavior is added only after the health model exists.

- [ ] **Step 5: Verify the shell and both builds**

Run: `pnpm test -- src/App.test.tsx && pnpm build && cargo test --manifest-path src-tauri/Cargo.toml`

Expected: the test passes, Vite emits `dist/`, and Cargo compiles both the library and desktop binary without warnings.

- [ ] **Step 6: Commit the bootstrap**

```bash
git add .gitignore package.json pnpm-lock.yaml tsconfig.json vite.config.ts index.html src src-tauri
git commit -m "build: bootstrap Tauri and React application"
```

### Task 2: Define Domain Contracts and Versioned Hook Catalogs

**Files:**
- Create: `src-tauri/src/model.rs`
- Create: `src-tauri/src/actions.rs`
- Create: `src-tauri/src/error.rs`
- Create: `src-tauri/src/events/mod.rs`
- Create: `src-tauri/src/events/catalog.rs`
- Create: `src-tauri/resources/capabilities/claude-code-2.1.218.json`
- Create: `src-tauri/resources/capabilities/codex-0.145.0.json`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `src-tauri/src/events/catalog.rs` and `src-tauri/src/actions.rs`

**Interfaces:**
- Consumes: `cc_reminder_lib` from Task 1.
- Produces: all structs in **Shared Contracts** plus `CapabilityCatalog`, `HookCapability`, `CapabilityResolution`, `catalog_for(agent: AgentKind, version: &Version) -> CapabilityResolution`, and `new_v1_action_fields() -> (Uuid, Option<String>, Vec<ActionCapability>)`.

- [ ] **Step 1: Write failing catalog and v1 action contract tests**

```rust
#[test]
fn verified_codex_catalog_has_exact_lifecycle_events() {
    let result = catalog_for(AgentKind::Codex, &Version::parse("0.145.0").unwrap());
    assert_eq!(result.verification, CatalogVerification::Exact);
    assert_eq!(result.catalog.hooks.iter().map(|h| h.source_event.as_str()).collect::<Vec<_>>(), vec![
        "SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse",
        "PermissionRequest", "PreCompact", "PostCompact", "SubagentStart", "SubagentStop", "Stop",
    ]);
}

#[test]
fn v1_actions_are_correlated_but_not_actionable() {
    let (correlation_id, action_id, capabilities) = new_v1_action_fields();
    assert!(!correlation_id.is_nil());
    assert_eq!(action_id, None);
    assert!(capabilities.is_empty());
}
```

In the same module, assert that Claude 2.1.218 contains all 30 names from design section 8.2, a newer patch in the same declared compatibility line selects the nearest catalog with `CompatibleUnverified`, and an unknown major/minor line returns only `SessionStart`, `SessionEnd`, `PermissionRequest`, and `Stop` with `UpgradeRequired`.

- [ ] **Step 2: Run the tests and verify the intended failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml events::catalog && cargo test --manifest-path src-tauri/Cargo.toml actions::tests`

Expected: FAIL because the model, catalogs, and resolution functions do not exist.

- [ ] **Step 3: Add the exact domain and capability types**

Implement the **Shared Contracts** and these capability types:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityCatalog {
    pub agent: AgentKind,
    pub verified_version: Version,
    pub hooks: Vec<HookCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookCapability {
    pub source_event: String,
    pub label_zh: String,
    pub label_en: String,
    pub category: EventCategory,
    pub phase: String,
    pub supports_matcher: bool,
    pub matcher_target: Option<String>,
    pub input_fields: Vec<InputField>,
    pub sensitivity: Sensitivity,
    pub high_frequency: bool,
    pub neutral_output: NeutralOutput,
    pub status: CapabilityStatus,
    pub min_verified_version: Version,
    pub max_verified_version: Version,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogVerification { Exact, CompatibleUnverified, UpgradeRequired }

pub struct CapabilityResolution {
    pub catalog: CapabilityCatalog,
    pub verification: CatalogVerification,
}
```

`InputField` contains `name`, `sensitivity`, and `persist_by_default`; `Sensitivity` is `Public`, `Sensitive`, or `Forbidden`; `NeutralOutput` is `Empty` or `EmptyObject`; `CapabilityStatus` is `Stable`, `Experimental`, or `Deprecated`. Populate both JSON resources with every event and truthful metadata from design section 8; never infer installability from executable strings.

- [ ] **Step 4: Implement deterministic catalog resolution**

Use embedded resources through `include_str!`. Exact versions select exact catalogs. Each catalog declares a compatibility line equal to its verified `major.minor`; a non-exact patch in that line selects the closest lower-or-equal verified patch, falling back to the sole same-line fixture, and marks it `CompatibleUnverified`. A version with no matching declared line returns a code-defined four-event safe subset and `UpgradeRequired`. This intentionally does not treat all Codex `0.x` minors as compatible.

`new_v1_action_fields()` must generate UUID v7 and always return `None` plus an empty vector. Do not define inbound provider, policy, transport, or action handler implementations.

- [ ] **Step 5: Run focused and package tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml events::catalog && cargo test --manifest-path src-tauri/Cargo.toml actions::tests && cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all catalog names/order/status tests, serialization round trips, and v1 action contract tests pass.

- [ ] **Step 6: Commit the contracts**

```bash
git add src-tauri/src src-tauri/resources src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add event contracts and Hook catalogs"
```

### Task 3: Normalize Hook Events and Resolve Projects Without Persisting Paths

**Files:**
- Create: `src-tauri/src/events/normalize.rs`
- Create: `src-tauri/src/projects/mod.rs`
- Create: `src-tauri/src/projects/resolver.rs`
- Create: `tests/fixtures/claude-code/2.1.218/permission-request.json`
- Create: `tests/fixtures/claude-code/2.1.218/stop.json`
- Create: `tests/fixtures/codex/0.145.0/permission-request.json`
- Create: `tests/fixtures/codex/0.145.0/stop.json`
- Modify: `src-tauri/src/events/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `normalize.rs` and `resolver.rs`

**Interfaces:**
- Consumes: `AgentKind`, `CapabilityCatalog`, `EventEnvelope`, and action fields from Task 2.
- Produces: `CapturedHookEvent`, `SafeIngressEvent`, `capture_hook_json(agent: AgentKind, event: &str, source_version: Version, raw: Value) -> Result<CapturedHookEvent, AppError>`, `NormalizeContext`, `normalize_event(event: CapturedHookEvent, context: &NormalizeContext) -> Result<EventEnvelope, AppError>`, `ProjectRegistration`, and `resolve_project(cwd: &Path, projects: &[ProjectRegistration], platform: PathPlatform) -> ProjectMatch`.

- [ ] **Step 1: Add sanitized fixtures and failing normalization tests**

Fixtures must contain representative public and sensitive fields but fake all secrets and identifiers. Test both preservation and exclusion:

```rust
#[test]
fn codex_permission_keeps_source_name_and_hmac_references() {
    let raw = fixture("codex/0.145.0/permission-request.json");
    let captured = capture_hook_json(AgentKind::Codex, "PermissionRequest", Version::new(0, 145, 0), raw).unwrap();
    let event = normalize_event(captured, &context_with_key([7_u8; 32])).unwrap();
    assert_eq!(event.source_event, "PermissionRequest");
    assert_eq!(event.category, EventCategory::Permission);
    assert_ne!(event.session_ref.as_deref(), Some("raw-session-id"));
    assert!(event.action_id.is_none());
    assert!(event.action_capabilities.is_empty());
}

#[test]
fn unmatched_cwd_keeps_only_leaf_and_hmac_fingerprint() {
    let result = unmatched_project("/Users/alice/secret/client", &[9_u8; 32]);
    assert_eq!(result.display_name.as_deref(), Some("client"));
    assert!(!result.fingerprint.contains("/Users/alice"));
}
```

In `resolver.rs`, assert Unix separators, Windows drive/case folding, segment-aware longest-prefix selection (`/repo-one` must not match `/repo`), symlink-canonicalized registered roots, and worktree aliases resolving to the selected parent project.

- [ ] **Step 2: Run focused tests and verify the intended failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml events::normalize && cargo test --manifest-path src-tauri/Cargo.toml projects::resolver`

Expected: FAIL because capture, normalization, HMAC reference, and project resolution functions are absent.

- [ ] **Step 3: Implement white-listed capture and stable references**

Define the IPC-safe capture shape explicitly:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedHookEvent {
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub occurred_at: DateTime<Utc>,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub public_fields: BTreeMap<String, ScalarValue>,
    pub sensitive_fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SafeIngressEvent {
    pub event_id: Uuid,
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub project_id: Option<ProjectId>,
    pub project_display_name: Option<String>,
    pub cwd_fingerprint: Option<String>,
    pub session_ref: Option<String>,
    pub turn_ref: Option<String>,
    pub public_fields: BTreeMap<String, ScalarValue>,
}
```

`capture_hook_json` selects only fields declared by the matching capability. Forbidden fields are discarded before constructing `CapturedHookEvent`; unknown fields are ignored. Use HMAC-SHA256 with the local correlation key for session, turn, and unmatched-cwd references. Never run `git` or access the keyring in this path.

- [ ] **Step 4: Implement cross-platform project matching**

```rust
pub fn resolve_project(
    cwd: &Path,
    projects: &[ProjectRegistration],
    platform: PathPlatform,
) -> ProjectMatch
```

Normalize separators, remove `.` segments, compare case-insensitively only for Windows, require whole path-segment prefixes, and choose the registered root/alias with the greatest component count. Canonicalize roots when the user registers them and store that canonical value; do not call filesystem canonicalization from the Hook helper.

- [ ] **Step 5: Run tests and inspect fixtures for forbidden values**

Run: `cargo test --manifest-path src-tauri/Cargo.toml events::normalize && cargo test --manifest-path src-tauri/Cargo.toml projects::resolver`

Run: `rg -n 'real|Bearer|access_token|BEGIN .*PRIVATE KEY|/Users/[^a]|[A-Za-z]:\\Users' tests/fixtures`

Expected: Rust tests pass; the fixture scan has no real credentials or personal absolute paths (the explicit fake `/Users/alice/...` unit-test literal remains in Rust source, not fixtures).

- [ ] **Step 6: Commit normalization and matching**

```bash
git add src-tauri/src/events src-tauri/src/projects src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock tests/fixtures
git commit -m "feat: normalize Hook events and resolve projects"
```

### Task 4: Resolve Global Rules, Project Patches, Filters, and Timing Policies

**Files:**
- Create: `src-tauri/src/rules/mod.rs`
- Create: `src-tauri/src/rules/resolve.rs`
- Create: `src-tauri/src/rules/policy.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: unit tests colocated in `resolve.rs` and `policy.rs`

**Interfaces:**
- Consumes: `RuleConfig`, `RulePatch`, `EventEnvelope`, and `HookCapability` from Tasks 2-3.
- Produces: `resolve_rule(global: &RuleConfig, patch: Option<&RulePatch>) -> RuleConfig`, `resolve_stored_rule(global: &StoredGlobalRule, patch: Option<&StoredRulePatch>) -> ResolvedRule`, `matches_filters(event: &EventEnvelope, filters: &FilterGroup) -> bool`, `evaluate_policy(input: &PolicyInput) -> PolicyDecision`, `required_hook_selection(global: &[StoredGlobalRule], overrides: &[StoredRulePatch]) -> BTreeSet<(AgentKind, String)>`, and `default_rule(agent: AgentKind, event: &str) -> RuleConfig`.

- [ ] **Step 1: Write failing inheritance and selection tests**

```rust
#[test]
fn empty_targets_override_instead_of_inherit() {
    let global = enabled_rule_with_targets(vec![target(1), target(2)]);
    let patch = RulePatch { targets: Some(vec![]), ..RulePatch::default() };
    assert_eq!(resolve_rule(&global, Some(&patch)).targets, Vec::new());
}

#[test]
fn double_option_can_clear_quiet_hours() {
    let global = rule_with_quiet_hours("22:00", "08:00");
    let patch = RulePatch { quiet_hours: Some(None), ..RulePatch::default() };
    assert_eq!(resolve_rule(&global, Some(&patch)).quiet_hours, None);
}

#[test]
fn quiet_hours_json_distinguishes_missing_from_explicit_null() {
    let inherited: RulePatch = serde_json::from_value(json!({})).unwrap();
    let cleared: RulePatch = serde_json::from_value(json!({"quiet_hours": null})).unwrap();
    assert_eq!(inherited.quiet_hours, None);
    assert_eq!(cleared.quiet_hours, Some(None));
}

#[test]
fn project_enablement_requires_hook_installation() {
    let selected = required_hook_selection(&[], &[stored_patch(AgentKind::Codex, "PostToolUse", Some(true))]);
    assert!(selected.contains(&(AgentKind::Codex, "PostToolUse".into())));
}
```

In the same module, assert that absent patch fields inherit, each present field replaces atomically, a project `enabled: Some(false)` disables a globally enabled rule, and globally/project-enabled events are selected exactly once.

- [ ] **Step 2: Write failing filter, quiet, cooldown, aggregation, and TTL tests**

```rust
#[test]
fn permission_requests_never_aggregate() {
    let input = policy_input("PermissionRequest", DeliveryMode::Aggregate { window_seconds: 60 });
    assert_eq!(evaluate_policy(&input), PolicyDecision::SendNow);
}

#[test]
fn expired_offline_event_is_not_sent_under_new_rules() {
    let input = policy_input_occurred_minutes_ago("Stop", 31, 1800);
    assert_eq!(evaluate_policy(&input), PolicyDecision::Expire);
}

#[test]
fn offline_event_that_occurred_during_global_pause_is_suppressed() {
    let input = policy_input_with_pause(
        occurred_at("2026-07-29T14:30:00+08:00"),
        pause("2026-07-29T14:00:00+08:00", "2026-07-29T15:00:00+08:00"),
    );
    assert_eq!(evaluate_policy(&input), PolicyDecision::Suppress(SuppressReason::GlobalPause));
}
```

Cover AND across non-empty filter dimensions, OR within each dimension, empty-dimension wildcard, overnight quiet ranges, weekday selection, severity bypass, suppress versus defer, cooldown, per-window cap, and aggregate window key.

- [ ] **Step 3: Run rule tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml rules::`

Expected: FAIL because resolution and policy functions do not exist.

- [ ] **Step 4: Implement field-level merge and deterministic policy order**

`resolve_rule` must clone the global rule and replace only fields whose patch options are present. `resolve_stored_rule` returns the global rule ID, merged config, and a lowercase SHA-256 version of length-prefixed global rule ID, optional project ID, and canonical serde JSON of the effective `RuleConfig`. This version changes whenever effective behavior changes and remains stable when a patch is removed/recreated with identical behavior. `evaluate_policy` returns exactly one of:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    SendNow,
    Aggregate { bucket_key: String, release_at: DateTime<Utc> },
    DeferUntil(DateTime<Utc>),
    Suppress(SuppressReason),
    Expire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressReason {
    UnsupportedCapability,
    Disabled,
    FilterMismatch,
    GlobalPause,
    QuietHours,
    Cooldown,
    WindowLimit,
}

pub struct PolicyInput<'a> {
    pub event: &'a EventEnvelope,
    pub capability: &'a HookCapability,
    pub rule: &'a RuleConfig,
    pub notification_pause: Option<&'a NotificationPause>,
    pub now: DateTime<Utc>,
    pub recent_delivery_times: &'a [DateTime<Utc>],
}
```

Define the persisted rule records used by storage and Hook selection:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredGlobalRule {
    pub id: RuleId,
    pub agent: AgentKind,
    pub source_event: String,
    pub version: u64,
    pub config: RuleConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredRulePatch {
    pub project_id: ProjectId,
    pub agent: AgentKind,
    pub source_event: String,
    pub version: u64,
    pub patch: RulePatch,
}

#[derive(Clone, Debug)]
pub struct ResolvedRule {
    pub id: RuleId,
    pub version: String,
    pub config: RuleConfig,
}
```

Apply checks in this order: capability, enabled, filters, TTL, global pause interval, quiet hours, cooldown/window cap, aggregation. An active pause suppresses current processing; an offline event whose `occurred_at` falls within the retained pause interval is also suppressed after restart. Override aggregation to `SendNow` for `PermissionRequest`. Parse `QuietHours` strings strictly as `HH:MM`; reject invalid times, weekdays outside 1-7, TTL outside 1-86,400 seconds, max attempts outside 1-10, aggregate windows outside 10-3,600 seconds, cooldown/window durations above 86,400 seconds, window caps outside 1-100, body limits above 4,000 characters, and more than 20 targets with `configuration.rule_invalid`.

Defaults must enable Claude `PermissionRequest`, `Notification`, `Stop`, `StopFailure` and Codex `PermissionRequest`, `Stop`; `SessionEnd` and every high-frequency event remain disabled. Permission TTL is 600 seconds; other TTL is 1800 seconds; default retries are five. Default privacy has no `allowed_sensitive_fields`; completion/failure events use `NativeSummary` capped at 500 Unicode characters from the Hook-native last message/error after redaction, while all other events use `MetadataOnly`. Full prompt, command/tool input, file content/path, transcript, environment, and full assistant response remain opt-in or forbidden according to the capability catalog.

- [ ] **Step 5: Run all rule tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml rules::`

Expected: inheritance, validation, selection, filtering, quiet, cooldown, aggregation, and expiry tests all pass.

- [ ] **Step 6: Commit the rule engine**

```bash
git add src-tauri/src/rules src-tauri/src/lib.rs
git commit -m "feat: add inherited notification rule engine"
```

### Task 5: Enforce Privacy, Redaction, and Restricted Templates

**Files:**
- Create: `src-tauri/src/security/mod.rs`
- Create: `src-tauri/src/security/redact.rs`
- Create: `src-tauri/src/rules/template.rs`
- Modify: `src-tauri/src/rules/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `redact.rs` and `template.rs`

**Interfaces:**
- Consumes: `EventEnvelope`, `PrivacyPolicy`, `NotificationDocument`, and capability field sensitivity from Tasks 2-4.
- Produces: `Redactor::compile(patterns: &[String]) -> Result<Redactor, AppError>`, `Redactor::redact(&self, input: &str) -> String`, `build_template_context(event: &EventEnvelope, allowed_fields: &[String]) -> TemplateContext`, and `render_document(template: &str, context: &TemplateContext, redactor: &Redactor, max_chars: usize) -> Result<NotificationDocument, AppError>`.

- [ ] **Step 1: Write failing mandatory-redaction tests**

```rust
#[test]
fn removes_tokens_webhooks_private_keys_and_named_secrets() {
    let input = concat!(
        "Authorization: Bearer abc.def.ghi\n",
        "OPENAI_API_KEY=sk-test-1234567890\n",
        "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake-secret\n",
        "-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----"
    );
    let output = Redactor::compile(&[]).unwrap().redact(input);
    assert!(!output.contains("abc.def.ghi"));
    assert!(!output.contains("sk-test"));
    assert!(!output.contains("fake-secret"));
    assert!(!output.contains("BEGIN PRIVATE KEY"));
    assert!(output.contains("[REDACTED]"));
}
```

In the same redactor test, assert Anthropic/GitHub/cloud key shapes, URL parameters named `access_token`, `key`, and `secret`, and environment names containing Secret/Token/Password/Credential regardless of case.

- [ ] **Step 2: Write failing restricted-template tests**

```rust
#[test]
fn template_cannot_access_field_removed_by_privacy_policy() {
    let context = context_with_only(&[("event.label", "需要授权")]);
    let error = render_document("{{event.label}} {{event.full_prompt}}", &context, &redactor(), 500).unwrap_err();
    assert_eq!(error.code, "configuration.template_field_not_allowed");
}

#[test]
fn renders_default_native_summary_then_redacts_and_truncates() {
    let context = context_with_summary("finished with token=very-secret and extra text");
    let document = render_document(DEFAULT_TEMPLATE_ZH, &context, &redactor(), 24).unwrap();
    assert!(!document.body.contains("very-secret"));
    assert!(document.body.chars().count() <= 24);
}
```

In the same template test, assert rejection of blocks, functions, loops, unknown roots, malformed braces, templates over 16 KiB, custom patterns over 512 characters, more than 32 custom patterns, and rendered output over the supplied Unicode character limit.

- [ ] **Step 3: Run privacy tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml security::redact && cargo test --manifest-path src-tauri/Cargo.toml rules::template`

Expected: FAIL because the redactor, allow-listed context, and restricted renderer are absent.

- [ ] **Step 4: Implement bounded regex compilation and one-pass redaction**

Use Rust's linear-time `regex` engine and `RegexBuilder::size_limit(1_048_576)` for mandatory and user patterns. Reject user inputs outside the limits before compilation. Replace every match with `[REDACTED]`; never include the matched value in `AppError` or tracing fields.

The context exposes only these roots and leaf names: `agent.name`, `agent.version`, `project.name`, `event.name`, `event.label`, `event.status`, `event.severity`, `event.summary`, `event.duration`, `event.tool_name`, and `event.occurred_at`. `build_template_context` omits fields forbidden by the capability catalog or not selected by `PrivacyPolicy`. It never reads a transcript or invokes a model.

Implement a small parser that recognizes only `{{root.leaf}}`, text, and newline tokens. A missing authorized value renders as an empty string; an unauthorized or unknown path is an error. Render, redact, then truncate on Unicode scalar boundaries. Construct `NotificationDocument` with the rendered body and stable facts for Agent, project, Hook, status, and time.

- [ ] **Step 5: Run privacy and package tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml security::redact && cargo test --manifest-path src-tauri/Cargo.toml rules::template && cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all secret families are removed, invalid patterns/templates return stable redacted error codes, and authorized templates render deterministic documents.

- [ ] **Step 6: Commit privacy enforcement**

```bash
git add src-tauri/src/security src-tauri/src/rules src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add redaction and restricted templates"
```

### Task 6: Create the WAL Database, Versioned Migration, and Event Repositories

**Files:**
- Create: `migrations/0001_initial.sql`
- Create: `src-tauri/src/storage/mod.rs`
- Create: `src-tauri/src/storage/db.rs`
- Create: `src-tauri/src/storage/events.rs`
- Create: `src-tauri/src/security/permissions.rs`
- Modify: `src-tauri/src/security/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `db.rs` and `events.rs`

**Interfaces:**
- Consumes: event, rule, channel, project, error, and encrypted reference types from Tasks 2-5.
- Produces: `ensure_private_directory`, `ensure_private_file`, `ensure_current_user_dacl`, `Database::open(path: &Path) -> Result<Database, AppError>`, `Database::open_ingress_writer(path: &Path) -> Result<Connection, AppError>`, `Database::migrate()`, `EventProcessingOutcome`, `EventRepository::insert_ingress`, `EventRepository::take_ingress_batch`, `EventRepository::insert_event`, and `EventRepository::list_history`.

- [ ] **Step 1: Write failing migration and schema tests**

```rust
#[test]
fn migration_creates_every_v1_table_and_enables_wal() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(file.path()).unwrap();
    let tables = db.table_names().unwrap();
    assert_eq!(tables, BTreeSet::from([
        "agent_installations", "app_settings", "channels", "config_snapshots",
        "delivery_attempts", "delivery_jobs", "events", "global_rules",
        "hook_installations", "ingress_events", "project_paths", "project_rule_overrides",
        "projects", "schema_migrations",
    ].map(str::to_owned)));
    assert_eq!(db.pragma_string("journal_mode").unwrap(), "wal");
    assert_eq!(db.schema_version().unwrap(), 1);
}

#[test]
fn applying_migrations_twice_is_idempotent() {
    let file = tempfile::NamedTempFile::new().unwrap();
    Database::open(file.path()).unwrap();
    let reopened = Database::open(file.path()).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 1);
}
```

In `db.rs`, execute a deliberately invalid second migration against a copied test migration list, then assert that the transaction rolls back and the version remains 1.

- [ ] **Step 2: Write failing safe-ingress and history tests**

```rust
#[test]
fn ingress_round_trip_contains_safe_envelope_only() {
    let repo = test_repository();
    let input = safe_ingress_with_summary("metadata only");
    repo.insert_ingress(&input).unwrap();
    let stored = repo.take_ingress_batch(10).unwrap();
    assert_eq!(stored, vec![input]);
    let bytes = std::fs::read(repo.database_path()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("raw_prompt"));
}

#[test]
fn history_returns_redacted_documents_and_attempt_metadata() {
    let repo = repository_with_succeeded_delivery();
    let page = repo.list_history(&HistoryFilter::default(), PageRequest::first(50)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].delivery_status, DeliveryStatus::Succeeded);
    assert!(!format!("{page:?}").contains("access_token"));
}
```

- [ ] **Step 3: Run storage tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::db && cargo test --manifest-path src-tauri/Cargo.toml storage::events`

Expected: FAIL because no migration or repositories exist.

- [ ] **Step 4: Add the complete v1 schema in one transactional migration**

Use `TEXT` for UUIDs/RFC3339 timestamps/enums/JSON and `BLOB` only for nonce/ciphertext. The migration contains these exact primary and unique keys:

```sql
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);
CREATE TABLE app_settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE agent_installations (
  agent TEXT PRIMARY KEY,
  executable_path TEXT,
  version TEXT,
  capability_verification TEXT NOT NULL,
  health_status TEXT NOT NULL,
  last_checked_at TEXT NOT NULL
);
CREATE TABLE hook_installations (
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  command_fingerprint TEXT NOT NULL,
  definition_fingerprint TEXT NOT NULL,
  helper_version TEXT NOT NULL,
  config_hash TEXT NOT NULL,
  trust_status TEXT NOT NULL,
  health_status TEXT NOT NULL,
  last_seen_at TEXT,
  observed_command_fingerprint TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (agent, source_event)
);
CREATE TABLE config_snapshots (
  id TEXT PRIMARY KEY,
  agent TEXT NOT NULL,
  config_path TEXT NOT NULL,
  hook_subtree_ciphertext BLOB NOT NULL,
  nonce BLOB NOT NULL,
  aad TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  file_mode INTEGER,
  created_at TEXT NOT NULL
);
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  canonical_root TEXT NOT NULL,
  worktree_mode TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE project_paths (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  canonical_path TEXT NOT NULL,
  kind TEXT NOT NULL,
  UNIQUE(canonical_path)
);
CREATE TABLE channels (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  credential_ref TEXT NOT NULL UNIQUE,
  public_config_json TEXT NOT NULL,
  health_status TEXT NOT NULL,
  paused_reason_code TEXT,
  consecutive_auth_failures INTEGER NOT NULL DEFAULT 0,
  last_succeeded_at TEXT,
  next_allowed_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE global_rules (
  id TEXT PRIMARY KEY,
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  version INTEGER NOT NULL,
  config_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(agent, source_event)
);
CREATE TABLE project_rule_overrides (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  agent TEXT NOT NULL,
  source_event TEXT NOT NULL,
  version INTEGER NOT NULL,
  patch_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(project_id, agent, source_event)
);
CREATE TABLE ingress_events (
  id TEXT PRIMARY KEY,
  safe_envelope_json TEXT NOT NULL,
  received_at TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','processing'))
);
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  source_version TEXT NOT NULL,
  source_event TEXT NOT NULL,
  category TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  received_at TEXT NOT NULL,
  project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
  project_display_name TEXT,
  unmatched_cwd_fingerprint TEXT,
  session_ref TEXT,
  turn_ref TEXT,
  model TEXT,
  permission_mode TEXT,
  severity TEXT NOT NULL,
  public_fields_json TEXT NOT NULL,
  sensitive_blob_id TEXT,
  sensitive_fields_blob BLOB,
  correlation_id TEXT NOT NULL,
  action_id TEXT,
  action_capabilities_json TEXT NOT NULL,
  processing_outcome TEXT NOT NULL,
  outcome_reason_code TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE delivery_jobs (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  rule_id TEXT NOT NULL,
  rule_version TEXT NOT NULL,
  channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  idempotency_key TEXT NOT NULL UNIQUE,
  document_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','sending','retry_wait','succeeded','failed','expired')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  lease_owner TEXT,
  lease_expires_at TEXT,
  aggregate_key TEXT,
  aggregate_release_at TEXT,
  last_error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE delivery_attempts (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES delivery_jobs(id) ON DELETE CASCADE,
  attempt_number INTEGER NOT NULL,
  started_at TEXT NOT NULL,
  completed_at TEXT NOT NULL,
  outcome TEXT NOT NULL,
  http_status INTEGER,
  platform_code TEXT,
  error_code TEXT,
  retry_at TEXT,
  redacted_detail TEXT,
  UNIQUE(job_id, attempt_number)
);
```

Add indexes for ingress state/time, event occurred time/project/source event, due jobs `(state, next_attempt_at, lease_expires_at)`, attempts by job, and project paths by project. Enable foreign keys, WAL, `synchronous=NORMAL`, and a 2-second desktop busy timeout on every connection.

- [ ] **Step 5: Implement migration and typed repositories**

Embed migrations with `include_str!("../../../migrations/0001_initial.sql")`. Apply each migration and its `schema_migrations` insert in one immediate transaction. `open_ingress_writer` uses a 20 ms busy timeout and refuses to migrate; it returns `storage.schema_unavailable` if version 1 is absent so the helper can use spool.

Repositories serialize only typed safe structures. `insert_event` serializes the typed per-field nonce/ciphertext map to bytes and binds it as `sensitive_fields_blob`; it never accepts raw Hook JSON or plaintext sensitive fields. Define `EventProcessingOutcome` as the closed serde enum `Queued | Suppressed | Expired | NoTargets` plus a stable optional reason code on the stored row, so history can explain non-delivery without retaining raw rule input. All list APIs take bounded pagination (`1..=200`) and return command DTOs with no credential or ciphertext fields.

On Unix, `ensure_private_directory` applies `0700` and `ensure_private_file` applies `0600`. On Windows, `ensure_current_user_dacl` builds and verifies a DACL containing the current SID and required system/owner entries but no Everyone/Users write access; call it before the first database connection. Unit tests inspect the resulting mode/security descriptor on their native target.

- [ ] **Step 6: Run storage tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::db && cargo test --manifest-path src-tauri/Cargo.toml storage::events`

Expected: migrations are atomic and idempotent, WAL/foreign keys are active, safe events round-trip, invalid pagination is rejected, and history contains only redacted DTOs.

- [ ] **Step 7: Commit database foundations**

```bash
git add migrations src-tauri/src/storage src-tauri/src/security src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add WAL storage and event repositories"
```

### Task 6B: Persist Configuration and Integration State

**Files:**
- Create: `src-tauri/src/storage/config.rs`
- Create: `src-tauri/src/storage/integrations.rs`
- Modify: `src-tauri/src/storage/mod.rs`
- Modify: `src-tauri/src/model.rs`
- Test: unit tests colocated in `config.rs` and `integrations.rs`

**Interfaces:**
- Consumes: migrated `Database`, both capability catalogs, `default_rule`, stored rule records, project/channel IDs, and encrypted snapshot bytes.
- Produces: `ConfigRepository`, `IntegrationRepository`, `ProjectRecord`, `ProjectPathRecord`, `ProjectMatchCacheFile`, `ChannelRecord`, `ChannelPublicConfig`, `AppSettings`, `AgentInstallationRecord`, `HookInstallationRecord`, and `ConfigSnapshotRecord`.

- [ ] **Step 1: Write failing default-rule seed and preservation tests**

```rust
#[test]
fn first_start_seeds_one_complete_global_rule_per_catalogued_hook() {
    let repository = test_config_repository();
    let report = repository.ensure_global_rules(&verified_catalogs()).unwrap();
    assert_eq!(report.inserted, 41);
    let rules = repository.list_global_rules().unwrap();
    assert_eq!(rules.len(), 41);
    assert_eq!(rules.iter().filter(|rule| rule.config.enabled).count(), 6);
}

#[test]
fn catalog_refresh_adds_missing_rules_without_overwriting_user_configuration() {
    let repository = seeded_config_repository();
    repository.save_global_rule(customized_stop_rule()).unwrap();
    repository.ensure_global_rules(&catalogs_with_one_new_event()).unwrap();
    assert_eq!(repository.get_global_rule(AgentKind::Codex, "Stop").unwrap(), customized_stop_rule());
    assert!(!repository.get_global_rule(AgentKind::Codex, "NewCatalogEvent").unwrap().config.enabled);
}
```

- [ ] **Step 2: Write failing rule-patch, project, channel, and settings tests**

```rust
#[test]
fn project_patch_round_trip_preserves_explicit_quiet_clear_and_reset_field() {
    let repository = seeded_config_repository();
    repository.save_project_patch(project_id(), AgentKind::Codex, "Stop", &patch_clearing_quiet_hours()).unwrap();
    assert_eq!(repository.get_project_patch(project_id(), AgentKind::Codex, "Stop").unwrap().patch.quiet_hours, Some(None));
    repository.reset_project_patch_field(project_id(), AgentKind::Codex, "Stop", PatchField::QuietHours).unwrap();
    assert_eq!(repository.get_project_patch(project_id(), AgentKind::Codex, "Stop").unwrap().patch.quiet_hours, None);
}

#[test]
fn channel_storage_accepts_only_public_config_and_opaque_credential_reference() {
    let repository = test_config_repository();
    repository.save_channel(&channel_record("cc-reminder/channel/fake-id")).unwrap();
    let bytes = std::fs::read(repository.database_path()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("access_token="));
}
```

In the same module, assert project/path cascade, canonical-path uniqueness, worktree/alias kinds, rule version increment, an explicit empty target list, settings defaults/bounds, channel reference uniqueness, and deletion refusal while an active rule targets the channel.

- [ ] **Step 3: Write failing integration-state and encrypted-snapshot tests**

```rust
#[test]
fn observed_hook_updates_only_the_matching_expected_fingerprint() {
    let repository = integration_repository_with_two_hooks();
    repository.mark_hook_seen(AgentKind::Codex, "Stop", "expected-fingerprint", now()).unwrap();
    assert_eq!(repository.hook(AgentKind::Codex, "Stop").unwrap().trust_status, TrustStatus::ObservedWorking);
    assert_eq!(repository.hook(AgentKind::Codex, "PermissionRequest").unwrap().trust_status, TrustStatus::NeedsUserConfirmation);
}

#[test]
fn snapshot_repository_has_no_plaintext_hook_subtree_api() {
    let repository = test_integration_repository();
    repository.save_snapshot(&encrypted_snapshot_fixture()).unwrap();
    let stored = repository.latest_snapshot(AgentKind::ClaudeCode).unwrap();
    assert_eq!(stored.ciphertext, encrypted_snapshot_fixture().ciphertext);
    assert!(!format!("{stored:?}").contains("foreign hook command"));
}
```

Also assert Agent detection upsert, Hook selection replacement in one transaction, helper version/config hash/definition fingerprint persistence, last-seen timestamp, command-fingerprint mismatch leaving trust unchanged, and bounded snapshot retention (latest five per Agent).

- [ ] **Step 4: Run configuration repository tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::config && cargo test --manifest-path src-tauri/Cargo.toml storage::integrations`

Expected: FAIL because typed configuration/integration repositories do not exist.

- [ ] **Step 5: Implement typed records and transactional repositories**

Use these non-secret public records:

```rust
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind { DingTalk, WeCom }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelPublicConfig {
    DingTalk { keyword_prefix: Option<String> },
    WeCom,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelHealth { Unknown, Healthy, PausedAuthentication, Error }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustStatus { NotRequired, NeedsUserConfirmation, ObservedWorking }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Locale { ZhCn, En }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme { System, Light, Dark }

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchField { Enabled, Targets, Filters, Privacy, Delivery, QuietHours }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub id: ChannelId,
    pub kind: ChannelKind,
    pub name: String,
    pub credential_ref: String,
    pub public_config: ChannelPublicConfig,
    pub health_status: ChannelHealth,
    pub paused_reason_code: Option<String>,
    pub consecutive_auth_failures: u8,
    pub last_succeeded_at: Option<DateTime<Utc>>,
    pub next_allowed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub autostart: bool,
    pub close_to_tray: bool,
    pub locale: Locale,
    pub theme: Theme,
    pub event_retention_days: u16,
    pub log_retention_days: u16,
    pub notification_pause: Option<NotificationPause>,
    pub debug_until: Option<DateTime<Utc>>,
    pub onboarding_completed: bool,
}

```

`ProjectRecord` contains ID, name, canonical root, worktree mode, and timestamps; `ProjectPathRecord` contains ID, project ID, canonical path, and `Root | Alias | Worktree`. `ProjectMatchCacheFile` is a versioned, 1 MiB-bounded list of project ID/display name/canonical registered paths written atomically with user-only permissions after every committed project/path mutation. Define installation health and snapshot ciphertext/nonce/AAD records as closed serde enums/structs, never free-form status strings at repository boundaries.

`ConfigRepository` validates referenced capabilities/channels/projects and performs each rule/project/channel/settings mutation in one immediate transaction. `ensure_global_rules` inserts only absent `(agent, source_event)` keys at version 1. Every global rule save and project patch save/reset increments that row's integer version. `reset_project_patch_field` deserializes the typed patch, clears exactly one `PatchField`, and deletes the row if every patch field becomes absent. After a project transaction commits, regenerate `project-paths.json` through a same-directory temp file, sync, and atomic rename; cache failure leaves database state valid but raises a health issue until regeneration succeeds.

`IntegrationRepository` stores typed Agent/Hook health plus separate command and serialized-definition fingerprints, accepts snapshots only as encrypted bytes, marks observed trust only on an exact expected command fingerprint, and prunes snapshots beyond five per Agent after successful insertion. Its read DTOs omit snapshot ciphertext unless the installer explicitly requests recovery data.

- [ ] **Step 6: Run repository and schema tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::config && cargo test --manifest-path src-tauri/Cargo.toml storage::integrations && cargo test --manifest-path src-tauri/Cargo.toml storage::db`

Expected: defaults, non-destructive catalog refresh, patch reset, project/channel/settings constraints, Hook observation, and encrypted snapshot retention pass.

- [ ] **Step 7: Commit configuration repositories**

```bash
git add src-tauri/src/storage src-tauri/src/model.rs
git commit -m "feat: persist notification configuration state"
```

### Task 7: Store Credentials Securely and Encrypt Opt-In Sensitive Fields

**Files:**
- Create: `src-tauri/src/security/credentials.rs`
- Create: `src-tauri/src/security/crypto.rs`
- Modify: `src-tauri/src/security/mod.rs`
- Modify: `src-tauri/src/storage/events.rs`
- Modify: `src-tauri/src/model.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `credentials.rs` and `crypto.rs`

**Interfaces:**
- Consumes: `EncryptedBlobRef`, event storage, channel IDs, and `AppError`.
- Produces: `CredentialStore`, `CredentialPayload`, `CredentialAvailability`, `FieldCipher`, `EncryptedFields`, and `CorrelationKey::load_or_create(data_dir: &Path) -> Result<CorrelationKey, AppError>`.

- [ ] **Step 1: Write failing credential behavior tests with the test-only memory backend**

```rust
#[test]
fn saved_credential_returns_only_an_opaque_reference() {
    let store = CredentialStore::memory_for_test();
    let reference = store.put(channel_id(), &wecom_payload("fake-key")).unwrap();
    assert!(reference.starts_with("cc-reminder/channel/"));
    let loaded = store.get(&reference).unwrap();
    assert_eq!(loaded.expose_wecom_webhook_for_use().expose_secret(), "fake-key");
    assert!(!format!("{reference:?}").contains("fake-key"));
}

#[test]
fn unavailable_secure_storage_refuses_persistence() {
    let store = CredentialStore::unavailable_for_test("Secret Service unavailable");
    let error = store.put(channel_id(), &wecom_payload("fake-key")).unwrap_err();
    assert_eq!(error.code, "secret_store.unavailable");
    assert!(!format!("{error:?}").contains("fake-key"));
}
```

Also verify replace uses the same opaque reference, delete removes the secret, and no command DTO serializes `CredentialPayload`.

- [ ] **Step 2: Write failing authenticated-encryption tests**

```rust
#[test]
fn sensitive_fields_round_trip_only_with_matching_event_and_field_aad() {
    let cipher = FieldCipher::from_key([4_u8; 32]);
    let event_id = Uuid::now_v7();
    let encrypted = cipher.encrypt_fields(event_id, &BTreeMap::from([("prompt".into(), "secret text".into())])).unwrap();
    assert_eq!(cipher.decrypt_fields(event_id, &encrypted).unwrap()["prompt"], "secret text");
    assert!(cipher.decrypt_fields(Uuid::now_v7(), &encrypted).is_err());
}

#[test]
fn correlation_key_file_is_random_and_not_credential_encryption_material() {
    let directory = tempfile::tempdir().unwrap();
    let first = CorrelationKey::load_or_create(directory.path()).unwrap();
    let second = CorrelationKey::load_or_create(directory.path()).unwrap();
    assert_eq!(first.expose_for_hmac(), second.expose_for_hmac());
    assert_ne!(first.expose_for_hmac(), &[0_u8; 32]);
}
```

- [ ] **Step 3: Run security tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml security::credentials && cargo test --manifest-path src-tauri/Cargo.toml security::crypto`

Expected: FAIL because secure-store and cipher wrappers do not exist.

- [ ] **Step 4: Implement the platform credential wrapper without plaintext fallback**

Use the `keyring` crate with service `cc-reminder` and username `channel/<uuid>`. Keep the public credential value non-serializable:

```rust
#[derive(Clone)]
pub enum CredentialPayload {
    DingTalk { webhook: secrecy::SecretString, signing_secret: Option<secrecy::SecretString> },
    WeCom { webhook: secrecy::SecretString },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAvailability { Available, Unavailable { reason_code: String } }
```

Implement a manual redacted `Debug` for `CredentialPayload`. Encode/decode through a private tagged `CredentialRecord` containing ordinary strings only inside `credentials.rs`; expose each `SecretString` only while building that temporary record, zeroize the serialized buffer after the keyring call, and never implement `Serialize` for `CredentialPayload` itself. `CredentialStore` contains a system backend in production and an internal `#[cfg(test)]` memory/unavailable mode, not a plugin interface. Map missing Linux Secret Service and locked/unavailable stores to stable redacted errors. Never log payload serialization or return it through a Tauri command.

- [ ] **Step 5: Implement XChaCha20-Poly1305 field encryption**

Store a random 256-bit data key in OS secure storage at `cc-reminder/data-key`. Encrypt each selected field with a random 192-bit nonce and AAD bytes `cc-reminder:event:<event_uuid>:field:<field_name>`. `EncryptedFields` stores a bounded map from field name to nonce/ciphertext, serializes that encrypted-only map with `serde_json::to_vec` into the event BLOB, and exposes only its `blob_id` outside the security/storage layer.

Snapshots use the same cipher with AAD `cc-reminder:snapshot:<snapshot_uuid>:hooks`; do not reuse a nonce. Correlation uses a separate 256-bit random file `correlation.key` created with exclusive creation and user-only permissions. The helper may read only this correlation key and never reads the keyring data key.

- [ ] **Step 6: Run tests and scan the database bytes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml security::credentials && cargo test --manifest-path src-tauri/Cargo.toml security::crypto && cargo test --manifest-path src-tauri/Cargo.toml storage::events`

Run an integration test that saves `known-sensitive-plaintext-4197`, closes SQLite, reads the database/WAL bytes, and asserts the marker is absent.

Expected: authentication fails with altered AAD/ciphertext, nonce values differ across writes, unavailable secure storage rejects writes, and plaintext markers are absent from database files.

- [ ] **Step 7: Commit credential and encryption support**

```bash
git add src-tauri/src/security src-tauri/src/storage/events.rs src-tauri/src/model.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: secure credentials and sensitive event fields"
```

### Task 8: Build the Bounded Hook Helper, User-Only IPC, and Safe Spool Fallback

**Files:**
- Create: `src-tauri/src/ipc/mod.rs`
- Create: `src-tauri/src/ipc/protocol.rs`
- Create: `src-tauri/src/ipc/client.rs`
- Create: `src-tauri/src/ipc/server.rs`
- Create: `src-tauri/src/storage/spool.rs`
- Create: `src-tauri/src/hook_command.rs`
- Create: `src-tauri/src/paths.rs`
- Create: `src-tauri/src/bin/cc-reminder-hook.rs`
- Create: `src-tauri/tests/hook_contract.rs`
- Modify: `src-tauri/src/storage/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `CapturedHookEvent`, `SafeIngressEvent`, correlation key, `Database::open_ingress_writer`, and stable errors.
- Produces: `AppPaths::discover`, `HookCommand`, `canonical_hook_command`, `command_fingerprint`, `IngressRequest`, `IngressResponse`, `AgentVersionCacheFile`, `IpcServer::bind`, `send_ingress(endpoint: &Endpoint, request: &IngressRequest)`, `Spool::write_exclusive`, `Spool::drain`, and the `cc-reminder-hook --owner cc-reminder --agent <agent> --event <event>` executable contract.

- [ ] **Step 1: Write failing helper contract tests**

```rust
#[test]
fn valid_hook_invocation_is_neutral_even_when_every_sink_is_unavailable() {
    let environment = HookTestEnvironment::with_unavailable_sinks();
    let output = environment.run_helper_with_args(
        ["--owner", "cc-reminder", "--agent", "codex", "--event", "Stop"],
        br#"{"session_id":"raw-session-id","cwd":"/private/client"}"#,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
}

#[test]
fn oversized_or_too_deep_input_never_reaches_spool() {
    let environment = HookTestEnvironment::new();
    let output = environment.run_helper(vec![b'x'; MAX_HOOK_BYTES + 1]);
    assert!(output.status.success());
    assert_eq!(environment.spool_entries(), 0);
    assert_eq!(environment.ingress_rows(), 0);
}
```

In the same contract test, assert invalid owner/Agent/event arguments, mismatch between CLI event and JSON `hook_event_name`, invalid JSON, wrong top-level type, more than 256 fields, nesting deeper than 32, more than 4,096 nodes, unknown fields, missing source/cache version, missing correlation key, offline project-cache match, project-cache miss, a 64 KiB safe-envelope limit, a 4,096-file spool cap, IPC success, IPC unavailable plus SQLite success, SQLite busy plus spool success, and all sinks unavailable. Missing version creates no event; missing correlation key may create a safe event only after raw IDs/cwd fingerprint are omitted. A project-cache hit persists project ID/display name but not cwd; a miss persists only the leaf/fingerprint. In every case assert bounded stdout/stderr, no raw Hook JSON in files, and no helper attempt to launch the desktop binary.

- [ ] **Step 2: Write failing IPC/spool permission and recovery tests**

```rust
#[test]
fn spool_contains_only_safe_metadata_and_drains_once() {
    let spool = test_spool();
    let raw_secret = "prompt body never persisted";
    spool.write_exclusive(&safe_event("Stop")).unwrap();
    let file_bytes = std::fs::read(spool.only_entry()).unwrap();
    assert!(!String::from_utf8_lossy(&file_bytes).contains(raw_secret));
    let drained = spool.drain(100).unwrap();
    assert_eq!(drained.len(), 1);
    assert!(spool.entries().unwrap().is_empty());
}
```

On Unix assert endpoint/spool permissions are socket/file `0600` and directory `0700`. On Windows assert the security descriptor grants the current SID and denies broad principals such as Everyone.

In `hook_command.rs`, assert that `/Users/a b/it's/bin/cc-reminder-hook` is POSIX-quoted without executing the apostrophe, `C:\Users\a & b\cc-reminder-hook.exe` remains one Windows executable token, and installer/helper calls over the same canonical path/arguments produce identical fingerprints.

In `paths.rs`, inject macOS/Windows/Linux data roots and assert every platform appends the exact identifier `com.ccreminder.app`, with deterministic children `cc-reminder.sqlite3`, `spool`, `logs`, `bin`, `agent-versions.json`, `project-paths.json`, and `correlation.key`. Unix uses `<data>/ipc/hook.sock`; Windows uses `\\.\pipe\cc-reminder-<first-16-hex-of-SHA256-current-SID>` so the name contains no raw SID.

- [ ] **Step 3: Run the contract tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract`

Expected: FAIL because the helper target, protocol, IPC, and spool do not exist.

- [ ] **Step 4: Implement bounded capture and neutral output**

Set constants explicitly:

```rust
pub const IPC_PROTOCOL_VERSION: u16 = 1;
pub const MAX_HOOK_BYTES: usize = 1_048_576;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_JSON_FIELDS: usize = 256;
pub const MAX_JSON_NODES: usize = 4_096;
pub const MAX_SAFE_ENVELOPE_BYTES: usize = 65_536;
pub const MAX_SPOOL_FILES: usize = 4_096;
pub const IPC_CONNECT_TIMEOUT: Duration = Duration::from_millis(35);
pub const IPC_TOTAL_TIMEOUT: Duration = Duration::from_millis(75);
```

`AppPaths::discover` uses `directories::BaseDirs::data_dir()` plus `com.ccreminder.app`, matching `tauri.conf.json`. Add a Cargo feature `test-support = []`; only builds with that feature honor `CC_REMINDER_TEST_DATA_DIR`, and process-level contract tests are compiled only with that feature. Release/default builds accept no data-directory CLI argument or environment override.

Require the literal owner `cc-reminder`, one known `AgentKind`, and a CLI event present in that Agent's bounded union of embedded catalog names before reading stdin. Read stdin through `Read::take((MAX_HOOK_BYTES + 1) as u64)` and reject oversize before deserialization. Parse exactly one JSON value with `serde_json::Deserializer`'s default recursion protection still enabled, require EOF, require a top-level object, require `hook_event_name` to equal the fixed CLI event when that input field is present, then iteratively walk the value and reject depth over 32, more than 256 total object fields, or more than 4,096 total nodes. Take `source_version` from the Agent's known version field when present; otherwise read the last detected version from a user-only, 16 KiB-bounded `agent-versions.json`. Define its exact serde shape now as `AgentVersionCacheFile { schema_version: 1, agents: BTreeMap<AgentKind, CachedAgentVersion> }`, where `CachedAgentVersion` contains semantic version and RFC3339 detection time only. Task 9 writes it atomically. Select the versioned catalog only after resolving this version, require the CLI event to exist in that selected catalog, and only then call `capture_hook_json` to retain its declared fields and discard unknown/forbidden fields. If neither source supplies a valid version, drop the notification attempt rather than inventing a version, then return neutral success. Write exactly the selected catalog's neutral response; currently both verified catalogs use `{}` plus newline. Load or exclusively create the separate random correlation key with user-only permissions; if that fails, omit session/turn/cwd references from the safe envelope instead of persisting raw identifiers. The outer `main` catches every error, writes nothing to stderr, writes neutral stdout, and exits `0`; helper failures are reflected later through spool/rejected/last-seen health rather than synchronous diagnostics.

- [ ] **Step 5: Implement framed local IPC and current-user permissions**

On Unix use `std::os::unix::net::{UnixListener, UnixStream}` on a dedicated blocking accept thread. On Windows use `CreateNamedPipeW`/`ConnectNamedPipe` for the server and `CreateFileW` for the client, passing the current-user-only `SECURITY_ATTRIBUTES` created by Task 6 at pipe creation. Frame one request as a 4-byte big-endian length followed by JSON; reject frames over `MAX_HOOK_BYTES`. The protocol is:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngressRequest {
    pub protocol_version: u16,
    pub helper_version: String,
    pub command_fingerprint: String,
    pub event: CapturedHookEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IngressResponse { Accepted { event_id: Uuid }, Rejected { error_code: String } }
```

Bind a per-user endpoint under the application data directory. On Unix set parent `0700` and socket `0600`; on Windows verify the created Named Pipe DACL allows the current SID plus required system entries and excludes broad write principals. Refuse protocol-version mismatches without persisting input. The blocking transport hands accepted requests through a bounded Tokio channel to the injected async callback; it does not run the delivery worker on the connection thread.

Define `HookCommand { command: String, command_windows: Option<String> }` in `hook_command.rs`. `canonical_hook_command` uses tested POSIX single-quote escaping on Unix and tested Windows command-line quoting on Windows for the canonical helper path plus the exact fixed arguments (`--owner`, `--agent`, `--event`); no Hook payload or user template value contributes. `command_fingerprint` hashes each canonical string as a big-endian length followed by UTF-8 bytes with SHA-256. The helper regenerates this exact structure from `current_exe` and its validated fixed arguments; Task 10 uses the same functions when writing config.

- [ ] **Step 6: Implement SQLite then exclusive spool fallback**

When IPC fails, load the user-only, 1 MiB-bounded `project-paths.json` and use Task 3's pure path resolver against cwd in memory; never invoke Git. Convert immediately to `SafeIngressEvent`, retaining only matched project ID/display name or unmatched leaf/HMAC fingerprint while dropping all `sensitive_fields`, raw cwd, and raw IDs. Reject a serialized safe envelope over 64 KiB. Attempt the 20 ms ingress writer. If it is busy, missing schema, or inaccessible, count at most 4,097 directory entries and drop the new safe event when 4,096 spool files already exist; otherwise create `<uuid-v7>.json.tmp` with `create_new(true)`, user-only permissions, `sync_all`, then atomically rename to `<uuid-v7>.json`.

`Spool::drain(limit)` claims by atomic rename to `.processing`, validates each safe envelope, inserts it into `ingress_events`, then deletes the claimed file. Invalid files move to a user-only `rejected/` directory with only a hashed filename and stable error logged; never echo contents. Startup drains at most 500 entries per pass.

At startup, reclaim orphaned `.processing` spool files before claiming new `.json` files; exclusive filenames and event UUID uniqueness make this replay idempotent. A crash after database insert but before spool delete therefore produces no duplicate ingress row.

- [ ] **Step 7: Run contracts and a release-build latency smoke test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract`

Run: `cargo build --release --manifest-path src-tauri/Cargo.toml --bin cc-reminder-hook`

Run the contract harness for 500 local IPC invocations and assert the measured p95 is below 100 ms on the current machine; record but do not fail CI on shared runners, while failing dedicated release smoke jobs.

Expected: all paths return neutral success, only white-listed data crosses IPC, offline persistence is safe, and permissions tests pass on their target OS.

- [ ] **Step 8: Commit the Hook ingress path**

```bash
git add src-tauri/src/ipc src-tauri/src/storage src-tauri/src/bin src-tauri/src/hook_command.rs src-tauri/src/paths.rs src-tauri/src/lib.rs src-tauri/tests/hook_contract.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add safe Hook ingress and offline spool"
```

### Task 9: Detect Agent Executables, Versions, and Capability Health

**Files:**
- Create: `src-tauri/src/agents/mod.rs`
- Create: `src-tauri/src/agents/detect.rs`
- Create: `src-tauri/src/agents/claude.rs`
- Create: `src-tauri/src/agents/codex.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `detect.rs`, `claude.rs`, and `codex.rs`

**Interfaces:**
- Consumes: `AgentKind`, capability resolution, storage records, and stable errors.
- Produces: the `AgentIntegration` trait from design section 7.1, `ClaudeIntegration`, `CodexIntegration`, `Detection`, `AgentVersionCache`, `HookSelection`, `Installation`, and `HookHealth`.

- [ ] **Step 1: Write failing version parser and candidate-order tests**

```rust
#[test]
fn parses_current_agent_version_outputs() {
    assert_eq!(parse_version(AgentKind::ClaudeCode, "2.1.218 (Claude Code)").unwrap(), Version::new(2, 1, 218));
    assert_eq!(parse_version(AgentKind::Codex, "codex-cli 0.145.0").unwrap(), Version::new(0, 145, 0));
}

#[test]
fn explicit_configured_path_precedes_path_and_known_locations() {
    let candidates = executable_candidates(AgentKind::Codex, Some(Path::new("/chosen/codex")), &test_environment());
    assert_eq!(candidates[0], PathBuf::from("/chosen/codex"));
    assert_eq!(deduplicated(&candidates), candidates);
}
```

Cover invalid output, process failure, two-second timeout, symlink resolution, filenames `claude`/`claude.exe` and `codex`/`codex.exe`, and known user install directories on all three OS families.

- [ ] **Step 2: Write failing integration health tests**

```rust
#[test]
fn unknown_major_is_visible_but_install_is_blocked() {
    let integration = CodexIntegration::with_detection(detected("9.0.0"));
    let capability = integration.capabilities(&Version::new(9, 0, 0));
    assert_eq!(capability.verification, CatalogVerification::UpgradeRequired);
    assert_eq!(integration.validate_install_version(false).unwrap_err().code, "integration.agent_upgrade_required");
}
```

In the same Agent test modules, assert exact and compatible-unverified states, missing executable state, and automatic re-detection updating the database without rewriting any Agent config.

- [ ] **Step 3: Run Agent tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agents::`

Expected: FAIL because Agent integrations and detection are absent.

- [ ] **Step 4: Implement bounded executable detection**

Search in order: user-configured path, current `PATH`, then exact known locations. On macOS/Linux try `~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin`, `~/.volta/bin`, `~/.asdf/shims`, `~/.bun/bin`, `/opt/homebrew/bin`, and `/usr/local/bin`; on Windows try `%USERPROFILE%\.local\bin`, `%APPDATA%\npm`, `%USERPROFILE%\scoop\shims`, and `%LOCALAPPDATA%\Microsoft\WinGet\Links`, appending only the Agent's documented executable/`.cmd` names. Deduplicate canonical paths and never enumerate parent directories or scan disks.

Invoke `<candidate> --version` without a shell and close stdin. Read stdout/stderr concurrently through separate `Read::take(32 * 1024)` threads so a noisy process cannot fill a pipe or memory, terminate after two seconds using `wait-timeout`, kill on timeout, then join both bounded readers.

Return:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detection {
    pub agent: AgentKind,
    pub executable_path: Option<PathBuf>,
    pub version: Option<Version>,
    pub capability_verification: Option<CatalogVerification>,
    pub state: DetectionState,
    pub checked_at: DateTime<Utc>,
}
```

Do not include full process output in errors. Store the selected path/version/health, then emit a typed health update consumed by the GUI. After every successful detect, atomically write a bounded `agent-versions.json` map under the user data directory with Agent kind, semantic version, and detection time; apply current-user-only permissions. This file contains no executable path. The helper reads it only as a fallback when Hook input omits a version, so an offline event can carry the last observed Agent version without launching the Agent.

- [ ] **Step 5: Define the fixed integration interface and read-only implementations**

```rust
pub trait AgentIntegration {
    fn detect(&self) -> Detection;
    fn capabilities(&self, version: &Version) -> CapabilityResolution;
    fn install_hooks(&self, selection: &HookSelection) -> Result<Installation, AppError>;
    fn inspect_hooks(&self) -> Result<HookHealth, AppError>;
}
```

`ClaudeIntegration` resolves only user settings; `CodexIntegration` resolves only `$CODEX_HOME/hooks.json` or `~/.codex/hooks.json` when `CODEX_HOME` is unset. In this task, provide inherent `detect`, `capabilities`, `validate_install_version`, and read-only path methods; define the complete trait but defer its concrete impl blocks until Task 11, after installer code exists. `validate_install_version` rejects `UpgradeRequired` and rejects `CompatibleUnverified` unless its boolean confirmation is true.

- [ ] **Step 6: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml agents:: && cargo test --manifest-path src-tauri/Cargo.toml events::catalog`

Expected: candidate order, version parsing, bounded subprocess behavior, catalog mapping, and install gates pass.

- [ ] **Step 7: Commit Agent detection**

```bash
git add src-tauri/src/agents src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: detect Claude Code and Codex installations"
```

### Task 10: Patch Claude JSONC and Codex Hook Configuration Without Disturbing User Entries

**Files:**
- Create: `src-tauri/src/installer/mod.rs`
- Create: `src-tauri/src/installer/jsonc.rs`
- Create: `src-tauri/src/installer/atomic.rs`
- Create: `tests/fixtures/configs/claude-settings.jsonc`
- Create: `tests/fixtures/configs/codex-hooks.json`
- Create: `src-tauri/tests/installer_roundtrip.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `AgentKind`, `HookSelection`, catalog matcher/output metadata, helper path, shared `HookCommand`/`canonical_hook_command`/`command_fingerprint`, and stable errors.
- Produces: `OwnedHookEntry`, `ConfigPatch`, `hook_definition_fingerprint(agent: AgentKind, entry: &OwnedHookEntry) -> String`, `patch_claude_settings`, `patch_codex_hooks`, `inspect_owned_entries`, and `atomic_replace_checked`.

- [ ] **Step 1: Create complex fixtures and failing preservation tests**

The Claude fixture includes leading/trailing comments, comments inside `hooks`, mixed indentation, a foreign `PreToolUse` entry, unknown fields, and no final newline. The Codex fixture includes trusted foreign entries before and after an existing CC Reminder entry.

```rust
#[test]
fn claude_install_and_uninstall_preserve_every_foreign_byte() {
    let original = fixture_bytes("configs/claude-settings.jsonc");
    let installed = patch_claude_settings(&original, &owned_entries(&["PermissionRequest", "Stop"])).unwrap();
    let uninstalled = patch_claude_settings(&installed.bytes, &[]).unwrap();
    assert_eq!(foreign_projection(&uninstalled.bytes), foreign_projection(&original));
    assert!(parse_jsonc(&uninstalled.bytes).is_ok());
}

#[test]
fn uninstall_requires_both_owner_marker_and_command_fingerprint() {
    let original = fixture_bytes("configs/codex-hooks.json");
    let result = patch_codex_hooks(&original, &[]).unwrap();
    assert!(String::from_utf8(result.bytes).unwrap().contains("foreign-owner-cc-reminder-lookalike"));
}
```

In the same round-trip test, assert install, update, repair, and uninstall behavior for absent/present `hooks`, empty objects, CRLF, file mode, unknown top-level fields, duplicate foreign matchers, paths containing spaces, and selection changes that add/remove only owned events.

- [ ] **Step 2: Write failing drift and atomic-write tests**

```rust
#[test]
fn external_change_after_inspection_is_reported_without_overwrite() {
    let fixture = AtomicFixture::new("{\"hooks\":{}}");
    let inspected_hash = fixture.hash();
    fixture.external_write("{\"hooks\":{},\"userChange\":true}");
    let error = atomic_replace_checked(fixture.path(), inspected_hash, b"replacement", fixture.mode()).unwrap_err();
    assert_eq!(error.code, "integration.config_drift");
    assert!(fixture.contents().contains("userChange"));
}
```

In the same atomic-write test, assert same-directory temp placement, sync-before-rename, original permission restore, parse-after-write, and an injected pre-rename failure that leaves the original intact.

- [ ] **Step 3: Run installer tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test installer_roundtrip`

Expected: FAIL because structured patching and atomic replacement do not exist.

- [ ] **Step 4: Implement parser-backed minimal text patches**

Use `jsonc-parser` to parse a syntax tree with comments/ranges. Locate the top-level `hooks` property and each event array structurally; never locate braces or keys with regex/string search. Generate exact owned entries:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedHookEntry {
    pub source_event: String,
    pub matcher: Option<String>,
    pub command: HookCommand,
    pub timeout_seconds: u8,
}

pub fn owned_command(
    helper: &Path,
    agent: AgentKind,
    event: &str,
    platform: CommandPlatform,
) -> HookCommand {
    canonical_hook_command(
        helper,
        ["--owner", "cc-reminder", "--agent", agent.as_str(), "--event", event],
        platform,
    )
}
```

Set `timeout_seconds` to `1`. For Codex, serialize the official string `command` field and `commandWindows` only when needed; for Claude, serialize only fields present in its runtime schema. Fingerprint the shared canonical command structure directly. The current Codex configuration and trust behavior source of truth is <https://learn.chatgpt.com/docs/hooks> and the user-level location is <https://learn.chatgpt.com/docs/config-file/config-advanced#hooks>.

Define `hook_definition_fingerprint(agent: AgentKind, entry: &OwnedHookEntry) -> String` as lowercase SHA-256 over length-prefixed `source_event` plus the schema-specific canonical serde JSON of every field serialized for that Agent (`matcher`, `command`, optional `commandWindows`, and timeout when supported). This is an app-local change detector, not an attempt to reproduce Codex's private trust hash. Store it separately from `command_fingerprint`: replacing helper bytes at the same path changes neither value, while any emitted Hook definition change changes the definition fingerprint.

Serialize only the owned array/object fragments, splice only AST-derived source ranges, and preserve every byte outside those ranges. When `hooks` is absent, use the root AST closing-token range and detected indentation/newline style to insert one property. Never write ownership fields outside Agent schemas; ownership is recognized by canonical helper path, exact `--owner cc-reminder`, and SHA-256 command fingerprint together.

Claude emits its documented matcher/command object shape. Codex emits the official `hooks.json` shape and omits unsupported fields. Validate the complete result through the parser before returning `ConfigPatch { bytes, before_hooks_subtree, before_hash, after_hash }`.

- [ ] **Step 5: Implement checked atomic replacement**

Acquire an application-specific lock file beside the target, re-read and hash the current bytes, and compare to the inspection hash. Write a randomly named same-directory temp file with original permissions, flush and `sync_all`, then atomically replace: `rename` plus parent-directory sync on Unix, and `ReplaceFileW`/`MoveFileExW` with write-through flags on Windows. Restore/verify original mode or DACL, re-read, parse, and verify exact owned entries. If any pre-replace operation fails, remove only the explicit temp path and leave the target unchanged.

- [ ] **Step 6: Run round-trip and package tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test installer_roundtrip && cargo test --manifest-path src-tauri/Cargo.toml installer::`

Expected: foreign text/semantics survive install-uninstall, drift never overwrites, file modes survive, and owned selection matches exactly.

- [ ] **Step 7: Commit structured config editing**

```bash
git add src-tauri/src/installer src-tauri/src/lib.rs src-tauri/tests/installer_roundtrip.rs tests/fixtures/configs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: safely patch Agent Hook configuration"
```

### Task 11: Install the Signed Helper and Manage Hook Install, Repair, Trust, and Uninstall

**Files:**
- Create: `src-tauri/src/installer/helper.rs`
- Create: `src-tauri/resources/helper-manifest.json`
- Modify: `src-tauri/src/installer/mod.rs`
- Modify: `src-tauri/src/agents/claude.rs`
- Modify: `src-tauri/src/agents/codex.rs`
- Modify: `src-tauri/src/storage/integrations.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/Cargo.toml`
- Test: unit tests colocated in `helper.rs` and `installer/mod.rs`
- Test: extend `src-tauri/tests/installer_roundtrip.rs`

**Interfaces:**
- Consumes: selected Hook set, structured patches, atomic replace, `FieldCipher`, Agent integration trait/structs, ingress command fingerprint, `IntegrationRepository`, and `TrustStatus`.
- Produces: `HelperInstaller::install`, `HookInstaller::inspect`, `HookInstaller::apply`, `HookInstaller::uninstall`, `HookAction`, and the concrete `AgentIntegration` implementations/lifecycle transitions for the existing `HookHealth` and `TrustStatus` types.

- [ ] **Step 1: Write failing helper integrity and upgrade tests**

```rust
#[test]
fn helper_is_copied_only_after_manifest_hash_matches() {
    let fixture = HelperFixture::new(b"signed helper bytes", manifest_for(b"signed helper bytes"));
    let installed = fixture.installer().install().unwrap();
    assert_eq!(std::fs::read(installed.path).unwrap(), b"signed helper bytes");
}

#[test]
fn hash_mismatch_keeps_existing_helper() {
    let fixture = HelperFixture::with_existing(b"working old helper", b"tampered package", manifest_for(b"expected"));
    let error = fixture.installer().install().unwrap_err();
    assert_eq!(error.code, "update.helper_integrity_failed");
    assert_eq!(fixture.installed_bytes(), b"working old helper");
}
```

Add executable permission, temporary file, sync, atomic replacement, and version downgrade rejection tests. The stable path is `<user-data>/bin/cc-reminder-hook` plus `.exe` on Windows.

- [ ] **Step 2: Write failing lifecycle, snapshot, drift, and trust tests**

```rust
#[test]
fn apply_snapshots_only_the_previous_hook_subtree_then_installs_selection() {
    let environment = InstallerEnvironment::claude_fixture();
    environment.apply(HookAction::Install, selection(&["PermissionRequest", "Stop"])).unwrap();
    assert_eq!(environment.owned_events(), BTreeSet::from(["PermissionRequest", "Stop"]));
    assert!(environment.snapshot_is_encrypted());
    assert!(!environment.snapshot_plaintext_contains("foreign command"));
}

#[test]
fn codex_change_waits_for_official_trust_until_matching_hook_is_observed() {
    let environment = InstallerEnvironment::codex_fixture();
    let installation = environment.apply(HookAction::Install, selection(&["Stop"])).unwrap();
    assert_eq!(installation.trust_status, TrustStatus::NeedsUserConfirmation);
    environment.observe_ingress("Stop", installation.command_fingerprint.clone());
    assert_eq!(environment.inspect().unwrap().trust_status, TrustStatus::ObservedWorking);
}
```

Cover install, repair, helper upgrade, selection shrink, safe uninstall, external drift, unknown-major block, compatible-version explicit confirmation, snapshot encryption-store unavailable, and a lookalike owner entry that must remain untouched. Assert separately that a binary-only helper upgrade preserves both fingerprints and observed Codex trust, while a matcher, timeout, command string, `commandWindows`, or other serialized Hook definition change changes `definition_fingerprint` and resets trust.

- [ ] **Step 3: Run installer lifecycle tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml installer:: && cargo test --manifest-path src-tauri/Cargo.toml --test installer_roundtrip`

Expected: FAIL because helper and Hook lifecycle orchestration do not exist.

- [ ] **Step 4: Implement helper manifest verification and atomic installation**

Build/package the helper as a Tauri external binary. Generate `helper-manifest.json` during release packaging with target triple, helper version, filename, length, and SHA-256. At runtime select only the current target entry, hash the packaged bytes before copying, write a same-directory temporary file under `<user-data>/bin`, apply `0700` directory and `0700` executable permissions on Unix/current-SID DACL on Windows, sync, rename, then hash the installed file again.

Do not accept a manifest path or checksum from runtime configuration. Reject a lower semantic helper version unless the operation is an explicit rollback from an encrypted snapshot.

- [ ] **Step 5: Implement the full checked Hook mutation transaction**

Expose exact actions:

```rust
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookAction { Install, Repair, UpgradeHelper, Uninstall }
```

For every mutation: acquire the app lock, re-read/hash, stop on drift, produce a structured patch, encrypt only the previous `hooks` subtree plus source hash/mode into `config_snapshots`, install/verify helper, atomically replace config, parse and inspect exact owned entries, then record installation rows with separate command and definition fingerprints. If snapshot encryption is unavailable, do not write Agent config.

Implement `AgentIntegration` for `ClaudeIntegration` and `CodexIntegration` now: `detect`/`capabilities` delegate to Task 9, while `install_hooks` and `inspect_hooks` call `HookInstaller` with the integration's fixed user-level path and schema adapter. No implementation accepts a caller-supplied config path.

Uninstall removes only entries matching stable helper path, owner argument, and stored command fingerprint; it does not restore the entire old subtree over subsequent user changes. A snapshot is offered only as explicit disaster recovery after hash comparison.

- [ ] **Step 6: Implement truthful Codex trust and Hook health**

Claude entries use `TrustStatus::NotRequired`. New Codex entries or a change from the previously stored `definition_fingerprint` use `NeedsUserConfirmation`; the UI instruction is the literal official `/hooks` command and a recheck action. Since config presence cannot prove approval, move to `ObservedWorking` only when an ingress request arrives for the same Agent/event with the stored expected command fingerprint, thereby observing the currently stored definition in use. A binary-only helper replacement at the same canonical command path/definition preserves trust status; if an upgrade changes command, matcher, timeout, `commandWindows`, or another serialized definition field, reset affected entries to `NeedsUserConfirmation`. Never pass the bypass flag.

`inspect` reports `Healthy`, `Missing`, `Drifted`, `HelperMismatch`, `NeedsTrust`, or `AgentUpgradeRequired` per entry and returns a common aggregate health. It is read-only.

Also report `SelectionOutOfDate` when installed owned events differ from `required_hook_selection`. Applying `HookAction::Repair` with explicit UI confirmation adds newly required events and removes no-longer-required owned events in one checked patch. Rule-save commands never rewrite Agent files implicitly, which prevents a Codex definition hash from changing without the user seeing the trust consequence.

- [ ] **Step 7: Run lifecycle and security scans**

Run: `cargo test --manifest-path src-tauri/Cargo.toml installer:: && cargo test --manifest-path src-tauri/Cargo.toml agents:: && cargo test --manifest-path src-tauri/Cargo.toml --test installer_roundtrip`

Run: `rg -n -- '--dangerously-bypass-hook-trust|access_token|signing_secret' src-tauri/src/installer src-tauri/src/agents src-tauri/resources`

Expected: tests pass; the scan finds no bypass flag and no credentials in config/resources.

- [ ] **Step 8: Commit installation lifecycle**

```bash
git add src-tauri/src/installer src-tauri/src/agents src-tauri/src/storage/integrations.rs src-tauri/resources src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/tests/installer_roundtrip.rs
git commit -m "feat: manage owned Hook installation lifecycle"
```

### Task 12: Implement the Durable Delivery Queue, Leases, Aggregation, and Retry Policy

**Files:**
- Create: `src-tauri/src/storage/queue.rs`
- Create: `src-tauri/tests/storage_recovery.rs`
- Modify: `src-tauri/src/storage/mod.rs`
- Modify: `src-tauri/src/model.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `NotificationDocument`, policy decisions, channel/event/rule IDs, and v1 database schema.
- Produces: `QueueRepository::enqueue`, `claim_due`, `complete`, `retry`, `fail`, `expire_due`, `manual_retry`, `set_channel_next_allowed`, `queue_stats`, `idempotency_key`, `RetryPolicy::classify`, and `ClaimedDelivery`.

- [ ] **Step 1: Write failing idempotency and state-machine tests**

```rust
#[test]
fn one_event_rule_version_and_target_enqueues_once() {
    let queue = test_queue();
    let job = new_job(event_id(), rule_id(), "effective-v3", channel_id());
    assert_eq!(queue.enqueue(&job).unwrap(), EnqueueResult::Inserted(job.id));
    assert_eq!(queue.enqueue(&job).unwrap(), EnqueueResult::AlreadyExists(job.id));
    assert_eq!(queue.count_jobs(), 1);
}

#[test]
fn invalid_state_transition_is_rejected() {
    let queue = queue_with_succeeded_job();
    let error = queue.retry(queue.only_job_id(), Utc::now(), redacted_error()).unwrap_err();
    assert_eq!(error.code, "storage.invalid_delivery_transition");
}
```

Cover every allowed transition in design section 15 and reject all other edges. Compute idempotency exactly as lowercase hex SHA-256 of `event_id || 0x00 || rule_version || 0x00 || channel_id`; `rule_id` remains stored for history but is not an extra idempotency component.

- [ ] **Step 2: Write failing lease, crash recovery, aggregate, and expiry tests**

```rust
#[test]
fn expired_lease_can_be_reclaimed_but_live_lease_cannot() {
    let queue = queue_with_due_job();
    let first = queue.claim_due("worker-a", now(), Duration::minutes(1), 10).unwrap();
    assert_eq!(first.len(), 1);
    assert!(queue.claim_due("worker-b", now() + Duration::seconds(30), Duration::minutes(1), 10).unwrap().is_empty());
    assert_eq!(queue.claim_due("worker-b", now() + Duration::seconds(61), Duration::minutes(1), 10).unwrap().len(), 1);
}

#[test]
fn due_aggregate_claim_contains_all_jobs_in_the_bucket() {
    let queue = queue_with_three_jobs_in_one_due_bucket();
    let claims = queue.claim_due("worker-a", now(), Duration::minutes(1), 10).unwrap();
    assert!(matches!(&claims[0], ClaimedDelivery::Aggregate { jobs, .. } if jobs.len() == 3));
}

#[test]
fn channel_rate_limit_delays_only_that_channel() {
    let queue = queue_with_due_jobs_on_two_channels();
    queue.set_channel_next_allowed(channel_a(), now() + Duration::seconds(30)).unwrap();
    let claims = queue.claim_due("worker-a", now(), Duration::minutes(1), 10).unwrap();
    assert_eq!(claims.iter().flat_map(ClaimedDelivery::jobs).map(|job| job.channel_id).collect::<Vec<_>>(), vec![channel_b()]);
}
```

In `storage_recovery.rs`, assert simultaneous two-connection claiming, application restart, pending/retry expiry, a partial aggregate bucket before release, and atomic completion/failure of every job in one aggregate claim.

- [ ] **Step 3: Write failing retry classification tests**

```rust
#[test]
fn retry_after_wins_over_jittered_backoff() {
    let policy = RetryPolicy::with_deterministic_jitter([0.25, 0.5]);
    let decision = policy.classify(attempt(2), &temporary_http(429, Some(Duration::seconds(90))));
    assert_eq!(decision, RetryDecision::RetryAt(now() + Duration::seconds(90)));
}

#[test]
fn credentials_and_format_errors_fail_without_retry() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.classify(attempt(1), &invalid_credential()), RetryDecision::Fail);
    assert_eq!(policy.classify(attempt(1), &invalid_format()), RetryDecision::Fail);
}
```

Cover network, timeout, 408, 429, 5xx, explicit temporary platform errors, authentication/signature/permission/format failures, max five attempts, and expiration before next retry.

- [ ] **Step 4: Run queue tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::queue && cargo test --manifest-path src-tauri/Cargo.toml --test storage_recovery`

Expected: FAIL because queue repository, leases, and retry classification do not exist.

- [ ] **Step 5: Implement transactional queue operations**

Use `BEGIN IMMEDIATE` for enqueue and claim. Claim candidates in stable `(next_attempt_at, created_at, id)` order, then update only rows whose state/lease predicate still matches. Return:

```rust
pub enum ClaimedDelivery {
    Single { job: DeliveryJob },
    Aggregate { aggregate_key: String, jobs: Vec<DeliveryJob> },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus { Pending, Sending, RetryWait, Succeeded, Failed, Expired }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryJob {
    pub id: Uuid,
    pub event_id: Uuid,
    pub rule_id: RuleId,
    pub rule_version: String,
    pub channel_id: ChannelId,
    pub idempotency_key: String,
    pub document: NotificationDocument,
    pub state: DeliveryStatus,
    pub attempts: u8,
    pub next_attempt_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub aggregate_key: Option<String>,
    pub aggregate_release_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnqueueResult { Inserted(Uuid), AlreadyExists(Uuid) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAt(DateTime<Utc>),
    PauseChannel { reason_code: String },
    Fail,
    Expire,
}
```

For an aggregate, use one `aggregate_key` per rule/target/project/window and claim all pending/retry jobs whose release time is due in one transaction. Exclude paused channels and channels whose persisted `next_allowed_at` is in the future. A send outcome writes one redacted attempt per constituent job and updates every state atomically. Permission jobs have no aggregate key by Task 4 policy.

`manual_retry` only moves `failed` to `pending`, resets attempts to zero, keeps the same idempotency key, and refuses expired jobs or paused/unavailable channels.

- [ ] **Step 6: Implement retry timing and channel pausing signals**

Backoff base is 2 seconds, cap is 5 minutes, and full jitter samples uniformly from zero through the capped exponential delay. Prefer a valid `Retry-After` seconds/date value. Authentication failures produce `PauseChannel { reason_code }`; other permanent failures produce `Fail`; retryable failures produce `RetryAt`; no retry may exceed job TTL or max attempts.

Permit only one in-flight HTTP request per channel ID. After any request, atomically set that channel's `next_allowed_at` to at least one second after completion; a valid server `Retry-After` may move it farther into the future. Other channel IDs remain claimable and aggregates consume one channel request regardless of constituent count.

- [ ] **Step 7: Run queue and recovery tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage::queue && cargo test --manifest-path src-tauri/Cargo.toml --test storage_recovery`

Expected: unique enqueue, legal states, crash recovery, concurrent lease exclusion, aggregates, expiry, retry timing, and manual retry tests pass.

- [ ] **Step 8: Commit the queue**

```bash
git add src-tauri/src/storage src-tauri/src/model.rs src-tauri/tests/storage_recovery.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: add durable notification delivery queue"
```

### Task 13: Send Official DingTalk and WeCom Webhook Payloads Safely

**Files:**
- Create: `src-tauri/src/channels/mod.rs`
- Create: `src-tauri/src/channels/http.rs`
- Create: `src-tauri/src/channels/dingtalk.rs`
- Create: `src-tauri/src/channels/wecom.rs`
- Create: `src-tauri/tests/channel_contract.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `ChannelSender`, `CredentialPayload`, `NotificationDocument`, `DeliveryReceipt`, and `DeliveryError`.
- Produces: `ChannelConfig`, `DingTalkSender`, `WeComSender`, `validate_official_webhook`, `render_markdown`, `render_text`, and platform response classification.

- [ ] **Step 1: Write failing host and credential parsing tests**

```rust
#[test]
fn accepts_only_exact_official_https_hosts_and_paths() {
    assert!(validate_official_webhook(ChannelKind::DingTalk, "https://oapi.dingtalk.com/robot/send?access_token=fake").is_ok());
    assert!(validate_official_webhook(ChannelKind::WeCom, "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake").is_ok());
    assert!(validate_official_webhook(ChannelKind::WeCom, "http://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake").is_err());
    assert!(validate_official_webhook(ChannelKind::DingTalk, "https://oapi.dingtalk.com.attacker.example/robot/send?access_token=fake").is_err());
}
```

Reject usernames/passwords, fragments, extra credential query names, wrong paths, missing key/token, non-default ports, localhost/IP literals, redirects to any host, and Unicode/punycode lookalikes. Accept only `oapi.dingtalk.com/robot/send?access_token=...` and `qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...`.

- [ ] **Step 2: Write failing signing, payload, fallback, and response tests**

```rust
#[test]
fn dingtalk_signing_matches_fixed_vector() {
    assert_eq!(
        dingtalk_signature(1_609_459_200_000, "SECtest"),
        "p5mXVLdX%2FBTrc2KtuhTs6ZcGOXtsKU5g1oE3WtfH4hY%3D"
    );
}

#[tokio::test]
async fn wecom_sends_markdown_and_maps_success() {
    let server = MockPlatform::wecom_success();
    let sender = WeComSender::for_contract_test(server.endpoint());
    let receipt = sender.send(&document("build complete")).await.unwrap();
    assert_eq!(receipt.platform_code.as_deref(), Some("0"));
    server.assert_json(json!({"msgtype":"markdown","markdown":{"content":"build complete"}}));
}
```

In the same contract test, assert DingTalk keyword prefix, no `@all`, Markdown escaping/truncation, format rejection followed by one text fallback, 408/429/5xx, `Retry-After`, platform temporary codes, credential/signature/permission errors, malformed/oversized responses, connect timeout 5 seconds, total timeout 10 seconds, and redirects disabled.

- [ ] **Step 3: Run channel contracts and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test channel_contract`

Expected: FAIL because channel validation and adapters do not exist.

- [ ] **Step 4: Implement common hardened HTTP behavior**

Build one `reqwest::Client` with rustls, system proxy/certificate behavior, connect timeout 5 seconds, request timeout 10 seconds, redirect policy `none`, and a fixed `CC-Reminder/<version>` user agent. Read at most 64 KiB of response body. Logs may contain method, official host, status, duration, and redacted platform code; they must never contain URL query, signature, credential payload, request body, or full response.

`#[cfg(test)]` constructors accept the local contract server endpoint after test-only bypass; production constructors always call `validate_official_webhook` and keep URLs in `SecretString`.

- [ ] **Step 5: Implement platform-independent Markdown and DingTalk**

Render the document into a conservative Markdown subset of headings, plain facts, body, and footer. Escape platform-reserved characters and truncate without splitting Unicode characters or a Markdown escape. Enforce DingTalk text/Markdown content at 20,000 characters and WeCom Markdown at 4,096 UTF-8 bytes/text at 2,048 UTF-8 bytes; contract tests exercise exactly-at-limit and one-over-limit multibyte input. Prefix the configured DingTalk keyword to both Markdown and text.

For signed robots calculate `HMAC-SHA256(secret, timestamp + "\n" + secret)`, Base64, then URL-encode as `sign` with `timestamp`. Send `markdown` first. On explicit format rejection only, send one `text` fallback within the same worker attempt. Never include `at.isAtAll: true` and do not model phone lists in v1.

- [ ] **Step 6: Implement WeCom and response classification**

Send only `markdown` or the single text fallback shape accepted by WeCom group robots. Parse JSON error codes with serde into bounded structs. Return `DeliveryReceipt { http_status, platform_code, sent_at }` only for platform success; map retryable and permanent conditions to `DeliveryErrorKind` consumed by Task 12.

The test-connection method uses the same sender and sends the fixed localized body `CC Reminder 测试消息 / CC Reminder test message`, clearly marked as a test; there is no read-only health probe.

- [ ] **Step 7: Run channel contracts and secret scans**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test channel_contract`

Run: `rg -n 'println!|dbg!|response\.text|webhook.*\{|access_token.*tracing|key.*tracing' src-tauri/src/channels`

Expected: all contract cases pass and the scan finds no unsafe logging path.

- [ ] **Step 8: Commit channel adapters**

```bash
git add src-tauri/src/channels src-tauri/src/lib.rs src-tauri/tests/channel_contract.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: send DingTalk and WeCom notifications"
```

### Task 14: Connect Ingress, Rules, Privacy, Queue, and the Delivery Worker

**Files:**
- Create: `src-tauri/src/pipeline.rs`
- Create: `src-tauri/src/worker.rs`
- Create: `src-tauri/tests/pipeline.rs`
- Modify: `src-tauri/src/ipc/server.rs`
- Modify: `src-tauri/src/storage/events.rs`
- Modify: `src-tauri/src/storage/integrations.rs`
- Modify: `src-tauri/src/storage/queue.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: all core interfaces from Tasks 2-13.
- Produces: `EventPipeline::process_live(request: IngressRequest)`, `EventPipeline::process_safe_ingress`, `DeliveryWorker::run`, `DeliveryWorker::run_once`, and typed `CoreEvent` notifications for health/history/queue UI refresh.

- [ ] **Step 1: Write failing end-to-end pipeline tests**

```rust
#[tokio::test]
async fn enabled_live_permission_event_creates_one_redacted_job_per_target() {
    let harness = PipelineHarness::with_rule_and_two_targets();
    harness.process_live(ingress_request_with_event(captured_permission_with_secret("Bearer never-send-this"))).await.unwrap();
    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 2);
    assert!(!harness.all_persisted_bytes().contains("never-send-this"));
}

#[tokio::test]
async fn offline_safe_event_uses_current_rule_but_original_time_and_expires() {
    let harness = PipelineHarness::with_enabled_stop_rule();
    harness.insert_safe_ingress(safe_stop_occurred_minutes_ago(31));
    harness.recover_ingress().await.unwrap();
    assert_eq!(harness.pending_jobs(), 0);
    assert_eq!(harness.expired_event_decisions(), 1);
}

#[tokio::test]
async fn offline_safe_event_with_project_id_uses_current_project_patch() {
    let harness = PipelineHarness::with_project_stop_patch();
    harness.insert_safe_ingress(safe_stop_for_project(harness.project_id()));
    harness.recover_ingress().await.unwrap();
    assert_eq!(harness.pending_jobs_for_project(), 1);
}

#[tokio::test]
async fn accepted_live_ingress_marks_only_the_matching_owned_hook_as_observed() {
    let harness = PipelineHarness::with_codex_hooks_awaiting_trust([
        ("Stop", "expected-command"),
        ("PermissionRequest", "expected-permission-command"),
    ]);
    harness.process_live(ingress_request("Stop", "expected-command")).await.unwrap();
    assert_eq!(harness.hook_trust("Stop"), TrustStatus::ObservedWorking);

    harness.process_live(ingress_request("PermissionRequest", "unexpected-command")).await.unwrap_err();
    assert_eq!(harness.hook_trust("PermissionRequest"), TrustStatus::NeedsUserConfirmation);
}
```

Cover unsupported capability, disabled rule, project longest-prefix override, filter miss, quiet suppress/defer, cooldown, per-window cap, aggregate, empty targets, sensitive-field encryption only when the app is live, and local duplicate ingress.

- [ ] **Step 2: Write failing worker success/failure/restart tests**

```rust
#[tokio::test]
async fn worker_sends_then_records_redacted_success_attempt() {
    let harness = WorkerHarness::with_wecom_success_job();
    harness.worker.run_once().await.unwrap();
    assert_eq!(harness.job_state(), DeliveryStatus::Succeeded);
    assert_eq!(harness.attempts().len(), 1);
    assert!(!format!("{:?}", harness.attempts()).contains("fake-key"));
}

#[tokio::test]
async fn consecutive_authentication_failure_pauses_only_its_channel() {
    let harness = WorkerHarness::with_auth_failures(3);
    harness.worker.run_until_idle().await;
    assert_eq!(harness.channel_health(), ChannelHealth::PausedAuthentication);
    assert!(harness.other_channel_is_runnable());
}
```

Cover retry scheduling, `Retry-After`, TTL, lease loss, crash/restart, aggregate single HTTP request with all constituent states updated, format fallback, manual retry, and graceful cancellation.

- [ ] **Step 3: Run pipeline tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test pipeline`

Expected: FAIL because orchestration and worker loops do not exist.

- [ ] **Step 4: Implement the processing order as one database transaction per event**

`process_live` accepts the complete `IngressRequest`, validates its helper version and command fingerprint against the expected Agent/event installation row, then follows the exact order from design section 12.1: capability, project, global rule, project merge, enabled, filters, timing policy, allowed fields, mandatory redaction, per-target template, idempotency key, enqueue. Apply mandatory redaction to every string in persisted `public_fields`, even when that field is catalogued public. Encrypt selected sensitive fields before event insertion and keep only `EncryptedBlobRef` on the envelope. Store the redacted event, its `EventProcessingOutcome`, all jobs, the exact Hook last-seen timestamp, and the matching Codex `ObservedWorking` transition atomically so history never shows an event that silently lost intended jobs and an unrecognized helper cannot establish trust; disabled/filter/quiet/cooldown/no-target decisions still store metadata-only events with no sensitive blob and count as observed Hook execution.

`process_safe_ingress` has no sensitive fields or raw cwd. It uses event occurrence time and current rules; a cached project ID still present in `projects` receives its current patch, while a project deleted since capture falls back to global rules with `project_id = None` and the already-safe display name. It marks expired decisions and deletes the ingress row only after the processing transaction commits. Duplicate event UUIDs are idempotent.

Measure normalize-through-enqueue duration with `Instant`; a dedicated release benchmark asserts p95 below 50 ms while shared CI records the value.

- [ ] **Step 5: Implement the cancellable worker loop**

`run` waits on a Tokio interval and a `CancellationToken`, claims at most 20 deliveries per pass with 60-second leases, caps total concurrent network sends at four, and acquires a per-channel semaphore before each request. It loads credentials only after claiming, creates the fixed sender for channel kind, and zeroizes/drops credential payload after send. Lease completion checks owner and unexpired lease; a lost lease records no state change.

For aggregates, combine document titles/counts and bounded bodies into one `NotificationDocument`, send once, then atomically record the same redacted receipt/error against every constituent job. Reset `consecutive_auth_failures` on success, increment it on authentication/signature/permission failures, and pause only that channel at three consecutive failures; replacing credentials clears the counter/pause after validation. After each state change emit `CoreEvent::QueueChanged`; authentication pauses emit `CoreEvent::HealthChanged`.

- [ ] **Step 6: Wire live IPC and startup recovery**

The IPC callback invokes `process_live` and replies `Accepted` only after durable commit. App startup migrates first, calls `ConfigRepository::ensure_global_rules` for both active catalogs, drains spool to ingress, recovers stale `processing` ingress rows, processes a bounded batch, starts IPC, then starts the worker. Shutdown stops accepting IPC, cancels worker, waits up to 10 seconds for active sends, releases no live lease early, and closes SQLite.

- [ ] **Step 7: Run end-to-end Rust verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test pipeline && cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract && cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test storage_recovery && cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test channel_contract`

Expected: live and offline events follow rules, no sensitive plaintext persists, delivery state/retry behavior is durable, and all notification failures remain isolated from helper exit behavior.

- [ ] **Step 8: Commit the notification core**

```bash
git add src-tauri/src/pipeline.rs src-tauri/src/worker.rs src-tauri/src/ipc/server.rs src-tauri/src/storage src-tauri/src/lib.rs src-tauri/tests/pipeline.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat: connect Hook events to reliable delivery"
```

### Task 15: Expose Typed Tauri Commands, Shared Health, Single Instance, and Tray Controls

**Files:**
- Create: `src-tauri/src/health.rs`
- Create: `src-tauri/src/commands/mod.rs`
- Create: `src-tauri/src/commands/agents.rs`
- Create: `src-tauri/src/commands/rules.rs`
- Create: `src-tauri/src/commands/channels.rs`
- Create: `src-tauri/src/commands/projects.rs`
- Create: `src-tauri/src/commands/history.rs`
- Create: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/model.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Test: unit tests colocated in command modules and `health.rs`

**Interfaces:**
- Consumes: Agent, installer, catalog, rule, project, channel, history, queue, diagnostics-ready settings, and worker services.
- Produces: the exact command API listed below, `HealthSnapshot`, tray menu actions, `core://health-changed`, `core://queue-changed`, and `core://history-changed` events.

- [ ] **Step 1: Write failing command-boundary privacy and validation tests**

```rust
#[tokio::test]
async fn list_channels_never_serializes_saved_credentials() {
    let state = command_state_with_wecom_credential("never-return-this");
    let response = list_channels(State(state)).await.unwrap();
    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains("never-return-this"));
    assert!(response[0].credential_present);
}

#[tokio::test]
async fn rule_command_rejects_unknown_capability_and_invalid_patch() {
    let error = save_global_rule(State(test_state()), unknown_rule_input()).await.unwrap_err();
    assert_eq!(error.code, "configuration.unknown_hook");
}
```

In the same command test modules, assert malformed UUID, invalid pagination, unknown reset field, unverified install without confirmation, arbitrary URL, credential deletion while targeted, and manual retry eligibility.

- [ ] **Step 2: Write failing health and tray state tests**

```rust
#[test]
fn one_health_projection_drives_overview_tray_and_pages() {
    let snapshot = project_health(&HealthInputs::with_failed_jobs(2));
    assert_eq!(snapshot.overall, HealthLevel::Error);
    assert_eq!(snapshot.failed_jobs, 2);
    assert_eq!(snapshot.tray_label(), "CC Reminder - 2 个失败任务");
}

#[test]
fn pause_today_uses_local_midnight_and_does_not_change_rules() {
    let result = pause_until(PauseDuration::Today, local_time("2026-07-29T14:00:00+08:00"));
    assert_eq!(result.to_rfc3339(), "2026-07-30T00:00:00+08:00");
}
```

- [ ] **Step 3: Run command tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml commands:: && cargo test --manifest-path src-tauri/Cargo.toml health::`

Expected: FAIL because commands, shared health, and tray state do not exist.

- [ ] **Step 4: Implement the explicit command surface**

Register only these commands, each with typed serde inputs/results and `Result<T, AppError>`:

```text
get_bootstrap_state
get_health_snapshot
detect_agents
list_agent_integrations
apply_hook_action
list_hook_rules
save_global_rule
save_project_rule_patch
reset_project_rule_field
preview_notification
send_rule_test
list_channels
save_channel
replace_channel_credential
delete_channel
test_channel
list_projects
save_project
add_project_alias
remove_project_alias
list_history
get_history_detail
manual_retry_delivery
get_settings
save_settings
set_notification_pause
clear_notification_pause
check_for_updates
install_update
```

All path inputs are canonicalized in the core; URLs are validated before credentials enter keyring; unknown JSON fields are rejected with `serde(deny_unknown_fields)` on command input structs. Read commands never expose credentials, ciphertext, raw IDs, complete paths for unmatched events, Hook raw JSON, or full platform responses.

`apply_hook_action` accepts only Agent kind, closed `HookAction`, expected health revision, and the explicit compatible-version confirmation flag. The Rust core derives helper path and `required_hook_selection`; the frontend cannot supply commands, config paths, fingerprints, or event names to install.

After either rule-save/reset command commits, recompute `required_hook_selection`, compare it with installation rows, and emit a health revision with `SelectionOutOfDate` when they differ. Do not call the installer from a rule-save command; only the explicit `apply_hook_action` confirmation mutates Agent config.

- [ ] **Step 5: Implement shared health and event propagation**

`HealthSnapshot` contains overall level, Agent/integration states, channel states, pending/retry/failed/expired counts, spool/rejected counts, last success, and stable issues with suggested command/action IDs. Derive overview, tray, and page badges from this same snapshot. Core state changes emit one of the three fixed Tauri event names with a revision number; frontend responds by re-fetching typed state rather than trusting event payload details.

Run Agent detection once at startup and every six hours while the tray app remains alive. A newly detected version updates `agent-versions.json`, recomputes catalog/installation health, and emits a health revision, but never rewrites Agent configuration or clears Codex trust state automatically.

- [ ] **Step 6: Implement single-instance and tray behavior**

Second launch focuses the existing main window. Closing the window hides it when `close_to_tray` is enabled; explicit Quit performs graceful Task 14 shutdown. Tray menu contains Open, current non-clickable health, Pause 15 minutes, Pause 1 hour, Pause Today, Resume, and Quit, using native menu items and icons where supported. Pausing affects policy evaluation globally but does not mutate saved rules. The helper never opens the window or starts the app.

Register autostart through the official Tauri plugin and update it only from `save_settings`. Grant the WebView only window/event/autostart commands it actually uses; do not enable shell, filesystem, process, global HTTP, or localhost capabilities.

Run `pnpm add @tauri-apps/plugin-updater@2` and add `tauri-plugin-updater = "2"` to Cargo. Register it with signed update metadata and expose `check_for_updates`/`install_update`; never accept an update endpoint from a command. The install command verifies the updater signature, asks the worker for graceful shutdown, installs, and relaunches only after explicit user confirmation. Release signing and endpoint configuration are completed in Task 22.

- [ ] **Step 7: Run command and runtime tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml commands:: && cargo test --manifest-path src-tauri/Cargo.toml health::`

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: command validation/privacy tests pass, shared health is consistent, tray pause is deterministic, and the entire Rust package passes.

- [ ] **Step 8: Commit desktop runtime APIs**

```bash
git add src-tauri/src/commands src-tauri/src/health.rs src-tauri/src/lib.rs src-tauri/src/model.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json src-tauri/Cargo.toml src-tauri/Cargo.lock package.json pnpm-lock.yaml
git commit -m "feat: expose desktop commands and tray controls"
```

### Task 16: Build the Typed Frontend Boundary, App Shell, Localization, and Onboarding

**Files:**
- Create: `src/lib/contracts.ts`
- Create: `src/lib/backend.ts`
- Create: `src/lib/i18n.ts`
- Create: `src/shell/AppShell.tsx`
- Create: `src/shell/AppShell.test.tsx`
- Create: `src/onboarding/Onboarding.tsx`
- Create: `src/onboarding/Onboarding.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/App.test.tsx`
- Modify: `src/app.css`
- Modify: `src/test/setup.ts`

**Interfaces:**
- Consumes: Task 15 commands/events and their serialized DTOs.
- Produces: `Backend` typed interface, `TauriBackend`, `BackendProvider`, `useBackend`, `useI18n`, `AppShell`, and the onboarding state machine.

- [ ] **Step 1: Write failing typed-backend and shell tests**

```tsx
test("navigation is keyboard accessible and remembers the selected page", async () => {
  const user = userEvent.setup();
  render(<TestApp backend={configuredBackend()} />);
  await user.click(screen.getByRole("button", { name: "渠道" }));
  expect(screen.getByRole("heading", { name: "渠道" })).toBeVisible();
  expect(localStorage.getItem("cc-reminder:last-page")).toBe("channels");
  expect(screen.getByRole("button", { name: "渠道" })).toHaveAttribute("aria-current", "page");
});

test("core revision events refresh health instead of trusting payload data", async () => {
  const backend = configuredBackend();
  render(<TestApp backend={backend} />);
  backend.emit("core://health-changed", { revision: 4, overall: "forged" });
  await waitFor(() => expect(backend.getHealthSnapshot).toHaveBeenCalledTimes(2));
  expect(screen.queryByText("forged")).not.toBeInTheDocument();
});
```

In the same frontend test file, assert Chinese default, English switch, system-theme resolution, visible focus classes, all seven navigation labels, a 960 x 640 non-overlap fixture, and configured startup defaulting to Hook Rules.

- [ ] **Step 2: Write failing onboarding flow tests**

```tsx
test("onboarding follows detect install channel defaults test order", async () => {
  const backend = onboardingBackend();
  render(<TestApp backend={backend} />);
  expect(screen.getByRole("heading", { name: "检测 Agent" })).toBeVisible();
  await completeDetectAndInstall();
  expect(screen.getByRole("heading", { name: "添加渠道" })).toBeVisible();
  await completeChannelAndDefaults();
  expect(screen.getByRole("heading", { name: "发送测试" })).toBeVisible();
});

test("Codex trust is a separate blocking checklist item with official command", async () => {
  render(<TestApp backend={backendNeedingCodexTrust()} />);
  expect(screen.getByText("/hooks")).toBeVisible();
  expect(screen.getByRole("button", { name: "重新检测" })).toBeEnabled();
  expect(screen.queryByText(/bypass/i)).not.toBeInTheDocument();
});
```

- [ ] **Step 3: Run frontend tests and verify the intended failures**

Run: `pnpm test -- src/App.test.tsx src/shell/AppShell.test.tsx src/onboarding/Onboarding.test.tsx`

Expected: FAIL because the typed backend, shell, i18n, and onboarding flow do not exist.

- [ ] **Step 4: Mirror DTOs and wrap every invoke/listen call**

`contracts.ts` defines discriminated unions matching Rust `snake_case` JSON exactly and branded string aliases for `ProjectId`, `ChannelId`, and `RuleId`. `Backend` exposes one typed method per registered command; credentials appear only in write inputs:

```ts
export interface ChannelSummary {
  id: ChannelId;
  kind: "ding_talk" | "we_com";
  name: string;
  credential_present: boolean;
  health_status: ChannelHealth;
  last_succeeded_at: string | null;
}

export interface Backend {
  getBootstrapState(): Promise<BootstrapState>;
  getHealthSnapshot(): Promise<HealthSnapshot>;
  detectAgents(): Promise<AgentIntegrationSummary[]>;
  listAgentIntegrations(): Promise<AgentIntegrationSummary[]>;
  applyHookAction(input: ApplyHookActionInput): Promise<HookInstallationResult>;
  listHookRules(input: ListHookRulesInput): Promise<HookRuleRow[]>;
  saveGlobalRule(input: SaveGlobalRuleInput): Promise<HookRuleRow>;
  saveProjectRulePatch(input: SaveProjectRulePatchInput): Promise<HookRuleRow>;
  resetProjectRuleField(input: ResetProjectRuleFieldInput): Promise<HookRuleRow>;
  previewNotification(input: PreviewNotificationInput): Promise<NotificationDocument>;
  sendRuleTest(input: SendRuleTestInput): Promise<DeliveryReceiptDto[]>;
  listChannels(): Promise<ChannelSummary[]>;
  saveChannel(input: SaveChannelInput): Promise<ChannelSummary>;
  replaceChannelCredential(input: ReplaceChannelCredentialInput): Promise<ChannelSummary>;
  deleteChannel(input: DeleteChannelInput): Promise<void>;
  testChannel(input: TestChannelInput): Promise<DeliveryReceiptDto>;
  listProjects(): Promise<ProjectSummary[]>;
  saveProject(input: SaveProjectInput): Promise<ProjectSummary>;
  addProjectAlias(input: AddProjectAliasInput): Promise<ProjectSummary>;
  removeProjectAlias(input: RemoveProjectAliasInput): Promise<ProjectSummary>;
  listHistory(input: ListHistoryInput): Promise<HistoryPage>;
  getHistoryDetail(input: GetHistoryDetailInput): Promise<HistoryDetail>;
  manualRetryDelivery(input: ManualRetryInput): Promise<DeliveryJobSummary>;
  getSettings(): Promise<AppSettings>;
  saveSettings(input: SaveSettingsInput): Promise<AppSettings>;
  setNotificationPause(input: SetPauseInput): Promise<HealthSnapshot>;
  clearNotificationPause(): Promise<HealthSnapshot>;
  checkForUpdates(): Promise<UpdateCheckResult>;
  installUpdate(input: InstallUpdateInput): Promise<void>;
  subscribe(event: CoreEventName, handler: (revision: number) => void): Promise<() => void>;
}
```

Define every referenced DTO in `contracts.ts` by mirroring the corresponding Task 15 serde input/result, with no index signatures and no credential fields on read models. `TauriBackend` is the sole importer of `@tauri-apps/api/core` and `@tauri-apps/api/event`. Tests receive a deterministic in-memory fake through context; production never accepts arbitrary command names.

- [ ] **Step 5: Implement the quiet desktop shell and localization**

Use a fixed 184 px navigation rail, compact 48 px status header, and unframed content region. Navigation buttons use Lucide icons (`LayoutDashboard`, `Bot`, `ListChecks`, `Webhook`, `FolderGit2`, `History`, `Settings`) plus text and native tooltips/accessible labels. Reserve green/yellow/red strictly for health; use neutral gray surfaces, blue focus/selection, and no gradients, orbs, hero typography, or nested cards. Set letter spacing to `0`, use fixed/rem-based type sizes rather than viewport-scaled fonts, and cap card/modal/drawer item radii at 8 px.

Implement a small typed dictionary with complete `zh-CN` and `en` keys; locale defaults to Chinese and can follow a saved setting. Theme uses `prefers-color-scheme` unless explicitly set. All icon-only actions are 32 x 32 with stable dimensions and tooltips.

- [ ] **Step 6: Implement onboarding as the actual first screen when incomplete**

Use `BootstrapState` to show exactly five steps: Detect Agent, Install Hooks, Add Channel, Choose Default Rules, Send Test. Persist completion in `app_settings` only after a successful test; allow resuming at the first incomplete step. Codex `NeedsUserConfirmation` shows `/hooks`, a copy icon, and Recheck, never a bypass control. Once complete, render `AppShell` at saved page or Hook Rules.

- [ ] **Step 7: Run shell tests, typecheck, and build**

Run: `pnpm test -- src/App.test.tsx src/shell/AppShell.test.tsx src/onboarding/Onboarding.test.tsx && pnpm build`

Expected: shell/onboarding tests pass, TypeScript has no implicit `any`, and production assets build.

- [ ] **Step 8: Commit frontend foundation**

```bash
git add src/App.tsx src/App.test.tsx src/app.css src/lib src/shell src/onboarding src/test/setup.ts
git commit -m "feat: add desktop shell and onboarding"
```

### Task 17: Implement the Complete Hook Rules Table and Field-Level Override Drawer

**Files:**
- Create: `src/hooks/HookRulesPage.tsx`
- Create: `src/hooks/HookRuleDrawer.tsx`
- Create: `src/hooks/HookRulesPage.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/app.css`
- Modify: `src/lib/contracts.ts`
- Modify: `src/lib/backend.ts`

**Interfaces:**
- Consumes: rule list/save/reset, channel summaries, preview, send-test commands, capability/input metadata, global/project scope, and shared health.
- Produces: the primary Hook Rule UI, including every supported/unsupported catalog row and editable inherited fields.

- [ ] **Step 1: Write failing table visibility and filtering tests**

```tsx
test("shows every Hook including unavailable and high-frequency rows", async () => {
  renderRules({ backend: rulesBackend() });
  expect(await screen.findByRole("row", { name: /PermissionRequest/ })).toBeVisible();
  const unavailable = screen.getByRole("row", { name: /PostToolUseFailure/ });
  expect(within(unavailable).getByText("当前版本不支持")).toBeVisible();
  expect(within(unavailable).getByRole("switch")).toBeDisabled();
  expect(within(screen.getByRole("row", { name: /PreToolUse/ })).getByText("高频")).toBeVisible();
});

test("filters combine name phase enabled and sensitivity", async () => {
  const user = userEvent.setup();
  renderRules({ backend: rulesBackend() });
  await user.type(screen.getByRole("searchbox", { name: "搜索 Hook" }), "permission");
  await user.selectOptions(screen.getByLabelText("阶段"), "permission");
  expect(screen.getAllByRole("row")).toHaveLength(2);
});
```

In the same table test, assert Claude/Codex tabs, global/project scope, columns `开关/Hook/阶段/Agent/频率/渠道/配置来源/状态`, unsupported reason, experimental/deprecated state, and the search clear icon.

- [ ] **Step 2: Write failing inheritance and drawer-control tests**

```tsx
test("editing one inherited field creates only that project patch", async () => {
  const backend = projectRulesBackend();
  const user = userEvent.setup();
  renderRules({ backend, scope: projectScope() });
  await user.click(await screen.findByRole("row", { name: /Stop/ }));
  await user.click(screen.getByRole("switch", { name: "启用通知" }));
  expect(backend.saveProjectRulePatch).toHaveBeenCalledWith(expect.objectContaining({
    patch: { enabled: false },
  }));
});

test("reset icon removes one override and restores inherited display", async () => {
  const backend = projectRuleWithDeliveryOverride();
  const user = userEvent.setup();
  renderRules({ backend, scope: projectScope() });
  await openStopDrawer(user);
  await user.click(screen.getByRole("button", { name: "恢复发送策略继承" }));
  expect(backend.resetProjectRuleField).toHaveBeenCalledWith(expect.objectContaining({ field: "delivery" }));
  expect(screen.getByText("继承全局")).toBeVisible();
});

test("rule selection drift requires one explicit Hook apply action", async () => {
  const backend = rulesBackendWithSelectionOutOfDate();
  const user = userEvent.setup();
  renderRules({ backend });
  await user.click(await screen.findByRole("button", { name: "应用 Hook 变更" }));
  await user.click(screen.getByRole("button", { name: "确认应用 Hook 变更" }));
  expect(backend.applyHookAction).toHaveBeenCalledWith(expect.objectContaining({ action: "repair" }));
});
```

In the same drawer test, assert explicit empty targets versus inheritance, channel checkboxes, filter multi-selects, privacy field checkboxes, body-length numeric input, metadata/native-summary segmented control, immediate/aggregate segmented control, cooldown/window caps/TTL inputs, quiet-hours enable/time/weekdays/bypass, suppress/defer selection, and permission aggregation disabled with explanation in an accessible tooltip rather than visible instructional copy.

- [ ] **Step 3: Write failing preview and test-send privacy tests**

```tsx
test("preview shows redaction before any test send", async () => {
  const backend = previewBackend("摘要：[REDACTED]");
  const user = userEvent.setup();
  renderRules({ backend });
  await openStopDrawer(user);
  await user.type(screen.getByLabelText("模板"), " token={{event.summary}}");
  expect(await screen.findByText("摘要：[REDACTED]")).toBeVisible();
  expect(screen.queryByText("secret-raw-value")).not.toBeInTheDocument();
});
```

Cover unauthorized placeholder errors, custom redaction validation, simulated fixture preview, actual test confirmation naming the target group, test failure diagnostics, and no sensitive value in rendered DOM/error text.

- [ ] **Step 4: Run Hook UI tests and verify the intended failures**

Run: `pnpm test -- src/hooks/HookRulesPage.test.tsx`

Expected: FAIL because rules table/drawer are absent.

- [ ] **Step 5: Implement the dense capability table**

Render a semantic table with fixed column widths and no per-row cards. Keep row height 44 px and switches/icon buttons fixed so badges cannot resize rows. Disabled capability rows remain focusable for details but their switch is disabled. Fetch all rows for the selected Agent/scope, then apply client-side display filters; saving triggers a backend refresh so installed-selection drift can surface.

Use buttons/icons according to function: `Search`, `X`, `RotateCcw`, `AlertTriangle`, `Info`, and `Send`. Every unfamiliar icon has `title` and `aria-label`. Do not place feature descriptions or keyboard shortcuts in the page.

When shared health reports `SelectionOutOfDate`, show a compact Agent-specific status row and `应用 Hook 变更` command. Its confirmation lists added/removed owned event names and warns that Codex changes return to `/hooks` review. Refresh rules and health after the installer completes.

- [ ] **Step 6: Implement project patch editing and safe preview**

Track each drawer section as `inherited` or `overridden`. In project scope, editing a section sends only that top-level patch field; reset sends the exact field enum `enabled`, `targets`, `filters`, `privacy`, `delivery`, or `quiet_hours`. An explicitly cleared quiet-hours value sends `quiet_hours: null`; reset removes the patch key.

Use checkboxes/toggles for binary settings, segmented controls for modes, native time inputs, bounded numeric inputs, menus for enum sets, and checkboxes for channels/fields/weekdays. Debounce preview 250 ms, cancel stale calls with a monotonically increasing request ID, and display only backend-redacted preview documents.

- [ ] **Step 7: Run tests and production build**

Run: `pnpm test -- src/hooks/HookRulesPage.test.tsx && pnpm build`

Expected: every capability is visible, inheritance semantics are exact, controls are accessible, previews stay redacted, and TypeScript builds.

- [ ] **Step 8: Commit Hook Rules UI**

```bash
git add src/hooks src/App.tsx src/app.css src/lib
git commit -m "feat: add visual Hook rule configuration"
```

### Task 18: Implement Agent Integration, Channel, and Project Management Pages

**Files:**
- Create: `src/agents/AgentsPage.tsx`
- Create: `src/agents/AgentsPage.test.tsx`
- Create: `src/channels/ChannelsPage.tsx`
- Create: `src/channels/ChannelsPage.test.tsx`
- Create: `src/projects/ProjectsPage.tsx`
- Create: `src/projects/ProjectsPage.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/app.css`
- Modify: `src/lib/contracts.ts`
- Modify: `src/lib/backend.ts`
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: Agent integration/Hook actions, channel lifecycle/test, project/alias commands, health state, and Tauri dialog plugin.
- Produces: complete operational pages for integration, outbound destinations, and project matching.

- [ ] **Step 1: Write failing Agent action/state tests**

```tsx
test("drift offers repair while unknown major offers application upgrade only", async () => {
  render(<AgentsPage backend={agentsBackendWithDriftAndUnknownMajor()} />);
  expect(await screen.findByRole("button", { name: "修复 Claude Code Hook" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "安装 Codex Hook" })).toBeDisabled();
  expect(screen.getByText("需要升级 CC Reminder" )).toBeVisible();
});

test("uninstall confirmation states that foreign Hooks remain", async () => {
  const user = userEvent.setup();
  render(<AgentsPage backend={installedAgentsBackend()} />);
  await user.click(await screen.findByRole("button", { name: "卸载 Claude Code Hook" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("只移除 CC Reminder 创建的 Hook");
});
```

Cover Detect, Install, Repair, Upgrade Helper, Uninstall, compatible-version confirmation, helper mismatch, per-event health, Codex `/hooks` copy/recheck, loading/disabled states, and redacted actionable errors.

- [ ] **Step 2: Write failing channel credential and test tests**

```tsx
test("saved credentials are never placed into an input or DOM", async () => {
  render(<ChannelsPage backend={savedDingTalkBackend()} />);
  expect(await screen.findByText("已保存凭据")).toBeVisible();
  expect(screen.getByLabelText("Webhook")).toHaveValue("");
  expect(document.body.textContent).not.toContain("access_token=fake");
});

test("connection test requires confirmation because it sends a real group message", async () => {
  const user = userEvent.setup();
  render(<ChannelsPage backend={savedWeComBackend()} />);
  await user.click(await screen.findByRole("button", { name: "测试发送" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("将向目标群发送测试消息");
});
```

Cover DingTalk signing-secret optional input, keyword prefix, WeCom fields, official-host validation error, credential replace/delete, delete blocked while rules target channel, multiple instances, success time, paused-auth state, Markdown fallback result, and no `@all`/phone-list controls.

- [ ] **Step 3: Write failing project root/alias tests**

```tsx
test("adds only a user-selected directory and chooses worktree behavior", async () => {
  const backend = projectsBackend();
  const user = userEvent.setup();
  render(<ProjectsPage backend={backend} dialog={dialogReturning("/work/client")} />);
  await user.click(screen.getByRole("button", { name: "添加项目" }));
  await user.click(screen.getByLabelText("作为现有项目的路径别名"));
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(backend.saveProject).toHaveBeenCalledWith(expect.objectContaining({ selected_path: "/work/client" }));
});
```

Cover canonical root display, aliases, default worktree-as-alias choice, explicit independent project choice, duplicate/overlapping paths, override count, Agent selection, alias removal confirmation, and no whole-disk scan action.

- [ ] **Step 4: Run page tests and verify the intended failures**

Run: `pnpm test -- src/agents/AgentsPage.test.tsx src/channels/ChannelsPage.test.tsx src/projects/ProjectsPage.test.tsx`

Expected: FAIL because the management pages do not exist.

- [ ] **Step 5: Register the official dialog plugin and implement pages**

Add `@tauri-apps/plugin-dialog@2` and `tauri-plugin-dialog = "2"`; grant directory-open only. Use the native folder picker only after the user clicks Add Project/Alias. The Rust project command canonicalizes the selected root, inspects only that directory/parents for a Git root, and returns a choice if it appears to be a worktree; it never recursively scans unrelated directories.

Build each page as an unframed toolbar plus semantic table/list. Forms use one modal or right drawer, never a card inside a card. Commands use text/icon buttons for Install/Repair/Test/Save and icon-only buttons for copy/delete/refresh with Lucide icons and tooltips. Keep destructive confirmation target-specific.

- [ ] **Step 6: Run page tests and all frontend tests**

Run: `pnpm test -- src/agents src/channels src/projects && pnpm test && pnpm build`

Expected: actions map to exact backend calls, secrets never reappear, project selection is user initiated, and all frontend checks pass.

- [ ] **Step 7: Commit operational pages**

```bash
git add src/agents src/channels src/projects src/App.tsx src/app.css src/lib package.json pnpm-lock.yaml src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities/default.json
git commit -m "feat: add Agent channel and project management"
```

### Task 19: Implement Overview, Notification History, and Settings Pages

**Files:**
- Create: `src/overview/OverviewPage.tsx`
- Create: `src/overview/OverviewPage.test.tsx`
- Create: `src/history/HistoryPage.tsx`
- Create: `src/history/HistoryPage.test.tsx`
- Create: `src/settings/SettingsPage.tsx`
- Create: `src/settings/SettingsPage.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/app.css`
- Modify: `src/lib/contracts.ts`
- Modify: `src/lib/backend.ts`

**Interfaces:**
- Consumes: shared health, redacted history/detail, manual retry, settings, pause, and update commands.
- Produces: the remaining main navigation pages and operator workflows.

- [ ] **Step 1: Write failing overview consistency tests**

```tsx
test("overview presents the same health issues and queue counts as shared health", async () => {
  render(<OverviewPage backend={healthBackend({ failed_jobs: 2, pending_jobs: 4 })} />);
  expect(await screen.findByText("2 个失败任务")).toBeVisible();
  expect(screen.getByText("4 个待发送任务")).toBeVisible();
  expect(screen.getByRole("button", { name: "查看失败任务" })).toBeEnabled();
});
```

Cover missing Agent, drift, trust pending, unavailable credential store, paused channel, spool/rejected count, queue backlog, last success, recent failures, and action buttons navigating to the owning page.

- [ ] **Step 2: Write failing history privacy/filter/retry tests**

```tsx
test("history filters and detail show only redacted content", async () => {
  const backend = historyBackend();
  const user = userEvent.setup();
  render(<HistoryPage backend={backend} />);
  await user.selectOptions(screen.getByLabelText("结果"), "failed");
  await user.click(await screen.findByRole("row", { name: /Stop.*失败/ }));
  expect(screen.getByRole("dialog")).toHaveTextContent("[REDACTED]");
  expect(document.body.textContent).not.toContain("secret-raw-value");
});

test("manual retry is available only for eligible failed jobs", async () => {
  render(<HistoryPage backend={historyWithFailedExpiredAndSucceeded()} />);
  expect(screen.getByRole("button", { name: "重试失败任务" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "重试过期任务" })).toBeDisabled();
});
```

Cover time/project/Hook/channel/result filters, bounded pagination, job timeline/attempt metadata, no full unmatched cwd, retry confirmation and response, loading/empty/error states, and update on `core://history-changed`.

- [ ] **Step 3: Write failing settings and pause tests**

```tsx
test("settings use native controls and persist exact values", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(screen.getByRole("checkbox", { name: "开机启动" }));
  await user.selectOptions(screen.getByLabelText("语言"), "en");
  await user.clear(screen.getByLabelText("历史保留天数"));
  await user.type(screen.getByLabelText("历史保留天数"), "14");
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(backend.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ autostart: true, locale: "en", event_retention_days: 14 }));
});
```

Cover close-to-tray, system/light/dark theme, 1-365 day bounds, pause/resume, update check/install confirmation, and secure-store unavailable display. Clear-history, diagnostics export, and temporary debug logging controls are added with their working backend in Task 20.

- [ ] **Step 4: Run remaining page tests and verify the intended failures**

Run: `pnpm test -- src/overview/OverviewPage.test.tsx src/history/HistoryPage.test.tsx src/settings/SettingsPage.test.tsx`

Expected: FAIL because the pages do not exist.

- [ ] **Step 5: Implement scanning-first operational layouts**

Overview uses compact metric strips and issue/recent-failure lists, not decorative cards. History uses a semantic table and one detail drawer. Settings uses labeled full-width sections separated by borders, with native checkboxes/selects/radios/number inputs and icon buttons only for file/download actions. No visible prose advertises features or shortcuts.

Every asynchronous command has an in-place loading state, disables duplicate submission, displays `AppError.message` plus suggested action, and returns focus to the initiating control after a dialog closes. Announce background updates through a polite ARIA live region without repeatedly moving focus.

- [ ] **Step 6: Run all UI tests and build**

Run: `pnpm test && pnpm build`

Expected: all pages render consistent state, sensitive values stay absent, keyboard/focus tests pass, and production assets compile.

- [ ] **Step 7: Commit remaining pages**

```bash
git add src/overview src/history src/settings src/App.tsx src/app.css src/lib
git commit -m "feat: add overview history and settings"
```

### Task 20: Add Redacted Logging, Retention, Diagnostics, and History Clearing

**Files:**
- Create: `src-tauri/src/diagnostics.rs`
- Create: `src-tauri/src/storage/retention.rs`
- Modify: `src-tauri/src/storage/mod.rs`
- Modify: `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src/lib/contracts.ts`
- Modify: `src/lib/backend.ts`
- Modify: `src/settings/SettingsPage.tsx`
- Modify: `src/settings/SettingsPage.test.tsx`
- Test: unit tests colocated in `diagnostics.rs` and `retention.rs`

**Interfaces:**
- Consumes: mandatory redactor, settings, database, health snapshot, app/Agent/system versions, and settings UI.
- Produces: `Diagnostics::init`, `Diagnostics::set_debug_until`, `Diagnostics::export`, `RetentionService::run_once`, functional `export_diagnostics` and `clear_history` commands, and a once-daily cleanup task.

- [ ] **Step 1: Write failing log rotation and redaction tests**

```rust
#[test]
fn logger_redacts_before_writing_and_rotates_at_ten_mib() {
    let directory = tempfile::tempdir().unwrap();
    let diagnostics = Diagnostics::test(directory.path(), 10 * 1024 * 1024, 3);
    diagnostics.info("delivery", "Authorization: Bearer never-log-this");
    diagnostics.write_repeatedly_until_rotated("bounded diagnostic line");
    let bytes = diagnostics.all_log_bytes();
    assert!(!bytes.contains("never-log-this"));
    assert!(diagnostics.log_files().len() <= 3);
    assert!(diagnostics.log_files().iter().all(|path| std::fs::metadata(path).unwrap().len() <= 10 * 1024 * 1024));
}
```

In the same diagnostics test, assert that debug automatically returns to info at deadline, webhook queries/platform full bodies are absent, files are user-only, and errors carry domain/code/suggested action without raw causes.

- [ ] **Step 2: Write failing retention, clear, and diagnostic archive tests**

```rust
#[test]
fn retention_removes_expired_history_and_old_logs_but_keeps_configuration() {
    let harness = retention_harness_with_old_and_recent_records();
    harness.service.run_once(now()).unwrap();
    assert_eq!(harness.old_event_count(), 0);
    assert_eq!(harness.recent_event_count(), 1);
    assert_eq!(harness.rule_count(), 1);
    assert_eq!(harness.channel_count(), 1);
}

#[test]
fn diagnostic_archive_contains_hashes_stats_and_redacted_logs_only() {
    let archive = diagnostic_harness_with_secret("never-export-this").export().unwrap();
    assert!(archive.entry_names().contains(&"manifest.json".to_owned()));
    assert!(!archive.all_text().contains("never-export-this"));
    assert!(!archive.entry_names().iter().any(|name| name.contains("sqlite")));
}
```

Cover 30/7 day defaults, user-configured bounds, job/attempt cascade, SQLite checkpoint/vacuum scheduling, immediate clear-history preserving rules/projects/channels, and archive exclusion of credentials/ciphertext/raw configs/snapshots/spool/database.

Add the failing Settings integration test in `src/settings/SettingsPage.test.tsx`:

```tsx
test("exports diagnostics and clears only inactive history after confirmation", async () => {
  const backend = settingsBackend();
  const user = userEvent.setup();
  render(<SettingsPage backend={backend} />);
  await user.click(screen.getByRole("button", { name: "导出诊断" }));
  expect(backend.exportDiagnostics).toHaveBeenCalledWith();
  await user.click(screen.getByRole("button", { name: "清除历史" }));
  await user.click(screen.getByRole("button", { name: "确认清除历史" }));
  expect(backend.clearHistory).toHaveBeenCalledWith({ preserve_active_jobs: true });
});
```

- [ ] **Step 3: Run diagnostics tests and verify the intended failures**

Run: `cargo test --manifest-path src-tauri/Cargo.toml diagnostics && cargo test --manifest-path src-tauri/Cargo.toml storage::retention && cargo test --manifest-path src-tauri/Cargo.toml commands::settings && pnpm test -- src/settings/SettingsPage.test.tsx`

Expected: FAIL because diagnostics/retention services and their typed Settings controls do not exist.

- [ ] **Step 4: Implement redaction-first local diagnostics**

Wrap all tracing writes with structured fields and the Task 5 mandatory redactor before serialization. Rotate `cc-reminder.log`, `.1`, `.2` at 10 MiB, retaining at most three files; use exclusive user-only files. Default level is info. A user may enable debug for 15 or 60 minutes, stored as an expiry timestamp; startup never restores an already expired debug setting.

Stable error domains are exactly `integration`, `configuration`, `secret_store`, `delivery`, `storage`, and `update`. Add no telemetry/export transport. Panic reporting writes only app version, OS, redacted message, and backtrace addresses when locally enabled.

- [ ] **Step 5: Implement retention and safe diagnostic export**

Run retention once after startup and every 24 hours. In bounded transactions delete expired delivery attempts/jobs/events and processed ingress, checkpoint WAL, and vacuum only when more than 20 percent of pages are free and no worker lease is live. Delete logs older than configured log retention.

Export a ZIP containing only `manifest.json`, `health.json`, `queue-stats.json`, and redacted log files. Manifest contains app/helper/Agent versions, OS/architecture, capability versions, SHA-256 hashes of non-sensitive serialized settings/rules (not values), database schema version, and export time. Save through a user-selected path; never include SQLite, credentials, ciphertext, config snapshots, Agent configs, spool entries, project paths, or session references.

`clear_history` deletes attempts and terminal jobs, then deletes only events with no remaining pending/sending/retry job, plus processed ingress, in one transaction after exact confirmation. It preserves configuration and every active job's parent event; v1 UI exposes only this preserve-active-work form and does not offer cancellation.

Register `clear_history`, `export_diagnostics`, and `set_debug_logging` only now. Extend `Backend`/`TauriBackend` with `clearHistory(input: { preserve_active_jobs: true }): Promise<void>`, `exportDiagnostics(): Promise<DiagnosticExportResult>`, and `setDebugLogging(input: { duration_minutes: 0 | 15 | 60 }): Promise<AppSettings>`, then add matching Settings controls. `export_diagnostics` opens the native save dialog from Rust, writes only the path returned by that invocation using create/truncate after confirmation, and returns `cancelled` or a safe saved filename; it accepts no frontend path and grants no WebView filesystem permission. This keeps every command callable as soon as it appears in the command registry; there is no provisional command result in Task 15.

- [ ] **Step 6: Run diagnostics, UI, and full tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml diagnostics && cargo test --manifest-path src-tauri/Cargo.toml storage::retention && cargo test --manifest-path src-tauri/Cargo.toml commands::settings`

Run: `pnpm test -- src/settings/SettingsPage.test.tsx && cargo test --manifest-path src-tauri/Cargo.toml`

Expected: logs/archives contain no planted secrets, retention bounds hold, clear preserves configuration/active work, and all tests pass.

- [ ] **Step 7: Commit diagnostics and retention**

```bash
git add src-tauri/src/diagnostics.rs src-tauri/src/storage src-tauri/src/commands/settings.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/capabilities/default.json src/lib src/settings/SettingsPage.tsx src/settings/SettingsPage.test.tsx
git commit -m "feat: add private diagnostics and retention"
```

### Task 21: Add End-to-End UI, Accessibility, and Native Icon Checks

**Files:**
- Create: `playwright.config.ts`
- Create: `tests/e2e/app.spec.ts`
- Create: `assets/app-icon.svg`
- Create: `docs/images/hook-rules.png`
- Modify: `package.json`
- Modify: `pnpm-lock.yaml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: the completed desktop application and typed test fake backend.
- Produces: `pnpm test:e2e`, `pnpm verify`, reviewed screenshots/accessibility results, and generated native icons.

- [ ] **Step 1: Add Playwright and write failing desktop-layout workflows**

Install `@playwright/test` and `@axe-core/playwright` as development dependencies and add scripts:

```json
{
  "scripts": {
    "test:e2e": "playwright test",
    "verify": "pnpm test && pnpm test:e2e && pnpm build"
  }
}
```

Serve Vite with the deterministic browser fake backend selected only by `VITE_CC_REMINDER_TEST_BACKEND=1`. Write tests:

```ts
test("configures a project override and sees a redacted delivery", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Hook 规则" }).click();
  await page.getByLabel("作用域").selectOption("project:client");
  await page.getByRole("row", { name: /Stop/ }).click();
  await page.getByRole("switch", { name: "启用通知" }).click();
  await expect(page.getByText("继承全局")).toBeVisible();
  await expect(page.locator("body")).not.toContainText("secret-raw-value");
});

test("all primary pages fit at the minimum window without overlap", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await page.goto("/");
  for (const [label, snapshot] of [
    ["概览", "overview"], ["Agent 集成", "agents"], ["Hook 规则", "hooks"],
    ["渠道", "channels"], ["项目", "projects"], ["通知历史", "history"], ["设置", "settings"],
  ]) {
    await page.getByRole("button", { name: label }).click();
    await expect(page.locator("main")).toHaveScreenshot(`${snapshot}.png`);
  }
});
```

In the same spec, assert keyboard-only onboarding/rule edit, light/dark, English longest-label, 1280 x 800, 1440 x 900, 200 percent browser zoom, no horizontal page overflow, no text/button clipping, and no incoherent overlap.

- [ ] **Step 2: Run e2e tests and verify the intended failures**

Run: `pnpm exec playwright install chromium && pnpm test:e2e`

Expected: FAIL until test-backend startup, screenshot directories, and any discovered layout/accessibility issues are implemented/fixed.

- [ ] **Step 3: Implement the browser test backend and fix visual/accessibility defects**

The fake implements the same `Backend` interface and deterministic state transitions but is excluded from production through Vite dead-code elimination. It may contain fake secrets only in memory and must assert they never reach rendered DTOs. Configure Playwright `webServer` to run `pnpm dev --host 127.0.0.1`, Chromium only for browser UI checks, trace on first retry, and snapshots per platform.

Inspect screenshots at every listed viewport. Keep table/tool controls fixed, wrap long English labels, use `minmax(0, 1fr)` for content tracks, and scroll only table bodies/drawers where necessary. Run `axe-core` through `@axe-core/playwright` and fail on serious/critical accessibility violations.

After the 1280 x 800 Hook Rules screenshot passes review, export that exact deterministic image to `docs/images/hook-rules.png`; do not use a mockup or a screenshot containing credential fields.

- [ ] **Step 4: Create native icon source and generate platform assets**

Create a simple original app mark in `assets/app-icon.svg`: a square notification outline combined with two terminal prompt strokes, using neutral charcoal, white, and one red notification dot. It must remain legible at 16 px and contain no third-party logo/trademark. Generate Tauri icon outputs with:

Run: `pnpm tauri icon assets/app-icon.svg`

Expected: Tauri generates `.icns`, `.ico`, PNG sizes, and platform-specific icon assets under `src-tauri/icons/`; inspect 16, 32, 128, and 512 px outputs for clipping.

- [ ] **Step 5: Run UI verification**

Run: `pnpm test:e2e && pnpm verify`

Expected: all workflow, screenshot, viewport, zoom, and axe checks pass, and the production frontend build contains no test-backend branch.

- [ ] **Step 6: Commit UI verification and icons**

```bash
git add assets docs/images/hook-rules.png playwright.config.ts tests/e2e package.json pnpm-lock.yaml src-tauri/tauri.conf.json src-tauri/icons .gitignore
git commit -m "test: add desktop UI acceptance coverage"
```

### Task 22: Add Cross-Platform CI, Signed Packaging, and Artifact Verification

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `scripts/check-sensitive-artifacts.sh`
- Create: `scripts/verify-package.sh`
- Create: `scripts/verify-package.ps1`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: all automated checks, native icons, helper manifest format, updater configuration, and platform signing credentials.
- Produces: three-OS CI/release matrices, signed/checksummed packages, signed updater metadata, and explicit package-verification scripts.

- [ ] **Step 1: Add the continuous-integration matrix**

`ci.yml` runs on pull requests and pushes with least-privilege read permissions:

```text
ubuntu-22.04 quality:
  pnpm install --frozen-lockfile
  pnpm test
  pnpm exec playwright install --with-deps chromium
  pnpm test:e2e
  pnpm build
  cargo fmt --manifest-path src-tauri/Cargo.toml --check
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
  cargo test --manifest-path src-tauri/Cargo.toml

macos-13, macos-14, windows-2022, ubuntu-22.04 build-smoke:
  install pnpm/Rust/system Tauri prerequisites
  pnpm install --frozen-lockfile
  pnpm tauri build --no-bundle
  cargo test --features test-support --test hook_contract
```

Cache only pnpm store and Cargo registry/git/target keys derived from lockfiles. Upload test reports/screenshots on failure, never data directories or environment dumps.

- [ ] **Step 2: Add signed release packaging and artifact verification**

`release.yml` runs only on version tags and uses protected environment secrets:

```text
macOS: TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD,
       APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD,
       APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID
Windows: WINDOWS_CERTIFICATE, WINDOWS_CERTIFICATE_PASSWORD
Linux: TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

Build universal macOS for 12+, signed/notarized app/dmg; Windows 10/11 x64 signed installer; Ubuntu 22.04 x64 AppImage and deb. Build the helper first for the same target, apply its platform code signature where applicable, create the embedded helper manifest from those final signed bytes, then package/sign the app. Generate Tauri `latest.json` at the single compile-time HTTPS update endpoint, sign every updater artifact, generate SHA-256 for every published file, and upload only after `verify-package` succeeds. The endpoint and updater public key are build configuration, never a runtime user setting.

`verify-package.sh`/`.ps1` unpack into a temporary directory and assert: desktop/helper exist, helper hash matches manifest, the release helper contains no `CC_REMINDER_TEST_DATA_DIR` literal/test-support path, no plaintext test marker or credential query occurs, no forbidden bypass flag occurs, Linux artifacts have checksums, macOS codesign/notarization checks pass, and Windows Authenticode status is valid. Scripts use explicit artifact arguments and never delete caller directories.

`check-sensitive-artifacts.sh` treats its argument as the repository containing build outputs and scans only `dist/`, release package staging, non-test packaged resources, and final desktop/helper binaries for planted plaintext markers, concrete credential-value patterns, private-key blocks, and an executable bypass argument. Source, tests, fixtures, scripts, and prose documentation are excluded, so security assertions and documented terms do not cause false positives; fixture sanitization remains covered by Task 3 tests. Any runtime-artifact match fails with file and rule name but never prints the matched value.

- [ ] **Step 3: Exercise workflow-equivalent checks locally**

Run: `pnpm verify && cargo fmt --manifest-path src-tauri/Cargo.toml --check && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo test --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract`

Run: `./scripts/check-sensitive-artifacts.sh .`

Expected: quality checks pass and the artifact scan reports zero runtime secret/bypass findings without flagging design/operations prose.

- [ ] **Step 4: Commit CI and release packaging**

```bash
git add .github scripts src-tauri/tauri.conf.json
git commit -m "build: add signed cross-platform release pipeline"
```

### Task 23: Document Operations and Record Final Acceptance Evidence

**Files:**
- Create: `docs/operations.md`
- Create: `README.md`

**Interfaces:**
- Consumes: all completed workflows, actual UI labels, release artifacts, and the 12 design acceptance criteria.
- Produces: operator-facing installation/recovery instructions and a release checklist with evidence links for every supported OS.

- [ ] **Step 1: Document install, trust, operation, diagnostics, and safe uninstall**

`docs/operations.md` includes exact sections: supported OS/Secret Service prerequisite; first launch; Claude install; Codex `/hooks` trust and retrust when the serialized Hook definition changes; DingTalk/WeCom official robot creation; test message side effect; pause/resume; drift repair; queue retry/expiry/at-least-once duplicate caveat; diagnostic contents/exclusions; helper/Hook uninstall behavior; encrypted snapshot recovery; upgrade; complete application data removal; and confirmation that exiting prevents sending while safe offline metadata may queue.

State v1 non-goals explicitly: no remote reply/approval, personal WeChat, arbitrary Webhooks/scripts, transcript/model summaries, multimedia, cloud sync, Agent hosting, or local HTTP service.

Create a concise `README.md` with the actual app screenshot, supported platforms, installation links, privacy summary, supported Agents/channels, and links to design/operations docs. It opens with the usable product, not a marketing hero or feature tutorial.

- [ ] **Step 2: Run the complete acceptance suite and record evidence**

Run:

```bash
pnpm install --frozen-lockfile
pnpm verify
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract
./scripts/check-sensitive-artifacts.sh .
```

Then run on each target OS: first startup, tray open/pause/resume/quit, autostart toggle, secure-store availability, Agent detection, install, Codex trust, real test message to a dedicated test group, drift/repair, helper upgrade/retrust, offline spool recovery/TTL, network retry, uninstall preserving foreign Hooks, signed package verification, and application upgrade.

Expected: all automated commands pass; dedicated release measurements satisfy helper p95 under 100 ms, rule/enqueue p95 under 50 ms, normal-network delivery usually under 5 seconds, and idle resident memory under 100 MiB. Append a release checklist table to `docs/operations.md` with one row per design acceptance criterion and macOS/Windows/Linux evidence link or measured value; all 12 rows must be complete before release.

- [ ] **Step 3: Commit operations and acceptance evidence**

```bash
git add README.md docs/operations.md
git commit -m "docs: add operations and release acceptance guide"
```

## Design Traceability

| Design section | Implemented and verified by |
|---|---|
| 1. Summary | Global Constraints; Tasks 8, 14, 17, 23 |
| 2. Confirmed decisions | Global Constraints; Tasks 1, 7-8, 13, 15-20 |
| 3. Goals and non-goals | Tasks 16-20 and Task 23 operations/non-goals |
| 4. cc-connect comparison | Architecture boundary in header; Task 23 documents no bridge/session hosting |
| 5. Platform baseline | Tasks 1, 8, 21-23 build and per-OS checks |
| 6. Overall architecture | Tasks 6-8 and 12-15 end-to-end process boundaries |
| 7. Component responsibilities | Tasks 9-14 fixed Agent/Sender interfaces and pipeline |
| 8. Hook capability catalog | Tasks 2, 9, 11, 17 |
| 9. Hook installation safety | Tasks 10-11 and installer round-trip contracts |
| 10. Event model | Tasks 2-3, 7, 14 |
| 11. Project model and matching | Tasks 3, 6B, 15, 18 |
| 12. Rule model | Tasks 4, 14, 17 |
| 13. Templates and privacy | Tasks 5, 7, 14, 17 |
| 14. Channel design | Tasks 6B, 7, 13, 18 |
| 15. Reliable delivery | Tasks 12-14 and 19 |
| 16. Local storage | Tasks 6, 6B, 12, 20 |
| 17. Credentials and encryption | Tasks 7, 11, 18, 20, 22 |
| 18. GUI information architecture | Tasks 16-21 |
| 19. Security boundaries | Global Constraints; Tasks 3, 5, 7-8, 10-11, 13, 15, 20, 22 |
| 20. Errors and observability | Tasks 2, 15, 19-20 |
| 21. Remote intervention evolution | Task 2 action contract; no v1 inbound implementation |
| 22. Code organization | Repository Map and all task file lists |
| 23. Test design | TDD cycle in every behavior task; Tasks 8, 10, 12-14, 21-22 contract/e2e matrices |
| 24. Acceptance criteria | Task 23 per-OS evidence table and Final Cross-Task Verification |
| 25. Design tradeoffs | Header architecture and Global Constraints preserve Tauri/Rust/no-daemon/no-bridge choices |
| 26. References | Approved design remains normative; Task 10 cites current Codex Hook/config pages; Task 23 links design and official channel operations references |

## Final Cross-Task Verification

- [ ] Run `pnpm verify` and confirm React unit/e2e/type/build checks pass.
- [ ] Run `cargo fmt --manifest-path src-tauri/Cargo.toml --check` and confirm no diff.
- [ ] Run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` and confirm no warnings.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml` and confirm every Rust unit/integration/contract test passes.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml --features test-support --test hook_contract` and confirm the isolated helper process paths pass without touching production data directories.
- [ ] Run `./scripts/check-sensitive-artifacts.sh .` and confirm no credential, bypass flag, private key, raw Hook fixture, or planted secret marker appears in tracked/output artifacts.
- [ ] Run `git status --short` and confirm only intentionally generated release/screenshot artifacts are present; either commit required snapshots/icons/locks or remove reproducible untracked output by explicit path.
- [ ] Review the 12 acceptance criteria in design section 24 against CI reports and the per-OS release checklist; do not publish until every criterion has evidence or an explicitly blocked target release is omitted.
