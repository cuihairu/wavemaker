---
title: Process Supervisor
---

# Process Supervisor and Containerization

## Abstraction Principles

Containerization is not a replacement for the Bridge, nor a required dependency of Wavemaker.

Suggested abstraction:

```text
ProcessSupervisor
  ├── prepare(spec)
  ├── start()
  ├── stop_admission()
  ├── terminate(grace_period)
  ├── kill()
  ├── status()
  ├── resource_usage()
  ├── collect_logs()
  └── cleanup()
```

Deployment implementations:

- NativeProcessLauncher: required for v1.
- DockerProcessLauncher: optional later.
- PodmanProcessLauncher: optional later.
- KubernetesJobLauncher: not in v1; evaluate only when multi-machine cluster deployment requirements are explicit.

## ProcessSpec

```yaml
process:
  executable: /opt/kbengine/bin/bots
  args: []
  working_directory: /opt/kbengine
  environment:
    WM_SWELL_ID: swell-001
    WM_BACKEND_ID: kbengine-01
    WM_BRIDGE_ENDPOINT: unix:///run/wavemaker/kbengine-01.sock
  stdout: file
  stderr: file
  restart_policy: on-failure
  shutdown_grace_period_ms: 10000

resources:
  cpu_limit: null
  memory_limit_bytes: null
  file_descriptor_limit: null
```

Long-lived plaintext credentials must not be written into configuration. Sensitive values are passed through protected temporary files, environment injection, or a local credential channel.

## Process Failure Policy

- Unexpected process exit: record the exit code, signal, log tail, and last heartbeat.
- Automatic restart: must be explicitly allowed by the Swell's failure policy.
- A Backend after restart must use a new generation.
- Late events from an old generation must not overwrite new state.
- When it cannot be confirmed whether existing bots have disconnected, the reported state must be Unknown, not 0.
- Restart must not automatically refill load unless the plan and failure policy explicitly allow it.
