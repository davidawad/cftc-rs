#!/usr/bin/env bash
# Reproduce docs/showcase/cds-daily.{txt,png} from synthetic sample data.
# Needs cargo and python3; the PNG step needs charm `freeze` (skipped if absent).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cache=$(mktemp -d)
trap 'rm -rf "$cache"' EXIT
python3 "$here/make-sample.py" "$cache" > /dev/null
cd "$root"
cargo build -q --features cli
cmd='cftc --offline cds daily --regulator cftc --family CDX.NA.IG --from 2026-03-02 --to 2026-03-06'
{
  echo "\$ $cmd | docs/showcase/table.py"
  CFTC_RS_CACHE="$cache" target/debug/$cmd 2> /dev/null | python3 "$here/table.py"
} > "$here/cds-daily.txt"
cat "$here/cds-daily.txt"
if command -v freeze > /dev/null; then
  freeze "$here/cds-daily.txt" --language text --window --padding 20 --font.size 14 \
    --output "$here/cds-daily.png"
fi
