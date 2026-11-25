Test build. Not for navigation.

**Installers:** `TweedPlot_1.0.0-beta.6_x64-setup.exe` for most Windows PCs; `TweedPlot_1.0.0-beta.6_arm64-setup.exe` for Windows on Arm (Snapdragon, Surface Pro X …). The installer is not code-signed yet: Windows SmartScreen will warn ("Windows protected your PC") — choose More info › Run anyway. Installed betas offer this update themselves. From beta.6 on, TweedPlot asks first whether to check for updates at all (see below).

## What's new since beta.5

**Nothing goes online unasked**
- TweedPlot no longer assumes an internet connection. On first start it asks once whether to check GitHub for updates; until you answer, it doesn't look. Your answer can be changed in Settings › Diagnostics › Software updates.
- The New Zealand Nautical Almanac and the Annual Notices to Mariners open from this computer. When one isn't there, TweedPlot says so and offers to download it (about 9 MB for the Almanac); nothing is fetched until you agree.

**Instruments and connections**
- More from NMEA 0183:
  - rate of turn (ROT);
  - wind from older instruments that send VWR and VWT instead of MWV;
  - heel and pitch, battery voltage and engine revs (XDR, RPM);
  - rudder angle (RSA);
  - set and drift worked out by the instruments (VDR);
  - log and trip (VLW);
  - true heading from satellite compasses (THS);
  - speed through the water from dual-axis logs (VBW);
  - position quality: satellites, HDOP, 2D/3D fix and estimated error (GGA, GSA, GST).
- The same from Signal K: rate of turn, attitude, rudder angle, current, GNSS satellites and HDOP, batteries and engines.
- Each is a new group in Settings › Connections, so you can choose which source to use and which takes over if it goes quiet. Each also appears in the Live data view and can be added to the data panel (Boat and Position quality groups; Set and Drift use the instruments' own figures when they send them).
- **GPS datum:** a warning when the GPS reports a datum other than WGS 84 (DTM), since the boat would then be drawn away from where it is on the chart.

**Autopilot**
- Route › **Send to the autopilot** sends the leg being navigated once a second as RMB, APB, XTE, BOD and BWC, for the autopilot's Track or Nav mode. Choose the connection in Settings › Connections › Autopilot output (any serial or TCP input; talker GP, EC or II).
- It is off each time TweedPlot starts. Nothing is sent while nothing is navigated, during a replay, or without a current position, so an autopilot in Track mode raises its own lost-data alarm rather than steering for an old mark.
- Not yet: sending on a UDP input, or through Signal K.

**Alarms from other equipment**
- **DSC** distress, urgency and safety calls from the VHF (DSC, DSE):
  - a distress alert, relay or acknowledgement raises an alarm, which sounds again if it is silenced and left;
  - an urgency call raises a warning, and a safety call a caution;
  - the caller is drawn on the chart, using the more precise position when the radio sends one, and named from AIS when known. Click the alarm to show it.
- Alarms from other equipment appear in TweedPlot's alert list: NMEA ALR and ALF (a sounder's shallow alarm, an AIS's own alarms), and Signal K notifications (alert, warn, alarm, emergency). Each clears when its equipment says so. Acknowledge it at the equipment too.

**Radar**
- Radar (ARPA) targets on the chart, from TTM and TLL: a 6-minute vector and the radar's own CPA and time to CPA. A red ring marks a target passing within half a mile in the next 30 minutes. The radar does the alarming.

**Chart**
- A reserve or restricted area charted several times (overlapping charts, or repeated in one) no longer stacks its pink "i" squares: one shows where several would stand together. The chart features popup lists such a repeat once, with how many times it is charted.

**GRIB values at the cursor**
- Opened from the chart's right-click menu as "GRIB values at cursor" (it was "Pin the readout here"). It follows the cursor until you pin it with "Keep GRIB values for this place".
- Narrower, one fixed width (it no longer jumps as the text changes), the source letters boxed so they don't read as units, and currents at depth read "set" rather than "toward".

**Fixes**
- The Live data view slowed TweedPlot to a crawl and could end in a crash.
- Close in the Live data view closed TweedPlot. It now goes back to the view you were in.
- The values at the cursor lagged behind the mouse, held back like the time slider. They now update as the cursor moves.
- The Nautical Almanac wouldn't open: LINZ renamed the file from the 2025/26 edition.

**Not yet tested on real equipment:** the autopilot output, DSC, equipment alarms and radar targets were tested with simulated data. Please report how they behave with your instruments, radio, radar and autopilot.

Please report problems and ideas as GitHub issues, with a Capture screenshot where it helps.
