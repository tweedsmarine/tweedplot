#!/usr/bin/env python3
"""Import the TweedPlot User Manual into the app (app/public/manual).

The manual is written as a Claude Doc. Each tab is exported as Markdown; the
exports replace pictures and diagrams with text placeholders, which this script
turns back into images (screenshots, by alt text) and inline SVG diagrams (by
caption). The result is bundled into the app at build time, so the manual
always matches the version of the app it ships with.

    python tools/manual/import.py <exports dir> <screenshots dir> tools/manual/fig

<exports dir> holds Reference.md, "Route planning (tutorial).md",
"Weather routing (tutorial).md" and Glossary.md (the Screenshot list tab is
not shipped).
"""
import json, re, shutil, sys, datetime
from pathlib import Path

TABS = [
    ("reference", "Reference", "Reference.md"),
    ("planning", "Route planning", "Route planning (tutorial).md"),
    ("weather-routing", "Weather routing", "Weather routing (tutorial).md"),
    ("glossary", "Glossary", "Glossary.md"),
]

# screenshot alt text (start of it) -> file name in the screenshots dir
IMAGES = [
    ("The main window", "01-main-weather.jpg"),
    ("The popup for the", "02-light-popup.jpg"),
    ("Drawers closed", "03-slider-grib-panel.jpg"),
    ("A meteogram south of Africa", "04-meteogram.jpg"),
    ("Nelson to the Abel Tasman", "05-tasman-cruise.jpg"),
    ("A meteogram at a spot", "06-fishing-meteogram.jpg"),
    ("A Coastal Classic course", "07-race.jpg"),
    ("Wind disagreement", "08-agreement.jpg"),
    ("A gate sweep", "09-gate-sweep.jpg"),
    ("Library", "10-barriers.jpg"),
]
# diagram caption (start of it) -> SVG file in the diagrams dir
DIAGRAMS = [
    ("screen layout", "layout.svg"),
    ("passage planning", "stages.svg"),
    ("isochrone routing", "isochrones.svg"),
]

PLACEHOLDER = re.compile(r"&#91;(image|embedded content): (.*?)\\\]")


def convert(md: str, used: set) -> str:
    def repl(m):
        kind, text = m.group(1), m.group(2).strip()
        table = IMAGES if kind == "image" else DIAGRAMS
        for prefix, name in table:
            if text.startswith(prefix):
                used.add(name)
                folder = "img" if kind == "image" else "fig"
                return f"![{text}]({folder}/{name})"
        print(f"warning: no file for {kind} '{text[:50]}'", file=sys.stderr)
        return ""
    md = PLACEHOLDER.sub(repl, md)
    md = md.replace("&#32;", " ")
    # the doc's byline (date chip, author) is not useful in the app
    md = re.sub(r"^\w{3} \d{1,2}, \d{4} · @\w+\s*$", "", md, flags=re.M)
    md = re.sub(r"^Screenshots are marked .*$", "", md, flags=re.M)
    return re.sub(r"\n{3,}", "\n\n", md).strip() + "\n"


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    src, shots, figs = (Path(a) for a in sys.argv[1:])
    out = Path(__file__).resolve().parents[2] / "app" / "public" / "manual"
    if out.exists():
        shutil.rmtree(out)
    (out / "img").mkdir(parents=True)
    (out / "fig").mkdir()
    used, tabs = set(), []
    for tid, title, fname in TABS:
        md = convert((src / fname).read_text(encoding="utf-8"), used)
        (out / f"{tid}.md").write_text(md, encoding="utf-8")
        tabs.append({"id": tid, "title": title, "file": f"{tid}.md"})
    for _, name in IMAGES:
        if name in used:
            shutil.copy(shots / name, out / "img" / name)
    for _, name in DIAGRAMS:
        if name in used:
            shutil.copy(figs / name, out / "fig" / name)
    manual = {"title": "TweedPlot User Manual", "updated": datetime.date.today().isoformat(), "tabs": tabs}
    (out / "manual.json").write_text(json.dumps(manual, indent=2), encoding="utf-8")
    print(f"wrote {out} ({len(tabs)} tabs, {len(used)} pictures)")


if __name__ == "__main__":
    main()
