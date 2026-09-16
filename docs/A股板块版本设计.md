# Bull Arrives A 股板块中心设计

状态：设计基线 v0.1

目标：在现有 A 股行情、自选股和全市场筛选能力之上，增加一个可独立使用的“行业/概念板块中心”，形成：

```text
板块排行 → 板块详情 → 成分股 → 加入自选
```

本设计借鉴“市场魔方助手”的信息组织方式，不复用其私有接口、页面代码或未确认的数据供应商。

## 1. 现状与边界

### 1.1 已有能力

- 实时自选股：腾讯证券为主、新浪财经为备用。
- 全市场快照：东方财富为主、新浪财经兜底，已有缓存、分页、限流和陈旧数据标记。
- 已有的 `Board` 只表示交易所/代码市场分类：沪市主板、深市主板、创业板、科创板、北交所等。
- SQLite 已保存自选股、分组、持仓、提醒和行情缓存。

相关实现：

- `src-tauri/src/datasource/eastmoney_universe.rs`
- `src-tauri/src/commands/universe.rs`
- `src/components/screener/UniverseScreenerDialog.vue`
- `src/components/layout/TopBar.vue`

### 1.2 必须区分的两个“板块”概念

| 概念 | 当前状态 | 含义 |
|---|---|---|
| 市场板块 | 已有 | 沪主板、深主板、创业板、科创板、北交所 |
| 行业/概念板块 | 待新增 | “半导体”“新能源”“机器人”等成分关系 |

不能用股票代码前缀推导行业/概念成员关系。行业和概念成员关系来自外部数据源，并且会发生变化。

### 1.3 非目标

一期不做：

- 全球市场和美股板块；
- 小程序私有接口逆向或接口转发；
- 资讯、产业链图谱、AI 投资建议；
- 资金流、涨停原因等需要额外供应商确认的数据；
- 将板块数据混入 `watchlist` 表。

## 2. 一期产品范围

### 2.1 入口

在主窗口顶栏增加“市场板块”入口，打开懒加载的板块中心对话框。不要把所有板块表格直接塞进主页面，避免压缩自选股和详情面板的空间。

### 2.2 板块排行

提供两个切换页：

- 行业板块；
- 概念板块。

表格首版字段：

| 字段 | 说明 |
|---|---|
| 排名 | 当前排序下的序号 |
| 板块名称 | 展示名称 |
| 板块代码 | 内部稳定主键，接口请求优先使用代码 |
| 最新价 | 板块指数最新值 |
| 涨跌幅 | 相对上一交易日收盘 |
| 成交额 | 元，统一转换为 UI 展示单位 |
| 换手率 | 板块口径的换手率 |
| 上涨家数 | 数据源直接提供时展示 |
| 下跌家数 | 数据源直接提供时展示 |
| 领涨股票 | 名称及涨跌幅 |
| 数据时间/来源 | 明示新鲜、陈旧及来源 |

默认按涨跌幅降序；允许按涨跌幅、成交额、换手率排序，并支持名称搜索。

一期不强行展示没有可靠字段来源的“涨停家数”。如果后续通过成分股快照计算，必须标明计算口径和更新时间。

### 2.3 板块详情

点击板块后展示：

- 板块摘要：名称、代码、涨跌幅、成交额、上涨/下跌家数、领涨股；
- 成分股表格：代码、名称、最新价、涨跌幅、成交额、换手率；
- 操作：查看个股详情、加入自选；
- 预留历史行情区域，二期接入板块日 K。

成分股默认按涨跌幅降序，最多展示数据源允许的首批结果；如需完整列表，使用明确的分页，不在一次 IPC 中传输全部成员。

## 3. 数据源方案

### 3.1 选择

一期采用东方财富板块数据的 Rust 适配层，复用当前项目已经验证过的 HTTP client、请求超时、域名轮换、错误分类和陈旧数据策略。

AKShare 官方接口目录和源代码可作为字段及接口语义的对照：

- `stock_board_industry_name_em`：行业板块列表及排行字段；
- `stock_board_concept_name_em`：概念板块列表及排行字段；
- `stock_board_industry_cons_em` / `stock_board_concept_cons_em`：成分股；
- `stock_board_industry_hist_em` / `stock_board_concept_hist_em`：板块历史行情。

官方源代码中已展示这些接口的字段映射和东方财富页面入口：[行业板块适配器](https://github.com/akfamily/akshare/blob/main/akshare/stock/stock_board_industry_em.py)、[概念板块适配器](https://github.com/akfamily/akshare/blob/main/akshare/stock/stock_board_concept_em.py)。

### 3.2 不把 AKShare 运行时放进一期桌面包

一期不新增 Python、pandas 或 AKTools sidecar。原因：

- 高频板块排行需要和现有 Rust 调度、超时、缓存直接协同；
- 桌面分发不应因为低频板块查询额外携带 Python 运行时；
- AKShare 自身提示上游接口可能变化，不能把它当作稳定 SLA；
- 成分股、财务、资金流等长尾能力可以在验证后单独评估 AKTools。

如果直接 Rust 适配层维护成本变高，再把 AKTools 作为可选本地服务，而不是让 UI 直接依赖外部 Python 服务。

### 3.3 数据层分工

```text
板块目录/排行      → 东方财富板块列表接口       → 60 秒缓存
板块成分股          → 点击板块后按代码请求       → 30 秒缓存
板块历史 K 线       → 点击历史区域后按代码请求   → 5 分钟缓存
个股实时行情        → 复用现有 QuoteCache        → 不新增逐股轮询
```

板块排行优先使用数据源直接提供的板块统计字段，不为了计算排行而对每个板块逐一请求成分股。

## 4. 后端设计

### 4.1 新增模块

建议新增：

```text
src-tauri/src/datasource/sector.rs   # 东方财富板块请求、解析、缓存
src-tauri/src/commands/sector.rs     # Tauri IPC 命令
src/types/sector.ts                  # 前端类型
src/stores/sector.ts                 # 前端状态
src/components/sector/SectorDialog.vue
src/components/sector/SectorTable.vue
src/components/sector/SectorDetail.vue
```

不要把行业/概念解析逻辑继续堆进 `eastmoney_universe.rs`；全市场快照和板块成员是不同的数据生命周期。

### 4.2 类型

```text
SectorKind = industry | concept

SectorSummary {
  kind, code, name, rank,
  latest, change_amount, change_pct,
  amount, turnover_rate,
  up_count, down_count,
  leader_code, leader_name, leader_change_pct,
  as_of, source, stale
}

SectorMember {
  code, market, name,
  price, change_pct, amount, turnover_rate,
  as_of, source, stale
}

SectorHistoryBar {
  date, open, close, high, low,
  volume, amount, change_pct
}
```

`source` 和 `stale` 必须随响应返回，不能只写日志。数据不新鲜时，界面显示“陈旧数据 + 时间”，不伪装成实时行情。

### 4.3 IPC

建议命令：

```text
get_sector_summaries(kind, page, page_size, force_refresh)
get_sector_members(kind, code, page, page_size, force_refresh)
get_sector_history(kind, code, period, begin, end, force_refresh)
```

约束：

- `kind` 只接受 `industry` 或 `concept`；
- `code` 必须来自已加载的板块目录，不能直接把任意用户输入拼进 URL；
- `page_size` 限制在 20/50/100；
- 请求失败时返回上次成功结果并设置 `stale=true`，首次无缓存才返回错误；
- 同一 `kind + code + period` 同时只允许一个在途请求；
- 解析空数据、字段缺失、HTTP 非 2xx 都要区分记录。

### 4.4 缓存

一期使用进程内缓存：

| 数据 | TTL | 说明 |
|---|---:|---|
| 行业/概念目录与排行 | 60 秒 | 一种板块类型一轮请求 |
| 成分股第一页 | 30 秒 | 按板块代码缓存 |
| 历史 K 线 | 5 分钟 | 按板块代码、周期、日期范围缓存 |

一期不增加数据库表。若后续要求重启后仍显示板块收盘数据，再新增独立 `sector_cache` 表，不复用个股 `quote_cache` 的表意。

## 5. 前端交互与性能

### 5.1 交互闭环

```text
顶栏“市场板块”
  ↓
行业/概念排行（加载、刷新、搜索、排序）
  ↓ 点击板块
板块摘要 + 成分股
  ↓ 点击个股
复用已有个股详情
  ↓ 点击“加入自选”
复用 watchlist.addStock()
```

### 5.2 请求预算

- 打开板块中心：最多 2 个排行请求（行业、概念按需加载，默认只加载当前页）；
- 切换排序不联网，排序使用已加载数据；
- 点击板块才请求成分股；
- 成分股行情优先使用板块接口已经返回的字段，不对每只股票追加实时请求；
- 个股详情仍走现有 `QuoteStore` 和详情接口；
- 失败时保留上次数据，并在表头显示来源与时间。

## 6. 分阶段交付

### P0：数据契约与解析验证

- 固化行业、概念、成分股、历史 K 线的脱敏 JSON fixtures；
- 单测覆盖正常响应、空响应、字段缺失、错误状态、代码映射；
- 增加手动联网冒烟命令，不放入默认单测；
- 先验证 5 个行业和 5 个概念板块的名称、代码、涨跌幅和领涨股。

### P1：排行 MVP

- `sector.rs` + `commands/sector.rs`；
- 行业/概念排行表；
- 缓存、刷新、陈旧标记和来源标记；
- 顶栏入口和对话框；
- 目标：能稳定完成“打开 → 排序 → 刷新 → 看到数据时间”。

### P2：板块详情闭环

- 成分股分页；
- 个股详情复用；
- 加入自选；
- 目标：完成“板块排行 → 成分股 → 自选股”。

### P3：历史与派生指标

- 板块日 K / 周 K；
- 明确口径后增加涨停家数、连板统计等派生字段；
- 只有在成员快照、涨跌停规则和时间一致性验证通过后才展示。

### P4：增强数据

- 资金流、板块异动、涨停原因、资讯；
- 评估 AKTools 或授权数据供应商；
- 为每个增强字段保留数据源、更新时间和许可边界。

## 7. 验收标准

### 功能

- 行业和概念两个分页均可加载、搜索、排序和手动刷新；
- 点击板块可看到成分股并分页；
- 成分股可打开已有个股详情并加入自选；
- 切换回自选股、筛选器和设置不会丢失板块页面状态；
- 网络失败不会清空上次成功数据。

### 诚实性

- 每个结果显示数据时间和来源；
- 陈旧数据明确标记；
- 未取得“涨停家数”时不显示伪造的 0；
- 不把板块排行声明为交易所官方评级或投资建议；
- 不声称已经识别“市场魔方助手”的底层供应商。

### 工程

- `cargo test --lib --locked` 通过；
- `npm run build` 通过；
- 解析 fixtures 单测通过；
- 手动冒烟覆盖东财可用、东财失败后陈旧缓存、无缓存失败三种状态；
- 正常网络下排行首屏目标小于 3 秒，详情首屏目标小于 5 秒；最终以实际测量结果为准。

## 8. 当前决定

先实现 P0/P1/P2，不接入小程序私有接口，不引入 Python sidecar，不做全球市场。P3 的板块历史行情和派生指标仍保持后续阶段，避免在首版把未验证的统计口径展示给用户。

默认产品定位是：

> Bull Arrives 是 A 股实时看盘 + 板块轮动观察工具；板块数据服务于可追溯的行情观察和自选管理，不替用户生成不可验证的投资结论。

## 9. 当前 MVP 实施状态

当前 `codex/ashare-sector-mvp` 分支已落地：

- P0：行业、概念和成分股响应解析器、字段缺失/空响应/代码校验单测；
- P1：顶栏“市场板块”入口、行业/概念排行、搜索、分页、表格排序、刷新、来源与陈旧标记；
- P2：板块摘要、成分股分页、加入自选，以及复用现有个股详情面板的入口。

当前实现仍保留以下边界：

- 直接 Rust 请求东方财富公开板块接口；AKShare 只作为接口语义和字段映射的参考，不作为桌面运行时依赖；
- 成分股行情来自板块接口返回的快照，不为每只股票额外发起实时轮询；
- `source`、`as_of`、`stale` 随 IPC 返回，网络失败且存在缓存时展示陈旧数据；
- 目前未把实时联网冒烟纳入默认测试，东财接口受网络出口、频控和交易时段影响，需在可用网络下单独验收。

参考：

- [AKShare 官方仓库](https://github.com/akfamily/akshare)
- [AKShare 官方接口目录](https://github.com/akfamily/akshare/blob/main/docs/tutorial.md)
- [行业板块官方适配代码](https://github.com/akfamily/akshare/blob/main/akshare/stock/stock_board_industry_em.py)
- [概念板块官方适配代码](https://github.com/akfamily/akshare/blob/main/akshare/stock/stock_board_concept_em.py)
