# Notices and attribution

easyNode is Copyright (c) 2026 Matthias Mende and the easyNode contributors, and
is distributed under the MIT License. See `LICENSE`.

## BTX and Bitcoin Core

easyNode installs, configures and supervises `btxd`, the BTX node, which it does
not include or modify. BTX is MIT licensed and derives from Bitcoin Core, also
MIT licensed, Copyright (c) 2009-present The Bitcoin Core developers and
Copyright (c) 2026 The BTX developers.

Source: https://github.com/btxchain/btx

## Fonts

`apps/node/src/assets/fonts` contains JetBrains Mono, licensed under the SIL Open
Font License 1.1. The full licence text ships beside the font files as `OFL.txt`.

## Rust and JavaScript dependencies

The dependency graphs of `apps/node/src-tauri` and `crates/btx-core` resolve
entirely to permissive licences: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0 and
MPL-2.0. There is no GPL, AGPL or SSPL anywhere in either tree.

The MPL-2.0 crates are file level copyleft. We depend on them without
modification, which places no condition on this project's licence.

Run `cargo tree` or `npm ls` for the current full list.
