---
title: Distributed Management
---

# Distributed Management and Failure Handling

## Worker / Backend Registration

Registration information:

- worker_id
- backend_id
- host_id
- backend_type
- engine / runtime version
- adapter version
- protocol version
- capabilities
- resource capacity
- labels
- last_heartbeat
- generation

## Leases

The ADR's current parameters serve as the initial candidates:

- Heartbeat: 1 second.
- TTL: 5 seconds.
- Lost contact after 3 consecutive misses.

These parameters need validation under network jitter and high load.

Lost-contact policy:

1. Freeze new load.
2. Do not actively disconnect existing connections.
3. Record the lease invalidation time.
4. Allow the Backend to keep existing sessions according to policy.
5. After recovery, re-handshake and synchronize state.
6. After the maximum lost-contact duration, follow the Swell's DrainPolicy.
7. Events from an old generation are dropped or archived; they must not overwrite new state.

## Failure Classification

- Controller lost.
- Worker lost.
- Supervisor crash.
- Engine Bot process crash.
- Bridge disconnected.
- Monitoring provider lost.
- Target server unavailable.
- Local resource exhaustion.
- Event queue overflow.
- Inconsistent plan data.

Every failure type must define detection, state, recovery actions, report impact, and whether automatic retry is allowed.
