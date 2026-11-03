# Changelog

## Unreleased

- Add SQLite session history, timestamped resource observations, confirmed-loot statistics,
  wall spending and paged error/recovery history, with offline persistence tests.
- Add bounded, cancellable ADB/LDPlayer processes, device discovery, screenshot capture,
  guarded taps/swipes and multitouch with cleanup after interruption.
- Start the Rust core with OpenCV screen and digit recognition, framebuffer decoding,
  captured-fixture regression tests and pinned Windows native build dependencies.
- Move the Python migration baseline and its dashboard into `legacy/python/`.
- Make English the only calibration profile, retain shared icons and digits, and remove
  Indonesian profiles, text templates and fallback anchors for uncaptured screens.
- Add real English UI-only regression captures and explicit coverage gaps while retaining
  existing spending checks.
- Ignore profile-specific local settings; keep Home wall batching at 20M of both
  resources and retain 1M of each after upgrades in the English profile.
- Import the coc-auto Python baseline with upstream history and existing local wall,
  recovery, calibration and regression-test changes.
- Add project branding, contribution and agent guidance, GitHub templates and validation.
- Record PolyForm Noncommercial terms for original contributions and the unresolved
  upstream license scope.

The Rust rewrite is in progress. No desktop release is available.
