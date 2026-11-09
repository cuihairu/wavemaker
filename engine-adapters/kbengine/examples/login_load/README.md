# End-to-End Login Load Test Sample (WM-0217)

Mode A end to end on a real KBEngine deployment: a swell plan drives the
Bridge, the Bridge drives the engine's native bots, the bots log into a
real cluster, and the login progress comes back as connection events over
the Bridge protocol.

```text
plan.json -> driver.py (plays Supervisor) -> Bridge (bots process)
          -> KBEngine.addBots -> loginapp -> dbmgr (account rows)
          -> baseapp (Account entities) -> BotCreated events back
```

This is the runnable sample behind `WM-0217` (docs/roadmap.md). The
protocol side is proven cross-language by the conformance test
(crates/wm-supervisor/tests/bridge_conformance.rs); this sample adds the
part only a real engine can prove: actual login traffic under the
wavemaker plan model.

## Files

| File | Role |
|---|---|
| `plan.json` | the swell: 2 batches x 3 bots (offsets 0 ms / 2000 ms) |
| `driver.py` | Supervisor half of the wire protocol; prints events, heartbeats, summary |
| `run_e2e.sh` | installs the Bridge into the assets tree, starts cluster + bots, runs the driver |

## Prerequisites

1. A built KBEngine workspace (binaries under `<kbe_root>/kbe/bin/server/`,
   python_assets tree at `<kbe_root>/assets`) — see the engine's build
   notes for one machine's full record.
2. A reachable MySQL for dbmgr, configured in
   `assets/res/server/kbengine.xml` -> `dbmgr/databaseInterfaces`. A clean
   dedicated instance is enough; dbmgr creates the `kbe_*` tables itself:

   ```sh
   docker run -d --name wm-kbengine-mysql \
     -e MYSQL_ROOT_PASSWORD=... -e MYSQL_DATABASE=kbengine \
     -p 127.0.0.1:23306:3306 mysql:8.4
   ```

   dbmgr exits at startup when it cannot reach the database, and
   `run_e2e.sh` refuses to run a login sample against a cluster without
   one — a DB-less cluster accepts no logins.

## Run

```sh
export WAVEMAKER_KBE_ROOT=/path/to/kbengine-workspace
sh run_e2e.sh
```

Expected tail of the driver output:

```text
  event BotCreated       bot_id=2
  ...
summary: {'BotCreated': 6}
result: PASS (6/6 bots surfaced)
```

The `BotCreated` events are the login progress signal: a bot surfaces in
the engine's bot map as it works through login, and a failed or kicked
login shows up as `BotDisconnected`. The authoritative login record is
server-side — after a run, `kbe_accountinfos` holds one `bot_*` row per
logged-in bot:

```sql
SELECT COUNT(*) FROM kbe_accountinfos WHERE accountName LIKE 'bot_%';
```

The setup is idempotent: the cluster is reused when already running, the
stock `kbemain.py` is kept as `kbemain.py.stock` on first install, and the
`defaultAddBots=0` override (so the plan, not the engine, owns bot
creation) is written once into `assets/res/server/kbengine.xml`.

## As a regression test

```sh
WAVEMAKER_E2E_CLUSTER=1 \
WAVEMAKER_KBE_ROOT=/path/to/kbengine-workspace \
  python3 -m pytest tests/test_login_load_sample.py -q
```

Without `WAVEMAKER_E2E_CLUSTER` the test skips itself (there is no honest
way to fake a cluster), so the normal CI gate stays green.

## Honesty boundary

What this sample proves today: plan-driven bot creation and pacing, the
full login round-trip (account creation visible in the database), session
lifecycle (stop/drain/shutdown), and live heartbeat metrics from the bots
process. What it does not yet claim: per-bot login timing or success
classification — the Bridge's capability catalog still reports
`login_success: unknown` because login states (C_STATE_*) are not exposed
to the embedded Python; that stays a runtime-verification item, not a
claim.
