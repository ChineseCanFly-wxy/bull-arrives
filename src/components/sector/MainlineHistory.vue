<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton, NEmpty } from 'naive-ui';
import type { SectorKind } from '@/types/sector';
import type { MainlineNavigationTarget } from '@/types/navigation';

interface Snapshot { kind: SectorKind; sector_code: string; sector_name: string; as_of: string; fingerprint: string; complete: boolean; strong: boolean; metrics?: Record<string, unknown>; leaders?: { symbol: string; name: string; as_of: string }[] }
interface Notice { schema?: string; fingerprint?: string; mainline_snapshot?: Snapshot }
const props = defineProps<{ sectorCode?: string; kind?: SectorKind }>();
const emit = defineEmits<{ open: [target: MainlineNavigationTarget] }>();
const snapshots = ref<Snapshot[]>([]); const loading = ref(false); const loaded = ref(false); const error = ref(''); const unusable = ref(0);
let disposed = false; let epoch = 0;
const rows = computed(() => snapshots.value.filter(row => !props.sectorCode || row.sector_code === props.sectorCode && row.kind === props.kind));
function valid(row: Snapshot | undefined): row is Snapshot {
  return !!row && ['industry', 'concept'].includes(row.kind) && /^(BK[0-9]{4}|SW801[0-9]{3})$/.test(row.sector_code)
    && typeof row.sector_name === 'string' && !!row.sector_name.trim() && typeof row.fingerprint === 'string' && /^[0-9a-f]{64}$/i.test(row.fingerprint)
    && typeof row.as_of === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(row.as_of) && typeof row.complete === 'boolean' && typeof row.strong === 'boolean';
}
function strength(row: Snapshot, period: 20 | 60) { const value = row.metrics?.['rs' + period + '_vs_hs300']; return typeof value === 'number' && Number.isFinite(value) ? value : null; }
function number(value: number | null) { return value === null ? '--' : value.toFixed(2) + '个百分点'; }
function difference(row: Snapshot) {
  if (!row.complete) return null;
  const older = snapshots.value.find(other => other.kind === row.kind && other.sector_code === row.sector_code && other.as_of < row.as_of && other.complete);
  const current = strength(row, 20); const previous = older ? strength(older, 20) : null;
  if (!older || current === null || previous === null) return null;
  const change = current - previous;
  return '与' + older.as_of + '留存相比，20日相对强度差 ' + (change >= 0 ? '+' : '') + change.toFixed(2) + '个百分点（非连续日排名变化）';
}
function leadersText(row: Snapshot) {
  if (!row.complete || !row.strong) return '当时条件未通过，不展示领涨结论';
  const leaders = Array.isArray(row.leaders) ? row.leaders.filter(leader => leader && typeof leader.name === 'string' && typeof leader.symbol === 'string' && leader.as_of === row.as_of) : [];
  return leaders.slice(0, 3).map(leader => leader.name + ' ' + leader.symbol + '（' + leader.as_of + '）').join('、') || '未留存同日领涨记录';
}
async function refresh() {
  if (loading.value || disposed) return;
  const token = ++epoch; loading.value = true; error.value = '';
  try {
    const notices = await invoke<Notice[]>('get_mainline_alert_history');
    if (disposed || token !== epoch) return;
    const seen = new Set<string>(); const result: Snapshot[] = []; let invalid = 0;
    for (const notice of notices) {
      if (notice.schema !== 'mainline-research-notification-v1') continue;
      const row = notice.mainline_snapshot;
      if (!valid(row) || notice.fingerprint && notice.fingerprint !== row.fingerprint) { invalid++; continue; }
      if (!seen.has(row.fingerprint)) { seen.add(row.fingerprint); result.push(row); }
    }
    snapshots.value = result.sort((a, b) => b.as_of.localeCompare(a.as_of) || a.sector_code.localeCompare(b.sector_code));
    unusable.value = invalid; loaded.value = true;
  } catch (cause) { if (!disposed && token === epoch) error.value = '历史快照读取失败：' + cause + '；不会改用当前行情代替。'; }
  finally { if (!disposed && token === epoch) loading.value = false; }
}
function onToggle(event: Event) { if ((event.target as HTMLDetailsElement).open && !loaded.value) void refresh(); }
function open(row: Snapshot) { emit('open', { kind: row.kind, code: row.sector_code, name: row.sector_name, snapshotFingerprint: row.fingerprint }); }
onBeforeUnmount(() => { disposed = true; epoch++; });
</script>

<template>
  <details class="mainline-history" @toggle="onToggle">
    <summary>{{ sectorCode ? '这个板块' : '近期主线' }}的历史提醒快照（可选）</summary>
    <div class="history-toolbar"><span>最近14天已归档主线提醒</span><NButton class="refresh-button" size="small" :loading="loading" @click="refresh">刷新留存记录</NButton></div>
    <p class="history-note">查看的是当时已经保存的行情、领涨候选和证据，不会用当前扫描替换。只含留存提醒，缺少的日期与板块不补造；局部快照不能证明当时全市场最强，也不是连续历史排名或多年策略回测。</p>
    <p v-if="error" class="history-error" role="alert">{{ error }}</p>
    <p v-if="unusable" class="history-error">{{ unusable }}条主线提醒缺少可用快照标识，无法回看。</p>
    <article v-for="row in rows" :key="row.fingerprint" class="history-row">
      <header><b>{{ row.as_of }} · {{ row.sector_name }}</b><NButton size="tiny" @click="open(row)">查看当时证据</NButton></header>
      <p>{{ row.sector_code }} · 当时记录：{{ !row.complete ? '覆盖未通过' : row.strong ? '强主线观察' : '条件未通过' }}；打开时由后端核验保存的证据标识。</p>
      <p>相对沪深300：20日 {{ number(strength(row, 20)) }} · 60日 {{ number(strength(row, 60)) }}</p>
      <p v-if="difference(row)" class="history-note">{{ difference(row) }}</p>
      <p>当时领涨观察：{{ leadersText(row) }}</p>
    </article>
    <NEmpty v-if="loaded && !loading && !error && !rows.length" size="small" description="没有这个范围的已归档主线快照；不能用今天的数据回填" />
  </details>
</template>

<style scoped>
.mainline-history{margin-top:16px;border:1px solid var(--color-border-0);border-radius:var(--radius-md);padding:12px;font-size:var(--text-xs);color:var(--color-text-secondary);overflow-wrap:anywhere}.mainline-history summary{cursor:pointer}.history-toolbar,.history-row header{display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap}.history-toolbar{margin-top:12px}.history-note,.history-row p{line-height:1.7;margin:8px 0}.history-error{color:var(--color-warning)}.history-row{margin-top:12px;padding:12px;border-radius:var(--radius-sm);background:var(--color-surface-2)}.history-row b{color:var(--color-text-primary)}
</style>
