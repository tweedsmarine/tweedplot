"""Replace the hard-coded colours in style.css with OpenBridge palette variables.

The role of a colour is read from the property it is in (a surface, text, a border) and, for
neutral colours, how far it stands from the surface it sits on, so the dark drawers and data
panel and the light dialogs both land on the same OpenBridge roles. Saturated colours go to
the OpenBridge alert colours (red, amber, yellow, green) or the selected colour (blue), or to
a step of the matching OpenBridge colour ramp. Prints a table of what became what."""
import re, sys, colorsys, collections

src = open(sys.argv[1]).read()
report = collections.Counter()

def parse(c):
    c = c.strip().lower()
    m = re.fullmatch(r'#([0-9a-f]{3,8})', c)
    if m:
        h = m.group(1)
        if len(h) in (3, 4): h = ''.join(x * 2 for x in h)
        a = int(h[6:8], 16) / 255 if len(h) == 8 else 1.0
        return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a
    m = re.fullmatch(r'rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:[\s,/]+([\d.]+%?))?\s*\)', c)
    if m:
        a = m.group(4)
        a = 1.0 if a is None else (float(a[:-1]) / 100 if a.endswith('%') else float(a))
        return float(m.group(1)), float(m.group(2)), float(m.group(3)), a
    return None

def hls(r, g, b):
    h, l, s = colorsys.rgb_to_hls(r / 255, g / 255, b / 255)
    return h * 360, l, s

def family(h):
    if h >= 345 or h < 15: return 'red'
    if h < 40: return 'orange'
    if h < 65: return 'yellow'
    if h < 170: return 'mint'
    if h < 200: return 'teal'
    if h < 255: return 'blue'
    return 'purple'

ALERT_BG = {'red': '--alert-alarm-color', 'orange': '--alert-warning-color', 'yellow': '--alert-caution-color',
            'mint': '--alert-running-color', 'blue': '--selected-enabled-background-color',
            'teal': '--base-teal-400', 'purple': '--base-purple-400'}
ON = {'--alert-alarm-color': '--on-alarm-color', '--alert-warning-color': '--on-warning-color',
      '--alert-caution-color': '--on-caution-color', '--selected-enabled-background-color': '--on-selected-active-color',
      '--alert-running-color': '--on-selected-active-color', '--base-teal-400': '--on-selected-active-color',
      '--base-purple-400': '--on-selected-active-color'}

BG_PROPS = re.compile(r'^(background(-color)?|--.*(bg|panel|surface|fill-bg|road).*)$')
TEXT_PROPS = re.compile(r'^(color|fill|stroke|caret-color|--.*(ink|quiet|text|col|good|bad|accent|on).*|text-decoration-color)$')
BORDER_PROPS = re.compile(r'^(border.*|outline.*|--.*(line|border|grid|edge).*|column-rule.*)$')
SHADOW_PROPS = re.compile(r'^(box-shadow|text-shadow|filter|--.*shadow.*)$')

def role(prop, rgba):
    r, g, b, a = rgba
    h, l, s = hls(r, g, b)
    chrom = (max(r, g, b) - min(r, g, b)) >= 45 and s > 0.25 and 0.12 < l < 0.93
    kind = 'bg' if BG_PROPS.match(prop) else 'border' if BORDER_PROPS.match(prop) else \
        'shadow' if SHADOW_PROPS.match(prop) else 'text'
    if kind == 'shadow':
        return None  # shadows stay as they are
    if not chrom:
        if kind == 'bg':
            if l > 0.5:
                return '--container-global-color' if l >= 0.985 else '--container-background-color' if l >= 0.955 \
                    else '--container-section-color' if l >= 0.9 else '--container-backdrop-color'
            return '--container-background-color' if l <= 0.2 \
                else '--container-section-color' if l <= 0.26 else '--normal-hover-background-color'
        if kind == 'border':
            e = abs(l - 0.5) * 2
            return '--border-divider-color' if e > 0.75 else '--border-outline-color' if e > 0.45 else '--normal-enabled-border-color'
        e = abs(l - 0.5) * 2
        return '--element-active-color' if e > 0.7 else '--element-neutral-color' if e > 0.3 else '--element-inactive-color'
    fam = family(h)
    if kind == 'bg':
        if l > 0.85: return f'--base-{fam}-050'
        if l > 0.72: return f'--base-{fam}-100'
        return ALERT_BG[fam]
    if kind == 'border':
        if l > 0.8: return f'--base-{fam}-200'
        return ALERT_BG[fam] if fam in ('red', 'orange', 'yellow', 'blue') else f'--base-{fam}-400'
    # text, fill, stroke
    if fam == 'red': return '--alert-alarm-color'
    return f'--base-{fam}-500'

COL = re.compile(r'#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|\b(white|black)\b')

def convert_value(prop, val, bg_role):
    # leave data URIs (cursors, icons) alone
    parts = re.split(r'(url\([^)]*\))', val)
    out = []
    for p in parts:
        if p.startswith('url('):
            out.append(p); continue
        def rep(m):
            c = m.group(0)
            rgba = parse({'white': '#fff', 'black': '#000'}.get(c, c))
            if rgba is None: return c
            r, g, b, a = rgba
            if a < 1 and max(r, g, b) - min(r, g, b) < 20 and (max(r, g, b) < 40):
                return c  # black veils and shadows
            t = role(prop, rgba)
            if t is None: return c
            # white or near-white text on a saturated surface in the same rule: the "on" colour of that surface
            if bg_role in ON and t == '--element-active-color' and hls(r, g, b)[1] > 0.6 and prop in ('color', 'fill', 'stroke'):
                t = ON[bg_role]
            report[(c.lower(), prop if not prop.startswith('--') else 'var', t)] += 1
            v = f'var({t})'
            if a < 1:
                v = f'color-mix(in srgb, {v} {round(a * 100)}%, transparent)'
            return v
        out.append(COL.sub(rep, p))
    return ''.join(out)

# don't touch these properties: brand colours, map colours, and the colours the chart uses
KEEP_PROPS = {'--sea', '--land', '--coast', '--brand-navy', '--brand-ink', '--brand-sea', '--brand-amber', '--brand-sky',
              '--wind-true', '--wind-app', '--c'}
# set by hand (the core variables)
ROOT_SET = {
    '--panel-bg': 'var(--container-background-color)', '--ink': 'var(--element-active-color)',
    '--quiet': 'var(--element-neutral-color)', '--line': 'var(--border-divider-color)',
    '--good': 'var(--alert-running-color)', '--bad': 'var(--alert-alarm-color)',
    '--accent': 'var(--selected-enabled-background-color)',
    '--tile-bg': 'var(--container-section-color)', '--tile-ink': 'var(--element-active-color)', '--tile-quiet': 'var(--element-neutral-color)',
    '--warn-bg': 'var(--alert-warning-color)', '--warn-ink': 'var(--on-warning-color)',
    '--alarm-bg': 'var(--alert-alarm-color)', '--alarm-ink': 'var(--on-alarm-color)',
    '--btn-bg': 'var(--normal-enabled-background-color)', '--btn-bg-hover': 'var(--normal-hover-background-color)',
    '--btn-line': 'var(--normal-enabled-border-color)', '--btn-ink': 'var(--on-normal-active-color)',
    '--btn-on': 'var(--selected-enabled-background-color)',
}
# the dark drawers and data panel redefined these to suit a dark surface: the palette does it now
DROP_LOCAL = {'--good', '--bad', '--quiet', '--line', '--ink', '--btn-ink', '--btn-bg', '--btn-bg-hover', '--btn-line', '--tile-ink', '--tile-quiet'}

def convert_block(sel, body):
    decls = re.split(r';(?![^(]*\))', body)
    # the surface this rule paints, to choose the text colour on it
    bg_role = None
    for d in decls:
        m = re.match(r'\s*([-a-z]+)\s*:\s*(.*)', d, re.S)
        if m and m.group(1) in ('background', 'background-color'):
            c = COL.search(m.group(2))
            if c:
                rgba = parse({'white': '#fff', 'black': '#000'}.get(c.group(0), c.group(0)))
                if rgba: bg_role = role('background', rgba)
            elif 'var(--brand-navy' in m.group(2): bg_role = 'brand'
            elif 'var(--alarm-bg' in m.group(2) or 'var(--bad' in m.group(2): bg_role = '--alert-alarm-color'
            elif 'var(--warn-bg' in m.group(2): bg_role = '--alert-warning-color'
            elif 'var(--accent' in m.group(2) or 'var(--btn-on' in m.group(2): bg_role = '--selected-enabled-background-color'
    is_root = sel.strip() == ':root'
    out = []
    for d in decls:
        m = re.match(r'(\s*(?:/\*.*?\*/\s*)*)([-a-z0-9]+)(\s*:\s*)(.*)', d, re.S)
        if not m:
            out.append(d); continue
        ws, prop, colon, val = m.groups()
        if prop in KEEP_PROPS:
            out.append(d); continue
        if is_root and prop in ROOT_SET:
            out.append(f'{ws}{prop}{colon}{ROOT_SET[prop]}'); report[('root', prop, ROOT_SET[prop])] += 1; continue
        if not is_root and prop in DROP_LOCAL and not val.strip().startswith('var(--obc'):
            report[('dropped', prop, sel.strip()[:40])] += 1
            continue
        if prop == 'color-scheme' and not is_root:
            report[('dropped', prop, sel.strip()[:40])] += 1
            continue
        if bg_role == 'brand':
            out.append(d); continue
        if prop.startswith('--cur'):
            out.append(d); continue
        out.append(ws + prop + colon + convert_value(prop, val, bg_role))
    res = ';'.join(x for x in out)
    return res

# the palette section (filters and night cursors) is rewritten by hand: leave its colours alone
def process(css):
    out = []
    pos = 0
    for m in re.finditer(r'([^{}]*)\{([^{}]*)\}', css):
        out.append(css[pos:m.start()])
        sel, body = m.group(1), m.group(2)
        if 'data-palette' in sel or '@keyframes' in sel and False:
            out.append(m.group(0))
        else:
            # strip comments from the selector check only
            out.append(sel + '{' + convert_block(re.sub(r'/\*.*?\*/', '', sel, flags=re.S).split('\n')[-1], body) + '}')
        pos = m.end()
    out.append(css[pos:])
    return ''.join(out)

res = process(src)
open(sys.argv[2], 'w').write(res)
for k, n in sorted(report.items(), key=lambda x: -x[1]):
    print(n, *k, sep='\t')
