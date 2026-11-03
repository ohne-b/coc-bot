# Agent instructions

Read [CONTRIBUTING.md](CONTRIBUTING.md) before planning, editing, testing or reviewing.
Its PR checklist is mandatory. This is the single repository harness; do not create
alternate agent instruction files or links.

## Workflow

- Use descriptive `feat/` or `fix/` branches, focused conventional commits and PRs against
  `ohne-b/coc-bot:main`. Keep branches, commits and documentation free of assistant branding.
- Preserve existing user changes, calibration, settings, logs and backups. Do not alter
  a running emulator or bot unless the task authorizes it. A requested rewrite covers
  its necessary cleanup, not unrelated changes or deletion of personal data.
- Integrate current canonical main before final checks/review and again before merge
  if it advances. Keep incomplete PRs in draft. Report actual tested/reviewed revisions.
- Use a separate read-only reviewer for required independent adversarial review; the
  author cannot approve their own work. Follow CONTRIBUTING for evidence and rechecks.
- When merging is authorized, use a conventional merge subject with the PR number.
  Release publication is separate. Never weaken checks to make a PR appear complete.
- Keep README a short entry point. Preserve the inherited `LEGACY.md` and `docs/` media;
  new user guides belong in the published wiki. Do not add plans or duplicate docs trees.
- The desktop UI and calibration are English only. Indonesian profiles and text anchors
  were removed at the maintainer's request. Keep shared icons and digits used by English;
  missing English screens remain explicitly uncalibrated, with no language fallback.
- Keep LICENSE, NOTICE and bundled font notices. No upstream project license was found;
  do not claim PolyForm relicenses imported code or that a translation resolves permission.

## Current baseline and target

The Rust workspace currently implements calibrated screen and digit recognition in
`crates/core/`, with raw framebuffer decoding and offline regression tests. The Tauri
application and gameplay engine are not implemented yet. The Python coc-auto baseline,
dashboard, calibration and tests live in `legacy/python/`; retain them and their history
until equivalent Rust behavior has meaningful regression coverage.

The target is Windows + LDPlayer + ADB, a modular Rust backend, OpenCV recognition,
SQLite persistence and a Tauri 2 + React/TypeScript desktop UI. No Linux server, Docker,
hosted relay or game-protocol reverse engineering. The inherited relay, Netlify config
and web dashboard are reference material, not architecture for the new application.

When implementation begins, use `crates/core/` for recognition, device control, farming,
application state and persistence; `desktop/` for Tauri lifecycle/IPC; and `frontend/`
for React/TypeScript. Create modules as behavior is implemented, not as empty scaffolding.
Use concrete Rust structs, methods, composition and typed enums. Reuse shared policies;
avoid forwarding layers, speculative traits and duplicate bot logic in the frontend.

Use native Tauri IPC with scoped capabilities and validated inputs. Do not add a Python
sidecar or localhost server to the final desktop app. Bound background work, support
cancellation and drain owned work on shutdown. Keep blocking OpenCV/ADB work off the UI.
Pass subprocess arguments directly; do not interpolate settings into shell commands.
Persist settings and history transactionally in SQLite; failed reads must not silently
reset data. Keep personal app data separate from shared calibration and source.

## Recognition and gameplay contracts

- Preserve Home Village and Builder Base farming, troop/spell/hero deployment, resource
  collection, wall upgrades and bounded crash/disconnect recovery.
- The current calibration expects English at 1280×720 and DPI 240. Resolution,
  scale, language and device coordinates are calibration inputs, not universal constants.
- Gate input on a recognized screen. Unknown or unreadable state must not trigger blind
  taps. Keep dry-run input guards, cancellation, timeouts and bounded recovery attempts.
- Preserve resource reserves, price/icon checks and gem protections. Never spend gems,
  buy items or upgrade non-wall buildings. Do not remove safety checks to improve speed.
- The Python code already reads loot, resources and capacity, but does not expose every
  value as structured dashboard data. Preserve observations with their timestamps and
  validity. Show unknown/stale values honestly; do not convert failed reads into zero.
- Wall levels/distributions are not currently detected. Do not advertise them as working.
  Evaluate `nihatxp/clash-of-clans-vision` only when recognition work starts, including its
  model/code licenses, offline use and performance. Do not vendor it during repo setup.
- Port regression fixtures with behavior. Validate vision against real captured fixtures
  and test safe failure paths before claiming parity. Do not test against a live account
  without explicit authorization.

## Dashboard contracts

The planned UI provides start/stop, farming configuration, live screenshots, gold/elixir/
dark-elixir balances and capacity, attacks, loot, resources/hour, wall upgrade counts,
session history and errors. Wall-level distributions require additional recognition.
Use structured Rust events/snapshots instead of parsing human-readable log text.
Keep state ownership in Rust and reject stale frontend responses after a new session.

Reuse `.github/assets/clash-shield.svg`, preserving its artwork and aspect ratio.
Keep the UI accessible: keyboard operation, visible focus, meaningful labels, readable
contrast and proper loading/empty/error states. Use shared controls and plain language.
Show times in a local 24-hour clock, with explicit units for rates and capacities.
Avoid filler, technical implementation details and middle-dot separators in new UI copy.

## Validation

Use CONTRIBUTING's real commands for the current code. Automated tests use fixtures,
temporary data and fake device/transport boundaries; never live accounts or emulators.
Keep the legacy `legacy/python/web/` sources tracked until deliberately retired. Do not copy the
reference project's generated-web ignore rule onto this repository.
Add Rust and frontend checks alongside the implementation; never claim absent build
steps passed. Preserve workflow trust boundaries, immutable action pins and read-only
PR permissions. Ordinary CI and merges never deploy, contact a game account or publish.
