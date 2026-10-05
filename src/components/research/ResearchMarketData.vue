<script setup lang="ts">
import {computed,onBeforeUnmount,onMounted,ref} from 'vue';
import {useSettingsStore} from '@/stores/settings';
import {invoke} from '@tauri-apps/api/core';
interface DataState {state:string;expected_as_of:string;enabled_accounts:number;missing_accounts:number;last_refresh?:{mode?:string;as_of?:string;coverage?:number;required_coverage?:number;providers?:Record<string,number>;stockdb_error?:string};accounts:Array<{account_id:number;model_name:string;as_of:string;ready:boolean;error?:string}>;policy:string;limitation:string}
const settings=useSettingsStore();
const autoUpdate=computed(()=>settings.stockDbStatus?.autoUpdate);
const updateTime=computed(()=>autoUpdate.value?.time||settings.localHistoryAutoUpdateTime||'09:00');
const updateEnabled=computed(()=>autoUpdate.value?.enabled??settings.localHistoryAutoUpdateEnabled??true);
const historyEnabled=computed(()=>settings.localHistoryEnabled??true);
const data=ref<DataState|null>(null);const error=ref('');let timer:ReturnType<typeof setInterval>|undefined;let disposed=false;let reading=false;
const modeNames:Record<string,string>={provider_fallback:'近期行情备用',stockdb:'StockDB完成日',saved_current:'齐备快照',saved_snapshot:'原模型快照'};
const modeName=(mode?:string)=>modeNames[mode??'']??'模型完成日数据';
const providerNames=(providers:Record<string,number>)=>Object.entries(providers).map(([name,count])=>`${name==='sina'?'新浪':name==='tencent'?'腾讯':name} ${count}只`).join('、');
async function refresh(){if(reading||disposed)return;reading=true;try{const result=await invoke<DataState>('research_market_data_status');if(!disposed){data.value=result;error.value='';}}catch(cause){if(!disposed)error.value=String(cause);}finally{reading=false;}}
onMounted(()=>{void refresh();timer=setInterval(()=>void refresh(),10000);});onBeforeUnmount(()=>{disposed=true;if(timer)clearInterval(timer);});
</script>
<template>
  <section class="research-market-data" aria-label="模型行情数据状态" :class="{'waiting':data?.state==='waiting_data'}">
    <div class="market-data-heading"><b>模型行情数据</b><span v-if="data">{{data.state==='ready'?'已齐备':data.state==='no_active_accounts'?'启动后自动核对':'正在等待完整数据'}} · 应到 {{data.expected_as_of}}<template v-if="data.enabled_accounts"> · {{data.enabled_accounts-data.missing_accounts}}/{{data.enabled_accounts}} 账户齐备</template></span><span v-else>{{error?'状态暂不可用':'正在核对…'}}</span></div>
    <details class="market-data-details"><summary>自动更新与数据来源（可选）</summary>
    <p v-if="updateEnabled&&historyEnabled">StockDB每天 {{updateTime}}（北京时间，默认09:00）自动更新；当天超过时间才启动时自动补一次。</p><p v-else-if="updateEnabled">启用StockDB历史后，程序每天 {{updateTime}}（北京时间，默认09:00）自动更新；当天超过时间才启动时自动补一次。可在数据设置中连接StockDB。</p><p v-else>StockDB定时更新当前已暂停，可在数据设置中恢复；行情源兜底仍保留。</p><p>更新失败每1分钟重试，连续5次尝试失败后只弹一次失败提醒，无需配置重试次数。缺行情本身不另弹提醒，模型会等待合格数据。</p><p v-if="autoUpdate">更新状态：{{autoUpdate.message}}<template v-if="autoUpdate.failures"> · 本轮已失败 {{autoUpdate.failures}} 次</template></p><p v-if="autoUpdate?.lastError">最近更新失败原因：{{autoUpdate.lastError}}</p>
    <p>盘中实时报价与盘口继续走行情源；盘后优先StockDB，近期缺口自动尝试新浪/腾讯。数据不合格时自动等待更新，保留交易所需的日期与完整性检查。</p>
    <div v-if="data" class="market-data-sources"><p v-if="data.last_refresh?.as_of">最近采用：{{modeName(data.last_refresh.mode)}} · {{data.last_refresh.as_of}}<template v-if="data.last_refresh.coverage"> · 有效覆盖 {{data.last_refresh.coverage}} 只 / 要求 {{data.last_refresh.required_coverage}} 只</template><template v-if="data.last_refresh.providers"> · {{providerNames(data.last_refresh.providers)}}</template></p><p v-if="data.last_refresh?.stockdb_error">StockDB未齐备原因：{{data.last_refresh.stockdb_error}}<template v-if="data.last_refresh.mode==='provider_fallback'">；本次备用数据已通过独立核对</template>。</p><p v-for="row in data.accounts" :key="row.account_id">#{{row.account_id}} {{row.model_name}} · 截至 {{row.as_of || '日期未知'}} · {{row.ready?'齐备':'等待更新'}}<template v-if="!row.ready&&row.error">：{{row.error}}</template></p><p>{{data.limitation}}</p></div>
    </details>
    <p v-if="error" class="market-data-error" role="alert">无法读取数据状态：{{error}}。查看下方账户状态；没有按未知状态改变委托。</p>
  </section>
</template>
<style scoped>
.research-market-data{display:grid;gap:7px;padding:12px;border:1px solid var(--color-border-0);border-radius:10px;background:var(--color-surface-1);font-size:12px;color:var(--color-text-secondary);min-width:0;overflow-wrap:anywhere}.market-data-heading{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}.market-data-heading b{color:var(--color-text-primary)}.research-market-data p{margin:0;line-height:1.6}.research-market-data details p{margin-top:7px}.market-data-sources{display:grid;gap:7px}.research-market-data summary{cursor:pointer}.research-market-data.waiting{border-color:var(--color-warning)}.market-data-error{color:var(--color-danger)}
</style>
