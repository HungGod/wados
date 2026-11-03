# Original wallpapers for wadspaces (public/wallpapers/art): deterministic SVG scenes and patterns,
# each with its own palette, so no two wadspaces look alike.
import math, random, sys, os
out = sys.argv[1]
os.makedirs(out, exist_ok=True)
W, H = 1600, 1000

def svg(name, body, defs=''):
    s = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid slice"><defs>{defs}</defs>{body}</svg>\n'
    open(os.path.join(out, f'{name}.svg'), 'w').write(s)

def lin(id, stops, x2=0, y2=1):
    s = ''.join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in stops)
    return f'<linearGradient id="{id}" x1="0" y1="0" x2="{x2}" y2="{y2}">{s}</linearGradient>'

def rad(id, stops, cx=0.5, cy=0.5, r=0.6):
    s = ''.join(f'<stop offset="{o}" stop-color="{c}" stop-opacity="{op}"/>' for o, c, op in stops)
    return f'<radialGradient id="{id}" cx="{cx}" cy="{cy}" r="{r}">{s}</radialGradient>'

def ridge(rng, base, amp, step=40, rough=0.5):
    pts, y = [], base
    for x in range(-40, W + 80, step):
        y += rng.uniform(-amp, amp) * rough
        y = max(base - amp * 2, min(base + amp, y))
        pts.append((x, y))
    d = f'M-40,{H} ' + ' '.join(f'L{x:.0f},{y:.0f}' for x, y in pts) + f' L{W+80},{H} Z'
    return d

def wave(y0, amp, period, phase, x0=-40, x1=W + 40):
    d = f'M{x0},{y0 + amp * math.sin(phase):.1f}'
    for x in range(x0, x1 + 1, 20):
        d += f' L{x},{y0 + amp * math.sin(x / period + phase):.1f}'
    return d

# 1 Dunes: a sunset over layered sand.
rng = random.Random(1)
body = '<rect width="1600" height="1000" fill="url(#sky)"/><circle cx="1150" cy="380" r="120" fill="#ffe2b0" opacity="0.9"/>'
cols = ['#f2a65a', '#e2884a', '#c96a3c', '#a34f32', '#7a3a2c']
for i, c in enumerate(cols):
    y = 520 + i * 95
    d = f'M-40,{H} L-40,{y}'
    for x in range(-40, W + 81, 20):
        d += f' L{x},{y + 50 * math.sin(x / (260 + i * 40) + i * 1.7) + 20 * math.sin(x / 90 + i):.0f}'
    d += f' L{W+80},{H} Z'
    body += f'<path d="{d}" fill="{c}"/>'
svg('dunes', body, lin('sky', [(0, '#3b1d4a'), (0.55, '#d2607a'), (1, '#f7b37c')]))

# 2 Fjord: night mountains in blues, a moon.
rng = random.Random(2)
body = '<rect width="1600" height="1000" fill="url(#sky)"/><circle cx="380" cy="230" r="70" fill="#e8f0ff" opacity="0.95"/>'
for i in range(140):
    body += f'<circle cx="{rng.uniform(0, W):.0f}" cy="{rng.uniform(0, 520):.0f}" r="{rng.uniform(0.8, 2.2):.1f}" fill="#fff" opacity="{rng.uniform(0.3, 0.9):.2f}"/>'
for i, c in enumerate(['#2b4c7e', '#223d68', '#1a3053', '#12233e', '#0b1729']):
    body += f'<path d="{ridge(rng, 430 + i * 110, 90 - i * 8, 50, 1.2)}" fill="{c}"/>'
svg('fjord', body, lin('sky', [(0, '#0a1430'), (1, '#3a5a8c')]))

# 3 Aurora: green and violet ribbons over a ridge.
rng = random.Random(3)
defs = lin('sky', [(0, '#020b14'), (1, '#0c2a33')]) + '<filter id="b"><feGaussianBlur stdDeviation="28"/></filter>'
body = '<rect width="1600" height="1000" fill="url(#sky)"/>'
for i, (c, y) in enumerate([('#3dffa8', 260), ('#22d3c5', 330), ('#9b5cff', 220), ('#5cff7a', 380)]):
    body += f'<path d="{wave(y, 70, 220 + i * 60, i * 1.3)}" stroke="{c}" stroke-width="{90 - i * 12}" fill="none" opacity="0.55" filter="url(#b)"/>'
for i in range(90):
    body += f'<circle cx="{rng.uniform(0, W):.0f}" cy="{rng.uniform(0, 600):.0f}" r="{rng.uniform(0.6, 1.8):.1f}" fill="#fff" opacity="{rng.uniform(0.2, 0.8):.2f}"/>'
body += f'<path d="{ridge(rng, 760, 70, 40, 1.4)}" fill="#04121a"/>'
svg('aurora', body, defs)

# 4 Circuit: neon traces on a dark board.
rng = random.Random(4)
body = '<rect width="1600" height="1000" fill="#06110f"/>'
for i in range(70):
    x, y = rng.randrange(0, W, 40), rng.randrange(0, H, 40)
    d = f'M{x},{y}'
    for _ in range(rng.randint(3, 7)):
        dx, dy = rng.choice([(1, 0), (0, 1), (1, 1), (-1, 1), (0, -1)])
        n = rng.randint(2, 6) * 40
        x, y = x + dx * n, y + dy * n
        d += f' L{x},{y}'
    c = rng.choice(['#19f2c6', '#13b597', '#7cf2ff'])
    body += f'<path d="{d}" stroke="{c}" stroke-width="3" fill="none" opacity="{rng.uniform(0.35, 0.9):.2f}"/><circle cx="{x}" cy="{y}" r="7" fill="none" stroke="{c}" stroke-width="3"/>'
svg('circuit', body + f'<rect width="1600" height="1000" fill="url(#v)"/>', rad('v', [(0, '#000', 0), (1, '#000', 0.7)], r=0.8))

# 5 Bloom: magenta petals from the middle.
body = '<rect width="1600" height="1000" fill="#1b0620"/>'
for ring, (n, rx, ry, c, op) in enumerate([(16, 520, 120, '#ff3d9a', 0.18), (12, 380, 100, '#ff7ac8', 0.25), (10, 240, 70, '#ffc2e6', 0.35)]):
    for k in range(n):
        a = k * 360 / n + ring * 9
        body += f'<ellipse cx="800" cy="500" rx="{rx}" ry="{ry}" fill="{c}" opacity="{op}" transform="rotate({a:.1f} 800 500) translate({rx * 0.55:.0f} 0)"/>'
body += '<circle cx="800" cy="500" r="60" fill="#ffe36b" opacity="0.9"/>'
svg('bloom', body)

# 6 Tide: rows of waves in cyan on navy.
body = '<rect width="1600" height="1000" fill="url(#g)"/>'
for i in range(26):
    y = 80 + i * 36
    c = ['#2fd4ff', '#2a9df4', '#5ef2ff'][i % 3]
    body += f'<path d="{wave(y, 18 + i * 0.6, 120 + (i % 5) * 18, i * 0.7)}" stroke="{c}" stroke-width="3" fill="none" opacity="{0.25 + (i / 26) * 0.6:.2f}"/>'
svg('tide', body, lin('g', [(0, '#03122b'), (1, '#062f52')]))

# 7 Mesa: stepped plateaus under a pale sky.
rng = random.Random(7)
body = '<rect width="1600" height="1000" fill="url(#sky)"/>'
for i, c in enumerate(['#d98c5f', '#c4704a', '#a8573a', '#86422e']):
    y = 470 + i * 120
    d = f'M-40,{H} L-40,{y}'
    x = -40
    while x < W + 80:
        w = rng.randint(120, 320)
        h = rng.choice([0, -60, -110, 40])
        d += f' L{x},{y + h} L{x + w * 0.15:.0f},{y + h - 70} L{x + w * 0.85:.0f},{y + h - 70} L{x + w},{y + h}'
        x += w
    d += f' L{W+80},{H} Z'
    body += f'<path d="{d}" fill="{c}"/>'
svg('mesa', body, lin('sky', [(0, '#f6e0c4'), (1, '#e9b48a')]))

# 8 Skyline: neon blocks at dusk.
rng = random.Random(8)
body = '<rect width="1600" height="1000" fill="url(#sky)"/>'
for layer, (c, top, op) in enumerate([('#3c1f6b', 380, 1), ('#2a1450', 470, 1), ('#170a30', 560, 1)]):
    x = -20
    while x < W:
        w = rng.randint(60, 160)
        h = rng.randint(120, 420) - layer * 40
        body += f'<rect x="{x}" y="{H - h - (600 - top)}" width="{w}" height="{h + 700}" fill="{c}"/>'
        for _ in range(rng.randint(2, 9)):
            body += f'<rect x="{x + rng.randint(8, max(9, w - 16))}" y="{H - h - (600 - top) + rng.randint(10, max(11, h - 20))}" width="6" height="9" fill="{rng.choice(["#ff4fd8", "#59e3ff", "#ffd36b"])}" opacity="{0.6 + layer * 0.1:.2f}"/>'
        x += w + rng.randint(0, 14)
svg('skyline', body, lin('sky', [(0, '#12052a'), (0.6, '#7a1f6b'), (1, '#ff6a7a')]))

# 9 Koi: gold arcs on deep water.
rng = random.Random(9)
body = '<rect width="1600" height="1000" fill="url(#g)"/>'
for i in range(22):
    cx, cy, r = rng.uniform(0, W), rng.uniform(0, H), rng.uniform(60, 220)
    a = rng.uniform(0, 6.28)
    x1, y1 = cx + r * math.cos(a), cy + r * math.sin(a)
    x2, y2 = cx + r * math.cos(a + 2.2), cy + r * math.sin(a + 2.2)
    c = rng.choice(['#f2b632', '#ff8a3d', '#ffe08a', '#f7f3e8'])
    body += f'<path d="M{x1:.0f},{y1:.0f} A{r:.0f},{r:.0f} 0 0 1 {x2:.0f},{y2:.0f}" stroke="{c}" stroke-width="{rng.uniform(10, 26):.0f}" stroke-linecap="round" fill="none" opacity="{rng.uniform(0.5, 0.95):.2f}"/>'
svg('koi', body, lin('g', [(0, '#06203a'), (1, '#0b3a4f')]))

# 10 Moss: contour lines on dark green.
body = '<rect width="1600" height="1000" fill="#0f1d12"/>'
for (cx, cy, s) in [(420, 380, 1.0), (1180, 620, 1.2), (900, 180, 0.7)]:
    for k in range(1, 15):
        r = k * 34 * s
        d = ''
        for t in range(0, 361, 6):
            a = math.radians(t)
            rr = r * (1 + 0.12 * math.sin(3 * a + k * 0.4) + 0.06 * math.sin(5 * a))
            d += ('M' if t == 0 else 'L') + f'{cx + rr * math.cos(a):.0f},{cy + rr * 0.75 * math.sin(a):.0f} '
        body += f'<path d="{d}Z" stroke="#7fd36b" stroke-width="2" fill="none" opacity="{0.15 + 0.03 * (15 - k):.2f}"/>'
svg('moss', body)

# 11 Retro: a striped sun over a grid.
body = '<rect width="1600" height="1000" fill="url(#sky)"/>'
body += '<circle cx="800" cy="520" r="250" fill="url(#sun)"/>'
for i in range(7):
    y = 560 + i * 26
    body += f'<rect x="500" y="{y}" width="600" height="{6 + i * 2}" fill="#1b0b3a"/>'
body += '<rect x="0" y="640" width="1600" height="360" fill="#14062e"/>'
for i in range(-20, 21):
    body += f'<line x1="{800 + i * 40}" y1="640" x2="{800 + i * 260}" y2="1000" stroke="#ff3dd4" stroke-width="2" opacity="0.6"/>'
for i in range(10):
    y = 640 + (i ** 1.7) * 6
    body += f'<line x1="0" y1="{y:.0f}" x2="1600" y2="{y:.0f}" stroke="#ff3dd4" stroke-width="2" opacity="0.55"/>'
svg('retro', body, lin('sky', [(0, '#0d0221'), (1, '#3d0a5c')]) + lin('sun', [(0, '#ffe66b'), (1, '#ff3d8b')]))

# 12 Lagoon: soft turquoise bokeh.
rng = random.Random(12)
body = '<rect width="1600" height="1000" fill="url(#g)"/>'
for i in range(45):
    c = rng.choice(['#9ff7e5', '#4fe0d0', '#e6fff8', '#7ad7ff'])
    body += f'<circle cx="{rng.uniform(0, W):.0f}" cy="{rng.uniform(0, H):.0f}" r="{rng.uniform(20, 140):.0f}" fill="{c}" opacity="{rng.uniform(0.06, 0.22):.2f}"/>'
svg('lagoon', body, lin('g', [(0, '#0a5c66'), (1, '#0f8f8a')], x2=1, y2=1))

# 13 Ember: glowing flow lines on charcoal.
rng = random.Random(13)
body = '<rect width="1600" height="1000" fill="#140b08"/>'
for i in range(40):
    y = rng.uniform(100, 900)
    body += f'<path d="{wave(y, rng.uniform(20, 90), rng.uniform(120, 400), rng.uniform(0, 6))}" stroke="{rng.choice(["#ff7a1a", "#ffb347", "#ff4a1c", "#ffd27a"])}" stroke-width="{rng.uniform(1, 4):.1f}" fill="none" opacity="{rng.uniform(0.15, 0.6):.2f}"/>'
svg('ember', body)

# 14 Orchard: circles in a grid, greens and lemon.
rng = random.Random(14)
body = '<rect width="1600" height="1000" fill="#18301f"/>'
for gx in range(0, W + 100, 100):
    for gy in range(0, H + 100, 100):
        c = rng.choice(['#3f7a3a', '#5c9e45', '#a6d05a', '#e8e25c', '#2c5a2f'])
        r = rng.choice([22, 32, 44])
        body += f'<circle cx="{gx}" cy="{gy}" r="{r}" fill="{c}"/>'
        if rng.random() < 0.3:
            body += f'<path d="M{gx},{gy - r} q14,-22 30,-18" stroke="#2c5a2f" stroke-width="5" fill="none"/>'
svg('orchard', body)

# ------------------------------------------------------------ WadSpaces' own: its three colours (lime, pink, deep violet)
LIME, PINK, VIOLET, INK = '#c6ff1f', '#ff3d81', '#6c2bd9', '#0a0614'

def mix(a, b, t):
    a, b = [int(a[i:i + 2], 16) for i in (1, 3, 5)], [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return '#' + ''.join(f'{round(x + (y - x) * t):02x}' for x, y in zip(a, b))

# 15 Silk: the lock screen's. Ribbons of fine threads, violet into pink into lime, across a deep violet night;
# the left (where the desktop's icons are) kept calm. Lime only where it's bright: the threads' ends, sparks.
rng = random.Random(15)
def glowc(id, cx, cy, r, stops):
    s = ''.join(f'<stop offset="{o}" stop-color="{c}" stop-opacity="{op}"/>' for o, c, op in stops)
    return f'<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{cx}" cy="{cy}" r="{r}">{s}</radialGradient>'
defs = (lin('bg', [(0, '#0d0720'), (0.5, '#130a29'), (1, '#07040f')], x2=1, y2=1)
        + glowc('gv', 160, 120, 900, [(0, VIOLET, 0.5), (0.5, VIOLET, 0.16), (1, VIOLET, 0)])
        + glowc('gp', 1420, 960, 820, [(0, PINK, 0.4), (0.5, PINK, 0.12), (1, PINK, 0)])
        + '<linearGradient id="rib" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="1600" y2="0"><stop offset="0" stop-color="#6c2bd9"/><stop offset="0.55" stop-color="#ff3d81"/><stop offset="0.9" stop-color="#c6ff1f"/><stop offset="1" stop-color="#e9ff8f"/></linearGradient>'
        + '<filter id="soft" filterUnits="userSpaceOnUse" x="-300" y="-300" width="2200" height="1600"><feGaussianBlur stdDeviation="28"/></filter>')
body = '<rect width="1600" height="1000" fill="url(#bg)"/><rect width="1600" height="1000" fill="url(#gv)"/><rect width="1600" height="1000" fill="url(#gp)"/>'
def silk(x0, y0, c1, c2, x1, y1, n, spread, width, peak, under=0.16):
    out = f'<path d="M{x0},{y0} C{c1[0]},{c1[1]} {c2[0]},{c2[1]} {x1},{y1}" stroke="url(#rib)" stroke-width="70" fill="none" opacity="{under}" filter="url(#soft)"/>'
    for i in range(n):
        t = i / (n - 1)
        o = math.sin(t * math.pi)
        d = (t - 0.5) * spread
        out += (f'<path d="M{x0},{y0 + d:.1f} C{c1[0]},{c1[1] + d * 2.4:.1f} {c2[0]},{c2[1] - d * 1.4:.1f} {x1},{y1 + d * 0.9:.1f}" '
                f'stroke="url(#rib)" stroke-width="{width}" fill="none" opacity="{0.04 + peak * o:.3f}"/>')
    return out
body += silk(-120, 900, (420, 1080), (960, 250), 1720, 450, 72, 170, 1.0, 0.4)
body += silk(380, 1100, (900, 830), (1230, 110), 1720, 120, 40, 90, 0.9, 0.3, 0.1)
for i in range(80):
    x, y = rng.uniform(760, 1600), rng.uniform(30, 970)
    c = rng.choice([LIME, LIME, PINK, '#ffffff'])
    body += f'<circle cx="{x:.0f}" cy="{y:.0f}" r="{rng.uniform(0.6, 1.9):.1f}" fill="{c}" opacity="{rng.uniform(0.2, 0.75):.2f}"/>'
svg('silk', body, defs)

# 16 Duo: the desktop's. Midnight's two lights, mirrored: one fanning down from the top middle in WadSpaces'
# lime, one fanning up from the bottom middle in its pink, the same shape and size; the middle (where the icons
# and windows are) left dark.
def fan(id, cy, color, peak):
    # An ellipse as wide as the screen, its middle just off the edge: bright at the edge, fading fast.
    return (f'<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="0" cy="0" r="1" gradientTransform="translate(800 {cy}) scale(1250 560)">'
            f'<stop offset="0" stop-color="{color}" stop-opacity="{peak}"/><stop offset="0.22" stop-color="{color}" stop-opacity="{peak * 0.6:.3f}"/>'
            f'<stop offset="0.5" stop-color="{color}" stop-opacity="{peak * 0.2:.3f}"/><stop offset="0.8" stop-color="{color}" stop-opacity="{peak * 0.04:.3f}"/><stop offset="1" stop-color="{color}" stop-opacity="0"/></radialGradient>')
defs = (lin('bg', [(0, '#0b0617'), (0.5, '#06030d'), (1, '#0b0617')])
        + fan('top', -40, LIME, 0.66)
        + fan('bot', 1040, PINK, 0.8))
body = '<rect width="1600" height="1000" fill="url(#bg)"/><rect width="1600" height="1000" fill="url(#top)"/><rect width="1600" height="1000" fill="url(#bot)"/>'
svg('duo', body, defs)

print('ok')
