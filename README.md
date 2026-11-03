<p align="center">
  <img src=".github/assets/clash-shield.svg" width="128" alt="CoC Bot shield logo">
</p>

<h1 align="center">CoC Bot</h1>

<p align="center">Clash of Clans automation for LDPlayer, being rebuilt for Windows in Rust.</p>

<p align="center">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/original_code-PolyForm_Noncommercial-bb3535" alt="Original contributions: PolyForm Noncommercial"></a>
  <a href="https://github.com/ohne-b/coc-bot/actions/workflows/validation.yml"><img src="https://github.com/ohne-b/coc-bot/actions/workflows/validation.yml/badge.svg" alt="Validation"></a>
</p>

## Get started

This repository currently contains the Python [coc-auto](https://github.com/peenjeee/coc-auto)
baseline, including its history, calibration assets, dashboard and tests. The Rust application
has not been implemented yet; there is no desktop release to download.

The target is a Windows application with **Rust, OpenCV, ADB, Tauri 2, React/TypeScript
and SQLite**. It will preserve farming, troop deployment, resource collection, wall
upgrades and recovery, with a native dashboard for controls, screenshots and session statistics.

The original [Python guide](LEGACY.md) is retained for reference, including its upstream
links and optional relay/deployment instructions. Those hosted services are not part of
the Rust desktop plan. Development and offline checks are in [CONTRIBUTING.md](CONTRIBUTING.md).

> [!IMPORTANT]
> The inherited calibration targets LDPlayer at 1280×720, DPI 240, with the game in
> Bahasa Indonesia. Other resolutions or languages need calibration. Automation violates
> Supercell's rules and can result in an account ban. Not affiliated with Supercell.

The separate [English profile](config.english.json) uses the same resolution and DPI;
the Indonesian profile remains unchanged. Run it with
`python bot.py --config config.english.json --walls home --live` and put this PC's
device settings in ignored `config.english.local.json`. It retains 20M gold **and**
elixir batching and 1M reserves for Home walls. The current Home price calibration
targets green Hammer Jam discounts; revalidate it when that event ends. Screens not
yet captured in English are listed in `calibration_pending_screens`; an unfamiliar
dialog fails recognition and cannot be confirmed automatically.
Builder stage two is recognized, but surviving-camp selection and its stack deployment
point still need correction and another live check; unattended use remains Home-only.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for development, conventional commits and PR review.
Report problems through [GitHub issues](https://github.com/ohne-b/coc-bot/issues), with
redacted logs and your commit, emulator version, resolution and game language.
See [SECURITY.md](SECURITY.md) for private vulnerability reports.

The new desktop UI will be English only initially. Existing Python messages and the
Indonesian calibration are retained alongside the English recognition profile.

## License and credits

Original contributions by ohne-b (OhneB) use [PolyForm Noncommercial 1.0.0](LICENSE.md).
This is source-available work for noncommercial use. Keep its license and required notices.

Based on [peenjeee/coc-auto](https://github.com/peenjeee/coc-auto) and its contributors.
**The imported upstream revision has no project license.** PolyForm does not relicense
that code or grant permission to use, modify or redistribute it. See [NOTICE.md](NOTICE.md)
for the exact baseline, license scope and retained third-party notices.
