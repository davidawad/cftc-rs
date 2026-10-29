#!/usr/bin/env python3
"""Format `cftc cds daily` CSV (stdin) as a fixed-width table."""
import csv
import sys

num = lambda v: "-" if v == "" else f"{float(v):.1f}"
print(f"{'date':<10}  {'trades':>6}  {'off-scale':>9}  {'median bp':>9}  {'low':>6}  {'high':>6}  {'notional $mm':>13}")
for r in csv.DictReader(sys.stdin):
    capped = "+" if int(r["capped_trades"]) else " "
    print(f"{r['as_of']:<10}  {r['trades']:>6}  {r['off_scale_trades']:>9}  {num(r['level_median']):>9}  "
          f"{num(r['level_min']):>6}  {num(r['level_max']):>6}  {float(r['notional_lower_bound']) / 1e6:>12,.0f}{capped}")
