# nmea-sim: a simulated passage as NMEA 0183

For demonstrations, manual screenshots and testing without a boat. Needs only Node.

```
node tools/nmea-sim/nmea-sim.mjs make
node tools/nmea-sim/nmea-sim.mjs play nelson-wellington.nmea
```

`make` writes `nelson-wellington.nmea` in the current folder: a 25 m vessel at about 7.5 kn from off Nelson through French Pass and across Cook Strait to the Wellington entrance, 99.6 NM in 11 h 40 min, one second of sentences after another (about 17 MB; not kept in git). `play` sends it to TweedPlot a second at a time.

In TweedPlot: **Settings › Connections › NMEA data sources › Add source**, connection type **UDP listener**, local bind address `127.0.0.1:10110`. (Or `play … --tcp 10110` and a **TCP client** source to `127.0.0.1:10110`.)

## What is in the file

| Sentence | What |
| --- | --- |
| `GPRMC`, `GPGGA`, `GPVTG` | Position (DGPS quality), course and speed over the ground, variation 22°E |
| `HEHDT` | True heading: the course steered, crabbing across the stream, with a little yaw |
| `VWVHW` | Speed through the water, 7.2 to 7.8 kn |
| `SDDBT`, `SDDPT` | Depth below the transducer, and the transducer 1.5 m below the waterline |
| `WIMWV` (R and T), `WIMWD` | Apparent and true wind, and the true wind direction: north-westerly 12 kn in Tasman Bay, 25 to 30 kn in Cook Strait, gusty |
| `YXMTW`, `WIMDA` | Water temperature; barometer (falling slowly), air temperature, humidity. Every 5 s |
| `!AIVDM` | Six invented AIS targets: a ferry, a cargo ship, a trawler fishing in Tasman Bay, a yacht, a ship at anchor off Nelson and a fast launch. Messages 1, 5, 18 and 24 |

The depths, wind and tidal streams are made up to look right for each stretch (they are in the route file); they are not the chart's or a forecast's. The stream runs with the boat through French Pass, so the speed over the ground reaches about 11 kn there. **The AIS names, call signs and MMSIs (512 000 901 to 906) are invented.**

## Distress test

```
node tools/nmea-sim/nmea-sim.mjs distress
node tools/nmea-sim/nmea-sim.mjs play distress-test.nmea
```

`distress` writes `distress-test.nmea`: 13 minutes of own boat motoring at 5 kn, 045°T, from about 2.5 NM north of Nelson, with one distress event a minute, each placed round the boat a few cables to a few miles off so that they don't sit on top of each other:

| Time in | Event |
| --- | --- |
| 00:30 | AIS-SART, active (970 099 101): status 14 and "SART ACTIVE", drifting |
| 01:30 | AIS MOB device, active (972 099 102), a third of a mile astern |
| 02:30 | EPIRB-AIS, active (974 099 103) |
| 03:30 | AIS-SART in test (970 099 104): status 15 and "SART TEST", for 90 s |
| 04:30 | DSC distress alert, flooding (512 009 971), with a DSE (enhanced position) |
| 05:30 | DSC distress relay from a coast station (005 120 111), sinking (512 009 972) |
| 06:30 | DSC distress acknowledgement of the 04:30 alert |
| 07:30 | DSC all-ships urgency call (512 009 973) |
| 08:30 | DSC all-ships safety call (coast station) |
| 09:30 | DSC distress alert, man overboard, no position (512 009 974) |
| 10:30 | AIS MOB device in test (972 099 105), for 70 s |
| 11:30 | The 04:30 alert again (a DSC alert repeats until acknowledged) |

The AIS transmitters send a position every 20 s (a real one sends a burst once a minute) and their "… ACTIVE" or "… TEST" text (message 14) every four minutes. `play` puts the clock's time into the DSC calls as well as into RMC and GGA. **All the MMSIs are invented.** Use `play … --from 00:04` to start just before the DSC calls.

## Equipment test

```
node tools/nmea-sim/nmea-sim.mjs equipment
node tools/nmea-sim/nmea-sim.mjs play equipment-test.nmea
node tools/nmea-sim/nmea-sim.mjs listen --port 10111
```

`equipment` writes `equipment-test.nmea`: 10 minutes of own boat at 5 kn, 045°T, with two radar (ARPA) targets as TTM and TLL every 3 s (R1 on a collision course, its CPA under 0.05 NM and its TCPA inside 12 min from about 03:00; R2 passing clear) and alarms from other equipment, repeated every 30 s while they last:

| Time in | Event |
| --- | --- |
| 00:20 | Radar targets R1 and R2 acquired |
| 01:00 | Sounder ALR 012 SHALLOW WATER, until 05:00 (TweedPlot can acknowledge it: ACK) |
| 02:00 | Sounder ALF warning 3015, category B, until 07:00 (TweedPlot can acknowledge it: ACN) |
| 03:00 | Radar ALF alarm 3100, category A, until 06:00 (only at the radar) |

`listen` prints what arrives on a UDP port. In TweedPlot, give the UDP connection a **Send to** of `127.0.0.1:10111` to see the autopilot sentences and the acknowledgements TweedPlot sends.

## Options

- `make [route.json | route.gpx] [--out file.nmea] [--speed 7.5] [--start 2026-11-14T18:00:00Z] [--no-ais]`. A route file is a list of waypoints, each with `name`, `lat`, `lon` and, optionally, `depth` (m), `twd` and `tws` (true wind, °T and kn) and `set` and `drift` (the stream, °T and kn): see `nelson-wellington.json`. A GPX route exported from TweedPlot works too. `make` prints when the boat reaches each waypoint.
- `play file.nmea [--udp host:port | --tcp port] [--from hh:mm] [--rate 1] [--loop] [--keep-time]`. `--from 03:50` starts 3 h 50 min in (a few minutes before French Pass). `--rate 10` plays ten seconds of the passage each second. The times in the file are replaced by the computer's clock unless `--keep-time` is given, so tides, weather and history plots are for now.

`nelson-wellington.gpx` is the same route for the Library (Library menu › Import…), to have it as the route being sailed.

The waypoints were checked against TweedPlot's bundled land outline, which is coarse: **check the two French Pass waypoints on the chart** before relying on them for a picture, and move them in the JSON if the track cuts a corner.
