# TweedPlot Licence Manager

Tweedsmarine's own tool for S-63: it holds the IHO manufacturer key (M_KEY) and the
activation signing key, and keeps the customers, licences, seats and the HW_ID register.
It is never shipped to customers.

```powershell
cargo run --release -p tweedplot-licence
```

## How licences work

- **One licence = one vessel**: one HW_ID and one user permit. The customer gives the user
  permit to their chart supplier, and one set of chart permits then works on every
  computer on that boat.
- **Seats**: up to three computers on board (changeable per licence). Each seat is one
  installation of TweedPlot, activated with its own activation code (TPA2-…) made from that
  installation's code (TP2-…).
- Issuing again for the **same installation** gives the same seat; a **reinstall on the same
  computer** takes over that computer's seat. A fourth computer is refused unless you give a
  reason, which is logged.
- **Releasing**: Chart › Encrypted charts › Remove in TweedPlot deletes the decrypted charts and
  shows a release code (TPR2-…). **Release code…** here checks it against the licence's HW_ID
  and frees the seat. **End…** on a seat frees it without a code (lost or broken computer),
  with a reason in the log.
- The same computer on two vessels' licences is allowed (a laptop moved between boats) but
  you are told when it happens. TweedPlot shows the vessel and MMSI it is licensed to, and a
  note if the boat's MMSI in Settings differs.

## Where things are kept

The OEM folder is `%USERPROFILE%\.tweedplot-oem` (or `TWEEDPLOT_OEM_DIR`). Keep it out of
MEGA and other cloud sync.

| File | What | Secret? |
|---|---|---|
| `vault.key` | the vault's key, protected with Windows DPAPI for your Windows user | yes (useless elsewhere) |
| `licences.tpv` | the vault: M_KEY, signing key, customers, licences, seats, log (sealed) | yes (sealed) |
| `licences-test.tpv` | the test-mode vault | test data |
| `history\` | the last 20 versions of the vault, sealed | sealed |
| `oem.txt` | `M_ID=..` for building TweedPlot | no |
| `signing.pub` | the public key built into TweedPlot | no |
| `mode.json` | test mode on or off | no |

**Back up with a passphrase** (Settings › Backups): the vault only opens for your Windows
user on this PC, the backup opens anywhere with its passphrase. Keep it on two USB sticks in
two places and the passphrase on paper. Losing the vault loses every customer's HW_ID, and
they would need new chart permits.

## Getting started

1. Until the IHO sends the credentials, use **test mode** (the default): the IHO test
   credentials, HW_ID 12345 for every licence, so test builds of TweedPlot can open the S-64
   test data.
2. When the M_ID and M_KEY arrive: Settings › Credentials › *Tweedsmarine's own credentials*,
   enter them, **Save credentials**. This makes the signing key and writes `oem.txt` (M_ID
   only) and `signing.pub`.
3. Rebuild TweedPlot and the Licence Manager on this PC so both carry the new public key.
4. Make a passphrase backup.
5. Re-run the S-64 tests (`cargo run --release -p chart --example s64 -- <TDS folder>`): they
   always use the IHO test credentials.

If the old `tweedplot-oem` command-line tool left `signing.key`, an `M_KEY=` line in
`oem.txt` or `register.csv`, a banner offers to move them into the vault; the plain files are
then removed.

## Security notes

- TweedPlot itself holds no M_KEY. The HW_ID reaches an installation inside its activation
  code, encrypted to that installation's own device key (X25519, kept with DPAPI in machine
  scope), so a copied activation code is useless on another computer.
- The decrypted chart data (SENC) is sealed on disk with a key from the computer fingerprint,
  the HW_ID and the device key, and cell keys only exist in memory while cells are converted.
