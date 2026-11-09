# Weather routing

Weather routing finds the fastest way to a destination given a forecast and your boat's performance. Done well it shortens passages, avoids the worst weather and shows which decisions matter; done carelessly it routes you confidently through the errors in the forecast. This tutorial explains enough of the weather, the models and the method to use routing well.

## What weather routing is, and isn't

A weather router takes three things and returns a track and a timetable:

1. **A forecast**: wind (and perhaps waves and current) at every place and time along the way.
2. **The boat's performance**: how fast it goes at each angle to the wind and wind strength (its polar).
3. **The geography**: where the land, shallows, dangers and barriers are.

It answers "if this forecast were exactly right and we sailed exactly to our polar, what would be the fastest way?" That answer is useful, but it is an answer to a hypothetical question. The forecast won't be exactly right, the crew won't sail exactly to the polar, and the router knows nothing the GRIB doesn't contain: local sea breezes, gusts off the hills, a squall line, a tidal race.

So the skill in weather routing lies less in pressing Compute and more in asking the right follow-up questions: *How sensitive is this route to the forecast? What do other models say? Which decision does the route depend on, and when must I make it?* The rest of this tutorial gives you the background to ask them.

## Weather for routing

**Pressure systems**

Wind blows roughly along the isobars, turning slightly in towards low pressure; the closer the isobars, the stronger the wind. In the Southern Hemisphere wind circulates clockwise round a low and anticlockwise round a high (the reverse in the Northern Hemisphere). Turn on Isobars in the Weather drawer to see the pattern behind the wind. Most routing decisions are about where a high or low will be when you get there: which side of a ridge has the wind, how far round a low you must go for a favourable shift.

**Fronts and troughs**

A front or trough brings a wind shift (in New Zealand typically from north-west to south-west as a cold front passes), often a band of rain and squalls, and sometimes a sharp change in strength. Models usually agree on a front's existence and disagree on its timing by hours. Hours matter: the route that crosses the front in time to take the new breeze can gain hugely, the one that arrives after it can lose as much. Watch fronts on the meteogram (a sudden direction change and a pressure minimum) and compare the timing between models.

**Sea breezes and local winds**

On warm days the land heats faster than the sea and draws in a sea breeze from mid-morning to evening, often strong in summer, sometimes reversing at night as a land breeze. Hills and valleys funnel and accelerate the wind (gap winds, valley winds), and the wind drops and swirls in the lee of high land. Global models are too coarse to show most of this; the GRIB says 8 kn where a headland gives 20. Use local knowledge and the official forecast for the coast, and treat GRIB winds close to high land with suspicion.

**Gusts**

The GRIB wind is an average. Gusts in a fresh, unstable flow (behind a cold front, under showers) can be 30–50 % stronger. Show **Gusts** to see the model's gust forecast, and plan sail areas and power-boat limits on gusts, not the mean.

**Waves**

Wave height depends on wind strength, how long it has blown (duration) and over how much sea (fetch). Swell arrives from distant storms and can be large with no local wind. A significant wave height (Hs) of 2 m means the highest third of the waves average 2 m, and occasional waves will be nearly twice that. Waves against a current or over a shoal steepen sharply. Show **Wave height** and **Wave direction** from a wave GRIB to see the sea you'll meet.

## Forecast models and GRIB files

A weather model divides the atmosphere into a 3-D grid, starts from the best estimate of the weather now (the *analysis*, built from satellites, balloons, ships, buoys and aircraft) and steps the physics forward in time. Each start is a *run*, named by its time in UTC: "GFS 12Z" started at 12:00 UTC. A GRIB file is a slice of that output: chosen fields, over chosen area and times, on a regular grid.

**The main free global models**

| Model | Centre | Grid you can get | Runs a day | Range | In TweedPlot |
| --- | --- | --- | --- | --- | --- |
| GFS | NOAA (USA) | 0.25° | 4 | 16 days | Download (NOMADS), Saildocs |
| IFS | ECMWF (Europe) | 0.1° (~11 km grid; model about 9 km) and 0.25° open data | 4 | 15 days (06Z and 18Z runs: 6 days) | Download; Saildocs at 0.4° |
| AIFS | ECMWF | 0.25° | 4 | 15 days | Download |
| ICON | DWD (Germany) | 0.125° through Saildocs | 4 | 7.5 days | Saildocs |
| UKMO global | Met Office (UK) | About 10 km | 4 | 7 days (00Z, 12Z) | Planned |
| AIGFS | NOAA | 0.25° | 4 |  | Planned |

AIFS and AIGFS are machine-learning models: trained on decades of past weather, they forecast the large-scale pattern well and fast, but their fine detail is smoother than a physics model's. Regional models (a few km grid) resolve sea breezes and terrain far better, but cover small areas and short ranges.

**Resolution**

ECMWF's new **0.1° IFS** forecast is free open data, distributed on a finer grid than its 0.25° product. The grid points are about 11 km apart north–south at 0.1°, versus about 28 km at 0.25°; the finer grid has 6.25 times as many points over the same area. It is published about two hours after the 0.25° data. Both resolutions come from the same high-resolution IFS forecast (about 9 km); the 0.1° data is not a separate higher-resolution model run. ECMWF supplies its 0.1° surface and pressure-level products as separate files; TweedPlot's download choice currently uses the surface forecast fields and waves.

A 0.25° grid has points about 28 km apart north–south (less east–west at high latitude); features smaller than a few grid lengths simply aren't there. The 0.1° grid provides more detail, but neither global grid resolves most sea breezes, headlands or narrow channels. Near coasts, islands and mountains, the wind is still an average over grid cells. Treat GRIB wind within a few grid lengths of high land as a rough guide only; finer spacing does not guarantee a more accurate local forecast.

The 0.1° IFS forecast is available at 3-hour steps through 144 h, then 6-hour steps through 360 h for the 00Z and 12Z runs; 06Z and 18Z runs end at 144 h. Like the other ECMWF open data, its whole-globe fields are range-fetched and cut to the selected area. TweedPlot caches requested 0.1° global fields (up to 5 GiB, oldest first), so another area request for the same run and fields can reuse them.

The 0.1° surface products also include eastward and northward sea-water velocity (`sve`/`svn`). In TweedPlot, tick **Surface currents** when downloading ECMWF IFS 0.1°; the currents can be displayed and included in routing when **Use GRIB currents** is enabled. ECMWF's open-data documentation does not specify whether these fields include astronomical tidal-current components, so do not treat them as tide predictions.

**Lead time**

Forecasts get worse with time ahead. As a rule of thumb: the first two or three days are usually good for the big pattern (timing errors of hours, not days); days four to seven show the likely pattern but details shift run to run; beyond a week, use them only for broad trends. Each new run starts from fresher observations, so the newest run is usually the best, but don't chase every run: look for the trend across several.

**Ensembles**

An ensemble runs the same model many times from slightly different starts. Where the members agree the forecast is confident; where they spread, it isn't. TweedPlot shows an ensemble's spread on the meteogram and in the agreement displays, and treats each member as a separate forecast when every loaded file is an ensemble.

## How sure is the forecast?

No single GRIB tells you how much to trust it. Comparing several does. TweedPlot gives four ways to compare:

| Tool | Question it answers |
| --- | --- |
| **Values at the cursor** (table in the Weather drawer) | What does each loaded GRIB say here and now? |
| **Meteogram** (right-click) | How does the wind at this point change over time in each GRIB, and do they agree on the timing of shifts and fronts? |
| **Agreement displays** (Show > Agreement) | Where on the chart do the GRIBs disagree, and by how much? Is the forecast trending windier or lighter run to run? |
| **Verify runs** | Over the past days, which model has forecast this area best? |

**Rules of thumb**

- **Agreement means confidence, not correctness.** Models share data and physics; they can all be wrong together, but disagreement is a reliable warning.
- **Disagreement in timing is common; disagreement in pattern is serious.** Two models with the same front six hours apart: plan for both timings. Two models with a low in different places: the forecast for that area is genuinely uncertain.
- **Watch the trend between runs.** If successive runs keep moving a front later, it will probably be later still.
- **Trust the model that has been right lately** for your area and season, but only when the track record covers enough cases (Verify runs builds this up as you download).
- **Near land, trust local knowledge and the official forecast over any GRIB.**

## Polars

A polar is a table (or the curve drawn from it) of boat speed against true wind angle, one line for each true wind speed. It is the router's model of your boat: every decision the routing makes comes from it.

| Source | Notes |
| --- | --- |
| Designer or class association | Theoretical (from a velocity prediction program, VPP) for a fully crewed boat in flat water with perfect sails. Optimistic for real sailing |
| Rating certificates (ORC) | VPP polars from measured boat data; consistent between boats, still optimistic offshore |
| Your own logged data | The truest, once you have enough hours across the range of angles and wind. Track recording builds the data; building polars from it is planned |
| Another program's polar file | qtVlm CSV and .pol formats are read directly: Settings › Vessel › Performance › Import polar… (the file format, and where to find polars, are in the Reference manual under Your boat > Polars) |

**Efficiency**

The efficiency setting (Settings › Vessel › Performance) scales the whole polar. Realistic values: 95–100 % for a race crew in flat water, 85–90 % for a race offshore or a good cruising crew, 70–80 % for short-handed or cruising trim with a heavy boat. If your logged speeds keep falling short of the routing's, lower it.

**Tacks and gybes**

Each manoeuvre costs time while the boat slows and accelerates. The tack and gybe costs in Settings › Vessel › Performance (seconds lost, typically 30–90 s) stop the router zig-zagging for tiny gains.

**What a polar doesn't know**

- **Waves**: a polar is for flat water. Upwind in a big sea the boat may be much slower than the table.
- **Sail changes**: the polar assumes the best sail for every condition, instantly.
- **Fatigue and night**: crews sail slower at 3 am than in a polar.

For a power boat, see *Making a power polar* in the Route planning tutorial: a simple table of speed against wind angle and strength.

## How the router works

TweedPlot uses the isochrone method, the standard for sailing routers. An *isochrone* ("equal time") is the line of everywhere the boat could be after a given time.

![isochrone routing · steps, fan, land, route](fig/isochrones.svg)

1. From the start, the router tries a fan of headings (every 4° by default). For each it looks up the wind in the GRIB at that place and time, reads the boat speed from the polar, adds current if asked, and moves the boat for one time step. A change of tack or gybe costs its manoeuvre time.
2. Steps that cross land, water shallower than the safety depth, dangers, farms or barriers are dropped; so are places outside your wind, wave or angle limits.
3. Of all the new points, it keeps the ones that reach furthest in each direction (near land, the best point in each small grid cell, so the route can find its way into bays and round headlands). These form the next isochrone.
4. It repeats from every point of the new isochrone until one reaches the mark (its circle, or its rounding line), then traces the chain of steps back to the start: that chain is the route.
5. With several marks, each leg starts where and when the last one arrived.

**Settings that matter**

- **Time step**: shorter steps follow the coast and wind changes more closely but take longer. Automatic gives about 80 steps per leg.
- **Heading step and search angle**: fewer headings is faster but can miss a narrow gap or a small shift.
- **Arrival radius**: the router must reach the mark's circle; a tiny circle offshore forces the route through one point.

Isochrones are drawn on the chart for the active routing (untick Isochrones to hide them). Closely spaced isochrones mean slow progress (light wind or upwind); widely spaced, fast.

## Reading a routing

The line on the chart is the least interesting part of a routing. Read the rest:

| Look at | What it tells you |
| --- | --- |
| ETA and elapsed time | Compare between runs and models, not as a promise |
| Up / reach / down % and the wind range | What kind of passage it is: a beat in 25 kn is a different trip from a reach in 15 kn, even with the same ETA |
| Tacks and gybes | Many manoeuvres can mean the route is chasing small shifts the forecast can't really resolve |
| Isochrone spacing | Where progress is slow (bunched) or fast (spread); bunching near the mark often means a light patch or a beat at the end |
| Points (hover) | Heading, TWA, TWS and boat speed at each step: is the wind angle comfortable, is the speed realistic? |
| The triangle at the slider time | Scrub the time slider: where will you be when the front arrives, when the wind is strongest? |

**Sanity checks**

- **Does it make meteorological sense?** A good routing usually has an explanation: "go west early to be first into the new south-westerly", "stay inshore for the sea breeze". If you can't explain it, it may be routing the noise of one model run.
- **Does it go near land where the GRIB is unreliable?** A route that gains by hugging a coast in a model's wind is gaining in wind that may not be there.
- **Is it right at the edge of the GRIB?** "The weather runs out first" or a route that bends along the GRIB's edge means the area or time span was too small.
- **Would you sail it?** Check the angles and wind at night, round headlands and through the worst of the weather.

## Robust decisions

A routing from one GRIB is one possible future. The point of routing several ways is to find the decisions that are good in most of them.

**Several GRIBs** (Routing > **With each GRIB**)

Load GFS, ECMWF and others, or the last few runs of one. If every routing goes the same way, the decision is easy. If they split (some go left of an island, some right), you have found the decision that matters, and the next question is when you have to make it. Often the routes stay together for the first hours: sail that part and decide later with a newer forecast.

**Start times**

When you can choose when to leave (cruising, delivery, fishing), set a fixed start time, compute, change it by a few hours, compute again, and compare in the table. A later start into a better breeze can arrive sooner, more comfortably, or both.

**The gate sweep**

When routings split between options, put a row of gates across the split (Routing > Gates): along a meridian or a parallel, through the options. TweedPlot routes through each gate with each GRIB and ranks them by the average time lost against the best gate under each forecast. A gate that is never more than an hour off the best is a safer choice than one that is best under one model and four hours off under another. This is the same idea as an ocean racer's "gates" or waypoint tests, automated.

**Re-route as the forecast updates**

Forecasts improve as the time gets closer. Re-route from the boat's position with each new run (Course > From boat), and watch for the decision points creeping closer. The aim is not to follow the newest routing blindly but to keep the boat where most good futures are still open.

## Racing, cruising and power boats

The router always minimises time. Different boats want different things from it, and the settings are how you tell it.

|  | Racing yacht | Cruising yacht | Power boat |
| --- | --- | --- | --- |
| Goal | Fastest elapsed time | A safe, comfortable passage, arriving in daylight | Shortest safe track inside a weather window, with fuel to spare |
| Polar efficiency | 90–100 % | 75–90 % | Power polar at 100 % |
| Tack / gybe time | Crew's real times | Longer: tacking short-handed takes time | 0 |
| Wind limits | Usually none (max for safety offshore) | Max 20–25 kn; min to avoid motoring in calms (or route with the engine as a power polar) | Max for comfort and safety, often 15–20 kn for small boats |
| Wave limit | For safety offshore | 2–3 m for comfort | 1–1.5 m for small boats |
| TWA limits | None | Min 45–55° (don't pinch), max 165° (no dead runs) | None |
| Rounding | Marks per the instructions | Generous clearances off headlands | As close as possible at fishing spots |
| Robustness | Gate sweep and every GRIB | Start-time comparison | Meteogram windows, return leg as well as outbound |

**Cruisers: comfort over seconds**

A racing routing may tack downwind in 30 kn to gain ten minutes; a cruiser wants the reach in 18 kn that arrives an hour later. Set the limits, lower the efficiency, give headlands room, and compare departure times. The fastest routing you're happy to sail is the right one.

**Power boats: windows, not routes**

For most power-boat trips the route is short and obvious; the decision is *when*. Use meteograms at the exposed points, the wave GRIB, and the agreement displays to find a window that covers the return trip too. Then route with a power polar and limits to check the track and ETA.

**Motor-sailing**

To see when the engine pays, make a copy of your polar with a minimum speed at every angle (your motoring speed in light winds) and compare routings with the sailing polar.

## A routing workflow

**Days before**

- [ ] Route drawer: the route with every mark's rounding and radius; barriers for exclusion zones and reserves.
- [ ] Chart › Safety: safety depth and contour; Wx routing › Limits and barriers: route round dangers and farms on.
- [ ] Settings › Vessel › Performance: polar, efficiency, tack and gybe costs.
- [ ] Weather: download GFS and ECMWF (and waves) for the whole area and longer than the trip; then each new run.
- [ ] Look at the weather before the routing: isobars, fronts, the meteogram at key points, the agreement displays.

**Routing**

- [ ] Compute with the GRIB shown; check it makes sense (Reading a routing).
- [ ] With each GRIB; compare in the table. Same route: good. Split: find the decision.
- [ ] Gate sweep across any split; start-time comparison if you can choose when to leave.
- [ ] Set limits for your boat and crew and route again: does the plan still work?
- [ ] Save the routings you want to keep; export the chosen one as GPX for the plotter.

**On the day**

- [ ] Download the latest runs; re-route. Go or no-go against your written limits.
- [ ] Brief the crew on the plan, the decision points and the bolt holes.

**Under way**

- [ ] Live mode: record the track; watch the routing's targets (TWA, boat speed, heading) and % of polar.
- [ ] With each new forecast: re-route from the boat; compare with the previous plan; move towards the options that stay open.
- [ ] Afterwards: compare the track with the routing and the forecast (Boat at weather time); adjust efficiency and polar for next time.
