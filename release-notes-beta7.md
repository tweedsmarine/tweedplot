Test build. Not for navigation.

## Installers

Download the one for your computer from the Assets list below.

**Windows** (Windows 10 or 11)
- `TweedPlot_1.0.0-beta.7_x64-setup.exe` — most PCs (Intel and AMD).
- `TweedPlot_1.0.0-beta.7_arm64-setup.exe` — Windows on Arm (Snapdragon laptops, Surface Pro X and the like).
- Not code-signed yet: Windows SmartScreen warns "Windows protected your PC". Choose More info › Run anyway.

**Mac** (new; macOS 11 Big Sur or later)
- `TweedPlot_1.0.0-beta.7_universal.dmg` — one app for both Apple silicon (M1 and later) and Intel Macs.
- Open the .dmg and drag TweedPlot into Applications.
- Not notarised by Apple yet, so the first start is blocked ("Apple could not verify …"). Choose Done, then System Settings › Privacy & Security, scroll down and choose Open Anyway. After that it opens normally.

**Linux** (new; 64-bit Intel/AMD PCs, distributions from about 2022 on)
- `TweedPlot_1.0.0-beta.7_amd64.deb` — Debian, Ubuntu, Linux Mint, Pop!_OS: `sudo apt install ./TweedPlot_1.0.0-beta.7_amd64.deb`
- `TweedPlot-1.0.0-beta.7-1.x86_64.rpm` — Fedora, openSUSE, RHEL and the like: `sudo dnf install ./TweedPlot-1.0.0-beta.7-1.x86_64.rpm` (openSUSE: `sudo zypper install …`)
- `TweedPlot_1.0.0-beta.7_amd64.AppImage` — any other distribution, no installing: `chmod +x TweedPlot_1.0.0-beta.7_amd64.AppImage`, then run it.
- The .deb and .rpm bring in what TweedPlot needs (WebKitGTK 4.1); the AppImage needs it on the system already, as most current desktops have.
- For an NMEA input on a serial or USB-serial port (`/dev/ttyUSB0` and so on), add yourself to the `dialout` group once: `sudo usermod -aG dialout $USER`, then log out and back in.

**Updates:** installed copies offer this update themselves if you allowed update checks, on Windows and Mac and for the Linux AppImage. A .deb or .rpm is updated by installing the new package.

**New platforms:** the Mac and Linux versions are new and have had little use: please report anything that doesn't work. The S-63 Charts plugin (encrypted official charts) is Windows only for now.

## What's new since beta.6

**Safety**
- **Never navigating at start.** A route that was being navigated when TweedPlot closed is no longer navigated again by itself. At start TweedPlot asks "Carry on navigating …?", with the next mark chosen from the leg nearest the boat (any mark can be picked instead), or Don't navigate. Nothing goes to the autopilot until you choose.
- **AIS-SART, MOB devices and EPIRBs are always on the chart**, even with AIS hidden, filtered to your own fleet, or beyond the internet-target range. With AIS hidden, only they are drawn.
- **No internet, no hang.** The start-up screen goes after 20 seconds at most, saying what is still loading. Online maps notice when there's no connection, use the tiles already saved, and say so.

**Polars**
- Settings › Vessel › Performance › **View and edit…** shows the polar as a plot (a curve for each wind speed, the best upwind and downwind VMG marked) beside a table you edit like a spreadsheet: type, move with Enter, Tab and the arrows, select blocks, copy and paste with a spreadsheet (a whole table onto the top-left corner), scale by a percentage, add or delete angles and wind speeds, undo. Changed cells are shaded and the saved curve is shown dashed. Saving writes your own copy into your polar folder; the polars that come with TweedPlot are never changed.

**Weather**
- **Rain from ECMWF:** the IFS 0.25° and 0.1° and AIFS now include rain (total precipitation, shown in mm/h like GFS's). Files already downloaded show it too.
- The ECMWF 0.1° **surface currents** are read as currents, so they show on the chart and along the route.
- **GRIBs laid together:** a small detailed area downloaded with a big coarse one is drawn with the finer one on top, and areas of the same model side by side are joined (the weather drawer's "Lay finer and coarser GRIBs together", on to begin with). **Show** on a GRIB now shows it even if another file was chosen for that layer earlier.

**Chart**
- No faint lines where charts meet.
- Marine farms appear at their chart's own scale (from about 1:70,000 at the latest) rather than at every zoom, so the Sounds' farms no longer bury the small-scale chart.
- A repeated information "i" symbol is shown once where several stand together.
- A cluttered chart could stay in the tile cache for the rest of the session after a setting changed: fixed.

**Marks and targets**
- **Number-key marks:** keys 1 to 0 each drop a mark at the boat in one press, with the symbol, colour and name you set in Settings › Number-key marks. Key 1 is a filled triangle to begin with; the others are blank until you set them.
- **Targets** in the data panel have column headings and fixed columns, so the figures don't jump about. TCPA is always in minutes, shown as ">99" beyond 99 minutes, as on a radar.
- **DSC calls** open an information box when you click their mark or alarm: the call, nature of distress, caller (named from AIS when known), relaying or acknowledging station, position (enhanced or to the minute), range and bearing, time, and every call heard from that MMSI.

**Connections**
- **UDP output:** a UDP connection can now send, to a device's address or the network's broadcast address (Settings › Connections › Send to).
- **Autopilot:** the arrival alarm (AAM) is sent with RMB, APB, XTE, BOD and BWC, and the leg's two waypoint names are kept apart when 8 characters would make them the same.
- **Alarms from other instruments:** acknowledging a sounder's, radar's or AIS's alarm (ALR, ALF) in TweedPlot now acknowledges it on the instrument too (ACK, ACN), except alarms the standard says must be acknowledged at the instrument itself. You can turn this off in Alarms.
- **Radar (ARPA) targets** use your CPA and TCPA limits from Alarms › Targets, and can raise a collision alarm (Alarms for radar (ARPA) targets too; off to begin with, as radars sound their own).

**Night**
- Night cursors in the standard shapes (arrow, pointing hand, open and closed hand) in the night colour, dimmed with Brightness, and no more white Windows cursor anywhere at night.

**Fixes**
- The Nautical Almanac and Annual Notices to Mariners opened nothing on some PCs; they now open as a double-click does.
- The Highway in the data panel had a gap above XTE, TTG and ETA.

**Not yet tested on real equipment:** the autopilot output, UDP output, alarm acknowledgements, DSC and radar targets were tested with simulated data (tools\nmea-sim: `distress` and `equipment` test files). Please report how they behave with your instruments, radio, radar and autopilot.

Please report problems and ideas as GitHub issues, with a Capture screenshot where it helps.
