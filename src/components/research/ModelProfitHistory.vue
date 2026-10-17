<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { FollowProfitHistory } from '@/types/modelFollow';

const emit=defineEmits<{back:[]}>();
const accountId=ref('');
const keyword=ref('');
const start=ref('');
const end=ref('');
const result=ref<FollowProfitHistory|null>(null);
const busy=ref(false);
const error=ref('');
let disposed=false;
const applied=ref({account_id:null as number|null,keyword:'',start:null as string|null,end:null as string|null});
async function search(page=0,paginate=false) {
  if(busy.value)return;
  busy.value=true;error.value='';
  const filters=paginate?applied.value:{account_id:accountId.value?Number(accountId.value):null,keyword:keyword.value.trim(),start:start.value||null,end:end.value||null};
  try {
    const data=await invoke<FollowProfitHistory>('research_follow_profit_history',{query:{...filters,page}});
    if(!disposed){result.value=data;applied.value=filters;}
  } catch(cause){if(!disposed)error.value=String(cause);}
  finally{if(!disposed)busy.value=false;}
}
const money=(value:number)=>`¥${value.toLocaleString('zh-CN',{minimumFractionDigits:2,maximumFractionDigits:2})}`;
const percent=(value:number|null)=>value==null?'--':value.toFixed(2)+'%';
const direction=(value:number)=>value>0?'up':value<0?'down':'';
const timestamp=(value:string)=>new Date(value).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false});
onMounted(()=>void search());
onBeforeUnmount(()=>{disposed=true;});
</script>

<template>
  <section class="profit-history" aria-label="历史盈亏查询" :aria-busy="busy">
    <header><div><h3>历史盈亏</h3><p>自动模型账户实际卖出后的净盈亏，包含买入及卖出费用。</p></div><button type="button" class="profit-back" @click="emit('back')">返回自动模型交易</button></header>
    <form @submit.prevent="search()">
      <fieldset :disabled="busy">
        <label>模型账户<select v-model="accountId" data-profit-account><option value="">全部模型账户</option><option v-for="account in result?.accounts ?? []" :key="account.account_id" :value="String(account.account_id)">#{{account.account_id}} {{account.model_name}}</option></select></label>
        <label>股票名称 / 代码<input v-model="keyword" data-profit-keyword maxlength="80" placeholder="全部股票" /></label>
        <label>卖出起始日<input v-model="start" type="date" data-profit-start :max="end||undefined" /></label>
        <label>卖出截止日<input v-model="end" type="date" data-profit-end :min="start||undefined" /></label>
        <button type="submit" class="profit-search">{{busy?'查询中…':'查询盈亏'}}</button>
      </fieldset>
    </form>
    <p v-if="error" class="error" role="alert">{{error}}<template v-if="result"> · 下方保留上次成功查询的结果。</template></p>
    <template v-if="result">
      <p class="profit-period" role="status">查询结果：{{applied.start||'最早记录'}} 至 {{applied.end||'最新记录'}} · {{applied.account_id?'账户 #'+applied.account_id:'全部模型账户'}}{{applied.keyword?' · '+applied.keyword:''}}（北京时间）</p>
      <dl class="profit-summary">
        <div><dt>已实现净盈亏</dt><dd :class="direction(result.summary.profit_cny)">{{money(result.summary.profit_cny)}}</dd></div>
        <div><dt>已卖出成本收益率</dt><dd :class="direction(result.summary.profit_cny)">{{percent(result.summary.return_pct)}}</dd></div>
        <div><dt>已完成卖出</dt><dd>{{result.summary.count}} 笔</dd></div>
        <div><dt>卖出费用</dt><dd>{{money(result.summary.sell_fee_cny)}}</dd></div>
      </dl>
      <p class="profit-note">汇总覆盖全部符合查询条件的记录。收益率＝已实现净盈亏 ÷ 对应含买入费用成本；当前持仓浮盈亏在交易卡片查看。资金调整不计作盈利，研究回放收益不混入本页。</p>
      <div class="profit-table" tabindex="0" aria-label="已实现盈亏明细">
        <table><thead><tr><th>卖出时间</th><th>模型 / 股票</th><th>卖出数量 / 价格</th><th>含买费成本</th><th>卖出费用</th><th>净盈亏 / 收益率</th><th>退出原因</th></tr></thead><tbody>
          <tr v-for="row in result.records" :key="row.order_id"><td>{{timestamp(row.filled_at)}}</td><td>{{row.name||row.symbol}}<small>{{row.symbol}} · #{{row.account_id}} {{row.model_name}}</small></td><td>{{row.quantity}} 股<small>{{money(row.price_cny)}} / 股</small></td><td>{{money(row.cost_cny)}}</td><td>{{money(row.sell_fee_cny)}}</td><td :class="direction(row.profit_cny)"><b>{{money(row.profit_cny)}}</b><small>{{percent(row.return_pct)}}</small></td><td class="profit-reason">{{row.reason||'--'}}</td></tr>
          <tr v-if="!result.records.length"><td colspan="7" class="profit-empty">当前查询条件下没有已实现盈亏；仍持有或未成交的委托不会计入。</td></tr>
        </tbody></table>
      </div>
      <nav class="profit-pagination" aria-label="盈亏记录分页"><button type="button" :disabled="busy||result.page===0" @click="search(result.page-1,true)">上一页</button><span>第 {{result.page+1}} / {{Math.max(1,Math.ceil(result.summary.count/result.page_size))}} 页 · 每页 {{result.page_size}} 笔</span><button type="button" :disabled="busy||(result.page+1)*result.page_size>=result.summary.count" @click="search(result.page+1,true)">下一页</button></nav>
    </template>
  </section>
</template>

<style scoped>
.profit-history{padding:16px;border:1px solid var(--color-border-0);border-radius:12px;background:var(--color-surface-1)}.profit-history button{border:1px solid var(--color-border-0);border-radius:8px;padding:8px 12px;font:inherit;line-height:1.5;color:var(--color-text-primary);background:var(--color-surface-1);cursor:pointer}.profit-history .profit-search{border-color:var(--color-accent);color:var(--color-accent);background:var(--color-accent-dim)}.profit-history button:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}
.profit-history{display:grid;gap:14px;min-width:0}.profit-history header{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}.profit-history h3,.profit-history p,.profit-history dl{margin:0}.profit-history header p,.profit-note,.profit-period{font-size:12px;color:var(--color-text-secondary)}fieldset{border:0;padding:0;margin:0;display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:10px;align-items:end}label{display:grid;gap:5px;font-size:12px}input,select{font:inherit;font-size:13px;border:1px solid var(--color-border-0);border-radius:8px;background:var(--color-surface-1);color:inherit;padding:8px;min-width:0;width:100%;box-sizing:border-box}.profit-summary{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px}.profit-summary>div{border:1px solid var(--color-border-0);border-radius:10px;padding:12px;min-width:0}.profit-summary dt{color:var(--color-text-secondary);font-size:12px}.profit-summary dd{margin:3px 0 0;font-size:19px;font-weight:600;overflow-wrap:anywhere;font-variant-numeric:tabular-nums}.up{color:var(--color-up)}.down{color:var(--color-down)}.profit-table{overflow:auto;min-width:0;border:1px solid var(--color-border-0);border-radius:10px}table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}th,td{text-align:left;padding:10px;white-space:nowrap;border-bottom:1px solid var(--color-border-0);font-size:12px}td small{display:block;color:var(--color-text-secondary)}td.up small{color:var(--color-up)}td.down small{color:var(--color-down)}.profit-reason{white-space:normal;min-width:180px;max-width:320px}.profit-empty{white-space:normal;color:var(--color-text-secondary);padding:24px}.profit-pagination{display:flex;justify-content:flex-end;align-items:center;gap:10px;flex-wrap:wrap;font-size:12px}.error{color:var(--color-error);overflow-wrap:anywhere}button:disabled{opacity:.5;cursor:default}input:focus-visible,select:focus-visible,.profit-table:focus-visible{outline:2px solid var(--color-accent);outline-offset:2px}@media(max-width:600px){.profit-summary{grid-template-columns:repeat(2,minmax(0,1fr))}.profit-summary dd{font-size:17px}}
</style>
