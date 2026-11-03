# Contributing

Read this guide before planning, editing, testing or reviewing. It applies to people
and coding agents. The canonical repository is `ohne-b/coc-bot`, targeting `main`.
Keep [LICENSE.md](LICENSE.md), [NOTICE.md](NOTICE.md), upstream attribution and asset
licenses. The missing upstream project license must be resolved before distributing
a derivative application; PolyForm covers only eligible original contributions.

## Scope

This is a personal Windows desktop project using LDPlayer and ADB. The target is a
modular Rust backend with OpenCV, SQLite and a Tauri 2 + React/TypeScript dashboard.
No Linux server, Docker service or game-protocol reverse engineering is planned.
The runtime is Rust; the dashboard is React/TypeScript. No Python sidecar is required.
The retired coc-auto baseline remains in Git history at `a3b3346` for comparison.

Preserve farming, deployment, resource collection, wall upgrades and recovery. Add
features in coherent steps, with evidence that the old behavior survives. Substantial
scope changes need maintainer agreement; an explicit task covers its necessary work.

## Report a problem

Search [issues](https://github.com/ohne-b/coc-bot/issues) and
[PRs](https://github.com/ohne-b/coc-bot/pulls) first. Include the commit/version,
Windows and LDPlayer versions, resolution, DPI, game language, steps and expected/actual
behavior. Distinguish a misread screenshot from an incorrect action or dashboard value.
Feature requests need a concrete user problem and acceptance criteria.

Never post Supercell credentials, device/API keys, personal config, player identifiers
or unredacted logs/screenshots. Report vulnerabilities using [SECURITY.md](SECURITY.md).

## Development

Use Windows x64 with Visual Studio 2022 or newer Build Tools (Desktop development with C++
and a Windows SDK), Rustup and Node.js 24. Rust is pinned by
`rust-toolchain.toml`. The native dependency script downloads checksum-pinned OpenCV
4.12.0 and portable LLVM 20.1.8 into ignored `.tools/`; it needs no administrator rights.
The first download is about 1.1 GB. Dot-source it in each PowerShell session before
running Rust binaries so the OpenCV DLL can be found.

From the repository root:

```powershell
. ./.github/scripts/windows-dependencies.ps1
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p coc-bot-core --example inspect -- `
    assets/config.json crates/core/tests/fixtures/english-home.png
```

The inspection example reads a saved screenshot and prints a structured detection;
it never starts ADB or sends input. Recognition rejects unknown and ambiguous screens,
wrong dimensions and weak numeric readings. English screens listed as pending have
no matching templates. Indonesian profiles and text anchors have been removed; shared
icons and digits remain. Integrate ongoing English calibration additions with their fixtures.

History tests use temporary SQLite files. The store preserves last known resource readings
with timestamps and explicit failed/unavailable status, isolates sessions and villages, and
counts confirmed attack results and wall spending separately. Loot totals and averages carry
the number of measured attacks; unreadable results are not zero loot. Rates use the engine's
monotonic active session time, excluding pauses and including searching/recovery. They describe
confirmed earnings across the session, not estimates from storage changes. The engine must
checkpoint active time and own the blocking SQLite connection off the UI thread. Startup may
mark unfinished sessions interrupted only after acquiring the desktop single-instance lock.
Foreign, corrupt or newer databases fail without being reset. Schema version 2 adds
validated emulator settings while preserving version 1 session history.

Device tests replace the ADB/LDPlayer transport and use saved frames. Process tests
start disposable PowerShell children to check binary pipes, cancellation, timeouts and
output limits; they never contact an emulator. Observation mode rejects all input and
lifecycle changes. Live input requires the selected instance, calibrated display at
240 DPI, a freshly recognized expected screen and a non-cancelled operation.
Multitouch validates the Android kernel and touch axes and attempts a bounded release
after failure or cancellation. These offline checks do not establish live device compatibility.

Repository automation is checked separately:

```powershell
./.github/scripts/windows-signing.ps1
node --test .github/scripts/*.test.mjs
git diff --check
```

Tests use fixtures, temporary storage and fake device boundaries. Do not launch a real
bot or reuse a live emulator for automated validation. The Python runtime, relay and
old web dashboard have been retired; their guides and media remain in Git history.
Existing local settings, captures, logs and backups are never migrated or deleted
automatically. Desktop settings belong in SQLite, separate from shared calibration.
Never commit a virtual environment, runtime captures, logs, credentials or databases.

| Location | Current purpose |
| --- | --- |
| `crates/core/` | Rust recognition, guarded device control, typed telemetry and SQLite session history |
| `crates/core/tests/` | Offline Rust regressions; captured frames and annotations in `fixtures/` |
| `desktop/` | Tauri lifecycle and scoped native commands |
| `frontend/` | React/TypeScript dashboard and offline browser tests |
| `assets/` | Shared English calibration, digits and retained font notices |
| `.github/` | Repository policy, build/validation scripts and supplied branding |

The desktop supports observation and development farming/wall modes: selected-instance display
checks, screenshots, recognition, dynamic deployment, bounded recovery and session history.
Start enables guarded game input; Observe never does. Pause freezes active time and dispatches
no gameplay steps. Settings remain locked until Stop. Farming, device and deployment saves
patch their own sections so stale forms cannot overwrite unrelated preferences.
The Builder Elixir Cart can be a stopping target. It needs two stable post-collection
capacity readings; five failed cart visits stop farming without treating missing data as full.
Low available Windows memory can trigger bounded emulator recovery between attacks after
a 30-minute cooldown. Observe and Pause never trigger memory recovery. Restart waits for
the selected instance to stop; Stop cancels shutdown/startup polling.
Offline attack regressions cover Home and both Builder stages. Wall replays cover both
villages from list scanning through confirmation, reserve refusal and rejection of unchanged
post-confirmation balances. Their HUD positions reuse captured pixels; successful spending
accounting uses an explicitly simulated prior balance, not a captured live upgrade.
Live gameplay parity remains unverified.
Captured English result layouts require two independent guards and exactly one match. Absent currencies remain
unavailable; a separately displayed bonus is added only with explicit inclusion semantics.
The captured Home victory has unverified bonus inclusion, so its aggregate earnings remain
unavailable. Home silver stars and uncaptured recovery screens are also unavailable rather
than inferred. Native numeric regressions consume `crates/core/tests/fixtures/english-readings.json`, including
the HUD balance whose leading digits the Python baseline drops. The Rust reader preserves
that captured value without lowering confidence thresholds. Rust tests also verify fixture
redaction, annotation geometry and unavailable values. Blacked-out stars are not zero stars.
Capacity is displayed only when both the village and its calibrated storage tooltip are
recognized; isolated tooltip fixtures cannot identify a village. The UI follows TDM's
spacing, Manrope and Material Design Icons with a neutral dark palette, muted gold accents,
open sections and explanations behind round info buttons.

From the repository root, after preparing the native dependencies:

```powershell
npm ci --prefix frontend
npm --prefix frontend run format:check
npm --prefix frontend test
npm --prefix frontend run build
npm exec --prefix frontend -- playwright install chromium
npm --prefix frontend run test:browser
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run -p coc-bot-desktop --features desktop-fixture,custom-protocol --locked
```

The fixture build uses a captured English frame, temporary settings/history and a fake
transport. It has a separate application identifier and does not save window preferences.
Never package `desktop-fixture` for users. The browser suite mocks native IPC and checks
controls, errors, help popovers, narrow layouts and accessibility; it never opens an emulator.
For the real desktop, omit `desktop-fixture`. That app stores settings/history in its
Windows local app-data directory and connects only after selecting an instance and clicking
Observe or Start. Closing cancels device work and drains the owned worker. Screenshots stay in memory.
The unbundled binary requires the OpenCV DLL on PATH. `./.github/scripts/build-windows.ps1` builds
an x64 NSIS installer with OpenCV, Visual C++ runtime DLLs, calibration and applicable
notices beside the executable. It stages the installer and a source/hash manifest in
ignored `artifacts/windows/`. `-DebugBuild` is for local packaging checks and cannot be
published. Building never starts the desktop app or an emulator. An installer build alone
does not establish clean-machine installation or live gameplay compatibility.
The installed executable accepts `--check-installation`: it loads bundled assets and
native libraries, opens a temporary SQLite database and drains the worker, then exits
with status 0 on success or 1 on failure. It does not open a window, query a device or
touch personal app data. Run `./.github/scripts/test-windows-install.ps1 -Installer PATH_TO_SETUP_EXE` against a built installer
to check installation, this diagnostic and uninstallation in an isolated directory.
That script refuses an existing CoC Bot installation or running app.

Keep README short: logo, status, essential links, scope and attribution. Features and
fixes do not automatically need README additions. User guides belong in the GitHub wiki
once it is published; verify links before moving instructions. Retired guides and media
remain in Git history, not in a second documentation tree.
Keep wiki editing restricted to collaborators. Do not create a second planning/docs tree.
Contributor policy stays here, technical contracts in AGENTS and release notes in CHANGELOG.

## Pull requests and commits

1. Inspect the working tree and preserve existing user changes. Start a descriptive
   `feat/` or `fix/` branch from current canonical `main`.
2. Make one coherent change. Use conventional commits and PR titles:
   `type(scope): description`, with optional scope and optional `!` for breaking changes.
   Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`.
   Example: `fix(vision): reject unreadable wall prices`. No assistant branding.
3. Add meaningful regression coverage for changed behavior and update affected docs.
   Avoid unrelated dependency upgrades, formatting churn or speculative abstractions.
4. Fetch and integrate latest `origin/main` before final validation/review, and again
   before merge if it advances. Resolve conflicts deliberately and rerun affected checks.
5. Obtain independent adversarial review. Use the [PR template](.github/pull_request_template.md)
   and keep incomplete work in draft. Ordinary changes go through PRs, never directly to main.
6. When merging is authorized, use a merge commit with a conventional subject and PR
   number, such as `feat(desktop): add application shell (#12)`. Release publication
   needs separate explicit authorization; a normal merge publishes no release.

The initial import seeds the empty repository; subsequent setup and code changes follow
the PR policy. Never force-push shared history, rewrite published tags or bypass required checks.

## Validation and review

The required **Validation** check runs on Windows for PRs, main pushes and manual runs.
It checks whitespace, Rust formatting, Clippy, Rust fixture tests, frontend type/build,
browser/accessibility checks and repository automation tests. PR titles must be
conventional. CI uses read-only repository
permissions and no secrets; it never runs the live bot or publishes an artifact.

Use focused local checks while editing. A successful CI run on the final revision supplies
the complete baseline; do not repeat it locally without a reason. Documentation-only local
checks may focus on links, commands, accuracy and formatting, but CI still runs. Add Rust
fmt/Clippy/tests and frontend format/type/unit/build/accessibility checks when that code exists.
Mocks do not prove live gameplay. Disclose unavailable checks and inherited warnings.

Every PR, including docs, needs a human or a separate read-only review agent that did not
author the change. An author's reread is not independent review. For agent-authored work,
use a separate reviewer when available; otherwise request an independent human and keep
the PR in draft until review is complete. This review does not replace GitHub approvals.

Give the reviewer the goal, final diff/revision, this policy, acceptance criteria and test
evidence. Ask for correctness, security/privacy, missed cases, regressions, scope and false
documentation claims. Record the reviewer, revision, findings and resolutions. Recheck
substantive fixes; unresolved blockers prevent readiness. Never fabricate results.

- [ ] Latest canonical main is integrated.
- [ ] Diff is focused and excludes secrets, local data and unrelated changes.
- [ ] Applicable checks pass on the final revision; gaps are disclosed.
- [ ] README, AGENTS, affected guides and notices reflect actual behavior.
- [ ] Independent review is recorded and blocking findings are resolved.
- [ ] PR description matches the final implementation and validation evidence.

## Releases

Successful canonical main validation retains the exact Windows installer for 30 days.
PR validation has read-only permissions and receives no release secrets. No push or merge
publishes a release. Local dirty builds are marked and rejected by the publisher.

The owner can dispatch **Prepare release** with a new SemVer version. It opens a draft
version/lockfile PR using `PUBLISHER_TOKEN`; add reviewed bullets under `## VERSION` in
CHANGELOG.md, then complete normal validation and independent review. After merging,
**Publish release** downloads that revision's successful main artifact, checks its source
and SHA-256, signs it and verifies every uploaded draft asset before publication. It fails
if main has advanced or the tag already exists; it never rebuilds the application.

The `release` environment needs owner-held `RELEASE_TOKEN`, `TAURI_SIGNING_PRIVATE_KEY`
and its optional `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Resolve and record upstream permission
in NOTICE.md before setting repository variable `UPSTREAM_PERMISSION_CONFIRMED=true`.
Keep all secrets out of PR jobs, source, logs and chat. Environment approval rules can
provide an additional publication gate. These credentials and permissions are not provisioned
by committing the workflows, and no release has been published.

Following TDM's release layout, published assets are `coc-bot-VERSION-windows-x64-setup.exe`,
`latest.json` and `SHA256SUMS`. The title is `CoC Bot vVERSION`; notes use reviewed changelog
bullets followed by compare and issue links. The signed manifest supplies update metadata;
the desktop checks quietly at startup and offers verified downloads in Settings, with a
version link beside GitHub when an update is available. Downloading is cancellable and
does not stop the bot. Installation requires an explicit click, drains bot/database work
and runs the full NSIS installer before restarting. Failed installation requires a restart;
the drained engine cannot resume in place. Development and fixture builds cannot install.
Settings/history remain in local app data; the installer replaces the bundled program,
native libraries and calibration. The public verification key belongs in
`desktop/tauri.conf.json`; keep its private counterpart backed up outside this PC before
resetting it. GitHub can use its stored secret but cannot return it for a backup.
The checksum-pinned Minisign verifier checks publication signatures against the embedded
public key and version before metadata is written. Offline tests use a disposable public
signature fixture, never release credentials, a live account or a real update installation.
Updater signatures are separate from Windows Authenticode signing. Validate installation,
uninstallation, native dependencies and update signatures before the first public release.
