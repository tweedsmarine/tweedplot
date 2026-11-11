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
| `WIMWV` (R and T) | Apparent and true wind: north-westerly 12 kn in Tasman Bay, 25 to 30 kn in Cook Strait, gusty |
| `YXMTW`, `WIMDA` | Water temperature; barometer (falling slowly), air temperature, humidity. Every 5 s |
| `!AIVDM` | Six invented AIS targets: a ferry, a cargo ship, a trawler fishing in Tasman Bay, a yacht, a ship at anchor off Nelson and a fast launch. Messages 1, 5, 18 and 24 |

The depths, wind and tidal streams are made up to look right for each stretch (they are in the route file); they are not the chart's or a forecast's. The stream runs with the boat through French Pass, so the speed over the ground reaches about 11 kn there. **The AIS names, call signs and MMSIs (512 000 901 to 906) are invented.**

## Options

- `make [route.json | route.gpx] [--out file.nmea] [--speed 7.5] [--start 2026-11-14T18:00:00Z] [--no-ais]`. A route file is a list of waypoints, each with `name`, `lat`, `lon` and, optionally, `depth` (m), `twd` and `tws` (true wind, °T and kn) and `set` and `drift` (the stream, °T and kn): see `nelson-wellington.json`. A GPX route exported from TweedPlot works too. `make` prints when the boat reaches each waypoint.
- `play file.nmea [--udp host:port | --tcp port] [--from hh:mm] [--rate 1] [--loop] [--keep-time]`. `--from 03:50` starts 3 h 50 min in (a few minutes before French Pass). `--rate 10` plays ten seconds of the passage each second. The times in the file are replaced by the computer's clock unless `--keep-time` is given, so tides, weather and history plots are for now.

`nelson-wellington.gpx` is the same route for the Library (Library menu › Import…), to have it as the route being sailed.

The waypoints were checked against TweedPlot's bundled land outline, which is coarse: **check the two French Pass waypoints on the chart** before relying on them for a picture, and move them in the JSON if the track cuts a corner.
