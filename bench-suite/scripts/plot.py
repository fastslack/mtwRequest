#!/usr/bin/env python3
"""Plot bench-suite results into comparative PNG + SVG charts.

Reads every `*.json` file in a directory (produced by `bench-runner`) and
emits three figures:
    - <out>/fanout.png / .svg   — grouped bar of p50 / p99 one-way latency
                                   plus a companion throughput bar.
    - <out>/echo.png / .svg     — grouped bar of p50 / p99 round-trip latency.
    - <out>/connect.png / .svg  — connects/sec plus p99 connect latency.

Usage:
    python3 plot.py results/<timestamp>/
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from collections import defaultdict

import matplotlib.pyplot as plt
import matplotlib.ticker as mticker

SYSTEM_ORDER = ["mtw", "centrifugo", "socketio", "nats"]
SYSTEM_COLORS = {
    "mtw": "#c44545",
    "centrifugo": "#4c8bf5",
    "socketio": "#2ca02c",
    "nats": "#8c6dcf",
}


def load(results_dir: Path) -> list[dict]:
    records = []
    for p in sorted(results_dir.glob("*.json")):
        try:
            records.append(json.loads(p.read_text()))
        except Exception as e:
            print(f"skip {p}: {e}", file=sys.stderr)
    return records


def _group_by_scenario(records: list[dict]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = defaultdict(list)
    for r in records:
        out[r["scenario"]].append(r)
    return out


def _sort_systems(records: list[dict]) -> list[str]:
    present = {r["system"] for r in records}
    return [s for s in SYSTEM_ORDER if s in present] + sorted(present - set(SYSTEM_ORDER))


def plot_fanout(records: list[dict], out: Path) -> None:
    if not records:
        return
    # Group by (system, subs) so we can show a few subs tiers side by side.
    by_subs: dict[int, dict[str, dict]] = defaultdict(dict)
    for r in records:
        subs = int(r["params"].get("subs", 0))
        by_subs[subs][r["system"]] = r

    subs_tiers = sorted(by_subs.keys())
    systems = _sort_systems(records)

    fig, (ax_lat, ax_tp) = plt.subplots(1, 2, figsize=(14, 5))
    fig.suptitle("Fanout: 1 publisher → N subscribers", fontsize=14, weight="bold")

    width = 0.8 / max(1, len(systems))
    x = list(range(len(subs_tiers)))

    # Latency: p99 bars with p50 overlay.
    for i, sys_name in enumerate(systems):
        p99 = [by_subs[s].get(sys_name, {}).get("latency", {}).get("p99_ns", 0) / 1e6
               for s in subs_tiers]
        p50 = [by_subs[s].get(sys_name, {}).get("latency", {}).get("p50_ns", 0) / 1e6
               for s in subs_tiers]
        offset = (i - (len(systems) - 1) / 2) * width
        positions = [xi + offset for xi in x]
        ax_lat.bar(positions, p99, width=width, color=SYSTEM_COLORS.get(sys_name, "#888"),
                   label=f"{sys_name} p99", alpha=0.95, edgecolor="black", linewidth=0.4)
        ax_lat.bar(positions, p50, width=width, color=SYSTEM_COLORS.get(sys_name, "#888"),
                   alpha=0.55, edgecolor="black", linewidth=0.4)

    ax_lat.set_xticks(x)
    ax_lat.set_xticklabels([f"{s} subs" for s in subs_tiers])
    ax_lat.set_ylabel("one-way latency (ms, log)")
    ax_lat.set_yscale("log")
    ax_lat.yaxis.set_major_formatter(mticker.FormatStrFormatter("%g"))
    ax_lat.grid(True, axis="y", alpha=0.3)
    ax_lat.legend(fontsize=8, ncol=2)
    ax_lat.set_title("Latency (solid = p99, faded = p50)")

    # Throughput.
    for i, sys_name in enumerate(systems):
        tp = [by_subs[s].get(sys_name, {}).get("throughput_msgs_per_sec", 0) for s in subs_tiers]
        offset = (i - (len(systems) - 1) / 2) * width
        positions = [xi + offset for xi in x]
        ax_tp.bar(positions, tp, width=width,
                  color=SYSTEM_COLORS.get(sys_name, "#888"),
                  label=sys_name, edgecolor="black", linewidth=0.4)

    ax_tp.set_xticks(x)
    ax_tp.set_xticklabels([f"{s} subs" for s in subs_tiers])
    ax_tp.set_ylabel("deliveries / sec")
    ax_tp.yaxis.set_major_formatter(mticker.FuncFormatter(lambda v, _: f"{v/1e6:.1f}M" if v >= 1e6 else f"{v/1e3:.0f}k" if v >= 1e3 else f"{v:.0f}"))
    ax_tp.grid(True, axis="y", alpha=0.3)
    ax_tp.legend(fontsize=8)
    ax_tp.set_title("Throughput")

    fig.tight_layout()
    for ext in ("png", "svg"):
        fig.savefig(out / f"fanout.{ext}", dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"wrote {out}/fanout.png + .svg")


def plot_echo(records: list[dict], out: Path) -> None:
    if not records:
        return
    systems = _sort_systems(records)
    by_sys = {r["system"]: r for r in records}

    fig, ax = plt.subplots(figsize=(9, 5))
    fig.suptitle("Echo RTT: publisher → echo-bot → publisher",
                 fontsize=14, weight="bold")

    p50 = [by_sys.get(s, {}).get("latency", {}).get("p50_ns", 0) / 1e6 for s in systems]
    p99 = [by_sys.get(s, {}).get("latency", {}).get("p99_ns", 0) / 1e6 for s in systems]

    x = list(range(len(systems)))
    width = 0.35
    ax.bar([xi - width/2 for xi in x], p50, width, label="p50",
           color=[SYSTEM_COLORS.get(s, "#888") for s in systems], alpha=0.6,
           edgecolor="black", linewidth=0.4)
    ax.bar([xi + width/2 for xi in x], p99, width, label="p99",
           color=[SYSTEM_COLORS.get(s, "#888") for s in systems],
           edgecolor="black", linewidth=0.4)
    ax.set_xticks(x)
    ax.set_xticklabels(systems)
    ax.set_ylabel("RTT (ms)")
    ax.legend()
    ax.grid(True, axis="y", alpha=0.3)

    fig.tight_layout()
    for ext in ("png", "svg"):
        fig.savefig(out / f"echo.{ext}", dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"wrote {out}/echo.png + .svg")


def plot_connect(records: list[dict], out: Path) -> None:
    if not records:
        return
    systems = _sort_systems(records)
    # Aggregate across count tiers (pick the largest run per system).
    by_sys: dict[str, dict] = {}
    for r in records:
        cur = by_sys.get(r["system"])
        if cur is None or int(r["params"].get("count", 0)) > int(cur["params"].get("count", 0)):
            by_sys[r["system"]] = r

    fig, (ax_tp, ax_lat) = plt.subplots(1, 2, figsize=(12, 5))
    fig.suptitle("Connection storm", fontsize=14, weight="bold")

    cps = [by_sys.get(s, {}).get("throughput_msgs_per_sec", 0) for s in systems]
    p99 = [by_sys.get(s, {}).get("latency", {}).get("p99_ns", 0) / 1e6 for s in systems]
    colors = [SYSTEM_COLORS.get(s, "#888") for s in systems]

    ax_tp.bar(systems, cps, color=colors, edgecolor="black", linewidth=0.4)
    ax_tp.set_ylabel("connects / sec")
    ax_tp.grid(True, axis="y", alpha=0.3)
    ax_tp.set_title("Throughput")

    ax_lat.bar(systems, p99, color=colors, edgecolor="black", linewidth=0.4)
    ax_lat.set_ylabel("p99 connect latency (ms)")
    ax_lat.grid(True, axis="y", alpha=0.3)
    ax_lat.set_title("Latency")

    fig.tight_layout()
    for ext in ("png", "svg"):
        fig.savefig(out / f"connect.{ext}", dpi=150, bbox_inches="tight")
    plt.close(fig)
    print(f"wrote {out}/connect.png + .svg")


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 1
    results_dir = Path(sys.argv[1])
    if not results_dir.is_dir():
        print(f"not a directory: {results_dir}", file=sys.stderr)
        return 1

    records = load(results_dir)
    if not records:
        print("no results JSON found", file=sys.stderr)
        return 1

    grouped = _group_by_scenario(records)
    plot_fanout(grouped.get("fanout", []), results_dir)
    plot_echo(grouped.get("echo", []), results_dir)
    plot_connect(grouped.get("connect", []), results_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
