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
- Keep README as short as TDM's: logo, tagline, screenshot, getting-started links,
  essential project scope and license/credits. A new feature or fix does not require
  a README addition. Do not append implementation details, progress reports, test results,
  calibration notes, exhaustive feature lists or repeated explanations of UI controls.
  Update it only when essential setup, usage, links or project scope change.
- Put contributor instructions in CONTRIBUTING, technical contracts here, release changes
  in CHANGELOG and user guides in the published wiki. Retired Python/web guides and media
  belong in Git history; do not restore `LEGACY.md`, `docs/` or duplicate documentation trees.
- The desktop UI and calibration are English only. Indonesian profiles and text anchors
  were removed at the maintainer's request. Keep shared icons and digits used by English;
  missing English screens remain explicitly uncalibrated, with no language fallback.
- Keep LICENSE, NOTICE and bundled font notices. No upstream project license was found;
  do not claim PolyForm relicenses imported code or that a translation resolves permission.

## Current baseline and target

The Rust workspace currently implements calibrated screen and digit recognition in
`crates/core/`, with bounded ADB/LDPlayer processes, guarded input, framebuffer capture
and SQLite session history with typed telemetry and offline regression tests. The Tauri
application and React dashboard support observation, farming/wall controls, deployment editing,
settings and session history. Gameplay is validated offline against captured screens and fake
devices; uncaptured English result/recovery calibration and live parity remain unfinished.
Shared calibration lives in `assets/`; test captures and annotations live in
`crates/core/tests/fixtures/`. The Python runtime and old dashboard are retired;
their baseline remains in Git history at `a3b3346` for comparison.

The target is Windows + LDPlayer + ADB, a modular Rust backend, OpenCV recognition,
SQLite persistence and a Tauri 2 + React/TypeScript desktop UI. No Linux server, Docker,
hosted relay or game-protocol reverse engineering. Do not restore the retired relay,
Netlify configuration or old web dashboard as runtime dependencies.

Use `crates/core/` for recognition, device control, farming,
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
- Automatic and custom deployment use the same ordered-action executor. Plans follow army
  slots and categories across the complete bar; capture fresh portraits and quantities for
  each attack without requiring a saved unit's appearance. Read overflow by measured,
  overlapping scrolls; never infer movement from the requested swipe distance.
- Normalize zoom, certify pan limits and measure the camera range before mapping the outer
  border. Track motion within the current battle using spatially distributed OpenCV features,
  without scenery template packs. Keep gestures bounded and exclude game controls.
  Unverified movement or an unconfirmed drop/ability stops the plan; recovery must not replay
  uncertain input. Results, stage transitions and surrender limits interrupt preparation.
- A custom plan schedules its own spells and abilities. Known spent Home abilities and
  defeated Builder units can be skipped without tapping; unreadable states are not spent
  abilities. Automatic recurring Builder hero activation requires a fresh full-charge check.
- Builder stages share active-battle recognition. Establish stage two from its observed
  countdown; damage alone cannot start a new plan. Fresh inputs and manual army reads must
  respect that stage, and completed-stage damage remains latched until a verified transition.
- Preserve resource reserves, price/icon checks and gem protections. Never spend gems,
  buy items or upgrade non-wall buildings. Do not remove safety checks to improve speed.
- Preserve loot, resource and capacity observations with their timestamps and
  validity. Show unknown/stale values honestly; do not convert failed reads into zero.
- Wall levels/distributions are not currently detected. Do not advertise them as working.
  Evaluate `nihatxp/clash-of-clans-vision` only when recognition work starts, including its
  model/code licenses, offline use and performance. Do not vendor it during repo setup.
- Port regression fixtures with behavior. Validate vision against real captured fixtures
  and test safe failure paths before claiming parity. Do not test against a live account
  without explicit authorization.

## Dashboard contracts

The target UI provides start/stop, farming configuration, live screenshots, gold/elixir/
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
Keep React sources and fixture assets tracked. Do not copy the reference project's
generated-web ignore rule onto this repository.
Add Rust and frontend checks alongside the implementation; never claim absent build
steps passed. Preserve workflow trust boundaries, immutable action pins and read-only
PR permissions. Ordinary CI and merges never deploy, contact a game account or publish.
