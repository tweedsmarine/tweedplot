# Fishing tools

How to use TweedPlot's sea and weather tools to decide where to fish. The **Reference** tab describes each control; this tab is about what the pictures mean and how to put them together. Everything here also helps anyone cruising who wants to know where the warm water, the current and the rough patches are.

> **What these tools are, and are not.** They show the *sea around you* at the scale of kilometres, from satellites and an ocean model. They do not find fish. Fish, bait and birds are found by combining this picture with local knowledge, the sounder and your own eyes. Treat everything as a guide to *where to look first*.

## The short version

1. In **Weather › Download GRIBs** fetch, for your area: **MUR sea temperature**, **Chlorophyll-a**, and **HYCOM** with **Surface currents**, **Sea surface height** and **Temperature and salinity at a depth** ticked. Add a wind GRIB (ECMWF or GFS) for the day's weather.
2. Press the **Weather** button in the toolbar. The data panel opens its **Weather** controls.
3. In the panel choose the **Fishing** preset (sea temperature shaded, current arrows, contours and temperature breaks on). Drag the filter's three pointers to bracket the water you want, with the target in the middle.
4. Click through the other layers (chlorophyll, sea surface height, temperature at depth) and look for places where several of them agree.
5. Mark the candidate spots, go, and let the **Sounder history** and the **Sea at cursor** readings tell you what the water is actually doing.

## Getting the data

| You want to see | Download this | How old it is | Notes |
| --- | --- | --- | --- |
| Surface temperature, fine detail | **MUR sea temperature** | A day or two old, 1 km | The best picture of fronts and eddies. Fill gaps in cloud by itself |
| Surface temperature, today | **Himawari-9 sea temperature** | A few hours old, 2 km, hourly | Only where the sky is clear. Fetch 12–24 hours to fill cloud gaps. Reads the skin of the sea, so afternoon sun in calm weather warms it a degree or more |
| Plankton, green water | **Chlorophyll-a** | About three days old, 9 km | Daily, gap-filled. Slow to change |
| Temperature and salinity below the surface | **HYCOM** with **Temperature and salinity at a depth**, and the **Depth** you want (10–500 m) | Model: now to eight days ahead, 9 km | One depth at a time: download again for another depth |
| Currents | **HYCOM** with **Surface currents**, or **ECMWF IFS 0.1°** with **Surface currents** | Model, with a forecast | Non-tidal. Add the tidal stream from the chart's tidal diamonds (Reference › Tides) |
| Eddies and big currents | **HYCOM** with **Sea surface height** | Model, with a forecast | The tide is not in it |
| Wind and waves | **ECMWF IFS 0.1°** or **GFS**, with waves | Forecast | For safety, comfort and drift |

HYCOM's server is slow, especially for many time steps: a day's data can take a minute or two. Ask for a short forecast, every 3 or 6 hours, to keep it quick. Nothing is downloaded until you press Download, and the size estimate is shown first.

**Several sources of the same thing are fine.** Load MUR and Himawari together and the Weather panel's Show list offers both, with a drop-down to pick which file feeds each layer.

## The Weather panel

The toolbar's Weather button opens the data panel's Weather layers and filters section. **Hide weather on chart** and **Show weather on chart** switch all weather layers off and on without unloading the files. In the panel:

- The **time slider** moves everything together. Today's analyses are shown for a few days after their date, so your slider's "now" works for all of them.
- The **colour bar with its three pointers** is your *filter*. For whatever layer is shaded, drag the outer pointers to the lowest and highest values you want and nothing outside them is coloured. Drag the middle pointer to your **target**: the colours run from the low limit to the target over the cool half and from the target to the high limit over the warm half, so the water you are after is always the same colour in the middle and has its own bold contour. The track refits around the selected range when you release a pointer; push an outer pointer to an edge to make room to widen it. **Fit** restores the full grid range at chart time; it then follows the range as chart time changes. Manually set limits stay fixed. Click a pointer's number to type a value. Each layer keeps its own filter.
- **At the cursor** lists every value in the loaded files at the cursor: sea temperature, temperature at depth, salinity, chlorophyll, sea surface height, current speed and set, with the wind, waves and pressure. The rows are fixed, so it does not jump about as you move.
- **Show on the chart** picks what is shaded; **Overlays** adds wind barbs, wave direction, current arrows, isobars, temperature breaks and contours.
- **Presets** keep a set of choices: *Fishing* (sea temperature, current arrows, contours, temperature breaks), *Passage* and *Anchorage*. **Save…** keeps your own.

## Reading each layer

### Sea temperature and temperature breaks

Many fish and the bait they eat gather where water masses meet. A **temperature break** is a line where the temperature changes quickly over a short distance: warm blue water against cooler green water, a warm eddy against its surroundings, an upwelling against the water around it. Breaks that are sharp, long and persistent from day to day are worth more than short faint ones.

- Turn on **Contours**. Contour lines bunched together are a break; lines far apart are a gentle slope. TweedPlot chooses a spacing that suits the range you filter to.
- Turn on **Temperature breaks** in Overlays to shade the places where the temperature changes at least 0.2 °C in 10 km. This works on a sea temperature file, MUR or Himawari.
- Narrow the filter around the temperature you are looking for (a few degrees wide, with the target in the middle) so the colours spread out and the break stands out. A wide filter makes everything look similar.
- A break that *moves* is as interesting as one that sits still. Step the slider through the days: breaks pushed against a coast or against an eddy often hold fish longer.
- Compare MUR and Himawari. If they agree, trust the pattern. If Himawari is warmer in the afternoon on a calm day, that is the sun on the skin, not the water.

### Cold water at depth

Warm surface water usually lies on colder water, with a layer between where the temperature falls quickly: the **thermocline**. Where it is shallow, cold water is close to the surface; where it is deep, warm water goes a long way down.

- Download **Temperature and salinity at a depth** at the depth that matters to you (say 50 m for the thermocline's top, 100 m to see how deep the warm water goes) and choose **Sea temperature at depth** in Show.
- Compare the shallow and deeper pictures. Cold deep water directly under a cool patch on the surface is **upwelling**: the cold water, with its nutrients, is rising here.
- Patches where the deep water is warm under warm surface water often mark a **warm eddy** (see Sea surface height).
- The **Temp at depth** and **Salinity at depth** readings, under the cursor, show the depth the file was downloaded for.

The model sees broad features. It will not find a sharp thermocline a few metres thick, which is why the on-board water temperature reading and the sounder matter.

### Chlorophyll

Chlorophyll is the green of the plankton, the base of the food chain. It is shown on a log scale, so each step along the bar is a multiple (0.05 to 5 mg/m³ covers most of it).

- Higher values in blue water, away from rivers, often mean plankton blooms, upwelling or the edge of an eddy. Near river mouths and shallow estuaries high values are often sediment and runoff, so look at the salinity as well.
- The value is about three days old, and plankton change slowly, so it shows *where the productive water has been*. Fish and bait often sit at the edge of it, where it meets clearer water.
- The most interesting places are where **a colour break in chlorophyll lines up with a temperature break**. That is a water-mass boundary.

### Sea surface height and eddies

Sea surface height (SSH) is how high the sea surface stands above its long-term average, in metres, with the tide left out. It is blue where the sea is low and red where it is high.

- A **red, rounded patch** is a high in the sea surface: a **warm-core eddy**. In the Southern Hemisphere it turns anticlockwise. It pushes warm water deeper, and its edge is a good place to look.
- A **blue, rounded patch** is a low: a **cold-core eddy**, which pulls cold, nutrient-rich water up. Its rim and the water around it are often more productive.
- **Currents run along the contours of the surface height.** In the Southern Hemisphere the higher water is on the *left* of the flow looking downstream; in the Northern Hemisphere it is on the right. A steep change in SSH between blue and red is a strong current.
- Eddies drift slowly, so a pattern in SSH can be trusted for days ahead. Overlay SSH with sea temperature (switch between them in the Weather panel, or use the cursor readout) to see whether the eddy has a warm or cold core.
- Values are small. A change of 0.3 m across an eddy is strong. The model's resolution is about 9 km, so it cannot show small eddies.

### Currents

Turn on **Current direction** for blue arrows that point the way the water flows, longer where it flows faster, and shade **Current** for the speed.

- A current running across the temperature break holds bait to the edge; a current running along it moves the whole edge. The arrows and contours show which.
- The set and rate at the cursor appear in **At the cursor** as, for example, *0.8 kn toward 135°*.
- **Wind against current** makes steep, short seas. The panel warns when the wind is 15 knots or more and within 45° of the current's set against a current of 1 knot or more.
- The currents are the ocean's, not the tide's. Close inshore, add the tidal streams from the chart (Reference › Tides) and the tide times.
- Fish for drift: a current of 1 knot moves a drifting boat 24 NM in a day. The arrows tell you where to start the drift and where you will end up.

### Salinity

Fresher water is lighter and floats on saltier. Its edge is a **salinity front**.

- River plumes show as low salinity along the coast, often with high chlorophyll and a clear colour change. Fish hold on the edge.
- Open-ocean water in the region is salty and nearly uniform. A fall of 0.3 to 0.5 PSU is a significant change.
- At depth, differences in salinity show where different water masses lie on top of one another.

### Wind and waves

- Choose **Wind speed** and the **wind barbs** for the day's weather. **Wave height** and the **wave direction** arrows show the sea state, and the period shows how steep it will be. A short period with a big height is steep and uncomfortable.
- Use the slider to check the *whole day*, including the trip home, not just the morning.
- The **gusts** layer and the **Anchorage** preset show the exposed and sheltered places.

## A day on the water

**The evening before**

1. Download fresh data: MUR and chlorophyll are daily, HYCOM and wind and waves come with a forecast. The Weather panel's top line shows the run time; it turns red when the run is more than a day old.
2. Choose the **Fishing** preset. Slide through tomorrow's hours. Is the break moving? Will the current change?
3. Note two or three candidate areas where the sea temperature break, a chlorophyll edge or an eddy rim line up. Put each in a route as a mark (right-click the chart › **Add mark to route here**) and name it for the reason.
4. Check the wind and waves for the whole day.
5. Plan a **route** through your candidates, with the way home. The Route planning tab has a worked fishing-trip example.

**On the way**

1. Add the **Sea at cursor** panel (Instruments) so the temperature, salinity and current at the cursor are always in view.
2. Add the **Sounder history** panel to watch the bottom, ledges and any change as you cross a break.
3. When the boat reports its water temperature, it is logged along your track. In **Library › Tracks** set **Colour by** to **Sea temperature** to see where you crossed a break, and compare with the chart's picture.

**At the spot**

- Compare the satellite temperature (MUR) with the boat's water temperature. The difference, usually a degree or less, is your *calibration* for the day; it lets you read the rest of the chart in the boat's own numbers.
- Work the edge: run along the break, not across it, and cross it to find the best side.
- Press **Drop** to put a mark at the boat's position, stamped with the time, when something works. At the end of the season your marks are your own fishing atlas.

**Afterwards**

- Save the trip in the library. Your own soundings (Chart › Get data › Own soundings) build a seabed picture of your fishing grounds, coloured by depth, that you can show or hide with the seabed button.
- Come back and compare the chart's picture with where the fish were. This is how you learn what each layer means for *your* fishery.

## Worked examples

### 1. Finding a temperature break to troll along

Load MUR and a wind GRIB. Open the panel, choose **Fishing**. The shading is sea temperature; contours are on. Find a place where the contours bunch up: say 17 °C on one side and 19 °C on the other. Narrow the filter to 16–20 °C with the target at 18, so the break appears as a sharp colour change, and turn on **Temperature breaks**. Slide the time forward a day or two to see if it holds. A break that is long, sharp and stays in place across the current is a good first choice. Add a mark at each end of it.

### 2. Following an eddy

Download HYCOM with **Sea surface height**, **Surface currents** and **Temperature and salinity at a depth** (100 m). Choose **Sea surface height**: a rounded red patch is a warm eddy. Turn on **Current direction** to see the flow round it. Change to **Sea temperature at depth** at 100 m: if the deep water under the eddy is warmer than its surroundings, it has a warm core. Switch to MUR for the edge detail at the surface and look for a break where the eddy meets the cooler water. The rim, where the current is strongest, is where to start.

### 3. Cold water coming up

Along a coast with an offshore wind, look at MUR: a patch or a band of cooler water close to the shore, often with higher chlorophyll, suggests upwelling. Confirm with **Sea temperature at depth** at 50 m: is it cool water directly under the surface patch? Then see the chlorophyll over the next days: the green follows the cold water a few days later. Both the cold water and the green water's edges are worth a look.

## How far to trust it

- **Age**: MUR is a day or two old, chlorophyll about three days, HYCOM's analysis is the latest day and the rest is forecast. Himawari is the freshest. The panel's top line says how old each run is.
- **Cloud**: satellites see only through clear sky. MUR fills gaps with a smoothed estimate, so a sharp feature under cloud for days is partly a guess. Himawari leaves gaps; fetch several hours so earlier clear pixels fill in.
- **Skin and depth**: satellites measure the top millimetre. Wind and sun change it. The boat's own water temperature sensor is the truth where you are.
- **Resolution**: HYCOM and chlorophyll resolve features of about 9 km, MUR about 1 km. A feature smaller than the grid will not be there. Do not read the numbers more precisely than they are made.
- **Models are models**: HYCOM is a forecast. It is good at the big picture and the days ahead, and can be wrong about where an eddy's edge lies by tens of kilometres.
- **Safety first**: look at the wind, the waves and the weather for the whole trip. A good break that is 40 NM offshore into a rising southerly is not a good plan. Not for navigation: keep a lookout and use official charts.
