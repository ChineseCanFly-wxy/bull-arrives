import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { RotationStatus, SectorKind, SectorLimitUpStats, SectorMember, SectorMemberPage, SectorRotation, SectorSummary, SectorSummaryPage } from '@/types/sector';

export const useSectorStore = defineStore('sector', () => {
  const kind = ref<SectorKind>('industry');
  const keyword = ref('');
  const page = ref(1);
  const pageSize = ref(50);
  const rows = ref<SectorSummary[]>([]);
  const total = ref(0);
  const asOf = ref('');
  const source = ref('');
  const stale = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const selected = ref<SectorSummary | null>(null);
  const members = ref<SectorMember[]>([]);
  const memberPage = ref(1);
  const memberPageSize = ref(50);
  const memberTotal = ref(0);
  const memberAsOf = ref('');
  const memberSource = ref('');
  const memberStale = ref(false);
  const memberLoading = ref(false);
  const memberError = ref<string | null>(null);
  const limitUpStats = ref<SectorLimitUpStats | null>(null);
  const limitUpError = ref<string | null>(null);
  const rotationRows = ref<SectorSummary[]>([]);
  const rotationStatuses = ref<RotationStatus[]>([]);
  const rotationSource = ref('');
  const rotationLoading = ref(false);
  const rotationError = ref<string | null>(null);
  let generation = 0;
  let memberGeneration = 0;

  const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize.value)));
  const memberPageCount = computed(() => Math.max(1, Math.ceil(memberTotal.value / memberPageSize.value)));

  async function fetchSummaries(forceRefresh = false) {
    const request = ++generation;
    loading.value = true;
    error.value = null;
    try {
      const response = await invoke<SectorSummaryPage>('get_sector_summaries', {
        kind: kind.value,
        page: page.value,
        pageSize: pageSize.value,
        keyword: keyword.value,
        forceRefresh,
      });
      if (request !== generation) return;
      rows.value = response.items;
      total.value = response.total;
      page.value = response.page;
      pageSize.value = response.page_size;
      asOf.value = response.as_of;
      source.value = response.source;
      stale.value = response.stale;
    } catch (e) {
      if (request === generation) error.value = `获取板块排行失败: ${e}`;
      console.error('[sector] fetchSummaries failed:', e);
    } finally {
      if (request === generation) loading.value = false;
    }
  }

  async function selectKind(next: SectorKind) {
    if (kind.value === next) return;
    kind.value = next;
    page.value = 1;
    await fetchSummaries();
  }

  async function selectPage(next: number) {
    page.value = Math.max(1, next);
    await fetchSummaries();
  }

  async function selectSector(row: SectorSummary) {
    selected.value = row;
    memberPage.value = 1;
    members.value = [];
    memberError.value = null;
    limitUpStats.value = null;
    limitUpError.value = null;
    await Promise.all([fetchMembers(), fetchLimitUpStats()]);
  }

  function clearSector() {
    memberGeneration += 1;
    selected.value = null;
    members.value = [];
    memberError.value = null;
    limitUpStats.value = null;
    limitUpError.value = null;
    memberLoading.value = false;
  }

  async function fetchLimitUpStats() {
    const target = selected.value;
    if (!target) return;
    try {
      const response = await invoke<SectorLimitUpStats>('get_sector_limit_up_stats', { sectorCode: target.code });
      if (selected.value?.code === target.code) limitUpStats.value = response;
    } catch (e) {
      if (selected.value?.code === target.code) limitUpError.value = String(e);
    }
  }

  async function fetchMembers(forceRefresh = false) {
    const target = selected.value;
    if (!target) return;
    const request = ++memberGeneration;
    memberLoading.value = true;
    memberError.value = null;
    try {
      const response = await invoke<SectorMemberPage>('get_sector_members', {
        kind: target.kind,
        sectorCode: target.code,
        page: memberPage.value,
        pageSize: memberPageSize.value,
        forceRefresh,
      });
      if (request !== memberGeneration) return;
      members.value = response.items;
      memberTotal.value = response.total;
      memberPage.value = response.page;
      memberPageSize.value = response.page_size;
      memberAsOf.value = response.as_of;
      memberSource.value = response.source;
      memberStale.value = response.stale;
    } catch (e) {
      if (request === memberGeneration) memberError.value = `获取成分股失败: ${e}`;
      console.error('[sector] fetchMembers failed:', e);
    } finally {
      if (request === memberGeneration) memberLoading.value = false;
    }
  }

  async function selectMemberPage(next: number) {
    memberPage.value = Math.max(1, next);
    await fetchMembers();
  }

  async function fetchRotation() {
    rotationLoading.value = true;
    rotationError.value = null;
    try {
      const response = await invoke<SectorRotation>('get_sector_rotation');
      const succeeded = new Set(response.statuses.filter(status => status.ok).map(status => status.kind));
      // 失败的类别保留上次成功数据，成功的类别才替换。
      rotationRows.value = [
        ...rotationRows.value.filter(row => !succeeded.has(row.kind)),
        ...response.items,
      ];
      rotationStatuses.value = response.statuses;
      rotationSource.value = `${response.source} · ${response.request_count} 次请求`;
      if (!response.statuses.some(status => status.ok)) {
        rotationError.value = response.statuses.map(status => status.error).filter(Boolean).join('；') || '轮动数据获取失败';
      }
    } catch (e) {
      rotationError.value = `获取板块轮动失败: ${e}`;
      console.error('[sector] fetchRotation failed:', e);
    } finally {
      rotationLoading.value = false;
    }
  }

  return {
    kind,
    keyword,
    page,
    pageSize,
    rows,
    total,
    pageCount,
    asOf,
    source,
    stale,
    loading,
    error,
    selected,
    members,
    memberPage,
    memberPageSize,
    memberTotal,
    memberPageCount,
    memberAsOf,
    memberSource,
    memberStale,
    memberLoading,
    memberError,
    limitUpStats,
    limitUpError,
    rotationRows,
    rotationStatuses,
    rotationSource,
    rotationLoading,
    rotationError,
    fetchSummaries,
    selectKind,
    selectPage,
    selectSector,
    clearSector,
    fetchMembers,
    fetchLimitUpStats,
    selectMemberPage,
    fetchRotation,
  };
});
