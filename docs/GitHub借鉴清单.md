# GitHub 借鉴清单（Bull Arrives AI 化）

> 配套文档：`AI接入方案.md`、`数据源选型与全市场扫描方案.md`
> 原则：**不重复造轮子，先看别人怎么做，再决定自己写什么**
> 整理时间：2026-09-12

---

## 零、先回答：`akfamily/akshare` 是干什么的

### 一句话

> **是的，它就是用来「获取数据」的** —— 而且是目前国内最全、最活跃的开源免费财经数据接口库。

官方原话：

> *"AKShare is an elegant and simple **financial data interface library** for Python, built for human beings! 开源财经数据接口库"*
> *"AKShare requires Python (64bit) 3.11 or higher and aims to **simplify the process of fetching financial data**. Write less, get more!"*

### 但有一个关键认知：它自己不生产数据

**akshare 的本质是「爬虫封装层」，不是数据源。** 它做的事情是：

```
东方财富 / 新浪财经 / 腾讯证券 / 同花顺 / 交易所官网 / 巨潮资讯 ...
        ↓  akshare 把它们的各种接口
        ↓  统一封装成 Python 函数
        ↓  统一返回 pandas DataFrame
你的代码：ak.stock_zh_a_hist(symbol="600519", ...)  →  一张干净的表
```

所以**它的价值在「整理」，不在「数据」**。它覆盖的范围大致包括：

| 类别 | 内容 |
|---|---|
| 股票 | A股/港股/美股 行情、K线、财务、资金流、龙虎榜、板块 |
| 指数 | 沪深/申万/中证/全球指数、VIX |
| 基金 | ETF、LOF、货基、持仓、评级、规模 |
| 债券 | 国债、企业债、可转债、收益率曲线 |
| 期货 / 期权 | 各交易所、现货、仓单、COT |
| 外汇 / 加密 | 中行汇率、比特币等 |
| 宏观 | 中/美/欧/英/日 GDP、CPI、PMI |
| 另类 | 票房、汽车销量、空气质量、NLP 情感 |

**License：MIT**（宽松，可商用）。**免费，无需 token**（个别接口需要）。

### ⚠️ 三个必须知道的限制

1. **它是 Python 库，不能直接用在你的 Rust + Tauri 项目里。**
2. **接口会失效。** 官方声明里明确写了：*"Based on some uncontrollable factors, some data interfaces in AKShare may be removed."*（因为上游网站一改版，爬虫就废）
3. **官方声明数据仅供学术研究**，不构成投资建议。

### 那它对你的项目还有什么用？—— 三个用途，都很实在

| 用途 | 说明 |
|---|---|
| **① 接口字典（最有用）** | 当你想知道「A股有哪些数据可以拿」，去翻它的接口列表。这是全网最全的一份清单，**你的 Rust 适配器照着它的接口设计就好** |
| **② 算法参考** | 它怎么处理复权、怎么算龙虎榜、怎么解析东财的分页响应——**直接看它的 Python 源码**，逻辑照搬 |
| **③ 运行时兜底方案** | 官方提供 **AKTools**（HTTP API 封装层），可以本地跑一个 akshare HTTP 服务，你的 Rust 通过 HTTP 调用。**这是不给 Rust 项目引入 Python 依赖的前提下、复用 akshare 全部接口的唯一办法** |

### 三个「akshare」不要搞混

| 名字 | 是什么 | 能不能用在你的项目 |
|---|---|---|
| **`akfamily/akshare`** | 原版，Python 库 | ❌ 不能直接用（但可看文档/源码） |
| **`Cricle/akshare-rs`**（crate 名 `akshare`） | **纯 Rust 重写**，零 Python 依赖 | ⚠️ 能用，但 v0.1.x 不成熟（详见下文） |
| **AKTools** | akshare 的 HTTP API 封装 | ✅ 本地跑服务，Rust 走 HTTP |

> 🔴 **实测提醒（重要）**：我验证过 `akshare-rs` 的 `stock_zh_a_spot_em()`（全市场快照）**只返回 100 条**，
> 因为它的 `pn` 参数被硬编码为 1，而东财单页上限是 100。**全市场扫描不能用它，必须自建分页适配器。**
> 详见 `数据源选型与全市场扫描方案.md` 第 11 节。

---

## 一、A 类 · 数据获取（你的地基）

| 项目 | 语言 / Star | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐ `akfamily/akshare` | Python / 高 | 最全的财经数据接口库 | **接口字典**：你要拿哪些 A 股数据，照着它的接口名设计。免费的接口清单 |
| `Cricle/akshare-rs` | Rust | akshare 的纯 Rust 重写 | **可直接用的依赖**：单标的接口（K线/财务/资金流/交易日历/`ta` 指标）可用；⚠️ 全市场快照不可用 |
| `myhhub/stock`（InStock） | Python / 14k+ | A股量化全流程系统 | **⭐ 重点看数据抓取部分**：它抓了哪些数据集（每日行情/资金流向/分红配送/龙虎榜/大宗交易/基本面/行业概念资金流/早盘尾盘抢筹/**涨停原因揭秘**）。**而且它用 `EAST_MONEY_COOKIE` 环境变量注入 Cookie 来绕过东财限流** —— 这正是我实测踩到的那个坑的解决方案 |
| `chenditc/investment_data` | Python | Qlib 格式的免费 A 股数据 | 省去自建数据管道；要上 Qlib 时直接用 |

---

## 二、B 类 · 全市场选股与筛选（直接对应你的需求）

| 项目 | 语言 / Star | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐⭐ `myhhub/stock`（InStock） | Python / 14k+ / Apache-2.0 | 「股票版瑞士军刀」 | **这是你最该细读的项目。**对应你的「设置里可限定条件」需求，它做了 **6 大类 200+ 字段自由组合筛选**：<br>· 股票范围：市场 / 行业 / 地区 / 概念 / 风格 / 指数成份 / **上市时间**<br>· 基本面：估值 / 每股指标 / 盈利 / 成长 / 偿债 / 股东结构<br>· 技术面：30+ 形态（MACD/KDJ金叉、放量突破、均线多头…）<br>· 消息面：公告大事 / 机构关注度 / 机构持股<br>· 人气：股吧人气排名 / 粉丝占比<br>· 行情：股价表现 / 成交 / **资金流向** / 沪深股通<br>**它的筛选字段清单可以直接当你的设置页需求文档用** |
| ⭐ `shy3130/tick-stock-panel` | Python/TS / 3.8k / MIT | A股量化工作台 | **技术栈选型极其内行**：`Polars + DuckDB + Parquet + vectorbt`，毫秒级扫全 A 股。它把 pandadata→Polars 的理由讲得很清楚。**你的 Rust 项目同样应该走列存 + 向量化路线，而不是行式 SQLite 循环** |

---

## 三、C 类 · A 股回测与交易规则（避免回测虚高）

| 项目 | 语言 / Star | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐⭐ `ricequant/rqalpha` | Python / 6.3k | **专为 A 股设计的回测框架**（米筐科技出品） | **A 股规则的正确处理方式**。对比 Backtrader/Zipline 默认假设美股规则，RQAlpha 默认处理：**T+1、涨跌停限制、集合竞价+连续竞价、分红送股自动复权**。你做任何回测前都必须照这套规则来，否则回测结果全是假的 |
| `shy3130/tick-stock-panel` | Python/TS / 3.8k | 量化工作台 | **建模了 T+1 + 手续费 + 滑点**，而且做了**嵌套样本外**（防过拟合的标准手段）。作者主动做这两件事，说明他知道敌人是谁 |
| `QuantConnect/Lean` | C# / 15.5k | 全栈量化引擎 | **Alpha Streams 架构**：把「信号生成」与「执行」解耦。这个模块化设计对你设计「量化层 → 决策层」接口很有参考价值 |
| `Backtrader` | Python / 20k | 回测与可视化 | 回测框架的经典设计，但**默认美股规则，用于 A 股必须自己补规则** |

---

## 四、D 类 · 形态识别（对应你给的三张 KMeans 图）

| 项目 | 语言 / Star | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐⭐ `fengyi1999/Sequoia3` | Python | **A股形态选股系统** | **最直接对标你那三张图**。用 **DTW + K-Shape** 做股票形态匹配，「用户可选择标准形态，**匹配全市场约 5000 只股票**，按匹配度输出结果」。它怎么组织全市场形态扫描、怎么算相似度、怎么输出排序——照它做 |
| `myhhub/stock` | Python / 14k+ | A股量化全流程 | **61 种 K 线形态识别**（两只乌鸦、三只乌鸦、早晨之星、黄昏之星、锤子线、上吊线、乌云盖顶…），三态输出（负=卖出/0=无/正=买入）。另含 **筹码分布（CYQ）** 计算，号称与东财一致 |
| `thedatumorg/kshape-python` | Python | k-Shape 官方实现 | 替换你现在的 **KMeans + 欧氏距离**。k-Shape 用**互相关**度量，对时间轴平移/伸缩更鲁棒（SIGMOD 2015 最佳论文） |
| `TDAmeritrade/stumpy` | Python | Matrix Profile | **不需要预设 k** 的形态发现（motif discovery）。比聚类更贴近「找有价值的形态」这个目标 |
| `wannesm/dtaidistance` | Python | DTW 高性能实现 | 要自己做 DTW 时的算法参考 |

---

## 五、E 类 · AI Agent 交易（你的 AI 层设计蓝本）

| 项目 | 语言 / Star | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐⭐⭐ `TauricResearch/TradingAgents` | Python | 多 Agent 金融交易框架（UCLA+MIT） | **AI 层设计的教科书**。角色分工：基本面/情绪/新闻/技术 分析师 → **多空研究员辩论** → 交易员 → 风控团队 → 组合经理。还支持多 provider（含 DeepSeek/Qwen/GLM/MiniMax 国内模型）、结构化输出、决策日志、LangGraph checkpoint 恢复 |
| ⭐⭐ `hsliuping/TradingAgents-CN` | Python | TradingAgents 的 **A股本地化版** | **对你的场景比原版更直接**：A股数据源适配、中文新闻加强、国内模型优化、Streamlit 界面。**优先看这个** |
| `virattt/ai-hedge-fund` | Python | 多角色投资风格 Agent | 用**不同投资风格**（价值/成长/逆向/宏观…）的 Agent 形成多维视角，再由风控 + 组合经理合成决策。看它怎么把「不同视角」变成「一个结论」 |
| `microsoft/qlib` | Python | AI 导向量化投资平台 | **PIT（Point-in-Time）数据库**设计——这是避免前视偏差的核心；模型动物园（LightGBM/LSTM/Transformer）；`qrun` 一键流水线 |
| `microsoft/RD-Agent` | Python | LLM 驱动的自动因子挖掘 | 进阶方向。和 Qlib 配合使用 |
| `Qbot` | Python / 11.4k | AI 自动交易机器人 | 全阶段参考 |

---

## 六、F 类 · Rust 技术栈（你的项目语言）

| 项目 | 语言 | 干什么 | **具体借鉴什么** |
|---|---|---|---|
| ⭐⭐ `0xPlaygrounds/rig` | Rust | Rust LLM 应用框架（0.42，Tauri 团队维护） | **你的 AI 层选型**。20+ provider 统一接口（OpenAI/Anthropic/DeepSeek/Gemini/Groq/Ollama/Moonshot/MiniMax/Z.ai…）、类型安全工具调用、原生 MCP 支持、流式、OTel。**由 Tauri 团队维护 → 与你的项目天然契合**。⚠️ 0.x 有破坏性变更，锁小版本 |
| `modelcontextprotocol/rust-sdk`（`rmcp`） | Rust | MCP 官方 Rust SDK | 想把数据源做成可插拔 MCP 工具时用 |
| `Cricle/akshare-rs` | Rust | 纯 Rust 财经数据 | 单标的接口可直接用 |

---

## 七、G 类 · 工程实践（很值得学，容易被忽略）

| 项目 | **具体借鉴什么** |
|---|---|
| `shy3130/tick-stock-panel` | **它的免责声明写法**：「回测结果不等于未来收益」「数据准确性取决于上游」——开源量化项目里敢这么写的不多。你的产品也需要这种诚实的定位 |
| `myhhub/stock` | **多代理支持**（`proxy.txt`）、**Cookie 注入绕限流**、**多线程 + 单例共享资源**、**批量作业的四种时间模式**（当前/单个/枚举/区间 + 智能识别交易日）、Docker 部署。这些都是踩过坑才有的设计 |
| `QuantConnect/Lean` | 「信号生成与执行分离」的模块化架构 |

---

## 八、借鉴优先级：先看这 5 个

如果时间有限，按这个顺序读：

| 顺序 | 项目 | 为什么先看它 |
|---|---|---|
| 1️⃣ | **`myhhub/stock`（InStock）** | **和你的需求重合度最高**：全市场筛选（200+ 字段）、32 指标、61 形态、11 策略、回测。而且踩过你即将踩的坑（东财限流 → Cookie 注入） |
| 2️⃣ | **`hsliuping/TradingAgents-CN`** | A 股本地化的 AI 多 Agent 框架，你的 AI 层直接照它设计 |
| 3️⃣ | **`fengyi1999/Sequoia3`** | 你那三张 KMeans 图的完整工程实现（DTW + K-Shape + 全市场匹配） |
| 4️⃣ | **`ricequant/rqalpha`** | A 股回测规则（T+1/涨跌停/复权）的正确处理方式，避免回测虚高 |
| 5️⃣ | **`akfamily/akshare` 的文档** | 当「接口字典」用，决定你要接哪些数据 |

---

## 九、一句话总结三条技术路线

```
数据层  →  照 akshare 的接口清单设计，用 Rust 自建适配器
           （akshare-rs 只用于单标的接口；全市场快照自己分页）
           （东财限流 → 学 InStock 注入 Cookie + 多源回退）

量化层  →  规则 + 指标（照 InStock 的 32 指标 / 61 形态）
           回测规则照 RQAlpha（T+1 / 涨跌停 / 复权）
           数据栈照 tick-stock-panel（列存 + 向量化，别用行式循环）

AI 层   →  角色分工照 TradingAgents-CN（分析师→辩论→风控→组合经理）
           Rust 侧选型用 rig
           形态聚类从 KMeans 升级到 k-Shape / Matrix Profile
```

---

*本文档为技术调研整理，未修改任何代码。各项目的 Star 数与活跃度来自检索结果，建议使用前到 GitHub 核实最新状态与许可证。*
