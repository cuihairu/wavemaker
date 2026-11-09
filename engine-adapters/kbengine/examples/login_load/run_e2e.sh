#!/bin/sh
# End-to-end login load test sample (WM-0217).
#
# Starts a real KBEngine cluster and a real bots process running the
# wavemaker Bridge, then drives one login swell (plan.json) through the
# Bridge protocol and prints the observed connection events.
#
# Prerequisites (see README.md):
#   - KBEngine built at $WAVEMAKER_KBE_ROOT (binaries in kbe/bin/server)
#   - python_assets tree at $WAVEMAKER_KBE_ASSETS (the engine's demo assets)
#   - the cluster's MySQL reachable (dbmgr account storage)
#
# Usage: sh run_e2e.sh
set -e

SAMPLE_DIR=$(cd "$(dirname "$0")" && pwd)
ADAPTER_DIR=$(cd "$SAMPLE_DIR/../.." && pwd)         # engine-adapters/kbengine
REPO_ROOT=$(cd "$ADAPTER_DIR/../.." && pwd)

WAVEMAKER_KBE_ROOT=${WAVEMAKER_KBE_ROOT:-/home/cui/workspaces/kbengine-build}
WAVEMAKER_KBE_ASSETS=${WAVEMAKER_KBE_ASSETS:-$WAVEMAKER_KBE_ROOT/assets}
SWELL_ID=${WAVEMAKER_SWELL_ID:-sw-login-sample}
BACKEND_ID=${WAVEMAKER_BACKEND_ID:-kb-sample-1}

BOTS_SCRIPTS="$WAVEMAKER_KBE_ASSETS/scripts/bots"
RUN_DIR=$(mktemp -d /tmp/wm-login-sample.XXXXXX)
SOCKET_PATH="$RUN_DIR/bridge.sock"
LOG_DIR="$RUN_DIR/logs"
mkdir -p "$LOG_DIR"

echo "== KBEngine workspace: $WAVEMAKER_KBE_ROOT"
export KBE_ROOT="$WAVEMAKER_KBE_ROOT"
export KBE_RES_PATH="$KBE_ROOT/kbe/res/:$WAVEMAKER_KBE_ASSETS:$WAVEMAKER_KBE_ASSETS/res:$WAVEMAKER_KBE_ASSETS/scripts/"
export KBE_BIN_PATH="$KBE_ROOT/kbe/bin/server/"

cluster_running() {
    pgrep -x machine >/dev/null && pgrep -x loginapp >/dev/null
}

if cluster_running; then
    echo "== cluster already running"
else
    echo "== starting cluster (documented order; gaps let each app register)"
    (
    cd "$WAVEMAKER_KBE_ASSETS"
    for app in machine logger interfaces dbmgr baseappmgr cellappmgr \
               baseapp cellapp loginapp; do
        nohup "$KBE_BIN_PATH/$app" >/dev/null 2>&1 &
        sleep 3
    done
    )
    sleep 8
    cluster_running || { echo "cluster failed to start"; exit 1; }
fi

# dbmgr exits if the database backend is unreachable — a cluster without
# its DB accepts no logins, which would make this sample a lie.
if ! pgrep -x dbmgr >/dev/null; then
    echo "dbmgr is not running: check kbengine.xml databaseInterfaces"
    exit 1
fi

echo "== installing the Bridge into $BOTS_SCRIPTS"
if [ ! -f "$BOTS_SCRIPTS/kbemain.py.stock" ]; then
    cp "$BOTS_SCRIPTS/kbemain.py" "$BOTS_SCRIPTS/kbemain.py.stock"
fi
cp "$ADAPTER_DIR/kbemain.py" "$BOTS_SCRIPTS/kbemain.py"
rm -rf "$BOTS_SCRIPTS/bridge"
cp -r "$ADAPTER_DIR/bridge" "$BOTS_SCRIPTS/bridge"

# The plan must own bot creation: disable the engine's defaultAddBots
# (10 auto bots) via the documented kbengine.xml override mechanism.
KBE_XML="$WAVEMAKER_KBE_ASSETS/res/server/kbengine.xml"
if ! grep -q "wm-sample-defaultAddBots-override" "$KBE_XML"; then
    echo "== disabling defaultAddBots in $KBE_XML (plan owns creation)"
    sed -i 's|</root>|\
	<bots> <!-- wm-sample-defaultAddBots-override -->\
		<defaultAddBots>\
			<totalCount> 0 </totalCount>\
		</defaultAddBots>\
	</bots>\
</root>|' "$KBE_XML"
fi

echo "== starting bots with Bridge injection ($BACKEND_ID)"
(
    cd "$WAVEMAKER_KBE_ASSETS"
    WAVEMAKER_BRIDGE_IPC="unix:$SOCKET_PATH" \
    WAVEMAKER_BRIDGE_TOKEN=tok \
    WAVEMAKER_SWELL_ID="$SWELL_ID" \
    WAVEMAKER_BACKEND_ID="$BACKEND_ID" \
    nohup "$KBE_BIN_PATH/bots" >"$LOG_DIR/bots.log" 2>&1 &
)

echo "== driving the login swell"
timeout 90 python3 "$SAMPLE_DIR/driver.py" "$SOCKET_PATH" "$SAMPLE_DIR/plan.json"

echo "== stopping the bots process"
pkill -x bots || true

echo "== run artifacts in $RUN_DIR (bots.log, driver output above)"
