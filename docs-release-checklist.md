# TweedPlot open-source release: reference summary (31 Oct 2026)

## Decisions made
| Question | Decision |
|---|---|
| Repo name | Create a new repo **TweedPlot**, or rename racenav (GitHub redirects the old URL). Recommended: a new repo made from a clean snapshot. |
| Licence | **GPL-3.0-or-later** for the code (copyleft: a closed-source fork would breach the licence). **CC BY-SA 4.0** for the manual and artwork. |
| Why GPL rather than AGPL, LGPL or MPL | TweedPlot is a desktop app. GPL protects the whole program. AGPL only adds protection for hosted services. LGPL and MPL are weaker. Switch to AGPL later if a server component appears. |
| Name and brand | The GPL doesn't cover the name. A trademark policy is in place. Consider registering **TweedPlot** with IPONZ. |
| Contributions | You review and merge every PR. Contributors sign off commits (DCO) and grant you a relicensing licence (CLA-style wording). |
| Testers without GitHub accounts | A private repo can't serve them. Use a file-host link (OneDrive or similar) or your own site, or publish the open-source repo and put installers in its Releases. |
| Non-commercial licence | Dropped. It isn't OSI open source. Copyleft allows commercial use but forces forks to stay open. |

## Key facts established
- **S-63 activation codes** (TP1- / TPA1-) only unlock encrypted charts for one computer. They are not a software licence check.
- **No real secrets in the repo or its 226 commits.** The only hit was the standard `secrets.GITHUB_TOKEN` placeholder in `release.yml`.
- **The IHO test credentials are public, not secret:** test user permit `66B5…3130`, M_KEY `10121`, the test signing key (in a `#[cfg(test)]` block), and the test public key in `build.rs`.
- **Real M_ID, M_KEY, `signing.key` and `register.csv`** live in `~/.tweedplot-oem`, outside the repo, and were never committed.
- **Anyone building from source gets test credentials** and can open only the IHO test data.
- **Dependencies** (about 560 Rust crates, 45 JS packages) are all GPL-3.0 compatible. MPL-2.0 crates are fine if unmodified. `unescaper` and `r-efi` are used under their MIT option. The font is OFL-1.1.
- **Tracking sample** (`crates/tracking/tests/ggr2026_history_sample.b64`) holds only positions for two 1968 "ghost" boats: no personal data.
- **Personal data elsewhere:** only `chris@tweedsmarine.nz` (manifests, user-agents) and the public `query@saildocs.com`.

## Done in the repo
- Added `LICENSE` (GPL-3.0) and `LICENSE-DOCS.txt` (CC BY-SA 4.0).
- Added `NOTICE.md`, `TRADEMARKS.md`, `CONTRIBUTING.md`, `THIRD-PARTY-NOTICES.md`.
- Workspace `Cargo.toml`: `UNLICENSED` changed to `GPL-3.0-or-later` (all 16 crates inherit it). Licence added to `app/package.json`.
- README licence and contributing section.
- `.gitignore` extended: `oem.txt`, `signing.key`, `signing.pub`, `register.csv`, `.env*`, `*.pem`, `*.pfx`.

## Checklist before release

**Legal and licence**
- [ ] Confirm the copyright holder and years in `NOTICE.md` ("Chris Harris, TweedsMarine, 2024–2026").
- [ ] Have the CLA-style wording in `CONTRIBUTING.md` checked by a lawyer, or use CLA Assistant, before accepting outside code.
- [ ] Decide on trademark registration (IPONZ) for TweedPlot and the logos.
- [ ] Read the IHO S-63 manufacturer terms before shipping builds with your real credentials.
- [ ] Optionally add `SPDX-License-Identifier` headers to source files.

**Remove or replace test data**
- [ ] Remove the IHO S-63 test data: `crates/chart/tests/data/s63/` and `crates/chart/tests/data/1B5X02NE.000`.
- [ ] Check which tests use them, then fix or skip those tests (including `crates/chart/examples/s63_check.rs`).
- [ ] Check whether `crates/chart/src/enc/IHO.CRT` and `SA_PUBLIC_KEY` are needed at runtime. If so, decide how to supply them. Consider removing the IHO test user permit and M_KEY from `mod.rs` and tests too.
- [ ] Confirm you may redistribute `crates/chart/testdata/coastal_dem.tiff`, the `crates/grib/tests/data/*.grb2` files, and the polars `First40_7.pol` and `Morgane_median.pol` (the Morgane polar may be owned by someone else).
- [ ] Clear anything else with unclear provenance.

**Repository**
- [ ] Create the repo from a clean snapshot (no old history). That drops the 32 MB `world_depth.gpkg` from history and the removed files. Or use Git LFS for that file.
- [ ] Decide whether to commit the untracked `branding/icon-nav-fish*` and `branding/tweedplot-flyers.html`. Delete the stray empty root `package-lock.json`.
- [ ] Update remaining "racenav" references (user-agent strings, repo URL, `SETUP-WINDOWS.md`, which says to create a private `racenav` repo).
- [ ] Remove or replace the README link to the private Claude artifact spec.
- [ ] Re-run a secret scan on the final snapshot (gitleaks or trufflehog). Confirm no `.tweedplot-oem` content.

**GitHub setup**
- [ ] Start **private**; go public only after the items above are done.
- [ ] Enable secret scanning and push protection.
- [ ] Protect `main`: require PRs, require your review, block force-push.
- [ ] Add a `CODEOWNERS` file with your username.
- [ ] Add issue and PR templates mentioning the DCO and no-secrets rules.
- [ ] Optionally add the CLA Assistant bot, or enforce DCO sign-off with a check.

**Releases and S-63**
- [ ] Build official installers on your own PC with the OEM folder present.
- [ ] CI (`.github/workflows/release.yml`) builds with the IHO test credentials. Either store `oem.txt` and `signing.pub` as encrypted secrets, or don't publish CI installers as official.
- [ ] Generate and ship the full dependency licence texts with each installer:
  ```
  cargo install cargo-about
  cargo about init
  cargo about generate about.hbs > licenses-rust.html
  cd app && npx license-checker-rseidelsohn --production --csv > ../licenses-js.csv
  ```
- [x] Add LINZ, ECMWF and GSHHG attribution in the app, and the OFL licence for the font.
- [x] Online maps (OpenStreetMap, OpenSeaMap): optional, off by default, Rust-enforced no-traffic gate, CSP tightened. Bundled land is OSM-derived (ODbL), so the OSM credit stays.
- [ ] Test online maps in the real Windows app: tiles load when on, no requests when off, nothing breaks under the new CSP (csp in `tauri.conf.json`), attribution shows.
- [ ] OSM tile policy: fine for light use; for wider distribution point the base-map URL at your own or a commercial tile provider.
- [ ] Back up `~/.tweedplot-oem`. Losing `signing.key` or `register.csv` would be serious.

**Testers who don't use GitHub**
- [ ] Share installer links through a file host, or through public GitHub Releases once the repo is public.
- [ ] Optionally add a short tester notice (pre-release, no warranty, not for primary navigation).

**Final check**
- [ ] Run `cargo test` and `npm test` after the test data is removed.
- [ ] Build a fresh clone to confirm it compiles using only the test credentials.
- [ ] Re-read `README.md`, `NOTICE.md` and `TRADEMARKS.md` for accuracy.
