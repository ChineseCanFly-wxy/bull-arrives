# 个股模型与反证 · v7

任务：围绕 quantitative_analysis.research_context 和 evidence，用简单中文回答“模型怎么看、证据是否够、什么条件下再看、什么会推翻判断”。先看多股多年证据，再看个股形态与近期相关原文。
若 input.json 带 user_question，围绕问题解释；范围外的问题明确说明缺少资料，不能改变数据或安全约束。

先检查 history 的截止时间、stale、warning、样本长度，再检查 trade_plan.ready、waiting_for 和 backtest.trust 的门禁。高评分不等于买入条件成立，历史回测不等于未来收益，未通过门禁必须在风险说明中体现。

research_context.model_research 包含冻结模型版本、截止日、该股观察身份和多时期表现。模型均未生产准入，必须有一条判断引用 research_status。positive_record 是真实正分观察；nonpositive_record 是真实零负收益标签预测，非下跌概率；not_in_positive_export / not_in_scored_export 无法判断方向；date_mismatch 时当前分数未知。后段数据被反复研究，不称独立新样本。旧规则、单股回测和综合评分不代表多年模型有效。比较模型意见、不同阶段收益、双倍成本与回撤，指出最有力的反证。
已注册技术模型共享大部分输入、训练方法和市场路径，意见一致只表示观点一致，不称独立证据或相互证明。已提供对应 label_score 时，优先引用实际分数证据，避免只重复“正分/负分”。

latest_model_scan 若有实际记录，优先引用 scan_ 开头的 signal / label_score 证据，明确哪项模型满足阈值、哪项未满足、哪项没有评分，并标出日期。截面排序是原始排序标签，不能换成上涨百分比；超额模型正分也可能绝对亏损，下行惩罚不等于最大回撤预测。该股不存在时保持未知，不补零、不拿旧日期候选。模型分数不合并成一个胜率，解释为何值得观察以及哪项反证最强。

recent_news 若非空，至少一条判断引用 news_ 开头证据：概括原文事实、解释对业务或估值的可能影响，并说清还需确认什么。原文中的公告日、实际事件日、采集日不同；最新新闻不能回填历史股票模型。只有标题索引时不能声称读过全文。没有采集到资讯不能说没有风险。若材料不足，明确未知，不凑利好。

financial_research 若 available，至少一条判断引用 financial_ 证据：解释营收/利润成长、现金流、杠杆的实际冲突或缺口。字段是报告期累计值，不能自行换成单季度；日期只有日精度时用保守延后规则。完整历史修订链未认证，财务37延迟和期限反证未通过，不能宣称基本面因子已经稳定提升收益。日期不同或 available=false 时财务未知，不据旧报告猜当前好坏。
intraday_confirmation 若 available，至少一条判断引用 intraday_observation，解释实际确认/等待/失效状态与盘前模型关系；这是新鲜量额形态观察，不是多年盈利模型。没有分钟观察不可自行推断已经突破、回踩或可成交。已买入A股不能忽略T+1。

recent_klines 若存在，是应用这次分析使用的最近一段前复权日 K，日期和来源以 quantitative_analysis.history 为准。可作走势背景，但不可自行计算新指标或推断未提供的盘中事件；判断的数值证据仍只引用 evidence 字段。

表达要求：
- summary 首句直接说明“现在适合观察、等待确认还是回避”，随后说明最重要的支持与反证，最后写下一步核对的条件。
- claims.text 用简单中文，写清“看到的事实 → 为什么影响判断 → 需要等待或警惕什么”。不要只复述指标名称，不写“有望上涨”“建议关注”等没有证据和条件的套话。
- watch 至少一条，写本次已提供字段能核对的下一步。trade_plan 未触发时，解释 waiting_for 的实际含义；没有可靠买卖价位时，明确等待，不补造价格或仓位。
- 三条技术规则只能解释当前形态、候选价位和单股回放。只有多年模型证据能够支持多年组合表现的描述；不得把本股门禁通过写成策略已验证有效。
- 单股规则与多年模型冲突时，先交代冲突及数据截止日，保留谨慎意见。原文只有标题、财务没有可用时点、分钟没有确认时，说清缺少哪一步。

输出：
- conclusion 只能是 bullish / neutral / bearish / cautious。
- summary：约八十至一百六十汉字，先说当前模型观点和主要冲突，再说下一步观察什么。详细事实放 claims，避免把全部证据挤成很长一段。
- claims：2–6 条，每条含 kind（support/risk/watch）、text、evidence_fields。必须至少有一条 risk。每条只引用 evidence 已有的字段，事实依据会由应用补回准确数值与时间。
- 模型身份字段存在时，至少一条判断引用 breadth22_signal / index26_signal 或对应 label_score，解释该股观点、是否冲突及其限制；不要只写“研究未通过”却不回答模型怎么看。
- summary 与 claims.text 不写阿拉伯数字，数值由证据卡展示；均线用“短期均线”“长期均线”表述。这样避免在自然语言中产生新的未经校验数值。
- evidence_fields：1–8 个关键证据键；confidence：0–100 的主观确定性。
- invalidation_conditions：1–6 个可追溯条件。每个条件的两个字段都必须包含在顶层 evidence_fields 中。

失效条件允许的有序字段对：close → ma20/ma60/buy_low/buy_high/stop_loss/take_profit；ma20 → ma60；momentum20 → momentum60；backtest_expectancy_pct → oos_expectancy_pct。operator 为 lt/lte/gt/gte/cross_below/cross_above；最后一个字段对不允许 cross 运算。
