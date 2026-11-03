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
The Python source is the migration baseline, not a sidecar for the final application.
Repository setup does not implement the rewrite.

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
and a Windows SDK), Rustup, Python 3.13 and Node.js 24. Rust is pinned by
`rust-toolchain.toml`. The native dependency script downloads checksum-pinned OpenCV
4.12.0 and portable LLVM 20.1.8 into ignored `.tools/`; it needs no administrator rights.
The first download is about 1.1 GB. Dot-source it in each PowerShell session before
running Rust binaries so the OpenCV DLL can be found.

From the repository root:

```powershell
. ./scripts/windows-dependencies.ps1
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p coc-bot-core --example inspect -- `
    legacy/python/config.json legacy/python/fixtures/english-home.png
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

The Python baseline remains an offline comparison suite during migration:

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r legacy/python/requirements.txt
.\.venv\Scripts\python.exe -m unittest discover -s legacy/python -v
.\.venv\Scripts\python.exe -m unittest discover -s .github/scripts -p "test_*.py" -v
Get-ChildItem legacy/python/web -Filter *.js | ForEach-Object {
    Get-Content -Raw -Encoding UTF8 $_.FullName | node --input-type=module --check
    if ($LASTEXITCODE -ne 0) { throw "JavaScript syntax check failed: $_" }
}
git diff --check
```

Tests use fixtures, temporary storage and loopback test servers. Do not launch a real
bot or reuse a live emulator for automated validation. The inherited bot has optional
remote-control/relay behavior; consult [LEGACY.md](LEGACY.md) before running it manually.
Personal legacy settings belong in ignored `legacy/python/config.local.json`, not
the shared calibration. Existing root-level local settings and captures are not moved
automatically; copy settings into the legacy directory only if you still run that bot.
Never commit a virtual environment, runtime captures, logs, credentials or databases.

| Location | Current purpose |
| --- | --- |
| `crates/core/` | Rust recognition, guarded device control, typed telemetry and SQLite session history |
| `crates/core/tests/` | Offline Rust regressions against captured fixtures |
| `desktop/` | Tauri lifecycle and scoped native commands |
| `frontend/` | React/TypeScript dashboard and offline browser tests |
| `legacy/python/` | Python source, dashboard, settings examples, calibration and fixtures |
| `scripts/` | Native Windows build dependency setup |
| `.github/` | Repository policy, validation and supplied branding |

The desktop supports observation and development farming/wall modes: selected-instance display
checks, screenshots, recognition, dynamic deployment, bounded recovery and session history.
Start enables guarded game input; Observe never does. Pause freezes active time and dispatches
no gameplay steps. Settings remain locked until Stop. Farming, device and deployment saves
patch their own sections so stale forms cannot overwrite unrelated preferences.
Offline attack regressions cover Home and both Builder stages. Wall prices and currency
checks use captured English dialogs; full wall-flow and live gameplay parity remain unverified.
Result layouts require two independent guards and exactly one match. Absent currencies remain
unavailable; a separately displayed bonus is added only with explicit inclusion semantics.
Uncaptured result fields and recovery screens remain unavailable rather than inferred.
Capacity is displayed only when both the village and its calibrated storage tooltip are
recognized; isolated tooltip fixtures cannot identify a village. The UI uses TDM's dark
palette, spacing, Manrope and Material Design Icons, with explanations behind info buttons.

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
The unbundled binary requires the OpenCV DLL on PATH; installer packaging is separate work.

Keep README short: logo, status, essential links, scope and attribution. `LEGACY.md`
and inherited `docs/` media preserve the original instructions. Future user guides
belong in the GitHub wiki once it is published; verify links before moving instructions.
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
It checks whitespace, Rust formatting, Clippy, Rust fixture tests, the Python regression
suite, repository automation tests and legacy JavaScript syntax. PR titles must be
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

There is no Rust release pipeline yet. Keep planned work under `Unreleased` in CHANGELOG.
Do not publish a Python or Rust package as part of repository setup. Before the first
desktop release, resolve upstream permissions and add a Windows build/signing workflow
that publishes the exact validated artifacts, with all applicable license notices.
Keep signing credentials out of PR jobs, source, logs and chat.
