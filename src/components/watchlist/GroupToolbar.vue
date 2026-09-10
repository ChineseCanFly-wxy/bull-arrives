<script setup lang="ts">
import { computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { NButton, NInput, NModal, NSelect, useMessage } from 'naive-ui';
import { useWatchlistStore } from '@/stores/watchlist';
import type { WatchItem } from '@/types';
const store = useWatchlistStore();
const message = useMessage();
const show = ref(false);
const draft = ref('');
const busy = ref(false);
const rename = ref(false);
const confirmDelete = ref(false);
const currentName = computed(() => store.groups.find(g => g.id === store.activeGroupId)?.name ?? '全部');
const memberStock = ref<number | null>(null);
const allStocks = ref<WatchItem[]>([]);
const options = computed(() => allStocks.value.map(item => ({ label: `${item.name} ${item.code}`, value: item.id })));

async function run(action: () => Promise<unknown>) {
  if (busy.value) return;
  busy.value = true;
  try { await action(); await store.fetchWatchlist(); }
  catch (error) { message.error(String(error)); }
  finally { busy.value = false; }
}
function edit(isRename: boolean) {
  rename.value = isRename;
  draft.value = isRename ? currentName.value : '';
  show.value = true;
}
async function save() {
  if (!draft.value.trim()) { message.warning('请输入分组名称'); return; }
  await run(async () => {
    if (rename.value) await invoke('rename_watch_group', { id: store.activeGroupId, name: draft.value });
    else await invoke('create_watch_group', { name: draft.value });
    show.value = false;
  });
}
async function loadStocks() {
  try { allStocks.value = await invoke<WatchItem[]>('get_watchlist'); }
  catch (error) { message.error(String(error)); }
}
async function includeStock() {
  if (memberStock.value === null || !store.activeGroupId) return;
  await run(async () => {
    await invoke('set_group_member', { groupId: store.activeGroupId, watchId: memberStock.value, included: true });
    memberStock.value = null;
  });
}
</script>

<template>
  <div class="group-toolbar">
    <div class="group-tabs" role="tablist" aria-label="股票分组">
      <NButton size="small" role="tab" :aria-selected="store.activeGroupId === 0" :type="store.activeGroupId === 0 ? 'primary' : 'default'" :disabled="busy" @click="run(() => store.selectGroup(0))">全部</NButton>
      <NButton v-for="group in store.groups" :key="group.id" size="small" role="tab" :aria-selected="store.activeGroupId === group.id" :type="store.activeGroupId === group.id ? 'primary' : 'default'" :disabled="busy" @click="run(() => store.selectGroup(group.id))">{{ group.name }} · {{ group.item_count }}</NButton>
      <NButton size="small" dashed :disabled="busy" @click="edit(false)">＋ 新建分组</NButton>
    </div>
    <div v-if="store.activeGroupId" class="group-actions">
      <NSelect v-model:value="memberStock" :options="options" filterable placeholder="从全部自选加入当前组" size="small" :disabled="busy" @focus="loadStocks" />
      <NButton size="small" :disabled="memberStock === null || busy" @click="includeStock">加入</NButton>
      <NButton size="small" quaternary :disabled="busy" @click="edit(true)">改名</NButton>
      <NButton size="small" quaternary :disabled="busy" @click="confirmDelete = true">删除组</NButton>
    </div>
  </div>
  <NModal v-model:show="show" preset="card" :title="rename ? '重命名分组' : '创建分组'" style="width: 380px; max-width: 92vw;" :mask-closable="!busy">
    <NInput v-model:value="draft" placeholder="1–30 个字符" :maxlength="30" :disabled="busy" @keydown.enter="save" />
    <template #footer><NButton type="primary" :loading="busy" @click="save">保存</NButton></template>
  </NModal>
  <NModal v-model:show="confirmDelete" preset="dialog" title="删除当前分组" positive-text="仅删除分组" negative-text="取消" :loading="busy" @positive-click="run(async () => { await invoke('delete_watch_group', { id: store.activeGroupId }); confirmDelete = false; })">
    仅删除“{{ currentName }}”的分组关系，股票、持仓和提醒仍保留在“全部”。
  </NModal>
</template>

<style scoped>
.group-toolbar { display: flex; flex-direction: column; gap: 8px; padding: 8px 0; }
.group-tabs { display: flex; align-items: center; gap: 6px; overflow-x: auto; padding-bottom: 4px; }
.group-tabs > * { flex-shrink: 0; }
.group-actions { display: flex; gap: 6px; align-items: center; }
.group-actions :deep(.n-select) { max-width: 300px; min-width: 140px; }
</style>
