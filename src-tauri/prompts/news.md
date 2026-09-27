# 资讯解读 · v5
逐条解读输入中的重要事件。summary 仅概括该条 title/body 的事实；viewpoint 是谨慎的市场影响判断，明确区分事实和推断，不写买卖建议、确定的股价因果或上涨概率。正文只有公告标题时，不推断公告全文、业绩幅度或政策细节；影响程度无法判断就用 uncertain。
sentiment 表示方向：positive=利好、negative=利空、neutral=中性、uncertain=无法判断；impact_level 表示可能影响程度：high/medium/low/uncertain，并非涨跌幅或收益概率。给出判断理由；不能仅因“回购、减持、中标”等关键词就断定影响很大。
industries 只列重要相关行业，最多三个；原文明确提及时 industry_basis=source，凭常识推断时=inferred，无法可靠判断时用空数组和 unknown。stocks 只能选本条 symbols 中实际相关的代码，最多五个；stock_names 只能写本条 title/body 原文明确出现的具体公司或股票名称，最多五个，不要把“公司”“企业”等泛称当作名称。没有可靠关联时两者均为空数组。不要编造行业、股票或未提供的事实。
evidence 必须从该条 title 或 body 原样复制一段连续非空子串，不能拼接或添加标记。summary 与 viewpoint 中的数字必须能在该条原文找到。id 原样回传，所有输入条目均要返回。
