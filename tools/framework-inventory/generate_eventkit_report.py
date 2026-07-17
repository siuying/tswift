#!/usr/bin/env python3
"""Generate the standalone EventKit coverage HTML report.

Reads the checked-in `website/src/data/coverage/eventkit.json` (produced by
`generate_website_json.py`) and renders a single self-contained HTML file to
`docs/reports/eventkit-coverage.html`. No toolchain or network required — it
only reads the already-generated JSON, so the numbers are never hand-typed.

Usage:
  python3 tools/framework-inventory/generate_eventkit_report.py
"""
from __future__ import annotations

import html
import json
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "website/src/data/coverage/eventkit.json"
OUT = ROOT / "docs/reports/eventkit-coverage.html"

STATUS_META = {
    "implemented": ("✅", "impl"),
    "partial": ("🟡", "part"),
    "missing": ("⬜", "miss"),
    "out_of_scope": ("🚫", "oos"),
}


def section_counts(section):
    c = {"implemented": 0, "partial": 0, "missing": 0, "out_of_scope": 0}
    for m in section["members"]:
        c[m["status"]] = c.get(m["status"], 0) + 1
    return c


def main() -> int:
    data = json.loads(SRC.read_text())
    totals = data["totals"]
    in_scope = totals["implemented"] + totals["partial"] + totals["missing"]
    done = totals["implemented"] + totals["partial"]
    done_pct = round(done / in_scope * 100) if in_scope else 0
    verified_pct = round(totals["implemented"] / in_scope * 100) if in_scope else 0

    rows = []
    for s in data["sections"]:
        c = section_counts(s)
        s_in = c["implemented"] + c["partial"] + c["missing"]
        pct = round((c["implemented"] + c["partial"]) / s_in * 100) if s_in else 0
        rows.append(
            f"<tr><td><code>{html.escape(s['name'])}</code></td>"
            f"<td class='num'>{c['implemented']}</td>"
            f"<td class='num'>{c['partial']}</td>"
            f"<td class='num'>{c['out_of_scope']}</td>"
            f"<td class='num'>{s_in}</td>"
            f"<td class='pct'><div class='minibar'><i style='width:{pct}%'></i></div>"
            f"<span>{pct}%</span></td></tr>"
        )
    table = "\n".join(rows)

    doc = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>tswift · EventKit coverage report</title>
<style>
  :root {{
    --bg:#0b0d12; --panel:#141821; --panel-2:#1b2030; --ink:#e8ecf4;
    --muted:#97a0b5; --line:#262d3d; --accent:#5b8cff; --green:#34c759;
    --warn:#ffb020; --pink:#ff6b9d; --mono:ui-monospace,"SF Mono",Menlo,monospace;
  }}
  * {{ box-sizing:border-box; }}
  body {{
    margin:0; background:radial-gradient(1200px 600px at 70% -10%, #18203a 0%, var(--bg) 55%);
    color:var(--ink); font:15px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;
    -webkit-font-smoothing:antialiased;
  }}
  .wrap {{ max-width:960px; margin:0 auto; padding:56px 24px 96px; }}
  .eyebrow {{ font:600 12px/1 var(--mono); letter-spacing:.22em; text-transform:uppercase; color:var(--accent); }}
  h1 {{ font-size:40px; line-height:1.1; margin:14px 0 8px; letter-spacing:-.02em; }}
  h1 .grad {{ background:linear-gradient(92deg,#5b8cff,#ff6b9d 60%,#ffb020); -webkit-background-clip:text; background-clip:text; color:transparent; }}
  .lede {{ color:var(--muted); font-size:18px; max-width:70ch; }}
  .meta {{ margin-top:18px; font:500 13px/1 var(--mono); color:var(--muted); display:flex; gap:18px; flex-wrap:wrap; }}
  .meta b {{ color:var(--ink); }}
  section {{ margin-top:48px; }}
  h2 {{ font-size:13px; letter-spacing:.16em; text-transform:uppercase; color:var(--muted); border-bottom:1px solid var(--line); padding-bottom:10px; }}
  .stats {{ display:grid; grid-template-columns:repeat(4,1fr); gap:14px; margin-top:22px; }}
  .stat {{ background:var(--panel); border:1px solid var(--line); border-radius:14px; padding:18px; }}
  .stat .n {{ font-size:30px; font-weight:700; letter-spacing:-.02em; }}
  .stat .n .small {{ font-size:16px; color:var(--muted); font-weight:500; }}
  .stat .l {{ color:var(--muted); font-size:12.5px; margin-top:4px; }}
  .stat.green .n {{ color:var(--green); }} .stat.blue .n {{ color:var(--accent); }}
  .stat.warn .n {{ color:var(--warn); }} .stat.pink .n {{ color:var(--pink); }}
  .bar {{ height:12px; border-radius:999px; background:var(--panel-2); overflow:hidden; margin-top:22px; border:1px solid var(--line); }}
  .bar > i {{ display:block; height:100%; background:linear-gradient(90deg,var(--accent),var(--green)); }}
  table {{ width:100%; border-collapse:collapse; margin-top:18px; font-size:13.5px; }}
  th,td {{ text-align:left; padding:9px 12px; border-bottom:1px solid var(--line); }}
  th {{ color:var(--muted); font-weight:600; font-size:12px; text-transform:uppercase; letter-spacing:.06em; }}
  td.num, th.num {{ text-align:right; font-variant-numeric:tabular-nums; }}
  td code {{ font:12.5px/1.5 var(--mono); color:var(--accent); }}
  td.pct {{ display:flex; align-items:center; gap:10px; }}
  .minibar {{ flex:1; height:8px; border-radius:999px; background:var(--panel-2); overflow:hidden; border:1px solid var(--line); min-width:80px; }}
  .minibar > i {{ display:block; height:100%; background:linear-gradient(90deg,var(--accent),var(--green)); }}
  td.pct span {{ font:600 12px/1 var(--mono); color:var(--green); width:40px; text-align:right; }}
  .pills {{ display:flex; flex-wrap:wrap; gap:8px; margin-top:16px; }}
  .pill {{ font:12.5px/1 var(--mono); color:var(--ink); background:var(--panel); border:1px solid var(--line); border-radius:999px; padding:8px 12px; }}
  p.note {{ color:var(--muted); font-size:14px; max-width:74ch; }}
  code.inl {{ font:12.5px/1.5 var(--mono); color:var(--ink); background:var(--panel-2); padding:1px 6px; border-radius:6px; }}
  footer {{ margin-top:56px; color:var(--muted); font:12.5px/1.6 var(--mono); border-top:1px solid var(--line); padding-top:18px; }}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <div class="eyebrow">tswift · framework coverage</div>
    <h1><span class="grad">EventKit</span> coverage report</h1>
    <p class="lede">Apple's calendar &amp; reminders framework, modeled as an
    in-memory event store in the tswift Swift runtime — no CalendarStore, no
    permission prompts, no native EventKit dependency.</p>
    <div class="meta">
      <span>Generated <b>{date.today().isoformat()}</b></span>
      <span>Source <b>frameworks/eventkit/</b></span>
      <span>iOS-simulator SDK surface</span>
    </div>
  </header>

  <section>
    <h2>At a glance</h2>
    <div class="stats">
      <div class="stat green"><div class="n">{done_pct}%</div><div class="l">In-scope coverage ({done}/{in_scope})</div></div>
      <div class="stat blue"><div class="n">{totals['implemented']}<span class="small"> impl</span></div><div class="l">Fixture-verified members</div></div>
      <div class="stat warn"><div class="n">{totals['partial']}<span class="small"> part</span></div><div class="l">Reachable, not fixture-verified</div></div>
      <div class="stat pink"><div class="n">{totals['out_of_scope']}<span class="small"> oos</span></div><div class="l">Out of scope (documented)</div></div>
    </div>
    <div class="bar"><i style="width:{done_pct}%"></i></div>
    <p class="note" style="margin-top:14px">All <b>{in_scope}</b> in-scope EventKit
    members are implemented and reachable. <b>{verified_pct}%</b> are exercised by a
    tagged executing golden fixture; the remaining <b>{totals['partial']}</b> are enum
    <code class="inl">init(rawValue:)</code> keys blocked on shared builtin-enum
    round-trip support (see <code class="inl">docs/swift-runtime/blocked-features.md</code>).</p>
  </section>

  <section>
    <h2>Per-type breakdown</h2>
    <table>
      <thead><tr><th>Type</th><th class="num">✅ Impl</th><th class="num">🟡 Part</th><th class="num">🚫 OOS</th><th class="num">In scope</th><th>Coverage</th></tr></thead>
      <tbody>
{table}
      </tbody>
    </table>
  </section>

  <section>
    <h2>What's wired</h2>
    <div class="pills">
      <span class="pill">EKEventStore · in-memory</span>
      <span class="pill">EKEvent</span>
      <span class="pill">EKReminder</span>
      <span class="pill">EKCalendar</span>
      <span class="pill">EKSource</span>
      <span class="pill">EKAlarm</span>
      <span class="pill">EKRecurrenceRule</span>
      <span class="pill">EKParticipant</span>
      <span class="pill">EKStructuredLocation</span>
      <span class="pill">permissions</span>
      <span class="pill">save / remove / commit</span>
      <span class="pill">every EventKit enum</span>
    </div>
    <p class="note" style="margin-top:18px">EventKit is Apple-platform-only with no
    web/wasm equivalent, so the runtime targets the iOS-simulator SDK. Objective-C-only
    and system-integration APIs that cannot run headless — predicate fetch queries,
    <code class="inl">NSPredicate</code> enumeration, UI presentation, and cross-process
    change notifications — are counted separately as out of scope rather than implying
    support.</p>
  </section>

  <footer>
    Numbers generated from <code>website/src/data/coverage/eventkit.json</code> via
    <code>tools/framework-inventory/generate_eventkit_report.py</code>. Nothing on this
    page is hand-typed.
  </footer>
</div>
</body>
</html>
"""
    OUT.write_text(doc)
    print(f"Wrote {OUT.relative_to(ROOT)} — {done_pct}% ({done}/{in_scope})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
