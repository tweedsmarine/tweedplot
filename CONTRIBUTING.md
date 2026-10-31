# Contributing to TweedPlot

Thank you for helping. The maintainer is Chris Harris (TweedsMarine), who decides what is merged.
Open an issue before starting anything large.

## Making a change

1. Fork, then branch from `main`.
2. Build and test as in [DEVELOPING.md](DEVELOPING.md) (`cargo test`; `npm test` in `app/`).
3. Open a pull request that says what changed and why. Keep it to one subject.
4. Every pull request is reviewed by the maintainer before it is merged.

Conventions: en-GB spelling, WGS84 decimal degrees, bearings in degrees true, distances in NM,
speeds in knots, times in UTC.

## Licence of your contribution

Contributions are accepted under the project licence, **GPL-3.0-or-later**
(documentation under CC BY-SA 4.0). In addition, by submitting a pull request you agree that:

1. You wrote the contribution, or have the right to submit it under these terms
   ([Developer Certificate of Origin 1.1](https://developercertificate.org/)).
2. You keep your copyright, and you grant TweedsMarine a perpetual, worldwide, non-exclusive,
   royalty-free, irrevocable licence to use, modify, sublicense and relicense your contribution
   under any licence, including a different open-source licence or a commercial one. This lets
   the project change licence later without asking every contributor.
3. Your contribution contains no secrets, personal data, or third-party material (data, code,
   charts, polars) that cannot be redistributed under the project licence.

Sign each commit off to confirm this: `git commit -s` adds `Signed-off-by: Your Name <you@example.com>`.

## Never commit

- S-63 or S-57/S-101 chart data, permits (`PERMIT.TXT`), user permits, manufacturer IDs or keys,
  activation or signing keys, or anything from a `.tweedplot-oem` folder;
- API keys, tokens, passwords, or your own tracks, positions and boat data;
- downloaded chart, GRIB or bathymetry data.

Report a security problem privately to chris@tweedsmarine.nz rather than in a public issue.
