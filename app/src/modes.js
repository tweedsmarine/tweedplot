// Whether a click on the chart is placing something (a ruler point, a route mark, a barrier
// corner, a GRIB area): then the information popups (chart features and seabed, fishing rules,
// tides, tracks, targets…) stand aside, so each click only does what the mode is for.
export const placing = () => !!document.querySelector(".armed, body.ruler-armed, .maplibregl-map.drawing");
