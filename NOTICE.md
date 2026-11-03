# License scope and attribution

Required Notice: Copyright (c) 2026 ohne-b (OhneB)

## Original contributions

[LICENSE.md](LICENSE.md) contains PolyForm Noncommercial 1.0.0. It applies only to
original contributions whose copyright holders offer them under those terms. It does
not replace third-party rights or supply a missing upstream license.

## Imported coc-auto baseline

- Source: [peenjeee/coc-auto](https://github.com/peenjeee/coc-auto).
- Upstream commit: [`1028b3b436fa3658ed9b52143ecf56cc9cf5f0e5`](https://github.com/peenjeee/coc-auto/commit/1028b3b436fa3658ed9b52143ecf56cc9cf5f0e5).
- Imported on 2026-11-03, preserving upstream Git history and author attribution.
- The local baseline also includes existing changes to `bot.py`, `test_bot.py` and
  the original README, three calibration images and `fixtures/wall-discount-price.png`.
  These are recorded separately in the `chore: import local coc-auto baseline` commit.
- The original README, including local additions, and its media remain in
  [Git history](https://github.com/ohne-b/coc-bot/blob/a3b3346df05ab905c7990f6697509ca5fa6b35a5/LEGACY.md).
- The Python runtime and old dashboard were retired after the Rust port; their final
  comparison baseline is preserved at commit `a3b3346`. Calibration and fixtures retain
  their history and attribution after relocation.

No project-wide LICENSE, COPYING or equivalent permission grant was found in that
upstream revision. Its authors retain their rights. This repository does not assert
that the inherited implementation is licensed under PolyForm or is freely reusable.
Resolve upstream permissions before distributing a derivative application. A Rust
translation alone does not resolve the inherited rights.

GitHub's [repository licensing guidance](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)
explains the distinction between viewing/forking a public repository and a software license.
Record any later permission grant here without erasing the original attribution.

## Bundled assets

The retired dashboard's Lilita One font notice is retained as
[SIL Open Font License 1.1](assets/licenses/Lilita-One-OFL.txt), including its copyright
and reserved font name. The unused font binary is no longer included.
Game screenshots, calibration crops and game names retain their respective owners'
rights; the project license does not grant rights to Supercell assets or trademarks.

The shield in `.github/assets/clash-shield.svg` is the artwork supplied by the project
maintainer. Preserve its vector artwork and aspect ratio when reusing it for the app.

The desktop dashboard adapts layout tokens and basic controls from
[ohne-b/twitch-drops-miner](https://github.com/ohne-b/twitch-drops-miner), whose original
contributions by ohne-b use PolyForm Noncommercial 1.0.0. This does not change inherited
coc-auto licensing. Manrope and Material Design Icons notices are retained in
[frontend/public/licenses/](frontend/public/licenses/) and bundled with the desktop app.

Windows installers bundle OpenCV 4.12.0, its Apache 2.0 license and the accompanying
third-party notices from the checksum-pinned Windows distribution. Visual C++ runtime DLLs
come from the build host's Visual Studio redistributable directory and retain Microsoft's
terms. No ADB, LDPlayer or game executable is redistributed with the desktop installer.
