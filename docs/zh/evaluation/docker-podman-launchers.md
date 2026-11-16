---
title: 评估：Docker / Podman 启动器
---

# 评估：Docker / Podman 启动器（WM-0609）

> 状态：已评估 —— 2026-11-12。本页是评估记录，不是实现。

## 问题

`docs/supervisor.md` 在必需的 `NativeProcessLauncher` 之外列出了两个可选的容器启动器（`DockerProcessLauncher`、`PodmanProcessLauncher`）。后续实现者需要做出一个选择，而且这个选择要对照 v1 已经交付的那个接缝来做。本文档就是这次比较与推荐。

本推荐**不**覆盖的范围：把容器变成硬依赖（R-010 —— 容器保持可选，`NativeProcessLauncher` 仍然是 v1 唯一必需的启动器，且 WM-0108 已经实现），以及多机编排（那仍然属于 Kubernetes）。

## 评估输入（2026-11-12 已核实）

| 问题 | 答案 |
|---|---|
| 仓库里是否已经有启动器接缝？ | 有：`wm-supervisor/src/lib.rs:124` 中的 `ProcessSupervisor` trait（`start` / `terminate` / `kill` / `status` / `resource_usage`）。容器启动器只是再多一个 trait 实现。 |
| 是否有可用的 Rust 客户端库？ | 有：[bollard](https://github.com/fussybeaver/bollard) `0.21.1`，Apache-2.0，持续维护中（仓库 2026-11 有推送，约 68M 下载量）。 |
| Podman 是否有 Docker 客户端能说的 REST API？ | 有：`podman system service` 提供 **Docker v1.40 兼容层**，外加 Podman 原生的 Libpod 层。 |
| bollard 能否与 Podman 通信？ | 能 —— 自 bollard 0.21 起就是一等支持：`Docker::connect_with_podman_defaults()` 依次探测 `$DOCKER_HOST`、rootless Podman socket、系统 Podman socket，最后是 Docker socket。 |

## 候选方案比较

| 维度 | Docker（经 bollard） | Podman（经 bollard，Docker 兼容层） | Podman（经 Podman 原生客户端） |
|---|---|---|---|
| 运行时可用性 | 主机或远程的 Docker Engine | Podman 4+，daemon 或 socket 激活 | Podman 4+ |
| Rust 客户端 | bollard —— 成熟，两种运行时共用一条代码路径 | bollard，走兼容层 | **不存在** —— 全新、未经尝试的代码 |
| API 面 | Docker Engine API | Docker v1.40 兼容层 + Podman 原生 Libpod | 仅 Libpod —— 要额外学的第二套 API |
| Socket / 安全模型 | `/var/run/docker.sock`；daemon 以 root 运行 —— 拿到访问权约等于拿到主机 root | Unix socket 加文件权限；默认 rootless；系统 socket 为 `/run/podman/podman.sock` | 同样的 socket 模型，路径不同 |
| 为 `ExitRecord` 抓取 stdout/stderr 尾部 | `logs()` 流 | 经兼容层用同一方法 | 重新实现日志流 |
| 资源占用（RSS / CPU） | `stats()` → `ContainerStatsResponse` | 经兼容层同上 | 重新实现 stats |
| 退出信息（退出码 / 信号） | `wait_container()` → `ContainerWaitResponse` | 经兼容层同上 | 重新实现 |
| rootless 运行（无需 root） | 否 —— daemon 以 root 运行 | **是** —— rootless 是默认 | 是 |
| 两种运行时同一套 API | — | **是 —— 一个客户端，两种运行时** | **否 —— 按客户端分叉** |
| `wm-supervisor` 中的维护成本 | 一个实现 | 两种运行时共用一个实现 | 第二个实现 + Libpod 语义 |

左列是部署面；右列是 supervisor 的代码。这是两个不同的问题，而本次评估发现它们可以用一个实现回答。

## 推荐

**实现一个启动器 —— `ContainerProcessLauncher` —— 用 bollard，并把 Docker 与 Podman 之别当作连接选项，而不是两个启动器。**

```text
ContainerProcessLauncher
  ├── connect: Docker::connect_with_local_defaults()
  │   auto-detects the Docker socket; Docker Engine semantics apply
  └── connect: Docker::connect_with_podman_defaults()
      probes $DOCKER_HOST, the rootless Podman socket, the system Podman
      socket, then the Docker socket — one launcher, either runtime
```

### 为什么是这个形状

1. **接缝是 `ProcessSupervisor`，不是运行时。** 这个 trait 需要针对 `ProcessSpec`（`executable`、`args`、`working_directory`、`environment`、`stdout`/`stderr`、`restart_policy`、`shutdown_grace_period_ms`）提供 start/terminate/kill/status/resource_usage。它从不询问主机上是哪种运行时，所以运行时的选择是连接细节，不是分支抽象。

2. **bollard 0.21 通过同一个客户端把 Podman 变成一等公民。** `podman system service` 中的 Docker v1.40 兼容层意味着两种运行时的方法面完全一致：`create_container`、`start_container`、`stop_container` + `StopContainerOptions { signal, timeout }`（即宽限期）、`kill_container` + `KillContainerOptions { signal }`、`logs()`、`stats()`、`wait_container()`。一个实现，一套测试，不分叉。

3. **不存在可以替代使用的 Podman 原生 Rust 客户端。** crates.io 上搜不到任何 Podman 客户端（2026-11-12 核实）；构造一条 Libpod 路径就是编写全新、未经尝试的代码来实现第二套 API —— 即 R-011 的协议分叉风险，落在进程内客户端上。

4. **它保持每一条已成文的原则不变：**
   - R-010 —— 本推荐只增加一个选项，绝不增加依赖；`NativeProcessLauncher` 仍然必需，也仍然是默认。
   - `docs/security.md` —— 边界就是 socket，而 Podman 在 Unix socket 上的文件权限模型是更安全的默认（而且是 rootless）。
   - supervisor.md 的 `ProcessSpec` —— 容器 spec 只是把主机可执行文件换成镜像；env、working directory 与流路由仍然是 spec 字段。
   - supervisor.md 的失败策略 —— generation 递增与迟到事件拒绝保持与启动器无关，且记录仍必须原样携带 `ExitRecord`（退出码 / 信号 / 尾部 / 运行时长）。

### 这会改变什么

- `docs/roadmap.md` 的 "v1 Optional: Docker / Podman" 仍然成立 —— 作为一对描述同一种可选容器部署的条目，而不是两个交付物。
- supervisor.md 的 "DockerProcessLauncher / PodmanProcessLauncher: optional later" 应读作同一个 `ContainerProcessLauncher` 的两个连接选项。
- KubernetesJobLauncher 仍不进入 v1。

### 实现者必须带走的注意事项

1. **失败要报出名字，绝不崩溃。** 如果没有任何运行时或任何 socket 不可达，`start` 必须浮出一个有名字的错误（扩展 `SupervisorError`），而同一个 Swell 仍然必须能在 `NativeProcessLauncher` 上运行。
2. **凭据透传不变。** `ProcessSpec.environment`（`WM_SWELL_ID`、`WM_BACKEND_ID`、`WM_BRIDGE_ENDPOINT`）作为容器 env 原样流入。`WM_BRIDGE_ENDPOINT` 是一个 Unix socket（`unix:///run/wavemaker/<backend>.sock`）—— 主机 socket 必须以 bind mount 挂进容器，而这个挂载正是本评估不替你决定的一项部署决策。
3. **退出语义比信号更微妙。** `wait_container()` 给出的是状态码，不是原始信号。把容器状态映射到 `ExitRecord` 只属于一个地方：这个启动器。
4. **`start` 今天返回 `u32`；容器启动器返回的是一个名字。** 应该被穿进 `Running { pid }` 语义的是容器 id —— 在实现时决定并写明它，而不是凭空造一个 pid。
5. **镜像可用性是部署问题，不是 supervisor 问题。** 启动器消费一个镜像引用；它不拉取、不构建、也不发明镜像，并且不得把 registry 变成硬依赖。

## 延伸阅读

- bollard README —— Podman 一等支持与 `connect_with_podman_defaults()`（2026-11-12 核实，v0.21.1）
- `podman-system-service(1)` —— REST API 的两层（Docker v1.40 兼容层 + Podman 原生 Libpod），以及经 `podman.socket` / `podman.service` 的 socket 激活（2026-11-12 核实）
- `docs/supervisor.md` —— `ProcessSpec`、启动器抽象，以及这个启动器必须遵守的失败策略

## 结论

一个启动器，两种运行时：**经 bollard 的 `ContainerProcessLauncher`，连接 Docker 或 Podman。** 不用 Podman 原生客户端。这就是 WM-0609 唯一明确的推荐。