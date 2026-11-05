const CLASS_SCALE = { a: 1, bp: 0.78, b: 0.55 };
// own-fleet tags sort above every target's outline and fill
const TAG_LIFT = 1e6;
const CLASS_A_OUTLINE_MM = 5.5;
const CLASS_A_OUTLINE_WIDTH = 2 * (8 + (2.2 * 8) / Math.hypot(8, 11));

function iconShape(icon) {
  const ship = icon.match(/^ais-[sf]-(a|bp|b)$/);
  if (ship) {
    const scale = CLASS_SCALE[ship[1]];
    return {
      points: [[0, -13], [8, 11], [0, 7], [-8, 11]].map(([x, y]) => [x * scale, y * scale]),
      fish: icon.startsWith("ais-f-"),
      fleet: true,
      scale,
    };
  }
  if (icon === "ais-air") return { points: [[0, -13], [2, -4], [13, 2], [13, 5], [2, 1], [2, 8], [6, 11], [-6, 11], [-2, 8], [-2, 1], [-13, 5], [-13, 2], [-2, -4]] };
  if (icon === "ais-aton") return { points: [[0, -11], [11, 0], [0, 11], [-11, 0]] };
  if (icon === "ais-base") return { points: [[-8, -8], [8, -8], [8, 8], [-8, 8]] };
  if (icon === "ais-sart") return { points: circle(10) };
  if (icon.includes("sleep")) return { points: circle(4.5), fish: icon.endsWith("-fish") };
  return { points: circle(5), fish: icon.endsWith("-fish") };
}

function circle(radius) {
  return Array.from({ length: 32 }, (_, i) => {
    const a = (2 * Math.PI * i) / 32;
    return [radius * Math.cos(a), radius * Math.sin(a)];
  });
}

function scaled(points, factor) {
  return points.map(([x, y]) => [x * factor, y * factor]);
}

function expanded(points, distance) {
  const center = points.reduce((c, p) => [c[0] + p[0] / points.length, c[1] + p[1] / points.length], [0, 0]);
  return points.map(([x, y]) => {
    const dx = x - center[0], dy = y - center[1], length = Math.hypot(dx, dy) || 1;
    return [x + (dx / length) * distance, y + (dy / length) * distance];
  });
}

function polygonFeature(ring, color, mmsi, z, opacity = 1) {
  return {
    type: "Feature",
    properties: { k: "shape", mmsi, col: color, z, opacity },
    geometry: { type: "Polygon", coordinates: [ring] },
  };
}

function geoRing(position, heading, points, zoom, pxPerMm) {
  const [lon, lat] = position;
  const latRad = (lat * Math.PI) / 180;
  const cosLat = Math.max(1e-6, Math.cos(latRad));
  const mPerPx = (78271.517 * cosLat) / 2 ** zoom;
  const mLon = 111320 * cosLat;
  const angle = (heading * Math.PI) / 180;
  const c = Math.cos(angle), s = Math.sin(angle);
  const zoomScale = Math.max(0.7, Math.min(1.1, 0.7 + (zoom - 6) * (0.4 / 6)));
  const size = zoomScale * (CLASS_A_OUTLINE_MM * pxPerMm / CLASS_A_OUTLINE_WIDTH);
  const ring = points.map(([x, y]) => {
    const east = x * size * mPerPx, north = -y * size * mPerPx;
    return [lon + (east * c + north * s) / mLon, lat + (-east * s + north * c) / 111320];
  });
  let area = 0;
  for (let i = 0; i < points.length; i++) {
    const a = points[i], b = points[(i + 1) % points.length];
    area += a[0] * b[1] - b[0] * a[1];
  }
  if (area > 0) ring.reverse();
  ring.push(ring[0]);
  return ring;
}

/** Build same-layer, sorted fill features so each target's outline and fill keep their overlap order. */
export function targetSymbolFeatures({ position, heading, icon, color, outline, zoom, pxPerMm = 3.78, mmsi, sortKey, fleetColor }) {
  const { points, fish = false, fleet = false, scale = 1 } = iconShape(icon);
  const features = [];
  const add = (shape, fill, order) => features.push(polygonFeature(geoRing(position, heading, shape, zoom, pxPerMm), fill, mmsi, sortKey + order));
  const iconScale = Math.max(0.7, Math.min(1.1, 0.7 + (zoom - 6) * (0.4 / 6)));

  // with a fleet tag the border wraps the whole shape: the hull plus the tag filling its stern notch
  const tagged = fleet && !!fleetColor;
  const silhouette = tagged ? [[0, -13 * scale], [8 * scale, 11 * scale], [-8 * scale, 11 * scale]] : points;
  add(expanded(silhouette, 2.2 / iconScale), outline, 0);
  add(points, color, 1);
  if (fish) add([[-11 * scale, 0.5 * scale], [11 * scale, 0.5 * scale], [11 * scale, 3.5 * scale], [-11 * scale, 3.5 * scale]], outline, 2);
  if (icon === "ais-aton") {
    add([[-6, -0.65], [6, -0.65], [6, 0.65], [-6, 0.65]], outline, 2);
    add([[-0.65, -6], [0.65, -6], [0.65, 6], [-0.65, 6]], outline, 3);
  } else if (icon === "ais-sart") {
    add([[-5.5, -6.5], [-6.5, -5.5], [5.5, 6.5], [6.5, 5.5]], outline, 2);
    add([[-5.5, 6.5], [-6.5, 5.5], [5.5, -6.5], [6.5, -5.5]], outline, 3);
  }
  if (tagged) add([[0, 7 * scale], [8 * scale, 11 * scale], [-8 * scale, 11 * scale]], fleetColor, TAG_LIFT + 4);
  return features;
}

/** Give true-scale hulls the same sortable white edge and colored fill as point symbols. */
export function hullSymbolFeatures({ ring, color, outline, zoom, length, mmsi, sortKey }) {
  const points = ring.slice(0, -1);
  const center = points.reduce((c, p) => [c[0] + p[0] / points.length, c[1] + p[1] / points.length], [0, 0]);
  const lengthPixels = length / ((78271.517 * Math.cos((ring[0][1] * Math.PI) / 180)) / 2 ** zoom);
  const inset = Math.max(0.7, 1 - 1.5 / lengthPixels);
  const inner = points.map(([lon, lat]) => [center[0] + (lon - center[0]) * inset, center[1] + (lat - center[1]) * inset]);
  inner.push(inner[0]);
  return [
    polygonFeature(ring, outline, mmsi, sortKey),
    polygonFeature(inner, color, mmsi, sortKey + 1, 0.45),
  ];
}
