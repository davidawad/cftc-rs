#!/usr/bin/env python3
"""Write a synthetic CFTC credit cache (one CDX.NA.IG week) for the showcase.

Every value is invented. Only the column names come from the public DTCC/CFTC
report layout (tests/fixtures/header.csv). Usage: make-sample.py CACHE_DIR
"""
import csv
import io
import random
import sys
import zipfile
from datetime import date, timedelta
from pathlib import Path

root = Path(__file__).resolve().parents[2]
cache = Path(sys.argv[1]) / "cftc" / "credits"
cache.mkdir(parents=True, exist_ok=True)
header = next(csv.reader(open(root / "tests" / "fixtures" / "header.csv")))
rng = random.Random(7)
start = date(2026, 3, 2)
mids = [54.1, 55.0, 57.6, 56.2, 58.9]

for i, mid in enumerate(mids):
    day = start + timedelta(days=i)
    out = io.StringIO()
    w = csv.DictWriter(out, fieldnames=header)
    w.writeheader()
    for n in range(rng.randint(9, 14)):
        bp = mid + rng.gauss(0, 0.6)
        # Day 3 carries one print reported in percent instead of decimal (100x off).
        level = bp / 100 if (i == 2 and n == 4) else bp / 1e4
        w.writerow({
            "Dissemination Identifier": f"SAMPLE-{i}{n:02d}",
            "Action type": "NEWT",
            "Event type": "TRAD",
            "Asset Class": "CR",
            "Execution Timestamp": f"{day}T{13 + n // 4:02d}:{(n * 13) % 60:02d}:00Z",
            "Effective Date": str(day),
            "Expiration Date": "2030-12-20",
            "Notional amount-Leg 1": "110,000,000+" if n % 5 == 0 else f"{rng.choice([10, 25, 50, 75])},000,000",
            "Notional currency-Leg 1": "USD",
            "Fixed rate-Leg 1": "0.01",
            "Spread-Leg 1": f"{level:.6f}",
            "Spread notation-Leg 1": "3",
            "UPI FISN": "NA/CDS Corp Idx",
            "UPI Underlier Name": "CDX.NA.IG",
            "Index factor": "1",
            "Platform identifier": "SAMPLE",
        })
    stem = f"CFTC_CUMULATIVE_CREDITS_{day:%Y_%m_%d}"
    with zipfile.ZipFile(cache / f"{stem}.zip", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(f"{stem}.csv", out.getvalue())
print(cache)
