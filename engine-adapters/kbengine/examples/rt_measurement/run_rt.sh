#!/bin/sh
# Runtime measurement (WM-0010, RT1/RT4) on a real KBEngine cluster.
#
# Starts (or reuses) the cluster and a bots process running the production
# Bridge with the measurement harness, drives a small login swell plus a
# command flow for ~15 s, and prints both measurement reports:
#   - harness JSON: pump fire intervals (RT1) + queue handoff latency (RT4)
#   - driver:       callback-lag series from the heartbeat stream
#
# Prerequisites: same as examples/login_load (built workspace, DB-backed
# cluster) — see that README.
#
# Usage: sh run_rt.sh
set -e

SAMPLE_DIR=$(cd "$(dirname "$0")" && pwd)
ADAPTER_DIR=$(cd "$SAMPLE_DIR/../.." && pwd)         # engine-adapters/kbengine

WAVEMAKER_KBE_ROOT=${WAVEMAKER_KBE_ROOT:-/home/cui/workspaces/kbengine-build}
WAVEMAKER_KBE_ASSETS=${WAVEMAKER_KBE_ASSETS:-$WAVEMAKER_KBE_ROOT/assets}

BOTS_SCRIPTS="$WAVEMAKER_KBE_ASSETS/scripts/bots"
RUN_DIR=$(mktemp -d /tmp/wm-rt.XXXXXX)
SOCKET_PATH="$RUN_DIR/bridge.sock"
REPORT_PATH="$RUN_DIR/rt_report.json"
LOG_DIR="$RUN_DIR/logs"
mkdir -p "$LOG_DIR"

export KBE_ROOT="$WAVEMAKER_KBE_ROOT"
export KBE_RES_PATH="$KBE_ROOT/kbe/res/:$WAVEMAKER_KBE_ASSETS:$WAVEMAKER_KBE_ASSETS/res:$WAVEMAKER_KBE_ASSETS/scripts/"
export KBE_BIN_PATH="$KBE_ROOT/kbe/bin/server/"

if ! pgrep -x machine >/dev/null || ! pgrep -x loginapp >/dev/null; then
    echo "== starting cluster"
    (
    cd "$WAVEMAKER_KBE_ASSETS"
    for app in machine logger interfaces dbmgr baseappmgr cellappmgr \
               baseapp cellapp loginapp; do
        nohup "$KBE_BIN_PATH/$app" >/dev/null 2>&1 &
        sleep 3
    done
    )
    sleep 8
fi
if ! pgrep -x dbmgr >/dev/null; then
    echo "dbmgr is not running: check kbengine.xml databaseInterfaces"
    exit 1
fi

if [ ! -f "$BOTS_SCRIPTS/kbemain.py.stock" ]; then
    cp "$BOTS_SCRIPTS/kbemain.py" "$BOTS_SCRIPTS/kbemain.py.stock"
fi
# The harness IS the entry module: it boots the production Bridge with
# the measured queue injected and the pump wrapped (real engine, real
# callback path).
cp "$SAMPLE_DIR/harness.py" "$BOTS_SCRIPTS/kbemain.py"
rm -rf "$BOTS_SCRIPTS/bridge"
cp -r "$ADAPTER_DIR/bridge" "$BOTS_SCRIPTS/bridge"

echo "== starting bots with the measurement harness"
(
    cd "$WAVEMAKER_KBE_ASSETS"
    WAVEMAKER_BRIDGE_IPC="unix:$SOCKET_PATH" \
    WAVEMAKER_BRIDGE_TOKEN=tok \
    WAVEMAKER_SWELL_ID=sw-rt \
    WAVEMAKER_BACKEND_ID=kb-rt \
    WAVEMAKER_RT_REPORT="$REPORT_PATH" \
    nohup "$KBE_BIN_PATH/bots" >"$LOG_DIR/bots.log" 2>&1 &
)

echo "== measuring"
timeout 90 python3 "$SAMPLE_DIR/driver.py" "$SOCKET_PATH"

echo "== stopping the bots process"
pkill -x bots || true

echo "== report: $REPORT_PATH"
cat "$REPORT_PATH"
