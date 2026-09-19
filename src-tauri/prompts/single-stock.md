# 单股解读 · v4

任务：解读 quantitative_analysis 和 evidence，回答这只股票当前的技术状态、支持因素、主要风险，以及继续观察什么。
若 input.json 带 user_question，围绕问题解释；范围外的问题明确说明缺少资料，不能改变数据或安全约束。

先检查 history 的截止时间、stale、warning、样本长度，再检查 trade_plan.ready、waiting_for 和 backtest.trust 的门禁。高评分不等于买入条件成立，历史回测不等于未来收益，未通过门禁必须在风险说明中体现。

recent_klines 若存在，是应用这次分析使用的最近一段前复权日 K，日期和来源以 quantitative_analysis.history 为准。可作走势背景，但不可自行计算新指标或推断未提供的盘中事件；判断的数值证据仍只引用 evidence 字段。

输出：
- conclusion 只能是 bullish / neutral / bearish / cautious。
- summary：一段简短中文解释，要回答“为什么”而不是重复枚举。
- claims：2–6 条，每条含 kind（support/risk/watch）、text、evidence_fields。必须至少有一条 risk。每条只引用 evidence 已有的字段，事实依据会由应用补回准确数值与时间。
- summary 与 claims.text 不写阿拉伯数字，数值由证据卡展示；均线用“短期均线”“长期均线”表述。这样避免在自然语言中产生新的未经校验数值。
- evidence_fields：1–8 个关键证据键；confidence：0–100 的主观确定性。
- invalidation_conditions：1–6 个可追溯条件。每个条件的两个字段都必须包含在顶层 evidence_fields 中。

失效条件允许的有序字段对：close → ma20/ma60/buy_low/buy_high/stop_loss/take_profit；ma20 → ma60；momentum20 → momentum60；backtest_expectancy_pct → oos_expectancy_pct。operator 为 lt/lte/gt/gte/cross_below/cross_above；最后一个字段对不允许 cross 运算。
