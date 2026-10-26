#!/usr/bin/env python3
"""A big test library for TweedPlot: thousands of marks and many miles of tracks, as a
commercial fisher builds up over the years. Writes the library's own files (marks.json,
tracks/<id>.json) into a new folder, which TweedPlot opens with RACENAV_LIBRARY_DIR.

    python make_library.py C:\\dev\\testlib --marks 20000 --tracks 300 --hours 10

Marks are spread over fishing grounds off the NZ coast (Tasman Bay, Cook Strait, the
west coast, Hawke Bay, Chatham Rise) as points of interest and waypoints in many
groups; tracks are boat-speed wanders (6-10 kn, a fix every 10 s) round the same grounds.
"""
import argparse, json, math, os, random, time

GROUNDS = [  # name, lon, lat, spread (degrees)
    ("Tasman Bay", 173.15, -41.05, 0.25), ("Golden Bay", 172.85, -40.65, 0.25), ("Cook Strait", 174.5, -41.3, 0.3),
    ("West Coast", 171.0, -42.3, 0.6), ("Taranaki Bight", 174.0, -39.9, 0.5), ("Hawke Bay", 177.2, -39.4, 0.4),
    ("Chatham Rise", 179.0, -43.5, 1.5), ("Hauraki Gulf", 175.2, -36.6, 0.35), ("Foveaux Strait", 168.2, -46.75, 0.3),
]
KINDS = ["fishing"] * 6 + ["foul", "wreck", "hazard", "waypoint", "anchorage", "info"]
WORDS = ["Reef", "Hole", "Edge", "Pinnacle", "Drop", "Hump", "Snag", "Patch", "Ledge", "Gutter", "Bank", "Corner"]


def mark(i, now):
    g, lon, lat, sp = random.choice(GROUNDS)
    kind = random.choice(KINDS)
    year = random.randint(2008, 2026)
    return {
        "id": f"tm{i:06d}", "name": f"{random.choice(WORDS)} {i}", "kind": kind,
        "lat": round(lat + random.gauss(0, sp / 2), 6), "lon": round(lon + random.gauss(0, sp / 2), 6),
        "notes": random.choice(["", "", "snapper", "tarakihi", "hapuku", "gurnard", "net snagged", "good in NW"]),
        "group": f"{g} {year}", "barrier_nm": 0.0, "modified": now - random.randint(0, 5 * 365 * 86400),
    }


def track(i, hours, now):
    g, lon, lat, sp = random.choice(GROUNDS)
    t = now - random.randint(0, 3 * 365 * 86400)
    x, y = lon + random.gauss(0, sp / 3), lat + random.gauss(0, sp / 3)
    cog, pts, dist, maxs = random.uniform(0, 360), [], 0.0, 0.0
    t0 = t
    for _ in range(int(hours * 360)):  # a fix every 10 s
        sog = max(0.5, random.gauss(7.5, 1.5))
        cog = (cog + random.gauss(0, 4)) % 360
        if math.hypot(x - lon, y - lat) > sp:  # turn back towards the ground
            cog = (math.degrees(math.atan2(lon - x, lat - y)) + random.gauss(0, 20)) % 360
        d = sog * 10 / 3600  # NM in 10 s
        y += d / 60 * math.cos(math.radians(cog))
        x += d / 60 * math.sin(math.radians(cog)) / math.cos(math.radians(y))
        dist += d
        maxs = max(maxs, sog)
        pts.append([t, round(y, 6), round(x, 6), round(sog, 2), round(cog, 1)])
        t += 10
    xs, ys = [p[2] for p in pts], [p[1] for p in pts]
    return {
        "id": f"tt{i:05d}", "name": f"{g} trip {i}", "notes": "", "colour": "", "source": "log", "modified": t,
        "stats": {"points": len(pts), "start": t0, "end": t, "distance_nm": round(dist, 1), "max_sog": round(maxs, 1),
                  "bbox": [min(xs), min(ys), max(xs), max(ys)]},
        "segments": [pts],
    }


def main():
    a = argparse.ArgumentParser()
    a.add_argument("out")
    a.add_argument("--marks", type=int, default=10000)
    a.add_argument("--tracks", type=int, default=100)
    a.add_argument("--hours", type=float, default=8, help="length of each track")
    a.add_argument("--seed", type=int, default=1)
    o = a.parse_args()
    random.seed(o.seed)
    now = int(time.time())
    os.makedirs(os.path.join(o.out, "tracks"), exist_ok=True)
    with open(os.path.join(o.out, "marks.json"), "w") as f:
        json.dump([mark(i, now) for i in range(o.marks)], f)
    total_nm, total_pts = 0.0, 0
    for i in range(o.tracks):
        tr = track(i, o.hours, now)
        total_nm += tr["stats"]["distance_nm"]
        total_pts += tr["stats"]["points"]
        with open(os.path.join(o.out, "tracks", tr["id"] + ".json"), "w") as f:
            json.dump(tr, f, separators=(",", ":"))
    print(f"{o.marks} marks, {o.tracks} tracks: {total_nm:,.0f} NM, {total_pts:,} fixes, in {o.out}")


if __name__ == "__main__":
    main()
