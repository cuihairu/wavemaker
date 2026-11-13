#!/usr/bin/env bash
# Run all three demo test plans against the stub backend.
#
# The stub backend is an in-process simulator (metrics are synthetic and
# every report labels them so); `--time-scale 0` compresses the real sleeps
# so the whole demo finishes in seconds while simulated durations stay
# full. Reports land in wm-runs/<plan>-<timestamp>/report.md.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -x target/debug/wm ]; then
    echo "building wm-cli..."
    cargo build -p wm-cli
fi

status=0
for plan in smoke baseline soak; do
    echo "=== ${plan} ==="
    target/debug/wm test run "examples/plans/${plan}.toml" \
        --backend stub --time-scale 0 || status=1
done

echo
if [ "$status" -eq 0 ]; then
    echo "all demo plans passed (reports under wm-runs/)"
else
    echo "a demo plan failed — see the report paths above" >&2
fi
exit "$status"
