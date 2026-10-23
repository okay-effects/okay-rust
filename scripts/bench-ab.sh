#!/usr/bin/env bash
# Alternating A/B of two bench binaries: rounds × (A then B), each run under JMH's lock; the median per lane.
# usage: scripts/bench-ab.sh <binA> <binB> <rounds> [lanes...]   (the binaries built before, copied aside)
set -euo pipefail
a=$1; b=$2; rounds=$3; shift 3
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d)
for r in $(seq 1 "$rounds"); do
  for arm in A B; do
    bin=$a; [ $arm = B ] && bin=$b
    load=$(uptime | sed 's/.*load averages*: *//' | awk '{print $1}')
    "$here/withlock.py" "$bin" "$@" | awk -v arm=$arm -v load="$load" '/us\/op/ {print arm, $1, $2, load}' >> "$out/runs"
    echo "round $r $arm done (load $load)" >&2
  done
done
python3 - "$out/runs" <<'PY'
import sys, statistics as st
rows = [l.split() for l in open(sys.argv[1])]
lanes = list(dict.fromkeys(r[1] for r in rows))
med = lambda arm, lane: st.median(float(r[2]) for r in rows if r[0] == arm and r[1] == lane)
loads = [float(r[3]) for r in rows]
print(f"# median of {len(rows)//max(1,2*len(lanes))} rounds; host load {min(loads):.1f}-{max(loads):.1f}")
print(f"{'lane':14} {'A':>10} {'B':>10} {'B/A':>7}")
for l in lanes:
    x, y = med("A", l), med("B", l)
    print(f"{l:14} {x:10.3f} {y:10.3f} {y/x:7.3f}")
PY
echo "raw: $out/runs" >&2
