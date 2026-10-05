<script setup lang="ts">
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
interface Period {net_return_pct:number;max_drawdown_pct:number}
interface Model {id:string;name:string;features:number|string;holding_days:number;train:Period;validation:Period;test:Period;double_cost_return_pct:number;win_rate_pct:number;closed_cycles:number;unique_stocks:number;baseline_percentile:{train:number;validation:number;test:number};status:string}
interface Evidence {run_id:string;as_of:string;data:{stocks:number;sessions:number;first_date:string|number;last_date:string|number;sha256:string;source:string};models:Model[];limitations:string[];next_checks:string[];integrity_note:string;content_sha256:string;annual_choices:unknown;stress:unknown;checks:unknown}
const evidence=ref<Evidence|null>(null);const busy=ref(false);const error=ref('');const notice=ref('');const view=ref('compare');const selectedId=ref('breadth22_h20');let generation=0;
const selected=computed(()=>evidence.value?.models.find(m=>m.id===selectedId.value));
const pct=(v:unknown)=>typeof v==='number'&&Number.isFinite(v)?`${v.toFixed(2)}%`:'--';
const json=(v:unknown)=>JSON.stringify(v,null,2);
async function load(){const request=++generation;busy.value=true;error.value='';try{const result=await invoke<Evidence>('get_research_evidence');if(request===generation){evidence.value=result;if(!result.models.some(m=>m.id===selectedId.value))selectedId.value=result.models[0]?.id??'';}}catch(e){if(request===generation)error.value=String(e);}finally{if(request===generation)busy.value=false;}}
async function importEvidence(){if(busy.value)return;busy.value=true;notice.value='';try{const path=await open({multiple:false,title:'导入冻结研究证据包',filters:[{name:'研究证据JSON',extensions:['json']}]});if(typeof path!=='string')return;const request=++generation;busy.value=true;error.value='';const result=await invoke<Evidence>('import_research_evidence',{path});if(request===generation){evidence.value=result;notice.value='完整性已校验并登记；没有创建账户或自动准入策略。';}}catch(e){error.value=String(e);}finally{busy.value=false;}}
onMounted(()=>void load());onBeforeUnmount(()=>{++generation;});
</script>
<template>
  <section class="evidence">
    <div class="head"><div><b>多年、多股的模型证据</b> <HelpTooltip label="多年模型证据使用说明">查看全部成功和失败的配置，先比较训练与验证，再看已观察后段、同池对照和成本。收益是各阶段独立模拟账户的累计收益，不能直接拼成复利。SHA校验只证明内部内容完整，导入不代表模型获准或可自动下单。</HelpTooltip></div><div><button :disabled="busy" @click="load">刷新证据</button><button :disabled="busy" @click="importEvidence">导入研究证据包</button></div></div>
    <p class="section-purpose">核对模型在训练、验证、后段与成本压力下的依据；判断回放是否可信时使用，刷新或导入只登记证据，不创建账户或订单。</p>
    <details>
      <summary>使用步骤：刷新与导入</summary>
      <p>刷新只重读已导入的证据；没有导入记录时读取程序内置包，不下载行情或重算研究。导入成功后保存该包，后续刷新优先读取它。</p>
      <ol>
        <li>进入本页会自动读取证据；需要重新读取时点“刷新证据”。</li>
        <li>有新的冻结证据包时点“导入研究证据包”，选择对应 JSON 文件；普通研究结果 JSON 不能直接当证据包导入。</li>
        <li>核对下方运行标识、截止日期和覆盖数量，再切换“策略比较”“数据覆盖”“实验与压力”。</li>
      </ol>
      <p>结果：成功或失败提示在本页顶部，已加载的指标和记录在下方三个子页。导入失败时按顶部原因检查文件。</p>
      <p>前提：查看可用内置包；导入须有可读取、格式兼容、不超过 1 MiB（约 1 MB）的探索性证据包，含正文与匹配的 SHA256，截止日期不能在未来。校验通过不等于收益或来源获认证。</p>
    </details>
    <p class="warning">研究观察 · 尚无模型获准用于生产选股。后段已经反复查看，属于回顾性探索；排序或收益代理值不代表上涨概率。</p>
    <p v-if="error" role="alert" class="error">{{error}}</p><p v-if="notice" role="status">{{notice}}</p><p v-if="busy&&!evidence" role="status">正在校验研究证据…</p>
    <template v-if="evidence">
      <div class="meta">{{evidence.run_id}} · 截至 {{evidence.as_of}} · {{evidence.data.stocks}} 个历史股票代码 · {{evidence.data.sessions}} 个市场交易日</div>
      <nav aria-label="研究证据页面"><button v-for="item in [{id:'compare',name:'策略比较'},{id:'data',name:'数据覆盖'},{id:'experiments',name:'实验与压力'}]" :key="item.id" :class="{active:view===item.id}" @click="view=item.id">{{item.name}}</button></nav>
      <template v-if="view==='compare'">
        <p class="section-purpose">比较不同阶段的扣费表现与同池对照；排查过拟合、回撤和成本敏感时使用。</p>
        <details>
          <summary>查看步骤与比较前提</summary>
          <ol>
            <li>先看训练与验证收益，比较同一机制的不同计划期限，保留失败配置一起看。</li>
            <li>点模型名称或所在行，在表格下方查看该配置的状态、输入数量、完成周期胜率和同池分位。</li>
            <li>再看后段回撤与双成本收益；有疑问时到“数据覆盖”核对口径，到“实验与压力”查看反证。</li>
          </ol>
          <p>结果：阶段收益在表格中，所选模型的详细说明在表格下方。点选只切换查看对象，不运行模型。</p>
          <p>前提：证据已加载，比较时核对日期、股票池、期限与费用口径。各阶段为独立模拟账户；后段已被观察，不能当新的样本外检验，也不能把阶段收益拼成连续复利。</p>
        </details>
        <p>训练 2020—2022；验证 2023—2024；后段 2025—{{evidence.as_of}}。费用、T+1、涨跌停和分红应收由回放引擎记录，未认证项见数据限制。</p>
        <div class="table-scroll"><table><thead><tr><th>模型 / 期限</th><th>训练收益</th><th>验证收益</th><th>后段收益</th><th>后段回撤</th><th>双成本收益</th><th>周期 / 股票</th></tr></thead><tbody><tr v-for="model in evidence.models" :key="model.id" :class="{selected:selectedId===model.id}" @click="selectedId=model.id"><td><button class="model" @click.stop="selectedId=model.id">{{model.name||model.id}} · {{model.holding_days}} 日</button></td><td :class="{negative:model.train.net_return_pct<0}">{{pct(model.train.net_return_pct)}}</td><td :class="{negative:model.validation.net_return_pct<0}">{{pct(model.validation.net_return_pct)}}</td><td>{{pct(model.test.net_return_pct)}}</td><td>{{pct(model.test.max_drawdown_pct)}}</td><td>{{pct(model.double_cost_return_pct)}}</td><td>{{model.closed_cycles}} / {{model.unique_stocks}}</td></tr></tbody></table></div>
        <div v-if="selected" class="selected-detail"><b>{{selected.id}} · {{selected.status}}</b><p>{{selected.features}} 项技术输入；计划 {{selected.holding_days}} 个交易日。完成周期胜率 {{pct(selected.win_rate_pct)}}。</p><p>同正分股票池五个固定排序种子中的数值分位：训练 {{pct(selected.baseline_percentile.train)}}、验证 {{pct(selected.baseline_percentile.validation)}}、后段 {{pct(selected.baseline_percentile.test)}}。种子共享同一行情，不是统计显著性或真实未来胜率。</p></div>
        <details>
          <summary>阶段指标与同池对照怎么读</summary>
          <p>净收益是扣费后账户净值相对起点的累计变化，不是年化收益；期末持仓估值和应收权益也可能计入，不能都当作已兑现现金。最大回撤是净值从此前高点到低点的最大跌幅，越大表示曾承受的亏损越深。</p>
          <p>双成本收益是提高费用与滑点后另一次回放的结果；成本会改变可买股数、入场和现金路径，所以收益不一定逐项下降。计划期限按交易日计，实际退出还受可成交性与权益锁定影响。</p>
          <p>周期数是已完成持有周期的数量，股票数是这些周期涉及的不重复股票数；完成周期胜率是其中盈利的比例，不包含未结持仓，也不是下一次上涨概率。技术输入数量表示模型使用多少项信息，不代表可靠程度。</p>
          <p>同池指同一模型分数大于零的候选池，五个固定种子只改变中性排序。分位越高，表示该阶段收益在五次对照中的位置越靠前；它们共享同段行情，不能当五份独立样本、统计显著性或未来胜率。</p>
        </details>
      </template>
      <template v-else-if="view==='data'">
        <p class="section-purpose">核对来源描述、覆盖日期、样本与数据限制；回放结果不一致或数据过期时使用。</p>
        <details>
          <summary>查看步骤与数据前提</summary>
          <ol>
            <li>核对本页来源、起止日期和顶部股票代码数、市场交易日数，确认是否覆盖要比较的期间。</li>
            <li>读数据限制，重点看历史 ST、成交代理价、分红到账及未结权益，排查收益口径差异。</li>
            <li>比对下方两个指纹，再展开“实际检查记录”查对应核验项。</li>
          </ol>
          <p>结果：来源与日期在下方，限制列表、数据/证据指纹及检查原文依次展示；这里不连接数据源补齐历史。</p>
          <p>前提：证据已加载；确认来源真实性和复现收益还需要原始行情、生成程序与账本，单凭本页描述或指纹不能完成认证。</p>
        </details>
        <p>{{evidence.data.source}} · {{evidence.data.first_date}} 至 {{evidence.data.last_date}}</p>
        <p>只研究沪深普通股；北交所全部剔除；历史按当日 ST 风险标记过滤，当前选股再排除最新 ST、*ST，不能用今天的身份倒填历史。</p>
        <ul><li v-for="line in evidence.limitations" :key="line">{{line}}</li></ul>
        <p class="hash">数据 SHA256：{{evidence.data.sha256}}</p>
        <p class="hash">证据 SHA256：{{evidence.content_sha256}}</p>
        <details><summary>指纹能证明什么</summary><p>SHA256 是内容指纹：数据指纹用于比对行情快照，证据指纹用于比对包正文。导入会重算正文指纹并检查是否匹配；数据指纹只检查格式，不重新读取原始行情验证。正文一致不代表发布者身份可信、来源已认证或收益计算正确。</p></details>
        <p>{{evidence.integrity_note}}</p>
        <details><summary>实际检查记录</summary><p>这是证据包内保存的检查结果，不是打开页面后重新执行的审计。true 表示该项记录为通过，false 表示未通过；数量表示本次覆盖，误差表示账本核对差额。可重点查现金非负、历史 ST/北交所违规数及未来数据变化是否影响过去信号。</p><pre>{{json(evidence.checks)}}</pre></details>
      </template>
      <template v-else>
        <p class="section-purpose">检查年度冻结选择、期限邻域、延迟和成本反证；决定是否需要进一步验证时使用。</p>
        <details>
          <summary>查看步骤与实验前提</summary>
          <ol>
            <li>看“年度开始前冻结选择”，核对目标年份与选择依据的历史年份，确认没有用当年结果挑配置。</li>
            <li>展开“期限邻域、延迟与成本压力”，比较同机制不同期限、延迟一天及双成本的变化。</li>
            <li>展开“仍需验证的事项”，结合失败结果判断还缺哪些新数据或独立核验。</li>
          </ol>
          <p>结果：下方折叠区展示包内实验原文与待核验列表；展开只查看记录，不启动实验或完成待核验项。</p>
          <p>前提：证据包包含对应实验记录，比较采用相同数据与执行口径。正收益和年度冻结都不能消除后段已被反复观察的局限。</p>
        </details>
        <p>配置及失败结果均保留，年度选择只允许使用该年之前的结果。15 日是研究偏好，不是保证退出的期限。</p>
        <details><summary>年度开始前冻结选择</summary><p>year 是目标年份，selection_years 是挑选配置所用的以前年份，selected_model 是冻结模型，style 是选择方式。收益和回撤按独立年度账户计算，不能连乘成真实复利；exposure_mean_pct 是平均有多少净值投入持仓，baseline_median_return_pct 是同池对照收益的中间值。</p><pre>{{json(evidence.annual_choices)}}</pre></details>
        <details><summary>期限邻域、延迟与成本压力</summary><p>holding_days 是计划持有交易日，signal_delay_days 是信号延迟的交易日数；train/validation/test 分别对应训练、验证和已观察后段，double_cost_return_pct 是双成本回放收益。修改这些条件就大幅变差，说明结果对执行条件敏感。</p><p>stock_concentration 记录盈利集中程度：前 5/10 只股票占已完成正利润的比例越高，盈利越集中。unclosed_pnl_share_total_net_pct 是未结持仓及应收盈亏占总净盈利的比例；应收是尚未到账的权益，锁股是尚不能卖出的新增股份，都不能当作已兑现收益。</p><pre>{{json(evidence.stress)}}</pre></details>
        <details><summary>仍需验证的事项</summary><p>这是后续研究清单；看到清单不代表已验证，通过导入也不会自动完成。</p><ul><li v-for="line in evidence.next_checks" :key="line">{{line}}</li></ul></details>
      </template>
    </template>
  </section>
</template>
<style scoped>
.evidence{font-family:"Microsoft YaHei UI","Microsoft YaHei","PingFang SC","Noto Sans CJK SC",var(--font-sans);font-size:14px;line-height:1.6}.head button{min-width:84px}.section-purpose{font-size:13px;color:var(--color-text-secondary)}.head{display:flex;justify-content:space-between;gap:12px;align-items:center;flex-wrap:wrap}.head button,nav button{margin:0 5px 5px 0;padding:6px 10px;border:1px solid var(--color-border-0);border-radius:5px;background:transparent;color:inherit;cursor:pointer}.warning{padding:10px;background:rgba(234,179,8,.09);border-left:3px solid #eab308}.meta{color:var(--color-text-secondary);margin:10px 0}nav{margin-bottom:14px}.active{border-color:#58a6ff!important}.table-scroll{overflow:auto}table{width:100%;border-collapse:collapse;white-space:nowrap}th,td{text-align:right;padding:8px;border-bottom:1px solid var(--border-color,#30363d)}th:first-child,td:first-child{text-align:left}.model{border:0;background:none;color:inherit;cursor:pointer}.selected{background:rgba(88,166,255,.10)}.negative,.error{color:#f85149}.selected-detail{padding:12px;background:rgba(127,127,127,.07);margin-top:14px}.hash{word-break:break-all;font-family:monospace}pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;max-height:420px;overflow:auto}details{margin:12px 0}li{margin:5px 0}
</style>
