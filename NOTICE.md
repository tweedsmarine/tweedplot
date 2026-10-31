# Licensing of TweedPlot

Copyright © 2024–2026 Chris Harris, TweedsMarine (https://tweedsmarine.nz)

| What | Licence |
| --- | --- |
| Source code (Rust crates, `app/src`, `app/src-tauri`, `tools`, build files) | [GNU GPL v3.0 or later](LICENSE) (`GPL-3.0-or-later`) |
| User manual (`app/public/manual`, `tools/manual`) and branding artwork other than the name and logos (see below) | [Creative Commons Attribution-ShareAlike 4.0](LICENSE-DOCS.txt) (`CC-BY-SA-4.0`) |
| The name "TweedPlot", "TweedsMarine" and their logos (`branding/`, `app/public/logo-sails.svg`, app icons) | Not licensed for use as the name or logo of a modified version: see [TRADEMARKS.md](TRADEMARKS.md) |
| Third-party code and data | Their own licences: see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) |

Each source file is covered by the GPL without a per-file header. A file may carry its own
`SPDX-License-Identifier` line where it differs.

## What the GPL means here

You may use, study, change and share TweedPlot, including commercially. If you distribute a
modified version, or a program that includes TweedPlot code, you must release the complete
source of it under the GPL too, keep the copyright and licence notices, and mark your changes.
You may not distribute it as a closed-source product.

## Official builds and the S-63 credentials

TweedsMarine's official installers are built with TweedsMarine's own IHO S-63 manufacturer
credentials and activation signing key, which are **not** part of this repository and are not
licensed by it. A build from this source uses the IHO's generic test credentials and opens
only IHO test data (see `crates/chart/build.rs`). Do not put real credentials in a commit.
