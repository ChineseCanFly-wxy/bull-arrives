<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {describeCondition,usesRetiredMinutes} from '@/types/conditions';
import HelpTooltip from '@/components/common/HelpTooltip.vue';
import { MODEL_CATALOG, type ConditionWatchView } from '@/types/research';
const view = ref<ConditionWatchView | null>(null); const error = ref(''); const busy = ref<number | null>(null); const expanded = ref(false);
let disposed = false; let refreshSequence = 0; let timer: ReturnType<typeof setInterval> | undefined;
const rows = computed(() => (view.value?.watches ?? []).slice(0, expanded.value ? 200 : 5));
const state = (value?: string) => ({ watching: '观察中', confirmed: '已确认', invalidated: '已失效', waiting_data: '等待数据' }[value ?? ''] ?? '待核验');
const name = (id: string) => MODEL_CATALOG.find(m => m.id === id)?.name ?? id;
const retired = (item:ConditionWatchView['watches'][number]) => ['model_confirm','model_mainline_confirm'].includes(item.config.preset) || !!(item.config.condition_tree && usesRetiredMinutes(item.config.condition_tree));
async function refresh(force = false) {
  if (disposed || (busy.value !== null && !force)) return;
  const sequence = ++refreshSequence;
  try {
    const value = await invoke<ConditionWatchView>('model_condition_watches');
    if (!disposed && sequence === refreshSequence) { view.value = value; error.value = ''; }
  } catch (e) { if (!disposed && sequence === refreshSequence) error.value = String(e); }
}
async function removeWatch(id: number) {
  if (disposed || busy.value !== null) return;
  busy.value = id; error.value = ''; ++refreshSequence;
  try {
    await invoke('model_condition_delete', { id });
    if (disposed) return;
    if (view.value) view.value.watches = view.value.watches.filter(item => item.id !== id);
    await refresh(true);
  } catch (e) { if (!disposed) error.value = '删除失败：' + String(e); }
  finally { if (!disposed) busy.value = null; }
}
onMounted(() => { void refresh(); timer = setInterval(() => void refresh(), 10000); });
onBeforeUnmount(() => { disposed = true; if (timer) clearInterval(timer); });
defineExpose({ refresh });
</script>
<template>
  <section class="condition-manager" aria-label="手动条件提醒清单">
    <header><b>手动条件提醒清单</b><HelpTooltip label="手动条件提醒说明">手动观察只检查你选定股票的条件，不下买卖单、不需要交易账户，不会影响自动模型的候选池、资金或持仓。应用运行且研究提醒开启时，条件新成立或失效会提醒。当前命中在加入时静默登记。盘后模型需重新筛选，或由研究中心对应信号来源账本开启盘后更新；实时涨跌幅条件需要交易时段的新鲜报价。主线组合需先在“市场主线”完成扫描。数据未知会等待，不当作满足或失效。不需要的提醒可删除，已产生的触发证据仍可在提醒记录查看。每轮最多检查20条观察，股票多或数据慢会延后；只观察你加入的股票。</HelpTooltip><button :disabled="busy !== null" @click="refresh()">刷新状态</button></header>
    <p class="manual-watch-scope">这里仅发送条件提醒，不自动买卖，也不使用任何交易账户。自动模型账户只供对应模型交易。</p>
    <p v-if="error" class="error">{{ error }}</p><p v-if="!rows.length" class="muted">从下方模型筛选结果点击“观察”，默认条件已按模型预选；保存后加入这份提醒清单，条件改变时通知你。</p>
    <article v-for="item in rows" :key="item.id">
      <div><b>{{ item.config.symbol.slice(2) }} · {{ name(item.config.model_id) }}</b><span :class="['state', item.observation.state]">{{ retired(item) ? '已停用' : item.enabled ? state(item.observation.state) : '已暂停' }}</span><button class="delete-watch" :disabled="busy !== null" :aria-label="'删除 ' + item.config.symbol.slice(2) + ' 的手动提醒'" @click="removeWatch(item.id)"><svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5h14M7 5V3h6v2M5 5l1 12h8l1-12M8 8v6m4-6v6" /></svg>{{ busy === item.id ? '删除中…' : '删除' }}</button></div>
      <p>{{ item.config.preset==='custom'?'自定义组合':view?.presets.find(p => p.id === item.config.preset)?.name || '已停用的分钟条件' }} · 核验日期 {{ item.observation.last_scan_day || '--' }}</p>
      <p v-if="item.config.condition_tree">模型命中 且 {{describeCondition(item.config.condition_tree)}}</p>
      <p v-if="retired(item)" class="paused-watch-note">旧分钟条件已停用；可重新从筛选结果添加可用提醒，或删除这条历史记录。</p>
      <p v-else-if="!item.enabled" class="paused-watch-note">这条旧提醒已暂停；重新从筛选结果点击“观察”可恢复，或直接删除。</p>
      <p>{{ item.observation.message || '等待后台第一次核验' }}</p>
    </article>
    <button v-if="(view?.watches.length ?? 0) > 5" @click="expanded = !expanded">{{ expanded ? '收起' : '全部手动提醒（' + view?.watches.length + '）' }}</button>
  </section>
</template>
<style scoped>
.condition-manager{border:1px solid var(--color-border-0,#d9e0e8);border-radius:12px;padding:12px;font-size:12px}.condition-manager header,.condition-manager article>div{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.condition-manager button{margin-left:auto;padding:5px 10px;border:1px solid var(--color-border-0,#d9e0e8);border-radius:7px;background:var(--color-surface-1,#fff);color:var(--color-text-primary,#26354a);cursor:pointer}.condition-manager article{padding:10px 0;border-bottom:1px solid var(--color-border-0,#d9e0e8)}.condition-manager article:last-of-type{border:0}.condition-manager p{margin:6px 0;color:var(--color-text-secondary,#69778c);line-height:1.6}.state{font-size:var(--text-xs);border-radius:12px;background:var(--color-surface-2,#edf2f9);padding:2px 7px;color:#53647a}.state.confirmed{color:var(--color-accent,#175bc2);background:var(--color-accent-dim,#e8f1ff)}.state.waiting_data{color:var(--color-warning,#8a651a);background:var(--color-warning-bg,#fff6dc)}.condition-manager .error{color:var(--color-error,#13714e);overflow-wrap:anywhere}
.condition-manager button:disabled{opacity:.5;cursor:default}.condition-manager .delete-watch{display:inline-flex;align-items:center;gap:4px;color:var(--color-text-secondary,#69778c);transition:color .15s,border-color .15s,background .15s}.condition-manager .delete-watch:hover:not(:disabled){color:var(--color-error,#c53f50);border-color:var(--color-error,#c53f50);background:var(--color-error-bg,#fff3f4)}.delete-watch svg{width:14px;height:14px;fill:none;stroke:currentColor;stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}.condition-manager .paused-watch-note{font-size:var(--text-xs)}
</style>
