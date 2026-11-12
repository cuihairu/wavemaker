---
title: Evaluation: Docker / Podman Launchers
---

# Evaluation: Docker / Podman Launchers (WM-0609)

> Status: Evaluated — 2026-11-12. This is an evaluation record, not an implementation.

## Question

`docs/supervisor.md` lists two optional container launchers
(`DockerProcessLauncher`, `PodmanProcessLauncher`) behind the required
`NativeProcessLauncher`. A later implementer needs one choice, made against
the seam that already shipped in v1. This document is that comparison and
the recommendation.

Scope the recommendation does **not** cover: making containers a hard
dependency (R-010 — containers stay optional, `NativeProcessLauncher` stays
the only required v1 launcher, and WM-0108 is already implemented), and
multi-machine orchestration (that stays with Kubernetes).

## Evaluation inputs (verified 2026-11-12)

| Question | Answer |
|---|---|
| Does the repo already have a launcher seam? | Yes: the `ProcessSupervisor` trait (`start` / `terminate` / `kill` / `status` / `resource_usage`) in `wm-supervisor/src/lib.rs:124`. A container launcher is one more trait impl. |
| Is a Rust client library available? | Yes: [bollard](https://github.com/fussybeaver/bollard) `0.21.1`, Apache-2.0, actively maintained (repo pushed 2026-11, ~68M downloads). |
| Does Podman have a REST API Docker clients can speak? | Yes: `podman system service` serves a **Docker v1.40 compatibility layer** plus the Podman-native Libpod layer. |
| Can bollard speak to Podman? | Yes — first-class since bollard 0.21: `Docker::connect_with_podman_defaults()` probes `$DOCKER_HOST`, then the rootless Podman socket, then the system Podman socket, then the Docker socket. |

## Candidate comparison

| Dimension | Docker (via bollard) | Podman (via bollard, Docker-compatibility layer) | Podman (via a Podman-native client) |
|---|---|---|---|
| Runtime availability | Docker Engine on host or remote | Podman 4+, daemon or socket-activated | Podman 4+ |
| Rust client | bollard — mature, one code path for both runtimes | bollard, over the compat layer | **None exists** — new, untried code |
| API surface | Docker Engine API | Docker v1.40 compat layer + Podman-native Libpod | Libpod only — a second API to learn |
| Socket / security model | `/var/run/docker.sock`; daemon is rootful — access is effectively root on the host | Unix socket with file permissions; rootless by default; system socket `/run/podman/podman.sock` | Same socket model, different path |
| Capturing stdout/stderr tails for `ExitRecord` | `logs()` stream | same method over the compat layer | re-implement log streaming |
| Resource usage (RSS / CPU) | `stats()` → `ContainerStatsResponse` | same via compat layer | re-implement stats |
| Exit info (code / signal) | `wait_container()` → `ContainerWaitResponse` | same via compat layer | re-implement |
| Rootless operation (no root needed) | No — the daemon is rootful | **Yes** — rootless is the default | yes |
| Same API for both runtimes | — | **Yes — one client, both runtimes** | **No — forks by client** |
| Maintenance cost in `wm-supervisor` | one impl | one impl for both | a second impl + Libpod semantics |

The left column is the deployment surface; the right column is the
supervisor's code. Those are different questions, and this evaluation finds
they can be answered with one implementation.

## Recommendation

**Implement one launcher — `ContainerProcessLauncher` — using bollard, and
treat Docker vs Podman as a connection option, not as two launchers.**

```text
ContainerProcessLauncher
  ├── connect: Docker::connect_with_local_defaults()
  │   auto-detects the Docker socket; Docker Engine semantics apply
  └── connect: Docker::connect_with_podman_defaults()
      probes $DOCKER_HOST, the rootless Podman socket, the system Podman
      socket, then the Docker socket — one launcher, either runtime
```

### Why this shape

1. **The seam is `ProcessSupervisor`, not the runtime.** The trait needs
   start/terminate/kill/status/resource_usage against `ProcessSpec`
   (`executable`, `args`, `working_directory`, `environment`,
   `stdout`/`stderr`, `restart_policy`, `shutdown_grace_period_ms`). It never
   asks which runtime is on the host, so the runtime choice is a connection
   detail, not a branching abstraction.

2. **bollard 0.21 makes Podman first-class through the same client.** The
   Docker v1.40 compatibility layer in `podman system service` means the
   method surface is identical for both runtimes: `create_container`,
   `start_container`, `stop_container` + `StopContainerOptions { signal,
   timeout }` (the grace period), `kill_container` + `KillContainerOptions {
   signal }`, `logs()`, `stats()`, `wait_container()`. One implementation,
   one test suite, no fork.

3. **No Podman-native Rust client exists to use instead.** crates.io returns
   nothing for a Podman client (checked 2026-11-12); building a Libpod path
   would be new, untried code implementing a second API — R-011's
   protocol-fork risk, applied to in-process clients.

4. **It keeps every already-documented principle intact:**
   - R-010 — the recommendation adds an option, never a dependency;
     `NativeProcessLauncher` stays required and stays the default.
   - `docs/security.md` — the boundary is the socket, and Podman's
     file-permission model on a Unix socket is the safer default (and it is
     rootless).
   - supervisor.md's `ProcessSpec` — a container spec is just the host
     executable replaced by an image; env, working directory and stream
     routing remain spec fields.
   - supervisor.md's failure policy — generation bump and late-event rejection
     stay launcher-agnostic, and the record must still carry `ExitRecord`
     verbatim (code / signal / tails / uptime).

### What this changes

- `docs/roadmap.md`'s "v1 Optional: Docker / Podman" stays true — as a pair
  describing one optional container deployment, not two deliverables.
- supervisor.md's "DockerProcessLauncher / PodmanProcessLauncher: optional
  later" reads as two connection options of one `ContainerProcessLauncher`.
- KubernetesJobLauncher stays out of v1.

### Caveats the implementer must carry

1. **Fail named, never crash.** If no runtime is present or no socket is
   reachable, `start` must surface a named error (extend `SupervisorError`)
   and the identical swell must still be runnable on
   `NativeProcessLauncher`.
2. **Credential pass-through is unchanged.** `ProcessSpec.environment`
   (`WM_SWELL_ID`, `WM_BACKEND_ID`, `WM_BRIDGE_ENDPOINT`) flows through as
   container env. `WM_BRIDGE_ENDPOINT` is a Unix socket
   (`unix:///run/wavemaker/<backend>.sock`) — the host socket must be bind
   mounted into the container, and that mount is the one deployment decision
   this evaluation does not make.
3. **Exit semantics are subtler than signals.** `wait_container()` yields a
   status code, not a raw signal. Mapping container state onto
   `ExitRecord` belongs in exactly one place: this launcher.
4. **`start` returns `u32` today; a container launcher returns a name.** The
   container id is what should be threaded into `Running { pid }` semantics —
   decide that at impl time and state it, rather than inventing a pid.
5. **Image availability is a deployment concern, not a supervisor concern.**
   The launcher consumes an image reference; it does not pull, build or
   invent one, and it must not make a registry a hard dependency.

## Read next

- bollard README — Podman first-class support and
  `connect_with_podman_defaults()` (checked 2026-11-12, v0.21.1)
- `podman-system-service(1)` — the REST API's two layers (Docker v1.40
  compat + Podman-native Libpod), and socket activation via
  `podman.socket` / `podman.service` (checked 2026-11-12)
- `docs/supervisor.md` — `ProcessSpec`, the launcher abstraction, and the
  failure policy this launcher must respect

## Conclusion

One launcher, two runtimes: **`ContainerProcessLauncher` via bollard,
connected to Docker or Podman.** No Podman-native client. That is the single
clear recommendation for WM-0609.
