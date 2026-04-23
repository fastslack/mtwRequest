#!/usr/bin/env python3
"""Reads bench-runner JSON results from a directory and writes a terminal-friendly
markdown summary. Uses only the Python standard library — no matplotlib needed.

Usage: python3 summary.py results/<timestamp>/
"""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path

SYSTEM_ORDER = ["mtw", "centrifugo", "socketio", "nats"]


def fmt_ns(ns: int) -> str:
    if ns == 0:
        return "—"
    if ns < 1_000:
        return f"{ns} ns"
    if ns < 1_000_000:
        return f"{ns / 1_000:.1f} µs"
    if ns < 1_000_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    return f"{ns / 1_000_000_000:.2f} s"


def fmt_rate(x: float) -> str:
    if x >= 1e6:
        return f"{x / 1e6:.2f}M/s"
    if x >= 1e3:
        return f"{x / 1e3:.1f}k/s"
    return f"{x:.0f}/s"


def sort_key(sys_name: str) -> int:
    return SYSTEM_ORDER.index(sys_name) if sys_name in SYSTEM_ORDER else 99


def render(results_dir: Path) -> str:
    records = []
    for p in sorted(results_dir.glob("*.json")):
        try:
            records.append(json.loads(p.read_text()))
        except Exception as e:
            print(f"skip {p}: {e}", file=sys.stderr)
    if not records:
        return "# bench-suite — no results found\n"

    out = [f"# bench-suite results — `{results_dir.name}`\n"]

    by_scenario: dict[str, list[dict]] = defaultdict(list)
    for r in records:
        by_scenario[r["scenario"]].append(r)

    # --------- fanout ---------
    fanout = by_scenario.get("fanout", [])
    if fanout:
        # group by subs tier
        by_subs: dict[int, dict[str, dict]] = defaultdict(dict)
        for r in fanout:
            subs = int(r["params"].get("subs", 0))
            by_subs[subs][r["system"]] = r

        out.append("## Fanout  (1 publisher → N subscribers)\n")
        out.append("One-way latency from publish stamp to subscriber receive.\n\n")
        for subs in sorted(by_subs.keys()):
            out.append(f"### {subs} subscribers\n")
            out.append("| system | p50 | p90 | p99 | deliveries | throughput | errors |")
            out.append("|---|---:|---:|---:|---:|---:|---:|")
            for sys_name in sorted(by_subs[subs].keys(), key=sort_key):
                r = by_subs[subs][sys_name]
                lat = r.get("latency") or {}
                out.append(
                    f"| **{sys_name}** "
                    f"| {fmt_ns(lat.get('p50_ns', 0))} "
                    f"| {fmt_ns(lat.get('p90_ns', 0))} "
                    f"| {fmt_ns(lat.get('p99_ns', 0))} "
                    f"| {r.get('messages_received', 0):,} "
                    f"| {fmt_rate(r.get('throughput_msgs_per_sec', 0))} "
                    f"| {r.get('errors', 0)} |"
                )
            out.append("")

    # --------- echo ---------
    echo = by_scenario.get("echo", [])
    if echo:
        out.append("## Echo  (round-trip via echo-bot)\n")
        out.append("RTT from requester → bot → requester.\n\n")
        out.append("| system | p50 | p90 | p99 | round-trips | rate | errors |")
        out.append("|---|---:|---:|---:|---:|---:|---:|")
        for r in sorted(echo, key=lambda x: sort_key(x["system"])):
            lat = r.get("latency") or {}
            out.append(
                f"| **{r['system']}** "
                f"| {fmt_ns(lat.get('p50_ns', 0))} "
                f"| {fmt_ns(lat.get('p90_ns', 0))} "
                f"| {fmt_ns(lat.get('p99_ns', 0))} "
                f"| {r.get('messages_received', 0):,} "
                f"| {fmt_rate(r.get('throughput_msgs_per_sec', 0))} "
                f"| {r.get('errors', 0)} |"
            )
        out.append("")

    # --------- connect ---------
    conn = by_scenario.get("connect", [])
    if conn:
        out.append("## Connect storm\n")
        out.append("Concurrent connection opens.\n\n")
        out.append("| system | count | p50 connect | p99 connect | connects/s | errors |")
        out.append("|---|---:|---:|---:|---:|---:|")
        for r in sorted(conn, key=lambda x: sort_key(x["system"])):
            lat = r.get("latency") or {}
            out.append(
                f"| **{r['system']}** "
                f"| {r['params'].get('count', 0):,} "
                f"| {fmt_ns(lat.get('p50_ns', 0))} "
                f"| {fmt_ns(lat.get('p99_ns', 0))} "
                f"| {fmt_rate(r.get('throughput_msgs_per_sec', 0))} "
                f"| {r.get('errors', 0)} |"
            )
        out.append("")

    return "\n".join(out) + "\n"


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 1
    results_dir = Path(sys.argv[1])
    if not results_dir.is_dir():
        print(f"not a directory: {results_dir}", file=sys.stderr)
        return 1
    md = render(results_dir)
    out_file = results_dir / "summary.md"
    out_file.write_text(md)
    print(md)
    print(f"[summary.py] wrote {out_file}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
