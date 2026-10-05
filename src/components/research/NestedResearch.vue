<script setup lang="ts">
import {computed,onBeforeUnmount,onMounted,ref} from 'vue';
import {invoke} from '@tauri-apps/api/core';
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import type {ResearchJob} from '@/types/research';
const props=defineProps<{busy:boolean}>();const emit=defineEmits<{job:[ResearchJob]}>();
const selected=ref<number|null>(null);const jobs=ref<ResearchJob[]>([]);const report=ref<ResearchJob|null>(null);const acting=ref(false);const error=ref('');let version=0;
const records=computed(()=>jobs.value.filter(j=>j.kind==='explore'));const rows=computed(()=>(report.value?.results??[]).flatMap(r=>(r.evaluations??[]).map(e=>({year:r.year,...e}))));
const label=(id:string)=>({balanced:'收益与回撤',return:'追求收益',defensive:'降低回撤',risk_on:'较强',risk_off:'较弱',mixed:'混合',unknown:'未知'}[id]??id);
const modelName=(id:string|null)=>{if(!id)return'等待 / 现金';const m=id.match(/^(trend|pullback|price_volume)_s([01])_h(10|20)$/);return m?({trend:'趋势结构',pullback:'回调结构',price_volume:'价量结构'}[m[1]]+' · '+(m[2]==='1'?'含市场交互':'纯个股')+' · '+m[3]+'日'):id;};
const pct=(v:unknown)=>typeof v==='number'&&Number.isFinite(v)?v.toFixed(2)+'%':'--';
async function refresh(){const v=++version;try{const value=await invoke<ResearchJob[]>('research_job_list');if(v!==version)return;jobs.value=value;if(!selected.value)selected.value=records.value[0]?.id??null;if(selected.value){const detail=await invoke<ResearchJob>('research_job_get',{id:selected.value});if(v===version)report.value=detail;}}catch(e){if(v===version)error.value=String(e);}}
async function start(){if(acting.value||props.busy)return;acting.value=true;error.value='';try{const job=await invoke<ResearchJob>('research_job_start',{request:{kind:'explore',models:['technical-nested-v1'],comparisons:['baseline'],continuation:null}});selected.value=job.id;emit('job',job);await refresh();}catch(e){error.value=String(e);}finally{acting.value=false;}}
onMounted(()=>void refresh());onBeforeUnmount(()=>{++version;});defineExpose({refresh});
</script>
<template>
<section class="nested-research"><header><h3>技术组合研究</h3><HelpTooltip label="嵌套研究说明">预先固定12种组合：趋势、回调、价量三类，分别有无市场状态交互，10/20日标签。每年用更早行情训练、前一年选配置，再看该年表现；标签结束日必须在下一阶段之前。三种风格独立选择，不挑选外层结果最好的一种冒充未见测试。</HelpTooltip></header>
<p>用于检查技术组合能否跨年份、成本与入场延迟保持稳定。范围固定为趋势、回调、价量三类，分别有无市场状态交互，配合10/20日标签，共12种组合；不包含财报、行业或任意AI因子。</p>
<p>运行会依次研究2022—2026五年：每年用更早行情训练，在前一年按“收益与回撤 / 追求收益 / 降低回撤”三种风格选组合，再评估该年；对选中项检查原费用、双费、延迟一天和同池随机排序，并统计不同市场状态的交易。每年核验通过后才保存结果。</p>
<details class="nested-guide"><summary>如何运行、选择年度并查看账户</summary>
<ol>
<li>先在“数据准备与更新 → 本机研究环境与数据路径”核对研究项目、Python、兼容快照和CSI300文件。需要numpy及研究程序已有依赖，应用不会自动安装；本实验读取已配置的快照，不主动更新行情。</li>
<li>点击“运行五年技术组合研究”，到“研究任务与已核验结果”看进度或取消。计算可能需要数分钟，取决于数据规模与本机性能；单个年份的Python计算超过600秒会停止，之前已核验年份保留。关闭页面后继续计算，退出应用会中断。</li>
<li>在“研究记录”选择任务号，再点“刷新研究结果”。按表格的“评估年”查看该年三种风格；年份用于查看结果，本页运行范围固定，不支持单独指定一年重算。未核验年份不显示，2026年只覆盖输入快照已有的完成行情。</li>
<li>先看净收益和回撤，再比较双费、延迟、同池随机收益；展开对应年份与风格的“市场状态表现”，查看较强、较弱、混合市场的平仓数和单笔净收益。“--”表示无对应数值。</li>
<li>完整账户在研究项目目录的 <code>research/nested-technical-v1/runs/job-任务号-…/</code> 中。打开对应年度报告，按其中的 <code>accounts</code> 清单找到 <code>*.ledger.json</code>，查看资金曲线、订单和未平仓；拟合参数与指纹也在该目录。这里的组合账户是独立研究输出，不进入用户模拟账户列表。</li>
</ol>
</details>
<p class="nested-boundary">结论状态始终为“研究观察”，不代表已通过实盘准入；“已核验”表示来源、时间边界和账本检查通过。出现“等待 / 现金”表示前一年没有净收益大于0且至少30笔闭合周期的合格候选，并非计算失败。已有历史曾被观察过，结果仍是回顾性研究，不自动加入选股模型、启用用户账户或接管模拟交易。</p>
<div class="nested-actions"><button class="primary" :disabled="acting||busy" @click="start">{{busy?'已有研究在运行':'运行五年技术组合研究'}}</button><button @click="refresh">刷新研究结果</button></div><p v-if="error" role="alert">{{error}}</p>
<label v-if="records.length">研究记录 <select v-model="selected" @change="refresh"><option v-for="j in records" :key="j.id" :value="j.id">#{{j.id}} {{j.created_at?.slice(0,10)||'日期未知'}} · {{j.completed}}/5年</option></select></label>
<p v-if="report">已核验 {{report.completed}}/5 年 · {{report.message}}</p>
<div v-if="rows.length" class="nested-table"><table><thead><tr><th>评估年</th><th>选择风格</th><th>前一年选中的组合</th><th>净收益</th><th>回撤</th><th>双费收益</th><th>延迟收益</th><th>同池随机收益</th></tr></thead><tbody><tr v-for="r in rows" :key="r.year+'-'+r.policy"><td>{{r.year}}</td><td>{{label(r.policy)}}</td><td>{{modelName(r.selected_id)}}</td><td>{{pct(r.metrics.net_return_pct)}}</td><td>{{pct(r.metrics.max_drawdown_pct)}}</td><td>{{pct(r.double_cost?.net_return_pct)}}</td><td>{{pct(r.delayed_entry?.net_return_pct)}}</td><td>{{pct(r.matched_pool_neutral?.net_return_pct)}}</td></tr></tbody></table></div>
<details v-for="r in rows.filter(v=>v.state_breakdown?.length)" :key="r.year+'-state-'+r.policy"><summary>{{r.year}} · {{label(r.policy)}} · 市场状态表现</summary><p v-for="s in r.state_breakdown" :key="s.state">{{label(s.state)}}：{{s.completed_cycles}}笔平仓 · 平均单笔净收益 {{pct(s.mean_cycle_net_pct)}}</p></details>
<p class="nested-boundary">各年账户从10万元独立开始；年度结果不能直接当作连续账户收益。市场状态只看当时已完成的大盘和股票广度，不按事后涨跌定义牛熊。</p>
</section>
</template>
<style scoped>
.nested-research{display:grid;gap:12px;min-width:0}.nested-research header,.nested-actions{display:flex;gap:10px;align-items:center;flex-wrap:wrap}.nested-research h3,.nested-research p{margin:0}.nested-research p{font-size:13px;line-height:1.8}.nested-boundary{color:var(--color-text-secondary)}.nested-research button,.nested-research select{padding:8px 12px;border:1px solid var(--color-border-0);border-radius:8px;background:var(--color-surface-1);color:var(--color-text-primary);max-width:100%}.nested-research button{cursor:pointer}.nested-research button:disabled{opacity:.5}.nested-research .primary{background:var(--color-accent);color:#fff}.nested-table{overflow:auto;border:1px solid var(--color-border-0);border-radius:10px;max-height:420px}.nested-table table{border-collapse:collapse;white-space:nowrap;width:100%;font-size:12px}.nested-table th,.nested-table td{padding:10px;text-align:left;border-bottom:1px solid var(--color-border-0)}.nested-table th{position:sticky;top:0;background:var(--color-surface-2)}.nested-research details{font-size:12px}.nested-research summary{cursor:pointer}.nested-guide ol{margin:8px 0 0;padding-left:22px;font-size:13px;line-height:1.8}.nested-guide li+li{margin-top:6px}.nested-guide code{overflow-wrap:anywhere}
</style>
