import csv, math, os

_zenpng_out = os.environ.get('ZENPNG_OUTPUT_DIR', '/mnt/v/output/zenpng')
_project_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

with open(os.path.join(_zenpng_out, 'effort_curve', 'effort_curve.csv')) as f:
    rows = list(csv.DictReader(f))

efforts = list(range(1, 31))
agg = {}
for e in efforts:
    total_size = sum(int(r["e%d_size" % e]) for r in rows)
    t_list = sorted(int(r["e%d_ms" % e]) for r in rows)
    median_t = t_list[len(t_list)//2]
    agg[e] = (total_size, median_t)

# e31 (Brag) — measured separately via single_effort example
# Full e30 pipeline + 15 FullOptimal iterations
agg[31] = (8_310_765, 3515)
efforts.append(31)

# Pareto frontier (weakly dominated = excluded)
pts_all = [(e, agg[e][1], agg[e][0]) for e in efforts]
pareto = []
for e, t, s in pts_all:
    dominated = False
    for e2, t2, s2 in pts_all:
        if e2 == e:
            continue
        if t2 <= t and s2 <= s and (t2 < t or s2 < s):
            dominated = True
            break
    if not dominated:
        pareto.append(e)

presets = {1:"Fastest", 2:"Turbo", 7:"Fast", 13:"Balanced", 17:"Thorough", 19:"High",
           22:"Aggressive", 24:"Intense", 27:"Crush", 30:"Maniac", 31:"Brag"}

def color(e):
    if e <= 4: return "#4a90d9"
    if e <= 9: return "#50b86c"
    if e == 10: return "#d4a020"
    if e <= 17: return "#e07040"
    if e <= 22: return "#c050c0"
    if e <= 30: return "#7070e0"
    return "#d04040"  # FullOptimal (31+)

STYLE = """<style>
  :root { --fg: #24292f; --fg2: #57606a; --fg3: #8c959f; --grid: #d0d7de; --bg: #ffffff; }
  @media (prefers-color-scheme: dark) {
    :root { --fg: #e6edf3; --fg2: #8b949e; --fg3: #6e7681; --grid: #30363d; --bg: #0d1117; }
  }
  text { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; }
  .tick { font-size: 10.5px; fill: var(--fg2); }
  .title { font-size: 14px; font-weight: 600; fill: var(--fg); }
  .elabel { font-size: 9px; font-weight: 500; }
  .preset { font-size: 8.5px; font-style: italic; }
  .legend { font-size: 9.5px; fill: var(--fg2); }
  .axtitle { font-size: 10.5px; fill: var(--fg2); }
</style>"""

def gen_chart(title, effort_range, x_min_ms, x_max_ms, x_grid, x_grid_labels,
              y_min, y_max, y_grid, y_grid_labels, label_offsets, legend_items,
              width=660, height=378, log_x=True):
    left, right = 72, width - 28
    top, bottom = 30, 330
    pw = right - left
    ph = bottom - top

    if log_x:
        x_min_log = math.log10(x_min_ms)
        x_max_log = math.log10(x_max_ms)
        x_range = x_max_log - x_min_log
        def x_pos(ms):
            if ms <= 0: ms = 0.5
            return left + (math.log10(ms) - x_min_log) / x_range * pw
    else:
        def x_pos(ms):
            return left + (ms - x_min_ms) / (x_max_ms - x_min_ms) * pw

    def y_pos(size):
        return bottom - (size - y_min) / (y_max - y_min) * ph

    lines = []
    lines.append('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">' % (width, height, width, height))
    lines.append(STYLE)
    lines.append('<rect width="%d" height="%d" fill="var(--bg)" rx="6"/>' % (width, height))
    lines.append('<text x="%.1f" y="19" text-anchor="middle" class="title">%s</text>' % ((left + right) / 2, title))
    lines.append('<text x="%.1f" y="374" text-anchor="middle" class="axtitle">Median encode time (100 images)</text>' % ((left + right) / 2))
    lines.append('<text transform="rotate(-90)" x="%.1f" y="14" text-anchor="middle" class="axtitle">Aggregate encoded size</text>' % (-(top + bottom) / 2))

    # Y grid
    for val, label in zip(y_grid, y_grid_labels):
        y = y_pos(val)
        lines.append('<line x1="%d" y1="%.1f" x2="%d" y2="%.1f" stroke="var(--grid)" stroke-width="0.7"/>' % (left, y, right, y))
        lines.append('<text x="%d" y="%.1f" text-anchor="end" class="tick">%s</text>' % (left - 6, y + 3.5, label))

    # X grid
    for val, label in zip(x_grid, x_grid_labels):
        x = x_pos(val)
        if x < left - 5 or x > right + 5:
            continue
        lines.append('<line x1="%.1f" y1="%d" x2="%.1f" y2="%d" stroke="var(--grid)" stroke-width="0.7"/>' % (x, top, x, bottom))
        lines.append('<text x="%.1f" y="%d" text-anchor="middle" class="tick">%s</text>' % (x, bottom + 14, label))

    # Axes
    lines.append('<line x1="%d" y1="%d" x2="%d" y2="%d" stroke="var(--fg3)" stroke-width="1"/>' % (left, top, left, bottom))
    lines.append('<line x1="%d" y1="%d" x2="%d" y2="%d" stroke="var(--fg3)" stroke-width="1"/>' % (left, bottom, right, bottom))

    # Pareto frontier path — only Pareto-optimal points in range, sorted by time
    pareto_in_range = [e for e in pareto if e in effort_range]
    pareto_in_range.sort(key=lambda e: agg[e][1])
    if len(pareto_in_range) >= 2:
        path_pts = [(x_pos(agg[e][1]), y_pos(agg[e][0])) for e in pareto_in_range]
        path_d = "M%.1f,%.1f" % path_pts[0] + "".join(" L%.1f,%.1f" % p for p in path_pts[1:])
        lines.append('<path d="%s" fill="none" stroke="var(--fg3)" stroke-width="1" stroke-opacity="0.3"/>' % path_d)

    # All dots in range (including non-Pareto)
    all_in_range = sorted(effort_range, key=lambda e: agg[e][1])

    for e in all_in_range:
        s, t = agg[e]
        cx, cy = x_pos(t), y_pos(s)
        c = color(e)
        r = "3.0" if e not in pareto else "3.5"
        lines.append('<circle cx="%.1f" cy="%.1f" r="%s" fill="%s" stroke="var(--bg)" stroke-width="1"/>' % (cx, cy, r, c))

    # Labels with leader lines — colored to match dots
    for e in all_in_range:
        if e not in label_offsets:
            continue
        s, t = agg[e]
        cx, cy = x_pos(t), y_pos(s)
        dx, dy, anchor = label_offsets[e]
        c = color(e)
        tx, ty = cx + dx, cy + dy
        # Leader line from dot edge toward label
        dist = (dx**2 + dy**2)**0.5
        if dist > 6:
            # Start line 4px from dot center, end 2px from text
            sx = cx + dx * 4.0 / dist
            sy = cy + dy * 4.0 / dist
            ex = tx - dx * 2.0 / dist
            ey = ty - dy * 2.0 / dist
            lines.append('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke="%s" stroke-width="0.6" opacity="0.5"/>' % (sx, sy, ex, ey, c))
        lines.append('<text x="%.1f" y="%.1f" text-anchor="%s" class="elabel" fill="%s">e%d</text>' % (tx, ty, anchor, c, e))
        if e in presets:
            pdy = dy - 10 if dy <= 0 else dy + 10
            lines.append('<text x="%.1f" y="%.1f" text-anchor="%s" class="preset" fill="%s">%s</text>' % (cx + dx, cy + pdy, anchor, c, presets[e]))

    # Legend
    ly = height - 16
    for lx, lc, lt in legend_items:
        lines.append('<circle cx="%d" cy="%d" r="3" fill="%s"/>' % (lx, ly, lc))
        lines.append('<text x="%d" y="%d" class="legend">%s</text>' % (lx + 6, ly + 3, lt))

    lines.append('</svg>')
    return '\n'.join(lines)


# === Chart 1: Fast range (e1-e17, linear X) ===
fast_efforts = list(range(1, 18))
fast_labels = {
    1: (10, -10, "start"),
    2: (-10, -12, "end"),
    7: (-10, 10, "end"),
    9: (10, -12, "start"),
    10: (10, 10, "start"),
    13: (10, -10, "start"),
    17: (12, 10, "start"),
}
fast_legend = [
    (80, "#4a90d9", "Turbo (1-4)"),
    (170, "#50b86c", "FastHt (5-9)"),
    (275, "#d4a020", "Greedy (10)"),
    (380, "#e07040", "Lazy (11-17)"),
]
fast_svg = gen_chart(
    title="Compression vs Encode Time — Fast Range",
    effort_range=fast_efforts,
    x_min_ms=0, x_max_ms=95,
    x_grid=[10, 20, 30, 40, 50, 60, 70, 80, 90],
    x_grid_labels=["10ms", "20ms", "30ms", "40ms", "50ms", "60ms", "70ms", "80ms", "90ms"],
    y_min=8_900_000, y_max=10_600_000,
    y_grid=[9_000_000, 9_500_000, 10_000_000, 10_500_000],
    y_grid_labels=["9.0M", "9.5M", "10.0M", "10.5M"],
    label_offsets=fast_labels,
    legend_items=fast_legend,
    log_x=False,
)

# === Chart 2: Detail range (e17-e31, log X) ===
slow_efforts = list(range(17, 32))  # include e17 as bridge, e31 as Brag
slow_labels = {
    17: (12, -10, "start"),
    18: (12, 10, "start"),
    19: (12, -12, "start"),
    22: (12, -12, "start"),
    24: (0, -14, "middle"),
    27: (12, 10, "start"),
    28: (12, -10, "start"),
    30: (12, 10, "start"),
    31: (12, -12, "start"),
}
slow_legend = [
    (80, "#e07040", "Lazy (11-17)"),
    (190, "#c050c0", "Lazy2 (18-22)"),
    (320, "#7070e0", "NearOpt (23-30)"),
    (470, "#d04040", "FullOpt (31+)"),
]
slow_svg = gen_chart(
    title="Compression vs Encode Time — Detail Range",
    effort_range=slow_efforts,
    x_min_ms=50, x_max_ms=8000,
    x_grid=[100, 200, 300, 500, 1000, 2000, 3000, 5000],
    x_grid_labels=["100ms", "200ms", "300ms", "500ms", "1s", "2s", "3s", "5s"],
    y_min=8_280_000, y_max=9_000_000,
    y_grid=[8_300_000, 8_400_000, 8_500_000, 8_600_000, 8_700_000, 8_800_000, 8_900_000],
    y_grid_labels=["8.3M", "8.4M", "8.5M", "8.6M", "8.7M", "8.8M", "8.9M"],
    label_offsets=slow_labels,
    legend_items=slow_legend,
    log_x=True,
)

with open(os.path.join(_project_dir, 'effort_curve_fast.svg'), 'w') as f:
    f.write(fast_svg)
print("Wrote effort_curve_fast.svg")

with open(os.path.join(_project_dir, 'effort_curve_detail.svg'), 'w') as f:
    f.write(slow_svg)
print("Wrote effort_curve_detail.svg")

# Print Pareto info for verification
print("\nPareto-optimal efforts:", pareto)
print("Non-Pareto (dominated):", [e for e in efforts if e not in pareto])
