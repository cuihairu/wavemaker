# KBEngine Bot API 静态审计（WM-0007 / WM-0008 静态部分）

> 状态：Phase 0 静态审计完成；原 §8 运行时清单已于 2026-11-09 静态闭合 8/8（S1–S8），残留运行时项 RT1–RT4 标注 BLOCKED（§8.2，挂 WM-0009 / WM-0010）。
> 源码版本：`~/workspaces/kbengine`，KBEngine v2.5.10 fork（`kbe/src/lib/common/kbeversion.h:10-12`），审计时 HEAD commit `bf1a9cd6f4a82dd715fcf4a69697382ed531b4c0`（2026-11-05）。所有 `file:line` 均指向该 commit。
> 判定规则：与计划书描述不一致时**以源码为准**并在本文件标注；仅引用上游文档而未对照源码的结论一律标「待验证」。

## 0. 审计范围与总览

对应计划书 API 审计表（计划书 §"KBEngine API 审计"，6 个条目）：

| API | 本 fork 签名（源码已验证） | 桥接相关结论 |
|---|---|---|
| `KBEngine.addBots()` | `addBots(totalCount)` 或 `addBots(totalCount, tickCount, tickTime)`；仅接受 1 或 3 个参数 | 批次注入点；无返回值、无失败回调、无最大批次上限 |
| `KBEngine.bots` | 只读 mapping，键为 int32 appID，值为 `ClientObject` | Bot 清点入口；非 dict |
| `KBEngine.callback()` | `callback(delaySeconds, callable)`；**恰好 2 个参数** | 单次定时回调；粒度 = 1/gameUpdateHertz（默认 10 → 100 ms） |
| `KBEngine.cancelCallback()` | `cancelCallback(callbackID: int)`；恰好 1 个参数 | 未知 ID 静默失败（返回 None，无异常） |
| `onInit()` | 入口脚本方法，被调用时带 1 个 int 参数（值为 0） | 全部脚本加载完成后、主循环前调用；异常会**中止 bots 启动** |
| `onFinish()` | 入口脚本方法，无参数 | finalise 期调用；异常不中止退出流程 |

---

## 1. `KBEngine.addBots()`

**状态：已验证（源码证据）。**

- 注册：`kbe/src/server/tools/bots/bots.cpp:182`（`Bots::installPyModules`，挂到 bots 进程的 `KBEngine` 模块，`METH_VARARGS`）。
- 实现：`bots.cpp:344-388`（`Bots::__py_addBots`）。

签名与行为（`bots.cpp:350-388`）：

```python
KBEngine.addBots(totalCount: int)                          # 恰好 1 个参数
KBEngine.addBots(totalCount: int, tickCount: int, tickTime: float)  # 恰好 3 个参数
```

- 参数个数必须是 **1 或 3**，否则 `PyErr_Format(PyExc_TypeError, "KBEngine::addBots: args error!")` 并返回 NULL（`bots.cpp:380-386`）——传 2 个参数会直接抛 TypeError。
- 解析格式 `"I|I|f"`：`totalCount` 为 uint32，`tickCount` 为 uint32，`tickTime` 为 float 秒（`bots.cpp:364-365`）。
- 语义（`bots.cpp:366-376`）：`totalCount` 是**累加**到全局待创建登录总数（`reqCreateAndLoginTotalCount() + totalCount`）；`tickCount > 0` / `tickTime > 0` 时**覆盖**每 tick 创建数与 tick 时长，传 0 或负值不生效（保持默认）。
- 创建节奏执行体：`CreateAndLoginHandler`（`bots.cpp:249`，`Bots::run` 中启动；详见 `bots/create_and_login_handler.*`）。
- **返回值：`None`**（`S_Return` 宏）。

计划书差异标注（以源码为准）：

1. 计划书关注「返回值」——源码无返回值，Bot 创建进度只能通过 `KBEngine.bots` 清点或日志获得。
2. 计划书关注「失败回调」——源码**没有任何**失败回调/事件；登录失败在 `ClientObjectBase` 事件流里（`EventData_onKicked` 等，`clientobjectbase.cpp:240-247`），不在 addBots 层。
3. 计划书关注「最大批次」——源码**无上限校验**，累加语义意味着 Bridge 必须自己做批次预算与并发上限控制（StopAdmission 前置校验，WM-0210）。

**待运行时验证**：tickTime 与 bots 进程 tick 的实际换算与节流行为（静态只见 handler 存在，节奏数值未展开核对到每个分支）；`reqCreateAndLoginTickCount/Time` 的默认值（`bots.h`）需在运行时以日志确认。

## 2. `KBEngine.bots`

**状态：已验证（源码证据）。**

- 注册：`bots.cpp:180`——`registerPyObjectToScript("bots", pPyBots_)`，是 **PyBots 对象**，不是 Python dict（`pybots.cpp:16-66`，`SCRIPT_INIT(PyBots, ...)` 带 `PyMappingMethods`）。
- 支持的协议（`pybots.cpp:17-31`）：`len(bots)`（mp_length，= `clients().size()`，`pybots.cpp:72-75`）、`bots[key]`（mp_subscript，`pybots.cpp:77-97`）、方法 `has_key / keys / values / items / get`（`pybots.cpp:24-31`）。
- 键：**int32 clientID（appID）**——`mp_subscript` 用 `PyLong_AsLong` 转 int 后 `findClientByAppID(clientID)`（`pybots.cpp:80-90`）；`keys()` 返回各 client 的 `appID()`（`pybots.cpp:99-113`）。**键不是 entityID，也不是序号**。
- 值：`ClientObject`（`bots.h:103` `CLIENTS = map<Channel*, ClientObject*>` 经 `findClientByAppID` 返回）。缺失键抛 `PyExc_KeyError`（`pybots.cpp:92-96`）。
- **只读**：mp_ass_subscript = NULL（`pybots.cpp:16-22`），不能增删；Python 侧也没有创建单个 bot 的方法——唯一注入点是 `addBots`。

对 Bridge（WM-0208 Bot 数量清点）的含义：清点用 `len(KBEngine.bots)` + `keys()` 即可拿到 clientID 全集；逐 Bot 状态要走每个 `ClientObject` 的属性/事件，不能假定 `bots` 提供 dict 式迭代便利（如 `.items()` 返回的是 list of tuples 的 C++ 实现，语义兼容但类型细节以运行时为准）。

**待运行时验证**：`keys()`/`values()` 在 bot 动态增减时的快照一致性（C++ 端无锁快照还是实时遍历，静态读图不完全）；ClientObject 暴露给 Python 的可读属性清单。

## 3. `KBEngine.callback()`

**状态：已验证（源码证据）；粒度结论可静态推导，绝对精度待运行时验证。**

- 注册：模块级 `clientapp.cpp:195`（bots 进程复用 ClientApp 的 KBEngine 模块）；实体方法级 `clientobjectbase.cpp:22`（每个 ClientObjectBase 派生实体自带 `.callback`）。
- 实现：`clientapp.h:182-185` 委托 → `clientobjectbase.cpp:254-283`（`ClientObjectBase::__py_callback`）。

签名与行为：

```python
id = KBEngine.callback(delaySeconds: float, fn: callable)   # 恰好 2 个参数
id = entity.callback(delaySeconds: float, fn: callable)     # 实体侧同签名
```

- **恰好 2 个参数**：`PyTuple_Size(args) != 2` 即 TypeError `"KBEngine::callback: (argssize != (time, callback)) error!"`（`clientobjectbase.cpp:256-261`）。
- **不支持可变参数转发**：`PyArg_ParseTuple(args, "f|O", ...)` 只取 time + callable；回调被调用时不携带任何额外参数。这与上游 KBEngine 官方文档常见的 `callback(delay, cb, *args)` 记忆不符——**以本源码为准，Bridge 侧需用 functools.partial 自带闭包参数**。（上游差异对照：待验证，未对照上游源码。）
- callable 校验：`PyCallable_Check` 失败即 TypeError（`clientobjectbase.cpp:268-273`）。
- 返回：`ScriptID`（int，`PyLong_FromLong`，`clientobjectbase.cpp:280`）。**单次回调**：注册时 repeatOffset 固定传 0.0f（`clientobjectbase.cpp:277`）。

调度粒度（静态推导，`client_lib/script_callbacks.cpp:29-58`）：

- `initialTicks = g_kbetime + delaySeconds * hertz`，hertz 在 bots 进程取 `gameUpdateHertz()`（`script_callbacks.cpp:41-44`）；
- 默认 `gameUpdateHertz = 10`（`serverconfig.cpp:74`，可由 `kbengine_defaults.xml` `gameUpdateHertz` 覆盖，`serverconfig.cpp:484-486`）→ **默认回调粒度 = 100 ms**，延迟按 tick 量化（向下取整到 tick）。
- 到期执行点：`ClientObjectBase::tickSend → handleTimers → timers().process(g_kbetime)`（`clientobjectbase.cpp:252,248-250`），位于**主循环/网络线程**（EventDispatcher，`event_dispatcher.cpp:151,197`）。

**待运行时验证（WM-0010）**：实际 p50/p99 调度滞后与积压行为（静态只能给出 100 ms 量化下界，给不出负载下的滞后分布）；回调内抛异常后的传播路径（异常是否会中断当轮 tickSend、是否被吞）。

## 4. `KBEngine.cancelCallback()`

**状态：已验证（源码证据）。**

- 注册：`clientapp.cpp:196`；实体方法级 `clientobjectbase.cpp:23`。
- 实现：`clientobjectbase.cpp:287-312`（`ClientObjectBase::__py_cancelCallback`）→ `client_lib/script_callbacks.cpp:90-102`（`ScriptCallbacks::delCallback`）。

签名与行为：

```python
KBEngine.cancelCallback(callbackID: int)   # 恰好 1 个参数，"i" 解析
```

- **恰好 1 个参数**（int），否则 TypeError（`clientobjectbase.cpp:289-294`）。
- **未知/已触发的 ID 静默失败**：`delCallback` 找不到就返回 false，`__py_cancelCallback` 不检查返回值（`clientobjectbase.cpp:309`），Python 侧拿到 `None`，**无异常**。对 Bridge 的含义：取消竞态（回调已 fire 后再 cancel）不会报错，但也**无法据此感知竞态是否发生**。
- 句柄生命周期：`ScriptCallbacks` 用 `map<ScriptID, TimerHandle>` 管理（`script_callbacks.h:35` 附近），回调触发后句柄被释放（`releaseCallback`，`script_callbacks.cpp:105`）；cancel 一个已释放的 ID 即上面的静默路径。

**待运行时验证**：ScriptID 是否进程内单调复用（复用会让「取消旧 ID」误杀新回调——静态未见复用保护，需运行时确认 ID 空间与溢出行为）。

## 5. `onInit()`

**状态：已验证（源码证据）。**

- 调用点：`Bots::initializeEnd`（`bots.cpp:95-112`）：所有脚本加载完毕后、进入主循环前，`PyObject_CallMethod(entryScript, "onInit", "i", 0)`。
- **签名：`onInit(arg0: int)`，实参固定为 0**（格式串 `"i"` + `0`，`bots.cpp:101-105`）。Bridge 入口脚本不能把 onInit 声明成无参函数，否则 TypeError。
- 失败后果：返回 NULL 时 `SCRIPT_ERROR_CHECK()` 且 **`initializeEnd` 返回 false → bots 进程启动中止**（`bots.cpp:107-111`）。即 onInit 抛异常 = 进程起不来。
- 入口脚本来源：`kbengine_defaults.xml` `<bots><entryScriptFile>`（默认 `kbemain`，`kbengine_defaults.xml:833-837`），import 于 `bots.cpp:197-215`。
- client（非 bots）同模式：`clientapp.cpp:132`。

计划书关注「重复调用行为」：源码中 initializeEnd 在启动序列只走一次（`onInstallPyModules`/initialize 流程单次），静态未见重复调用路径。**待运行时验证**：进程 restart/recover 场景下 onInit 是否重入（Phase 5 恢复握手 WM-0509 联动）。

## 6. `onFinish()`

**状态：已验证（源码证据）。**

- 调用点：`Bots::finalise`（`bots.cpp:117-131`）：`PyObject_CallMethod(entryScript, "onFinish", "")` —— **无参数**。
- 失败后果：返回 NULL 仅 `SCRIPT_ERROR_CHECK()`（记录脚本错误），**不影响退出流程继续**（`bots.cpp:129-131` 后直接走 clients 清理）。即 onFinish 抛异常**不会阻止进程退出，也不会被等待**。
- 计划书关注「是否可等待异步收尾」：**静态结论：不可等待**——finalise 是同步顺序调用，onFinish 返回即继续；Bridge 的 Drain/Shutdown（WM-0211）不能依赖 onFinish 完成异步落盘，收尾逻辑必须同步做完或在 Controller 侧另行确认。

**待运行时验证**：telnet 强杀 / 崩溃路径下 onFinish 是否仍被调用（静态只覆盖正常 finalise 链）。

## 7. 与计划书描述不符项汇总（以源码为准）

| # | 计划书隐含假设 | 源码事实 | 影响 |
|---|---|---|---|
| D1 | addBots 有返回值/进度反馈 | 返回 None，无失败回调 | WM-0208 清点必须走 `KBEngine.bots` |
| D2 | addBots 可能有批次上限保护 | 无上限，totalCount 为累加语义 | Bridge 必须自建批次预算（WM-0207/0210） |
| D3 | callback 支持 *args 转发 | 恰好 2 参，无参数转发 | Bridge 用 partial/闭包包装 |
| D4 | callback 粒度未知/需实测 | 粒度 = 1/gameUpdateHertz，默认 10 Hz = 100 ms（静态推导） | 到达计划映射到引擎回调时按 100 ms 量化；实测仍需 WM-0010 |
| D5 | cancelCallback 有竞态反馈 | 未知 ID 静默 None | 竞态检测要靠 Bridge 侧自身记账 |
| D6 | onInit 无参钩子 | 带 1 个 int 实参（值 0）；异常中止启动 | Bridge bootstrap 签名必须 `def onInit(arg)` |
| D7 | onFinish 可等待异步收尾 | 同步调用、异常不阻断退出 | Drain/Shutdown 不依赖 onFinish |

## 8. WM-0009/WM-0010 检查表：静态闭合记录（2026-11-09）与残留运行时项

> 原清单的 8 个「必须运行时验证」项已于 2026-11-09 逐项静态验证。基线：fork HEAD `3082ab50d24f551336b52170a7388891914968fd`（2026-11-07）——较 §1–§7 审计时的 `bf1a9cd6` 新增一个 pyscript 缓冲修复，故个别行号存在 ±1–2 漂移；**本节行号均按 3082ab50 重新核对**。
> 结果：8/8 项静态闭合（含一项推翻文档注释的发现，见 S8）；真正需要运行时测量的残留项收敛为 RT1–RT4（§8.2），全部标注 BLOCKED 与原因。

### 8.1 静态闭合项（S1–S8）

**S1 单线程假设——闭合。**
bots 进程恰好一个线程池且永不使用：`ClientApp` 构造 `thread::ThreadPool`（`clientapp.h:251`）并 `createThreadPool(1,1,4)` 建 1 个工人（`clientapp.cpp:159-160`），该工人只在条件变量上等待（`thread/threadpool.cpp:23-29,779`），client_lib 与 bots 工具中**无任何** `addTask` 调用（全树仅 loginapp/baseapp/guiconsole 使用）。所有脚本入口都在主循环内：`kbeMainT` 单线程 initialize→run（`kbemain.h:272,281`）；`ClientApp::run` = `processUntilBreak()`（`clientapp.cpp:442-446`）；`processOnce` 串行执行 tasks→timers→network（`event_dispatcher.cpp:189-207`）；timer 回调、onInit/onFinish、telnet Python、console exec、实体回调全部在此循环内派发。**推论：跨线程直接调脚本 API 是未定义行为——Bridge IPC 线程必须经队列投递到主循环（wavemaker 侧纪律，RT3）。**

**S2 回调内阻塞的影响范围——闭合（机制）；容忍时长归 RT1。**
`processTimers` → 游戏 tick → `handleGameTick`（`clientapp.cpp:364-439`）串行做：`threadPool_.onMainThreadTick`、`processChannels`（收包→实体回调）、`tickSend`（`clientobjectbase.cpp:168-199`：handleTimers + 每 `g_channelExternalTimeout/2` 的 active-tick 心跳 + updatePlayerToServer）、poller 事件与逐 bot `gameTick`（`bots.cpp:274,279-293`）。timer 派发是无预算的 drain 循环（`timer.inl:107-140`）。**一个阻塞回调会拖住：定时器、收包、心跳、发包、断线检测**（`onChannelTimeOut` 同在此循环，`clientapp.cpp:567-574`——阻塞过久可被服务端踢）。空闲 poller 等待上限 0.1s（`event_dispatcher.cpp:19,131-142`）。

**S3 回调异常传播——闭合。**
路径：`timer.inl:252-268` → `ScriptCallbackHandler::handleTimeout`（`script_callbacks.cpp:169-185`）：`PyObject_CallFunction` 后 `SCRIPT_ERROR_CHECK()`，而该宏只做 `if (PyErr_Occurred()) PyErr_PrintEx(0)`（`pyscript/scriptobject.h:39-46`）——**异常被打印并吞掉**：无 C++ 错误位、不中断 tick、不影响连接。重复型 timer 无条件重臂（`timer.inl:258-266`），持续抛异常的重复回调会每个周期打印 traceback 并继续触发。一次性 timer 触发后照常删除。（注：审计早期草稿中的 `handleScriptTimer` 符号在本 fork 不存在，唯一路径即 `ScriptCallbackHandler::handleTimeout`。）

**S4 GIL 与 telnet——闭合（无竞争）；真实风险是内联阻塞。**
telnet server 挂在同一主 dispatcher 与主解释器上：`bots.cpp:90-95` `new TelnetServer(&dispatcher(), &networkInterface()); pScript(&getScript())`；监听/连接 fd 注册进主 poller（`telnet_server.cpp:87,168`），`TelnetServer/TelnetHandler` 均为 `InputNotificationHandler`，全部在主线程派发；`telnet_server.cpp/telnet_handler.cpp` 不创建任何线程。telnet **能**执行 Python：`processPythonCommand` → `pScript()->run_simpleString(...)`（`telnet_handler.cpp:795-814`），但始终在持 GIL 的主线程内——**无 GIL 竞争，静态风险与 S2 同源：任意 telnet Python 语句内联阻塞网络循环**。等价路径还有 console `onExecScriptCommand`（`bots.cpp:467-497`，`:485` run_simpleString）。

**S5 ScriptID 复用/溢出——闭合（复用成立，风险真实）。**
`typedef int32 ScriptID`（`platform.h:352`）；分配**非单调**：取最小空闲正整数（`script_callbacks.cpp:77-87` 逐号扫描）；回调触发/cancel 后 map 条目擦除（`script_callbacks.cpp:188-192,105-127,90-102`），擦除后同号即复用，**无任何复用保护**——旧 ID 在触发后再 cancel 会静默误杀同对象上新回调（与 §4「静默 None」复合）。命名空间是**每 `ClientObjectBase` 一个**（`clientobjectbase.h:496-497`），不同 bot 间 ID 1..N 重复。溢出需单 bot 同时 ~2^31 活跃回调（不可达；无显式护栏）。`addCallback` 在 `timers_.add` 失败时返回 0（`script_callbacks.cpp:73`），0 为保留非法值（ID 从 1 起）。
**Bridge 纪律（RT3 的一部分）：回调句柄只在「未触发」窗口内 cancel，触发后一律依赖记账而非 cancel 反馈。**

**S6 onInit 重入——闭合（每进程恰好一次）。**
`kbeMainT` 只调一次 `initialize()`（`kbemain.h:272`），失败即 finalise 返回 −1，无重试；bots 的 initialize 链单向到 `initializeEnd`（`clientapp.cpp:157-174`），全树 `initializeEnd` 仅此一个调用点；shutdown 走 `breakProcessing` → `processUntilBreak` 退出 → finalise（`kbemain.h:281-286`），终局不可逆。**「重启」只能是新进程 → 全新 onInit**；残留仅 wavemaker 侧问题：Supervisor 重启策略与 onInit 副作用的跨进程幂等（不在 kbe 树内，RT3 处理）。

**S7 `bots` 映射快照一致性——闭合。**
`pyKeys/pyValues/pyItems` 直接无锁迭代 live `std::map` 构建 Python list（`pybots.cpp:89-150`）。两个事实使竞态静态不可能：(a) 单线程（S1）——增（`create_and_login_handler.cpp:54-61`→`bots.cpp:500-506`）与删（`bots.cpp:286-290,515-525`）都发生在同一主循环 tick；(b) 迭代体内只做 `PyLong_FromLong/Py_INCREF/PyList_SET_ITEM`，不重入 Python。返回值是 INCREF 快照（后续 delClient 不使其失效），但列出的 `ClientObject` 之后可能被销毁——沿用 `bots.cpp:286-290` 的 `isDestroyed` 守卫模式。`len()`/`bots[id]` 读 live（`pybots.cpp:56-80`）。

**S8 addBots 节拍——闭合（推翻 `bots.h:124-129` 文档注释）。**
`CreateAndLoginHandler` 注册**固定 1 秒**重复 timer（`create_and_login_handler.cpp:20-21`，`addTimer` 默认重复，`event_dispatcher.inl:7-12`），与 gameUpdateHertz 无关。`handleTimeout`（`:37-62`）逻辑：`static float lasttick` 首次触发时捕获 `reqCreateAndLoginTickTime()`，每秒减 0.1 直到 ≤0（**tickTime 是一次性前置延迟、以 0.1 为单位计秒，不是「一个 tick 的时间」——bots.h 文档注释与实现不符**）；闸门打开后速率恒为每秒 `tickCount` 个 bot（`lasttick` 永不复位）。默认值 `totalCount=10, tickTime=0.1, tickCount=5`（`kbengine_defaults.xml:867-873`），可被 `server/kbengine.xml` 覆盖，无 C++ 兜底。两个附加事实：(a) 停止条件是「维持 N 存活」——bot 被踢后 `delClient` 缩量、handler 会补建（补建也受 StopAdmission 预算约束）；(b) `clients().size() > totalCount` 时 uint32 下溢产生巨大第一操作数，但 `count-- > 0` 钳制使单次爆发仍 ≤ tickCount。真实到达时刻是「下个 gameTick 的 `C_STATE_INIT`」（`clientobject.cpp:414-455`）——**到达精度上界 = 两个周期之和：~1s（建批）+ gameTick（100ms 默认）**，Mode A 的 `exact_arrival` 能力声明必须以此为界。

### 8.2 残留运行时项（BLOCKED）

| # | 残留项 | 挂靠 | BLOCKED 原因 |
|---|---|---|---|
| RT1 | 回调调度滞后 p50/p99、可容忍阻塞时长实测（S2/S4 的数值面） | WM-0010 | 需运行中的 bots 进程 + 引擎集群；本机无部署（kbengine-build 工作区存在构建产物，但无运行进程/集群配置），静态只能给出 100ms 量化下界（§3） |
| RT2 | CPython 派生行为：`PyErr_PrintEx` 遇 `SystemExit` 的进程退出路径、脚本级 `sys.excepthook` 的干预 | WM-0009 残留 | 取决于嵌入解释器行为，不在 kbe 源码内静态可证 |
| RT3 | wavemaker Bridge IPC 线程纪律（投递到主循环）、句柄 cancel 窗口（S5）、Supervisor 重启幂等（S6） | WM-0202/0206/0212 | Bridge 未实现（Phase 2）；落地时以 S1–S8 为实现约束并复测 |
| RT4 | 负载下 addBots/timer 节拍的墙钟抖动（S8 的数值面） | WM-0010 | 同 RT1，需真实负载环境 |

### 8.3 对 Phase 2 的实现约束（由 S1–S8 导出）

1. Bridge IPC 收包线程**只入队不调脚本**；主循环侧用 `KBEngine.callback` 驱动队列消费（粒度 100ms，§3/D4）。
2. 一切对 `KBEngine.bots` 的遍历须先取 `keys()` 快照并逐项 `isDestroyed` 守卫（S7）。
3. 回调 ID 记账由 Bridge 自管：仅对「已注册未触发」句柄 cancel；不复用引擎侧 ID 语义（S5/D5）。
4. Bridge 批次预算按 S8 换算建模到达（1s 批 + gameTick 量化），`exact_arrival` 声明 Degraded。
5. Bridge 不得在 onInit/onFinish 内做异步收尾（D6/D7）；脚本异常按 S3 语义「打印即吞」，Bridge 自身状态机不能依赖回调必然成功。

## 9. 结论

WM-0007/WM-0008 的静态部分闭合：六个 API 的签名、参数、调用时机、失败语义均有 file:line 级源码证据，7 项与计划书不符处以源码为准并已在 §7 登记。**WM-0009/WM-0010 检查表（原 §8）已于 2026-11-09 静态闭合 8/8 项（S1–S8，基线 3082ab50），残留收敛为 RT1–RT4 运行时测量，BLOCKED 原因见 §8.2；对 Phase 2 Bridge 的实现约束已导出（§8.3）。** 上游版本对照（官方 kbengine 2.5.10 与本 fork 的差异）不在本次范围，若切换目标引擎版本必须重跑本审计。
