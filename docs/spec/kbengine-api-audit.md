# KBEngine Bot API 静态审计（WM-0007 / WM-0008 静态部分）

> 状态：Phase 0 静态审计完成；运行时验证清单见 §8（挂 WM-0009 / WM-0010）。
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

## 8. WM-0009 运行时验证清单（Bridge 执行安全性）

静态结论：**所有 Python 脚本回调（含 timer 回调、事件、onInit/onFinish）都跑在 bots 主循环线程**（`clientobjectbase.cpp:248-250` handleTimers 由 `tickSend` 驱动；`event_dispatcher.cpp:151,197` processTimers 在主 dispatcher 循环）。源码中**没有**把脚本调用投递到其他线程的路径。以下项仍必须运行时验证（WM-0009）：

1. **单线程假设成立性**：确认 Bridge IPC 接收线程不直接调用任何脚本 API（跨线程调用 Python 是未定义行为；必须经队列投递到主循环）——静态建议成立，运行时以日志/断言验证。
2. **回调内长时间阻塞的后果**：callback 内 sleep/IO 会阻塞整个 bots 网络循环（含心跳与收包）——测量可容忍阻塞上限（与 WM-0010 的队列滞后测量合并做）。
3. **回调异常传播**：timer 回调抛异常后当轮 tick、后续 timer、以及连接状态是否受影响（静态未展开异常路径）。
4. **GIL 与 telnet 调试通道并发**：telnet server（`bots.cpp:90-93`）共享脚本运行时，调试期是否引发额外 GIL 竞争。
5. **ScriptID 复用/溢出**（见 §4）。
6. **onInit 重入**（见 §5）：恢复/重启场景。
7. **`bots` 映射快照一致性**（见 §2）：遍历期间 bot 增减。
8. **addBots 节流参数实际行为**（见 §1）：tickCount/tickTime 与 `CreateAndLoginHandler` 的真实换算。

## 9. 结论

WM-0007/WM-0008 的静态部分闭合：六个 API 的签名、参数、调用时机、失败语义均有 file:line 级源码证据，7 项与计划书不符处以源码为准并已在 §7 登记。运行时部分（粒度实测、异常传播、线程投递安全性）保留在 WM-0009/WM-0010，清单见 §8。**上游版本对照（官方 kbengine 2.5.10 与本 fork 的差异）不在本次范围，若切换目标引擎版本必须重跑本审计。**
