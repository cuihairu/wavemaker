# wavemaker Swells 页重设计 — 方向契约（2026-11-15 用户锁定）

THESIS: Swells 首页的海浪本源可视化——登录第一眼是全宽「海况带」叠积面积图（所有活动 run 的聚合 RPS 流），海面即总览；swell 表格每行左缘带小波形线呼应其在海面中的份额，点波段或行即下钻。拒绝的默认排布：无个性的表格堆 + 白底蓝链。

OWN-WORLD: 深色工作台基底（ground #0a141c / panel #0f1d28 / line #1d3344）；海色三阶（sea-deep #0b2239 / sea-mid #134e6f / sea-surface #1b7f9e）只用于海况带与波形线；accent #38bdf8 做游标/链接/选中；状态色 ok #34d399 / warn #fbbf24 / crit #f87171 独立于海色，只承载状态语义。字体系统栈（Segoe UI/system-ui），数字 tabular-nums。签名动作=海况带：时间游标 + 段落 hover 高亮 + 点击下钻。

STORY: 工程师打开页面 → 秒读「整体流量海面现在什么状态」（海面形状/高度/游标位置）→ 表格里看每条 swell 的份额与状态 → 点击进入 run 详情。空态（无运行）海面收成平静细波 + 中央「No active swells — launch from Spectrum」引导，不是死白板。

FIRST VIEWPORT: 1440×900——顶导航 52px（brand + 7 导航项 + 会话区）；海况带区（标题行 + 150px 全宽叠积 SVG + now 游标 + 图例/峰值摘要行）；swell 表（列：波形线/Swell/State pill/Runs/RPS now/p99/Errors/Started）；全部真实产品内容（checkout-swell/kb-baseline/login-storm 等示例数据标注 synthetic）。

FORM: 结构候选 #7 Waveform Status Band，DEALT INDICES 7,6,1（index 7 leads）。SEED REFERENCE: seed:f23f3eaa2de04f31。备选卡 6（单一大仪表）/1（运行墙）落 .impeccable/mocks/decision/ 可日后重访。FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.

工程边界（必须遵守）：
- Vue 3 + Vite，无 CSS 框架；现有 9 个 view 的路由与 data-test 属性**全部保留**（vitest e2e 依赖）
- 只动样式层与结构骨架：新增海况带组件（SwellListView 顶部），表格列扩充（RPS now/p99/Errors/Started 从 status API 可算）
- 空态/加载态/错误态三态都要设计（现有 error/loading/empty data-test 保留）
- 波形 sparkline 与海况带用内联 SVG，不引图表库；controller 不可达时错误卡显示重试按钮
- 英文 UI；深色单主题（本轮不做亮色）；键盘可达（表格行可 tab + enter 进详情）
