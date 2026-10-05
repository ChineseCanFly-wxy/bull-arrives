<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import HelpTooltip from '../common/HelpTooltip.vue';
interface Comparison {name:string;holding_days:number;net_return_pct:number;max_drawdown_pct:number;validation_return_pct:number;tail_return_pct:number;double_cost_return_pct:number;cycles:number;unique_stocks:number;status:string}
interface Study {title:string;status:string;message:string;as_of?:string;new_usable_models?:number;cash_accounts?:number;native_matcher_accounts?:number;comparisons?:Comparison[];metrics?:Array<{name:string;net_return_pct:number;max_drawdown_pct:number;cycles:number}>;limitations:string[];source_sha256?:string}
const studies=ref<Study[]>([]);const error=ref('');let disposed=false;
onMounted(async()=>{try{const result=await invoke<{studies:Study[]}>('research_extension_report');if(!disposed)studies.value=result.studies;}catch(e){if(!disposed)error.value=String(e);}});
onBeforeUnmount(()=>{disposed=true;});
</script>
<template>
  <section class="extension-study">
    <h3>扩展研究报告</h3>
    <p class="section-purpose">核对新增方法的收益、回撤与失败对照；评估能否改进冻结模型时使用，当前结果不新增可用模型，也不触发模型交易。</p>
    <p class="section-purpose">展开读取内置历史报告。查看报告不启动计算或行情检查；报告截止日期不等于今天的行情日期。</p>
    <details>
      <summary>本页查看步骤</summary>
      <ol>
        <li>选下面的研究卡，先看状态、结论及已有报告的截止日期。</li>
        <li>展开对应的对照表和“数据范围与局限”，按相同口径比较。</li>
      </ol>
      <p>结果：研究结论和表格在各自卡片。报告随程序内置，用于核对历史研究依据。</p>
    </details>
    <p v-if="error" role="alert" class="error">{{error}}</p>
    <article v-for="study in studies" :key="study.title">
      <h4>{{study.title}} <HelpTooltip :label="`${study.title}研究口径说明`">{{study.limitations.join('；')}}</HelpTooltip></h4><b>{{study.status}}</b>
      <p class="section-purpose">{{study.title==='TSP公开技术规则对照'?'检查公开技术规则能否经受跨年与费用检验；比较规则时使用。':study.title==='多日序列形态与退出研究'?'检查多日形态、持有期限与退出方式是否稳健；核对替代方法时使用。':study.title==='公告时点财务增量37'?'检查同覆盖股票池增加财务信息是否改善结果；核对基本面增量时使用。':'检查行情追加、前向延续与送转/股息记账；追查数据衔接或权益挂起时使用。'}}</p>
      <div v-if="study.cash_accounts" class="study-meta"><span>{{study.cash_accounts}} 次现金账户实验<span v-if="study.native_matcher_accounts"> + {{study.native_matcher_accounts}} 次原撮合对照</span></span><span>截至 {{study.as_of}}</span><span class="study-state">新增可用模型 {{study.new_usable_models??0}}</span></div>
      <p>{{study.message}}</p>
      <details v-if="study.title==='TSP公开技术规则对照'">
        <summary>公开技术规则：查看步骤与前提</summary>
        <ol>
          <li>看本卡状态和实验覆盖；25 条已测试规则不等于 25 个有效模型。</li>
          <li>展开收益、回撤与跨年对照，先比较同期限的连续收益和 2023—2024 段，再看后段与双费。</li>
          <li>读“数据范围与局限”，核对原撮合与本项目撮合口径的差异。</li>
        </ol>
        <p>结果：本卡结论和折叠表中的代表规则；不生成可用模型或买入提醒。前提：只在相同股票池、日期、期限与费用口径下比较，原撮合另算，不能合并为相同收益。</p>
      </details>
      <details v-else-if="study.title==='多日序列形态与退出研究'">
        <summary>多日形态与退出：查看步骤与前提</summary>
        <ol>
          <li>看突破回踩、恐慌二次探底、放量后缩量守位三种机制的总体结论。</li>
          <li>展开对照表，按同一机制比较 10/15/20 日，连同跨年亏损、回撤与双费一起看。</li>
          <li>核对“纯期限（事后探索）”与原支撑退出的差别，再读数据局限。</li>
        </ol>
        <p>结果：本卡摘要与对照表保留失败配置和后加的纯期限线索。前提：修改退出方式是不同实验；事后追加的阳性结果不是新的样本外，正收益不等于通过跨年或同信号对照，也不转成可用模型。</p>
      </details>
      <details v-else-if="study.title==='公告时点财务增量37'">
        <summary>财务增量：查看步骤与前提</summary>
        <ol>
          <li>先看状态与期限、延迟反证结论；37 项输入是原 22 项加 15 项财务信息。</li>
          <li>在下方“同覆盖对照”表比较“原22全池”和“原22财务覆盖池”，先识别股票池变化。</li>
          <li>再比较“同覆盖重训22”和“财务增量37”，结合“数据范围与局限”核对新增信息的效果。</li>
        </ol>
        <p>结果：下方表仅展示本卡注明的已观察后段，期限及延迟失败见摘要；这里不启动财务计算或新账户。前提：财务信息在公告/修订/入库三日期最晚之后才用；历史修订链未认证，缺失不补零、不用当前财务倒填历史。同覆盖才能区分信息增量与选池差异。</p>
      </details>
      <details v-else-if="study.title==='真实历史更新与权益记账核验'">
        <summary>历史链路与权益：查看步骤与前提</summary>
        <ol>
          <li>读本卡历史追加、前向延续和同日静默检查的结论。</li>
          <li>展开“数据范围与局限”，区分已核验权益日期与仍挂起的应收/锁股。</li>
          <li>需要逐笔核对时，到“冻结模型回放”选对应研究账本，查看权益尾部、净值与订单记录。</li>
        </ol>
        <p>结果：本卡只有核验摘要；详细结果在对应账本中，“20日＋已核验权益日期（记账修正）”用于对照记账变化。前提：只释放有实施公告支持的核验事件；未核验权益继续挂起。记账修正不是预测算法，历史重放通过不是下一交易日实盘验收，旧账本也不会因程序升级自动换身份。</p>
      </details>
      <details v-if="study.comparisons?.length" class="study-comparison"><summary>查看 {{study.comparisons.length}} 组收益、回撤与跨年对照</summary><p>连续列为累计扣费净收益；分段从10万元重置；双费为连续账户。均非年化或上涨概率，2025以后也是已观察的历史。</p><p>净收益是账户净值相对起点的变化，可能含期末持仓和未到账权益；最大回撤是净值从此前高点到低点的最大跌幅。周期是已买入后卖完的持有记录，股票数是涉及的不重复代码；双费是提高交易成本后的独立回放，不能用连续收益直接推算分段收益。</p><p>现金账户实验次数是按现金和成交约束回放的研究账户次数，原撮合次数是另行执行的原策略撮合对照；它们共享历史行情，不是用户账户数或独立样本数。</p><div class="table-scroll" tabindex="0" aria-label="外部策略及序列形态实验对照"><table><thead><tr><th>机制 / 期限</th><th>连续净收益</th><th>最大回撤</th><th>2023—2024</th><th>2025以后</th><th>双费收益</th><th>周期 / 股票</th><th>结论</th></tr></thead><tbody><tr v-for="row in study.comparisons" :key="row.name+'-'+row.holding_days"><td>{{row.name}} · {{row.holding_days}}日</td><td :class="{negative:row.net_return_pct<0}">{{row.net_return_pct.toFixed(3)}}%</td><td>{{row.max_drawdown_pct.toFixed(3)}}%</td><td :class="{negative:row.validation_return_pct<0}">{{row.validation_return_pct.toFixed(3)}}%</td><td :class="{negative:row.tail_return_pct<0}">{{row.tail_return_pct.toFixed(3)}}%</td><td :class="{negative:row.double_cost_return_pct<0}">{{row.double_cost_return_pct.toFixed(3)}}%</td><td>{{row.cycles}} / {{row.unique_stocks}}</td><td>{{row.status}}</td></tr></tbody></table></div></details>
      <div v-if="study.metrics?.length" class="table-scroll"><table><thead><tr><th>同覆盖对照</th><th>净收益</th><th>最大回撤</th><th>完整周期</th></tr></thead><tbody><tr v-for="m in study.metrics" :key="m.name"><td>{{m.name}}</td><td>{{m.net_return_pct.toFixed(3)}}%</td><td>{{m.max_drawdown_pct.toFixed(3)}}%</td><td>{{m.cycles}}</td></tr></tbody></table></div>
      <details><summary>数据范围与局限</summary><p>先核对覆盖日期、执行约束和未认证事项，再解释收益差异。研究证据指纹只用于比对本卡对应文件内容，不能证明来源身份、收益计算正确或模型获准。</p><p v-for="item in study.limitations" :key="item">{{item}}</p><small v-if="study.source_sha256">研究证据指纹 {{study.source_sha256}}</small></details>
    </article>
  </section>
</template>
<style scoped>
.extension-study{font-family:"Microsoft YaHei UI","Microsoft YaHei","PingFang SC","Noto Sans CJK SC",var(--font-sans);min-width:0;margin:0;line-height:1.7}.extension-study article{margin:16px 0;padding:16px;border:1px solid var(--color-border-0);border-radius:9px}.extension-study small{font-size:12px}.extension-study p,.extension-study small{color:var(--color-text-secondary);overflow-wrap:anywhere}button{font:inherit;color:var(--color-text-primary);background:var(--color-surface-2);padding:8px 12px;border:1px solid var(--color-border-0);border-radius:7px;cursor:pointer}button:disabled{opacity:.5;cursor:not-allowed}.help{display:inline-grid;place-items:center;width:18px;height:18px;border:1px solid var(--color-border-0);border-radius:50%;cursor:help;font-size:12px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto;font-size:12px} .table-scroll{max-width:100%;overflow-x:auto}table{width:100%;font-size:12px;border-collapse:collapse}td,th{padding:8px;text-align:left;border-bottom:1px solid var(--color-border-0)}.error{border-left:3px solid #d65c5c;padding:8px}button:focus-visible,.help:focus-visible,summary:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
.study-meta{display:flex;gap:8px 18px;flex-wrap:wrap;color:var(--color-text-secondary);font-size:12px;margin:6px 0}.study-state{padding:0 7px;background:var(--color-surface-2);border-radius:4px}.study-comparison summary{font-weight:600}.study-comparison p{font-size:12px}.study-comparison td:last-child{white-space:normal;min-width:140px}.negative{color:var(--color-warning)}.table-scroll:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
</style>
