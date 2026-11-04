---
title: Process Supervisor 与容器化
---

# Process Supervisor 与容器化

## 7.1 抽象原则

容器化不是 Bridge 的替代品，也不是 Wavemaker 的必选依赖。

建议抽象：

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

部署实现：

- NativeProcessLauncher：v1 必须。
- DockerProcessLauncher：后续可选。
- PodmanProcessLauncher：后续可选。
- KubernetesJobLauncher：不进入 v1，只有多机集群部署需求明确后再评估。

## 7.2 ProcessSpec

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

配置中不得写入明文长期凭证。敏感值通过受保护的临时文件、环境注入或本机凭证通道传递。

## 7.3 进程故障策略

- 进程意外退出：记录退出码、信号、日志尾部和最后心跳。
- 自动重启：必须由 Swell 的故障策略显式允许。
- 重启后的 Backend 必须使用新的 generation。
- 不允许旧 generation 的迟到事件覆盖新状态。
- 不能确认已有 Bot 是否断开时，报告状态应为 Unknown，而不是 0。
- 重启不得自动补足负载，除非计划和故障策略明确允许。
