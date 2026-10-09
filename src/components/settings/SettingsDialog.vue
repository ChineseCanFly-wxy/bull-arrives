<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { NAlert, NModal } from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import { emit as emitAppEvent } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, REFRESH_INTERVAL_AUTO } from '@/stores/settings';
import { useUniverseStore } from '@/stores/universe';
import WindowSizeSettings from './WindowSizeSettings.vue';
import GroupHotkeySettings from './GroupHotkeySettings.vue';
import { eventToHotkey, formatHotkeyLabel } from '@/utils/hotkey';
import { displayPath } from '@/utils/pathDisplay';

async function openReminderSettings() { if (close()) await emitAppEvent('notification-open-settings','price'); }
const props = defineProps<{ show: boolean; initialSection?: string }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const settings = useSettingsStore();
const universeStore = useUniverseStore();

/** 筛选结果分页大小的可选项 */
const universePageSizeOptions = [20, 50, 100];

function onUniversePageSizeChange(event: Event) {
  const input = event.target as HTMLSelectElement;
  const value = Number(input.value);
  void runAction('universe-page-size', () => universeStore.setPageSize(value))
    .catch(() => { input.value = String(universeStore.pageSize); });
}

type SectionKey = 'market' | 'alerts' | 'ai' | 'ticker' | 'appearance' | 'system';
const sections: Array<{ key: SectionKey; label: string; eyebrow: string }> = [
  { key: 'market', label: '行情', eyebrow: 'MARKET' },
  { key: 'alerts', label: '提醒', eyebrow: 'ALERTS' },
  { key: 'ai', label: '智能', eyebrow: 'AI / QUANT' },
  { key: 'ticker', label: '悬浮窗', eyebrow: 'TICKER' },
  { key: 'appearance', label: '外观', eyebrow: 'THEME' },
  { key: 'system', label: '系统', eyebrow: 'SYSTEM' },
];
const previousSection = sessionStorage.getItem('bull-settings-section');
const activeSection = ref<SectionKey>(
  sections.find(section => section.key === previousSection)?.key || 'market',
);
watch(() => props.initialSection, section => {
  const requested = sections.find(item => item.key === section);
  if (requested) activeSection.value = requested.key;
}, { immediate: true });
const sectionLoaded = new Set<SectionKey>();
const contentEl = ref<HTMLElement | null>(null);
watch(activeSection, section => {
  sessionStorage.setItem('bull-settings-section', section);
  if (contentEl.value) contentEl.value.scrollTop = 0;
});
const actionError = ref<string | null>(null);
const savingKeys = ref(new Set<string>());
interface BuildInfo { version: string; built_at: string; profile: string; exe_path: string }
interface DataPaths { data_dir: string; database: string; interactive_tasks: string }
interface LocalHistoryStatus {
  state: 'connected' | 'not_started' | 'not_found' | 'unavailable';
  message: string;
  source_format?: string;
  start_date?: string;
  end_date?: string;
  sample_count?: number;
  candidates: string[];
}
interface AgentStatus { installed: boolean; state: string; path: string | null; message: string; guidance: string; run_dir: string | null }
const buildInfo = ref<BuildInfo | null>(null);
const dataPaths = ref<DataPaths | null>(null);
const systemPaths = computed(() => [
  { label: '程序位置', path: buildInfo.value?.exe_path },
  { label: '数据目录', path: dataPaths.value?.data_dir },
  { label: '设置数据库', path: dataPaths.value?.database },
  { label: 'AI 会话文件', path: dataPaths.value?.interactive_tasks },
].filter(item => item.path));
const copyNotice = ref('');
const localHistoryStatus = ref<LocalHistoryStatus | null>(null);
const localHistoryUrlDraft = ref('http://127.0.0.1:7899');
const stockDbUpdateTimeDraft = ref('09:00');
const agentStatus = ref<AgentStatus | null>(null);
const agentPathDraft = ref('');
const agentRunRootDraft = ref('');
const agentTimeoutDraft = ref(180);
const agentBudgetDraft = ref(10);

const capturing = ref(false);
const capturedCombo = ref<string | null>(null);
const hotkeyError = ref<string | null>(null);
const opacityDraft = ref(100);
const intervalDraft = ref(3);
const intervalAuto = computed(() => settings.refreshInterval === REFRESH_INTERVAL_AUTO);
const tickerOpacitySupported = typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent);

watch(() => props.show, (open) => {
  if (open) {
    const requested = sections.find(item => item.key === props.initialSection);
    if (requested) activeSection.value = requested.key;
    opacityDraft.value = settings.tickerOpacity;
    intervalDraft.value = intervalAuto.value
      ? (settings.marketSession.interval_secs || 3)
      : settings.refreshInterval;
    actionError.value = null;
    copyNotice.value = '';
    localHistoryUrlDraft.value = settings.localHistoryUrl;
    stockDbUpdateTimeDraft.value = settings.localHistoryAutoUpdateTime;
    agentPathDraft.value = settings.settings['agent_claude_path'] || '';
    agentRunRootDraft.value = settings.settings['agent_run_root'] || '';
    agentTimeoutDraft.value = Number(settings.settings['agent_timeout_seconds'] || 180);
    agentBudgetDraft.value = Number(settings.settings['agent_budget_usd'] || 10);
    sectionLoaded.clear();
    void loadActiveSection();
  } else {
    stopCapture();
  }
}, { immediate: true });

async function loadActiveSection() {
  if (!props.show || sectionLoaded.has(activeSection.value)) return;
  const section = activeSection.value;
  sectionLoaded.add(section);
  try {
    if (section === 'market') {
      await settings.fetchMarketSession();
      await settings.fetchStockDbStatus();
      if (settings.localHistoryEnabled) await loadLocalHistoryStatus();

    } else if (section === 'ai') {
      await loadAgentStatus();
    } else if (section === 'system') {
      await loadBuildInfo();
      dataPaths.value = await invoke<DataPaths>('get_data_paths');
    }
  } catch (error) {
    sectionLoaded.delete(section);
    actionError.value = `读取${sections.find(item => item.key === section)?.label || '设置'}状态失败：${error}`;
  }
}
watch(activeSection, () => { void loadActiveSection(); });

function switchSection(section: SectionKey) {
  if (section === activeSection.value) return;
  // 离开分类不清空表单草稿；重新进入仍能继续编辑。
  activeSection.value = section;
}

function onKeyDown(event: KeyboardEvent) {
  event.preventDefault();
  event.stopPropagation();
  const combo = eventToHotkey(event);
  if (combo === 'cancel') return stopCapture();
  if (combo === 'confirm') {
    if (capturedCombo.value) void commitHotkey(capturedCombo.value);
    return;
  }
  if (combo) capturedCombo.value = combo;
}

function startCapture() {
  if (capturing.value) return;
  capturing.value = true;
  capturedCombo.value = null;
  hotkeyError.value = null;
  window.addEventListener('keydown', onKeyDown, true);
}
function stopCapture() {
  capturing.value = false;
  window.removeEventListener('keydown', onKeyDown, true);
}
async function commitHotkey(combo: string) {
  try {
    await runAction('hotkey', () => settings.setTickerHotkey(combo));
    stopCapture();
  } catch (error) {
    hotkeyError.value = String(error);
  }
}
function resetHotkey() {
  capturedCombo.value = null;
  hotkeyError.value = null;
  void commitHotkey('Alt+Q');
}

function onTickerModeChange(event: Event) {
  const value = (event.target as HTMLInputElement).value as 'carousel' | 'fixed';
  void safelyRun('ticker-mode', () => settings.setTickerDisplayMode(value));
}

function onTickerPageSizeChange(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  void safelyRun('ticker-page-size', () => settings.setTickerPageSize(value));
}

async function runAction(key: string, action: () => Promise<unknown>) {
  if (savingKeys.value.has(key)) return;
  const next = new Set(savingKeys.value);
  next.add(key);
  savingKeys.value = next;
  actionError.value = null;
  copyNotice.value = '';
  try {
    const result = await action();
    if (result === false) {
      throw new Error(settings.error || '设置保存失败');
    }
  } catch (error) {
    actionError.value = String(error);
    throw error;
  } finally {
    const done = new Set(savingKeys.value);
    done.delete(key);
    savingKeys.value = done;
  }
}
async function copyPath(path: string | null | undefined) {
  await navigator.clipboard.writeText(displayPath(path));
  copyNotice.value = '路径已复制';
}
function safelyRun(key: string, action: () => Promise<unknown>) {
  void runAction(key, action).catch(() => undefined);
}
function isSaving(key: string) {
  return savingKeys.value.has(key);
}
function setIntervalAuto(auto: boolean) {
  if (auto) {
    safelyRun('interval', () => settings.setRefreshInterval(REFRESH_INTERVAL_AUTO));
  } else {
    const seed = settings.marketSession.interval_secs || 3;
    intervalDraft.value = seed;
    safelyRun('interval', () => settings.setRefreshInterval(seed));
  }
}
function onIntervalInput(event: Event) {
  intervalDraft.value = Number((event.target as HTMLInputElement).value);
}
function onIntervalCommit(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  safelyRun('interval', () => settings.setRefreshInterval(value));
}
function onOpacityInput(event: Event) {
  opacityDraft.value = Number((event.target as HTMLInputElement).value);
}
function onOpacityCommit(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  void runAction('opacity', () => settings.setTickerOpacity(value))
    .catch(() => { opacityDraft.value = settings.tickerOpacity; });
}
function onColorCommit(event: Event) {
  const value = (event.target as HTMLInputElement).value;
  safelyRun('ticker-color', () => settings.setTickerTextColor(value));
}
function resetTickerAppearance() {
  if (!window.confirm('将悬浮窗透明度、单色显示和字体颜色恢复默认？其他设置不会改变。')) return;
  void runAction('ticker-reset', async () => {
    if (tickerOpacitySupported && !await settings.setTickerOpacity(100)) return false;
    opacityDraft.value = 100;
    if (!await settings.setTickerTextColor('#9AA5B1')) return false;
    return settings.setTickerSingleColor(false);
  }).catch(() => undefined);
}
/// 读取构建信息，用于确认当前运行的确实是刚构建出来的版本
async function loadBuildInfo() {
  try { buildInfo.value = await invoke<BuildInfo>('get_build_info'); }
  catch (error) { console.warn('[settings] 读取构建信息失败:', error); }
}
async function loadLocalHistoryStatus() {
  try { localHistoryStatus.value = await invoke<LocalHistoryStatus>('get_local_history_status'); }
  catch (error) { localHistoryStatus.value = { state: 'unavailable', message: String(error), candidates: [] }; }
}
async function testLocalHistory() {
  localHistoryStatus.value = await invoke<LocalHistoryStatus>('test_local_history', { url: localHistoryUrlDraft.value });
}
async function scanLocalHistory() {
  await settings.scanStockDb();
}
async function saveLocalHistoryUrl() {
  if (!await settings.setSetting('local_history_url', localHistoryUrlDraft.value.trim())) return false;
  await testLocalHistory();
}
async function browseStockDbEngine() {
  const path = await open({
    multiple: false,
    directory: false,
    title: '选择 stockdb.exe',
    filters: [{ name: 'stockdb', extensions: ['exe'] }],
  });
  if (typeof path === 'string') await settings.selectStockDbEngine(path);
}
async function browseStockDbUpdater() {
  const path = await open({
    multiple: false,
    directory: false,
    title: '选择 数据更新.exe',
    filters: [{ name: '数据更新程序', extensions: ['exe'] }],
  });
  if (typeof path === 'string') await settings.selectStockDbUpdater(path);
}
async function chooseStockDbCandidate(path: string) {
  await settings.selectStockDbEngine(path);
}
function stockDbStateLabel() {
  const labels: Record<string, string> = {
    disabled: '已关闭', locating: '正在查找', not_configured: '未配置', configured: '已配置',
    starting: '正在启动', running_owned: '正在运行', running_external: '外部服务',
    updating: '正在更新', restarting: '正在重启', error: '异常', unsupported: '不支持',
  };
  return labels[settings.stockDbStatus?.state || ''] || '正在检测';
}
async function loadAgentStatus() {
  try { agentStatus.value = await invoke<AgentStatus>('get_agent_status'); }
  catch (error) { agentStatus.value = { installed: false, state: 'unavailable', path: null, message: String(error), guidance: '请手动检查 Claude Code。', run_dir: null }; }
}
async function saveAgentPath() {
  if (!await settings.setSetting('agent_claude_path', agentPathDraft.value.trim())) return false;
  await loadAgentStatus();
}
async function browseAgentPath() {
  const path = await open({
    multiple: false,
    directory: false,
    title: '选择 Claude Code 可执行文件（claude.exe）',
    filters: [{ name: 'Claude Code', extensions: ['exe', 'cmd'] }],
  });
  if (typeof path !== 'string') return;
  agentPathDraft.value = path;
  await saveAgentPath();
}
async function saveAgentRunRoot() {
  if (!await settings.setSetting('agent_run_root', agentRunRootDraft.value.trim())) return false;
  await loadAgentStatus();
}
async function browseAgentRunRoot() {
  const path = await open({ multiple: false, directory: true, title: '选择 Agent 工作目录' });
  if (typeof path !== 'string') return;
  agentRunRootDraft.value = path;
  await saveAgentRunRoot();
}
async function resetAgentRunRoot() {
  agentRunRootDraft.value = '';
  return saveAgentRunRoot();
}
async function testAgentConnection() {
  agentStatus.value = await invoke<AgentStatus>('test_agent_connection');
}
async function saveAgentTimeout() {
  if (!Number.isInteger(agentTimeoutDraft.value) || agentTimeoutDraft.value < 15 || agentTimeoutDraft.value > 300) {
    throw new Error('Agent 超时必须为 15–300 的整数秒');
  }
  return settings.setSetting('agent_timeout_seconds', String(agentTimeoutDraft.value));
}
async function saveAgentBudget() {
  if (!Number.isFinite(agentBudgetDraft.value) || agentBudgetDraft.value < 10 || agentBudgetDraft.value > 50) {
    throw new Error('单次预算必须在 10–50 美元之间');
  }
  return settings.setSetting('agent_budget_usd', agentBudgetDraft.value.toFixed(2));
}
function close(): boolean {
  if (savingKeys.value.size) {
    actionError.value = '设置仍在保存，请等待完成后再关闭。';
    return false;
  }
  const unsavedAgent = agentPathDraft.value.trim() !== (settings.settings['agent_claude_path'] || '')
    || agentRunRootDraft.value.trim() !== (settings.settings['agent_run_root'] || '')
    || agentTimeoutDraft.value !== Number(settings.settings['agent_timeout_seconds'] || 180)
    || Number(agentBudgetDraft.value).toFixed(2) !== Number(settings.settings['agent_budget_usd'] || 10).toFixed(2);
  const unsavedUrl = localHistoryUrlDraft.value.trim() !== settings.localHistoryUrl;
  if ((unsavedAgent || unsavedUrl) && !window.confirm('有尚未保存的设置，确定放弃这些修改吗？')) return false;
  stopCapture();
  emit('update:show', false);
  return true;
}
defineExpose({ close });

onBeforeUnmount(stopCapture);
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    title="偏好设置"
    class="settings-modal"
    :style="{ width: settings.visualStyle === 'classic' ? 'min(860px, 96vw)' : 'min(920px, 96vw)' }"
    :bordered="false"
    :mask-closable="true"
    :segmented="{ content: 'soft', footer: 'soft' }"
    @update:show="(value: boolean) => !value && close()"
  >
    <div class="settings-shell">
      <nav class="section-nav" aria-label="设置分类">
        <button
          v-for="section in sections"
          :key="section.key"
          class="nav-item"
          :class="{ active: activeSection === section.key }"
          :aria-current="activeSection === section.key ? 'page' : undefined"
          @click="switchSection(section.key)"
        >
          <small>{{ section.eyebrow }}</small>
          <span>{{ section.label }}</span>
        </button>
      </nav>

      <main ref="contentEl" class="settings-content">
        <NAlert v-if="actionError" type="error" :show-icon="false" closable class="settings-error" @close="actionError = null">
          {{ actionError }}
        </NAlert>

        <p v-if="copyNotice" class="copy-feedback" role="status">{{ copyNotice }}</p>

        <section v-if="activeSection === 'market'" class="settings-panel">
          <header class="panel-heading"><span>01</span><div><h2>行情节奏</h2><p>平衡实时性与请求频率。</p></div></header>
          <article class="setting-card hero-card">
            <div class="card-title-row">
              <div><h3>自动调节刷新间隔</h3><p>根据交易时段使用推荐频率，休市时自动降频。</p></div>
              <button class="switch" :class="{ on: intervalAuto }" role="switch" aria-label="自动调节刷新间隔" :aria-checked="intervalAuto" :disabled="isSaving('interval')" @click="setIntervalAuto(!intervalAuto)"><span /></button>
            </div>
            <div v-if="intervalAuto" class="session-status">
              <i :class="{ trading: settings.marketSession.is_trading }" />
              <span>{{ settings.marketSession.session }}</span><b>{{ settings.marketSession.interval_secs }} 秒</b>
            </div>
            <div v-else class="slider-block">
              <div><span>固定间隔</span><b>{{ intervalDraft }}s</b></div>
              <input type="range" aria-label="固定刷新间隔（秒）" min="1" max="60" step="1" :value="intervalDraft" :disabled="isSaving('interval')" @input="onIntervalInput" @change="onIntervalCommit" />
              <p>固定模式不会跟随交易时段自动变化。</p>
            </div>
            <p class="session-calendar">{{ settings.marketSession.calendar }}</p>
          </article>
          <article class="setting-card">
            <h3>筛选结果每页条数</h3>
            <p class="card-desc">全市场筛选器结果表的分页大小，范围 1–100 条，默认 20 条。</p>
            <div class="inline-setting">
              <div><b class="page-size-label">每页显示</b><p>在筛选结果表底部也可以随时改。</p></div>
              <select
                class="page-size-select"
                aria-label="筛选结果每页显示条数"
                :value="universeStore.pageSize"
                :disabled="universeStore.loading"
                @change="onUniversePageSizeChange"
              >
                <option v-for="size in universePageSizeOptions" :key="size" :value="size">{{ size }} 条/页</option>
              </select>
            </div>
          </article>
          <article class="setting-card">
            <div class="card-title-row">
              <div><h3>本地历史数据</h3><p>默认关闭。开启后，Bull Arrives 会在启动时自动运行已选择的 stockdb。</p></div>
              <button class="switch" :class="{ on: settings.localHistoryEnabled }" role="switch" aria-label="本地历史数据" :aria-checked="settings.localHistoryEnabled" :disabled="isSaving('local-history-enabled') || settings.stockDbStatus?.busy" @click="safelyRun('local-history-enabled', () => settings.setLocalHistoryEnabled(!settings.localHistoryEnabled))"><span /></button>
            </div>
            <div class="history-status" :class="settings.stockDbStatus?.state">
              <b>{{ stockDbStateLabel() }}</b>
              <span>{{ settings.stockDbStatus?.message || '正在检测…' }}</span>
              <small v-if="settings.stockDbStatus?.owned">此进程由 Bull Arrives 管理，托盘真正退出时会一并关闭。</small>
              <small v-else-if="settings.stockDbStatus?.state === 'running_external'">更新时自动暂停已确认的本地 StockDB，完成后恢复；退出应用保留外部服务。</small>
            </div>
            <template v-if="settings.stockDbStatus?.platformSupported !== false">
              <div class="history-program-row">
                <span>stockdb 程序</span>
                <code class="path-text" :title="displayPath(settings.localHistoryEnginePath) || '尚未选择'">{{ displayPath(settings.localHistoryEnginePath) || '尚未选择 stockdb.exe' }}</code>
                <span class="field-btns"><button class="minor-btn" aria-label="复制 stockdb 程序路径" :disabled="!settings.localHistoryEnginePath" @click="safelyRun('copy-stockdb-engine', () => copyPath(settings.localHistoryEnginePath))">复制</button><button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('stockdb-engine')" @click="safelyRun('stockdb-engine', browseStockDbEngine)">浏览选择</button></span>
              </div>
              <div class="history-program-row">
                <span>数据更新程序</span>
                <code class="path-text" :title="displayPath(settings.localHistoryUpdaterPath) || '尚未找到'">{{ displayPath(settings.localHistoryUpdaterPath) || '尚未找到 数据更新.exe' }}</code>
                <span class="field-btns"><button class="minor-btn" aria-label="复制数据更新程序路径" :disabled="!settings.localHistoryUpdaterPath" @click="safelyRun('copy-stockdb-updater', () => copyPath(settings.localHistoryUpdaterPath))">复制</button><button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('stockdb-updater')" @click="safelyRun('stockdb-updater', browseStockDbUpdater)">浏览选择</button></span>
              </div>
              <div class="history-actions">
                <button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('local-history-scan')" @click="safelyRun('local-history-scan', scanLocalHistory)">自动查找</button>
                <button v-if="settings.localHistoryEnabled" class="minor-btn" :disabled="isSaving('local-history-test')" @click="safelyRun('local-history-test', testLocalHistory)">测试连接</button>
              </div>
              <div v-if="settings.stockDbStatus?.candidates.length" class="candidate-list">
                <div v-for="candidate in settings.stockDbStatus.candidates" :key="candidate.enginePath" class="candidate-row">
                  <button class="minor-btn candidate-btn" :title="displayPath(candidate.enginePath)" :disabled="settings.stockDbStatus?.busy || isSaving('stockdb-candidate')" @click="safelyRun('stockdb-candidate', () => chooseStockDbCandidate(candidate.enginePath))">{{ candidate.source }} · <code class="path-text">{{ displayPath(candidate.enginePath) }}</code></button>
                  <button class="minor-btn" :aria-label="'复制候选程序路径：' + displayPath(candidate.enginePath)" @click="safelyRun('copy-stockdb-candidate', () => copyPath(candidate.enginePath))">复制</button>
                </div>
              </div>
              <div v-if="settings.localHistoryEnabled" class="history-status">
                <b>{{ settings.localHistoryAutoUpdateEnabled ? '交易日自动更新 · 北京时间 ' + settings.localHistoryAutoUpdateTime : '交易日自动更新已暂停' }}</b>
                <span>{{ !settings.localHistoryAutoUpdateEnabled ? '自动更新暂停；仍可手动点更新数据' : settings.stockDbStatus?.autoUpdate?.message || '到点自动更新；启动时补做当天未完成的更新' }}</span>
                <small v-if="settings.stockDbStatus?.autoUpdate?.dataAsOf">已核对日线 {{ settings.stockDbStatus.autoUpdate.dataAsOf }}</small>
                <small v-if="settings.stockDbStatus?.autoUpdate?.lastError">{{ settings.stockDbStatus.autoUpdate.lastError }}</small>
              </div>
              <details v-if="settings.localHistoryEnabled" class="history-advanced">
                <summary>自动更新时间与高级连接设置</summary>
                 <label class="history-field"><span>交易日自动更新</span><button class="switch" :class="{ on: settings.localHistoryAutoUpdateEnabled }" role="switch" aria-label="StockDB交易日自动更新" :aria-checked="settings.localHistoryAutoUpdateEnabled" :disabled="isSaving('stockdb-auto-enabled')" @click="safelyRun('stockdb-auto-enabled', () => settings.setSetting('local_history_auto_update_enabled', settings.localHistoryAutoUpdateEnabled ? '0' : '1'))"><span /></button></label>
                 <label class="history-field"><span>北京时间</span><input v-model="stockDbUpdateTimeDraft" type="time" /><button class="minor-btn" :disabled="isSaving('stockdb-auto-time')" @click="safelyRun('stockdb-auto-time', () => settings.setSetting('local_history_auto_update_time', stockDbUpdateTimeDraft))">保存时间</button></label>
                 <small>默认在A股交易日09:00更新，休市日不自动执行；当天只执行一次，失败或中断后不自动重试，重启也不重复。可手动更新；未执行时，超过时间启动会补做一次。</small>
                <label class="history-field"><span>服务地址</span><input v-model="localHistoryUrlDraft" type="url" placeholder="http://127.0.0.1:7899" /><button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('local-history-url')" @click="safelyRun('local-history-url', saveLocalHistoryUrl)">保存并测试</button></label>
                <small v-if="localHistoryStatus?.start_date && localHistoryStatus?.end_date">样本区间 {{ localHistoryStatus.start_date }} → {{ localHistoryStatus.end_date }} · {{ localHistoryStatus.sample_count }} 根 · {{ localHistoryStatus.source_format }}</small>
              </details>
            </template>
            <p class="card-desc">开启后默认在A股交易日09:00后台更新，交易日晚启动会补做；顶栏“更新数据”保留，可随时手动执行。更新时暂停已核实的本地StockDB，同步核验后恢复原服务；退出应用保留外部启动的服务。近期数据源兜底继续保留。</p>
          </article>
        </section>

        <section v-else-if="activeSection === 'alerts'" class="settings-panel">
          <header class="panel-heading"><span>02</span><div><h2>提醒</h2><p>所有提醒记录和独立开关集中在“资讯与提醒”。</p></div></header>
          <article class="setting-card"><h3>资讯与提醒</h3><p>市场资讯、市场主线、研究中心、行情与风险、数据更新分别控制；重要操作悬浮窗的颜色和透明度也在这里调整。</p><button class="minor-btn" @click="openReminderSettings">打开资讯与提醒</button></article>
        </section>

        <section v-else-if="activeSection === 'ai'" class="settings-panel">
          <header class="panel-heading"><span>03</span><div><h2>AI / 量化智能</h2><p>总开关统一管理所有自动运行的智能功能。</p></div></header>
          <article class="setting-card accent-card">
            <div class="card-title-row">
              <div><h3>AI 智能总开关</h3><p>关闭后，后台自动运行的 AI / 量化功能全部停止；手动点击的分析与推荐仍可照常使用。</p></div>
              <button class="switch" :class="{ on: settings.aiEnabled }" role="switch" aria-label="AI 智能总开关" :aria-checked="settings.aiEnabled" :disabled="isSaving('ai')" @click="safelyRun('ai', () => settings.setSetting('ai_enabled', settings.aiEnabled ? '0' : '1'))"><span /></button>
            </div>
            <div v-if="!settings.aiEnabled" class="alert-guidance">
              <b>已全部停用</b>
              <p>智能监控已暂停，不会再产生任何后台计算与提醒。逐票的涨跌幅 / 固定价格提醒属于「提醒」分类，不受此处影响。</p>
            </div>
          </article>
          <article class="setting-card">
            <h3>本地 Agent · Claude Code</h3>
            <p class="card-desc">手动生成解读或多角色研判；开启资讯摘要、动态筛选时也会按对应规则调用。模型和登录沿用 Claude Code 配置，应用不保存凭据。</p>
            <div class="history-status" :class="agentStatus?.state === 'ready' ? 'connected' : 'not_found'">
              <b>{{ agentStatus?.state === 'ready' ? '连接正常' : agentStatus?.state === 'failed' ? '连接失败' : agentStatus?.installed ? '已安装 · 待验证' : '不可用' }}</b>
              <span>{{ agentStatus?.message || '正在检测…' }}</span>
              <small v-if="agentStatus?.path" class="status-path"><code class="path-text" :title="displayPath(agentStatus.path)">{{ displayPath(agentStatus.path) }}</code><button class="minor-btn" aria-label="复制检测到的 Claude 路径" @click="safelyRun('copy-agent-status', () => copyPath(agentStatus?.path))">复制</button></small>
            </div>
            <label class="history-field"><span>可执行文件</span><input :value="displayPath(agentPathDraft)" :title="displayPath(agentPathDraft)" type="text" placeholder="留空自动检测 claude.exe / claude.cmd" @input="agentPathDraft = ($event.target as HTMLInputElement).value" /><span class="field-btns"><button class="minor-btn" :disabled="isSaving('agent-browse')" @click="safelyRun('agent-browse', browseAgentPath)">浏览…</button><button class="minor-btn" :disabled="isSaving('agent-path')" @click="safelyRun('agent-path', saveAgentPath)">保存并检测</button><button class="minor-btn" aria-label="复制 Claude 配置路径" :disabled="!agentPathDraft" @click="safelyRun('copy-agent-path', () => copyPath(agentPathDraft))">复制</button></span></label>
            <label class="history-field"><span>工作目录</span><input :value="displayPath(agentRunRootDraft)" :title="displayPath(agentRunRootDraft)" type="text" placeholder="留空自动选择（推荐）" @input="agentRunRootDraft = ($event.target as HTMLInputElement).value" /><span class="field-btns"><button class="minor-btn" :disabled="isSaving('agent-root-browse')" @click="safelyRun('agent-root-browse', browseAgentRunRoot)">浏览…</button><button class="minor-btn" :disabled="isSaving('agent-root')" @click="safelyRun('agent-root', saveAgentRunRoot)">保存</button><button class="minor-btn" :disabled="isSaving('agent-root')" @click="safelyRun('agent-root', resetAgentRunRoot)">用自动</button><button class="minor-btn" aria-label="复制 Claude 工作目录" :disabled="!agentRunRootDraft" @click="safelyRun('copy-agent-root', () => copyPath(agentRunRootDraft))">复制</button></span></label>
            <label class="history-field"><span>单次超时（秒）</span><input v-model.number="agentTimeoutDraft" type="number" min="15" max="300" step="1" /><button class="minor-btn" :disabled="isSaving('agent-timeout')" @click="safelyRun('agent-timeout', saveAgentTimeout)">保存超时</button></label>
            <label class="history-field"><span>后台单次预算（美元）</span><input v-model.number="agentBudgetDraft" type="number" min="10" max="50" step="0.05" /><button class="minor-btn" :disabled="isSaving('agent-budget')" @click="safelyRun('agent-budget', saveAgentBudget)">保存预算</button></label>
            <p class="card-desc">此预算仅限制应用发起的每次后台 Claude Code 调用，不是账户余额；交互终端由你直接操作，不套用后台超时或这项预算。</p>
            <div class="history-program-row run-dir-line"><span>任务文件实际写入</span><code class="path-text" :title="displayPath(agentStatus?.run_dir)">{{ displayPath(agentStatus?.run_dir) || '待确定' }}</code><button class="minor-btn" aria-label="复制任务文件目录" :disabled="!agentStatus?.run_dir" @click="safelyRun('copy-agent-run-dir', () => copyPath(agentStatus?.run_dir))">复制</button></div>
            <p class="card-desc">留空即自动选择，应用会自动避开 Windows 的 8.3 短名目录（形如 <code>WEIXY4~1</code>）—— 那些目录下 Claude Code 会拒绝读写。</p>
            <div class="history-actions">
              <button class="minor-btn" :disabled="isSaving('agent-scan') || isSaving('agent-test')" @click="safelyRun('agent-scan', loadAgentStatus)">重新检测</button>
              <button class="minor-btn" :disabled="!agentStatus?.installed || isSaving('agent-test')" @click="safelyRun('agent-test', testAgentConnection)">{{ isSaving('agent-test') ? '测试连接中…' : '测试连接' }}</button>
              <button v-if="isSaving('agent-test')" class="minor-btn" @click="safelyRun('agent-cancel', () => invoke('cancel_agent_analysis'))">中止</button>
            </div>
            <p class="card-desc">{{ agentStatus?.guidance }}。测试连接会进行一次简短模型调用，可能产生服务商费用。后台任务单并发，默认超时 180 秒（15–300 秒可调），部分自动任务另有 30 秒上限；多角色研判分次执行，可中断。失败保留纯量化结果。</p>
          </article>
        </section>

        <section v-else-if="activeSection === 'ticker'" class="settings-panel">
          <GroupHotkeySettings />
          <header class="panel-heading"><span>04</span><div><h2>悬浮窗</h2><p>快捷唤起与低干扰显示。</p></div></header>
          <article class="setting-card">
            <h3>全局快捷键</h3><p class="card-desc">在任何界面显示或隐藏悬浮行情条。</p>
            <div class="hotkey-row">
              <button class="hotkey-box" :class="{ capturing }" @click="startCapture">
                {{ capturing ? (capturedCombo ? formatHotkeyLabel(capturedCombo) : '请按组合键…') : formatHotkeyLabel(settings.tickerHotkey) }}
                <small v-if="capturing">{{ capturedCombo ? 'Enter 确认 · Esc 取消' : '需包含修饰键' }}</small>
              </button>
              <button class="minor-btn" @click="capturing ? stopCapture() : startCapture()">{{ capturing ? '取消' : '录制' }}</button>
              <button class="minor-btn" @click="resetHotkey">重置</button>
            </div>
            <p v-if="hotkeyError" class="field-error">{{ hotkeyError }}</p>
          </article>
          <article class="setting-card">
            <h3>展示方式</h3>
            <p class="card-desc">自适应全显会跟随当前分组自选数量调整高度；股票较多时滚动查看。轮播适合保持小窗紧凑。</p>
            <div class="ticker-mode-options" role="radiogroup" aria-label="悬浮窗展示方式">
              <label><input type="radio" name="ticker-mode" value="carousel" :checked="settings.tickerDisplayMode === 'carousel'" :disabled="isSaving('ticker-mode')" @change="onTickerModeChange" /><span><b>轮播</b><small>每 3 秒翻页，鼠标悬停暂停</small></span></label>
              <label><input type="radio" name="ticker-mode" value="fixed" :checked="settings.tickerDisplayMode === 'fixed'" :disabled="isSaving('ticker-mode')" @change="onTickerModeChange" /><span><b>自适应全显</b><small>加多少显示多少，超出屏幕高度时可滚动</small></span></label>
            </div>
            <label v-if="settings.tickerDisplayMode === 'carousel'" class="ticker-count-row">
              <span><b>每页展示数量</b><small>可设置 1–20 只，超过当前分组数量时自动按实际数量展示。</small></span>
              <input type="number" min="1" max="20" step="1" :value="settings.tickerPageSize" :disabled="isSaving('ticker-page-size')" @change="onTickerPageSizeChange" />
            </label>
          </article>
          <article class="setting-card">
            <div v-if="tickerOpacitySupported" class="slider-block"><div><span>透明度</span><b>{{ opacityDraft }}%</b></div><input type="range" aria-label="悬浮窗透明度（百分比）" min="5" max="100" step="1" :value="opacityDraft" :disabled="isSaving('opacity')" @input="onOpacityInput" @change="onOpacityCommit" /><p>数值越低，悬浮窗越隐蔽。</p></div>
            <p v-else class="card-desc">当前平台暂不支持悬浮窗整体透明度。</p>
            <div class="inline-setting"><div><h3>单色显示</h3><p>统一文字颜色，代替红涨绿跌。</p></div><button class="switch" :class="{ on: settings.tickerSingleColor }" role="switch" aria-label="悬浮窗单色显示" :aria-checked="settings.tickerSingleColor" :disabled="isSaving('single-color')" @click="safelyRun('single-color', () => settings.setTickerSingleColor(!settings.tickerSingleColor))"><span /></button></div>
            <div v-if="settings.tickerSingleColor" class="color-row"><span>字体颜色</span><label><input type="color" aria-label="悬浮窗字体颜色" :value="settings.tickerTextColor" :disabled="isSaving('ticker-color')" @change="onColorCommit" /><code>{{ settings.tickerTextColor }}</code></label></div>
            <div class="ticker-preview" aria-label="悬浮窗外观预览">
              <span>预览（示例行情）</span>
              <div :style="{ opacity: opacityDraft / 100 }">
                <b :style="settings.tickerSingleColor ? { color: settings.tickerTextColor } : undefined">示例股票</b>
                <span :style="{ color: settings.tickerSingleColor ? settings.tickerTextColor : 'var(--color-up)' }">12.34　+2.18%</span>
              </div>
            </div>
            <button class="minor-btn" :disabled="isSaving('ticker-reset')" @click="resetTickerAppearance">恢复悬浮窗外观默认值</button>
          </article>
        </section>

        <section v-else-if="activeSection === 'appearance'" class="settings-panel">
          <header class="panel-heading"><span>05</span><div><h2>外观</h2><p>界面风格与明暗模式可分别选择，自动保存并在下次启动时恢复。</p></div></header>
          <h3 class="appearance-label">界面风格</h3>
          <article class="theme-grid style-grid" role="group" aria-label="界面风格">
            <button class="theme-card style-card classic-preview" :class="{ active: settings.visualStyle === 'classic' }" :aria-pressed="settings.visualStyle === 'classic'" :disabled="isSaving('visual-style')" @click="settings.visualStyle !== 'classic' && safelyRun('visual-style', () => settings.setVisualStyle('classic'))"><i aria-hidden="true"><span /><span /><span /></i><b>原版</b><small>保留当前熟悉的界面样式</small></button>
            <button class="theme-card style-card modern-preview" :class="{ active: settings.visualStyle === 'modern' }" :aria-pressed="settings.visualStyle === 'modern'" :disabled="isSaving('visual-style')" @click="settings.visualStyle !== 'modern' && safelyRun('visual-style', () => settings.setVisualStyle('modern'))"><i aria-hidden="true"><span /><span /><span /></i><b>清晰现代</b><small>更舒展的布局与清楚的层级</small></button>
            <button class="theme-card style-card elegant-preview" :class="{ active: settings.visualStyle === 'elegant' }" :aria-pressed="settings.visualStyle === 'elegant'" :disabled="isSaving('visual-style')" @click="settings.visualStyle !== 'elegant' && safelyRun('visual-style', () => settings.setVisualStyle('elegant'))">
              <i class="elegant-sheet" aria-hidden="true"><span class="preview-nav">行情　研究　账户</span><span class="preview-quote"><em>上证指数</em><strong>3,268.52</strong><small>+0.72%</small></span><span class="preview-lines" /></i>
              <b>中文雅致</b><small>中文易读 · 纸面留白 · 清楚的行情数字</small>
            </button>
          </article>
          <p class="appearance-note">中文雅致采用本机微软雅黑 / 苹方，搭配 Arial 对齐数字、舒展表格与玉青强调；三种风格均支持深浅切换。</p>
          <h3 class="appearance-label">明暗模式</h3>
          <article class="theme-grid" role="group" aria-label="明暗模式">
            <button class="theme-card light" :class="{ active: settings.theme === 'light' }" :aria-pressed="settings.theme === 'light'" :disabled="isSaving('theme')" @click="settings.theme !== 'light' && safelyRun('theme', () => settings.toggleTheme())"><i aria-hidden="true"><span /><span /><span /></i><b>浅色</b><small>清晰明快</small></button>
            <button class="theme-card dark" :class="{ active: settings.theme === 'dark' }" :aria-pressed="settings.theme === 'dark'" :disabled="isSaving('theme')" @click="settings.theme !== 'dark' && safelyRun('theme', () => settings.toggleTheme())"><i aria-hidden="true"><span /><span /><span /></i><b>深色</b><small>专注低光</small></button>
          </article>
        </section>

        <section v-else class="settings-panel">
          <header class="panel-heading"><span>06</span><div><h2>系统</h2><p>配置启动行为与运行方式。</p></div></header>
          <article class="setting-card">
            <div class="inline-setting"><div><h3>开机自启</h3><p>登录 Windows 时自动启动 Bull Arrives。</p></div><button class="switch" :class="{ on: settings.autoLaunch }" role="switch" aria-label="开机自启" :aria-checked="settings.autoLaunch" :disabled="isSaving('autostart')" @click="safelyRun('autostart', () => settings.toggleAutoLaunch())"><span /></button></div>
            <div class="system-line"><span>运行模式</span><b>{{ settings.isPortable ? '便携模式' : '标准安装' }}</b></div>
            <div class="system-line"><span>版本</span><b v-if="buildInfo">v{{ buildInfo.version }} · {{ buildInfo.profile }}</b><b v-else>读取中…</b></div>
            <div class="system-line"><span>构建时间</span><b v-if="buildInfo">{{ buildInfo.built_at }}</b><b v-else>—</b></div>
            <div v-for="item in systemPaths" :key="item.label" class="system-line history-program-row"><span>{{ item.label }}</span><code class="path-text" :title="displayPath(item.path)">{{ displayPath(item.path) }}</code><button class="minor-btn" :aria-label="'复制' + item.label + '路径'" @click="safelyRun('copy-' + item.label, () => copyPath(item.path))">复制</button></div>
            <p v-if="buildInfo" class="build-hint">排查问题时请核对程序位置、数据库路径与构建时间；切换便携模式会使用不同的数据目录。</p>
          </article>
          <WindowSizeSettings />
        </section>
      </main>
    </div>
    <template #footer><div class="settings-footer"><span>开关等自动保存；路径、超时和预算须点击对应的保存按钮</span><button class="done-btn" @click="close">完成</button></div></template>
  </NModal>
</template>

<style scoped>
.settings-shell { font-family: var(--font-sans); font-size: var(--text-base); display: grid; grid-template-columns: 154px minmax(0, 1fr); min-height: 0; gap: 18px; }
.section-nav { display: flex; flex-direction: column; gap: 5px; padding: 4px; border-right: 1px solid var(--color-border-0); }
.nav-item { position: relative; display: flex; flex-direction: column; align-items: flex-start; gap: 1px; padding: 10px 12px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--color-text-secondary); text-align: left; cursor: pointer; transition: background var(--transition-fast), color var(--transition-fast); }
.nav-item small { color: var(--color-text-secondary); font-family: var(--font-sans); font-size: var(--text-xs); letter-spacing: .08em; }
.nav-item span { font-size: var(--text-sm); font-weight: var(--font-weight-medium); }
.nav-item:hover { background: var(--color-surface-1); color: var(--color-text-primary); }
.nav-item.active { background: color-mix(in srgb, var(--color-accent) 12%, var(--color-surface-1)); color: var(--color-accent); }
.nav-item.active::before { position: absolute; left: 0; top: 10px; bottom: 10px; width: 2px; border-radius: 2px; background: var(--color-accent); content: ''; }
.settings-content { min-width: 0; min-height: 0; overflow-y: auto; padding: 4px 8px 4px 0; }
.settings-error { margin-bottom: 12px; }
.settings-panel { display: flex; flex-direction: column; gap: 12px; animation: panel-in 150ms ease-out; }
.panel-heading { display: flex; align-items: flex-start; gap: 10px; margin-bottom: 2px; }
.panel-heading > span { padding-top: 3px; color: var(--color-accent); font-family: var(--font-mono); font-size: var(--text-xs); }
.panel-heading h2 { margin: 0; color: var(--color-text-primary); font-size: 18px; letter-spacing: -.02em; }
.panel-heading p, .card-desc { margin: 3px 0 0; color: var(--color-text-secondary); font-size: var(--text-sm); }
.setting-card { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-0); box-shadow: var(--shadow-sm); }
html[data-style="modern"] .setting-card { padding: var(--panel-padding); background: var(--color-surface-1); }
html[data-style="modern"] .settings-shell { gap: var(--space-4); }
.hero-card { background: linear-gradient(145deg, color-mix(in srgb, var(--color-accent) 6%, var(--color-surface-0)), var(--color-surface-0) 58%); }
.accent-card { border-top: 2px solid var(--color-accent); }
.compact-card { padding: 14px 16px; }
.setting-card h3 { margin: 0; color: var(--color-text-primary); font-size: var(--text-base); }
.setting-card p { margin: 4px 0 0; color: var(--color-text-secondary); font-size: var(--text-sm); line-height: 1.65; }
.card-title-row, .inline-setting { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.switch { position: relative; width: 38px; height: 22px; flex: 0 0 38px; padding: 0; border: 0; border-radius: 999px; background: var(--color-border-1); cursor: pointer; transition: background var(--transition-fast); }
.switch span { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,.28); transition: transform var(--transition-fast); }
.switch.on { background: var(--color-accent); }
.switch.on span { transform: translateX(16px); }
.switch:disabled { opacity: .55; cursor: wait; }
.session-status { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 8px; margin-top: 16px; padding: 10px 12px; border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-secondary); font-size: var(--text-xs); }
.session-status i { width: 7px; height: 7px; border-radius: 50%; background: var(--color-text-tertiary); }
.session-status i.trading { background: #3fb950; box-shadow: 0 0 0 4px rgba(63,185,80,.12); }
.session-status b, .slider-block b { color: var(--color-accent); font-family: var(--font-numeric, var(--font-sans)); }
.session-calendar { margin-top: 8px; color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.5; }
.slider-block { margin-top: 16px; }
.slider-block > div { display: flex; justify-content: space-between; color: var(--color-text-secondary); font-size: var(--text-xs); }
.slider-block input[type='range'] { width: 100%; height: 4px; margin: 14px 0 5px; appearance: none; border-radius: 999px; background: var(--color-border-1); }
.slider-block input::-webkit-slider-thumb { width: 16px; height: 16px; appearance: none; border-radius: 50%; background: var(--color-accent); box-shadow: 0 1px 4px rgba(0,0,0,.3); cursor: pointer; }
.alert-guidance { margin-top: 16px; padding: 12px; border-left: 2px solid var(--color-accent); border-radius: 0 var(--radius-sm) var(--radius-sm) 0; background: var(--color-surface-1); }
.alert-guidance b { color: var(--color-text-secondary); font-size: var(--text-xs); }
.explain-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin-top: 12px; }
.explain-grid div { display: flex; flex-direction: column; gap: 3px; padding: 9px; border-radius: var(--radius-sm); background: var(--color-surface-1); }
.explain-grid b { color: var(--color-text-secondary); font-size: var(--text-xs); }
.explain-grid span { color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.4; }
.ticker-mode-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; margin-top: 12px; }
.ticker-mode-options label { display: flex; min-width: 0; gap: 8px; padding: 10px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); cursor: pointer; }
.ticker-mode-options label:has(input:checked) { border-color: var(--color-accent); background: color-mix(in srgb, var(--color-accent) 8%, transparent); }
.ticker-mode-options input { margin-top: 2px; accent-color: var(--color-accent); }
.ticker-mode-options span, .ticker-count-row > span { display: flex; min-width: 0; flex-direction: column; gap: 3px; }
.ticker-mode-options b, .ticker-count-row b { color: var(--color-text-primary); font-size: var(--text-xs); }
.ticker-mode-options small, .ticker-count-row small { color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.45; }
.ticker-count-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--color-border-0); }
.ticker-count-row input { width: 72px; min-height: 32px; padding: 0 8px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-numeric, var(--font-sans)); }
.ticker-count-row input:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.hotkey-row { display: flex; gap: 8px; margin-top: 12px; }
.hotkey-box { display: flex; flex: 1; min-height: 42px; align-items: center; justify-content: center; flex-direction: column; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-sans); font-size: var(--text-sm); cursor: pointer; }
.hotkey-box.capturing { border-color: var(--color-accent); color: var(--color-accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 10%, transparent); }
.hotkey-box small { color: var(--color-text-tertiary); font-family: var(--font-sans); font-size: var(--text-xs); }
.minor-btn { font-size: var(--text-sm); min-height: 32px; padding: 0 12px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-secondary); font-family: var(--font-sans); cursor: pointer; }
.notification-action { margin-top: 10px; }
.inline-setting { margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--color-border-0); }
.ticker-preview { width: 100%; margin: 12px 0; color: var(--color-text-tertiary); font-size: var(--text-xs); }
.ticker-preview > div { display: flex; justify-content: space-between; gap: 12px; margin-top: 7px; padding: 12px; border-radius: var(--radius-sm); background: var(--color-surface-2); color: var(--color-text-primary); }
.ticker-preview > div span { font-family: var(--font-mono); }
.color-row, .system-line { display: flex; align-items: center; justify-content: space-between; margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--color-border-0); color: var(--color-text-secondary); font-size: var(--text-xs); }
.color-row label { display: flex; align-items: center; gap: 8px; }
.color-row input { width: 34px; height: 24px; padding: 1px; border: 1px solid var(--color-border-1); border-radius: 4px; background: none; }
.color-row code, .system-line b { color: var(--color-text-primary); font-family: var(--font-sans); }
.build-hint { margin-top: 8px; color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.5; }
.page-size-label { color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-status { display: flex; flex-direction: column; gap: 3px; margin-top: 12px; padding: 10px 12px; border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-tertiary); font-size: var(--text-xs); }
.history-status.connected b { color: #3fb950; }
.history-status.unavailable b, .history-status.not_started b { color: #d29922; }
.history-status small { font-family: var(--font-sans); font-size: var(--text-xs); line-height: 1.6; }
.history-field { display: grid; grid-template-columns: 110px minmax(0, 1fr) auto; align-items: center; gap: 8px; margin-top: 10px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-field input { min-width: 0; min-height: 32px; font-family: var(--font-sans); font-size: var(--text-sm); padding: 0 9px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); }
.field-btns { display: flex; flex-wrap: wrap; gap: 6px; justify-content: flex-end; }
.run-dir-line { margin-top: 8px; }
.run-dir-line code { color: var(--color-text-primary); }
.history-actions, .candidate-list { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
.candidate-list { flex-direction: column; }
.candidate-row { display: flex; min-width: 0; align-items: flex-start; gap: 8px; }
.candidate-row .candidate-btn { flex: 1; min-width: 0; padding-block: 7px; white-space: normal; overflow-wrap: anywhere; }
.path-text { display: block; min-width: 0; font-family: var(--font-sans); font-size: var(--text-sm); line-height: 1.6; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
.status-path { display: flex; min-width: 0; align-items: flex-start; gap: 8px; }
.status-path code { flex: 1; }
.copy-feedback { margin: 0 0 8px; color: var(--color-text-secondary); font-size: var(--text-sm); }
.settings-shell :deep(.hotkey-box) { font-family: var(--font-sans); }
.settings-shell :deep(.hotkey-box small) { font-size: var(--text-xs); }
.history-program-row { display: grid; grid-template-columns: 108px minmax(0, 1fr) auto; align-items: center; gap: 8px; margin-top: 10px; font-size: var(--text-xs); color: var(--color-text-secondary); }
.history-program-row code { min-width: 0; padding: 7px 9px; overflow-wrap: anywhere; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-sans); }
.history-advanced { margin-top: 12px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-advanced summary { cursor: pointer; user-select: none; }
.history-advanced > small { display: block; margin-top: 8px; font-size: var(--text-xs); line-height: 1.6; }
.candidate-btn { text-align: left; }
.history-status.running_owned b, .history-status.running_external b { color: #3fb950; }
.history-status.error b, .history-status.not_configured b { color: #d29922; }
.page-size-select {
  min-height: 30px;
  padding: 0 8px;
  border: 1px solid var(--color-border-1);
  border-radius: var(--radius-sm);
  background: var(--color-surface-1);
  color: var(--color-text-primary);
  font-family: var(--font-sans);
  cursor: pointer;
}
.field-error { color: var(--color-warning) !important; }
.theme-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.appearance-label { margin: 4px 0 0; color: var(--color-text-secondary); font-size: var(--text-sm); }
.style-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }
@media (max-width: 760px) { .style-grid { grid-template-columns: 1fr; } }
.theme-card { display: flex; flex-direction: column; align-items: flex-start; gap: 5px; padding: 14px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-0); color: var(--color-text-primary); cursor: pointer; transition: transform var(--transition-fast), border-color var(--transition-fast); }
.theme-card:hover { transform: translateY(-2px); border-color: var(--color-border-1); }
.theme-card.active { border-color: var(--color-accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 12%, transparent); }
.theme-card > i { display: grid; grid-template-columns: 28% 1fr; grid-template-rows: repeat(2, 1fr); gap: 4px; width: 100%; height: 110px; margin-bottom: 6px; padding: 8px; border-radius: var(--radius-sm); background: #f4f6f8; box-shadow: inset 0 0 0 1px #d9dee5; }
.theme-card > i span { border-radius: 3px; background: #d8dee7; }
.theme-card > i span:first-child { grid-row: 1 / 3; background: #c6cfdb; }
.theme-card.dark > i { background: #111820; box-shadow: inset 0 0 0 1px #2b3541; }
.theme-card.dark > i span { background: #26313d; }
.theme-card.dark > i span:first-child { background: #1d2732; }
.style-card > i { height: 72px; grid-template-rows: 14px 1fr; background: var(--color-surface-1); box-shadow: inset 0 0 0 1px var(--color-border-0); }
.style-card > i span:first-child { grid-row: auto; grid-column: 1 / 3; background: var(--color-accent-dim); }
.style-card > i span { background: var(--color-surface-2); }
.style-card.modern-preview > i { gap: 7px; padding: 10px; border-radius: 12px; }
.style-card.modern-preview > i span { border-radius: 7px; }
.style-card.classic-preview > i { background: #f6f8fa; box-shadow: inset 0 0 0 1px #d0d7de; }
.style-card.classic-preview > i span { background: #e2e5ea; }
.style-card.classic-preview > i span:first-child { background: #d0d7de; }
.style-card.theme-card b { font-size: var(--text-sm); }
.appearance-note { margin: 0; color: var(--color-text-secondary); font-size: var(--text-xs); line-height: 1.7; }
.style-card.elegant-preview .elegant-sheet { height: 92px; grid-template-columns: 1fr 25%; grid-template-rows: 18px 1fr; gap: 7px; padding: 9px; border-radius: 4px; background: #f5f4f0; box-shadow: inset 0 0 0 1px #dddeda; font-style: normal; }
.elegant-sheet .preview-nav { grid-column: 1 / 3; color: #256b65; background: transparent; font: 12px "Microsoft YaHei UI", "Microsoft YaHei", sans-serif; border-bottom: 1px solid #d4dcd7; }
.elegant-sheet .preview-quote { display: grid; grid-template-columns: 1fr auto; grid-column: 1; padding: 3px 5px; border-radius: 2px; border-left: 2px solid #bc3f40; background: #fff; text-align: left; }
.preview-quote em { grid-column: 1 / 3; color: #586962; font: normal 12px "Microsoft YaHei", sans-serif; }
.preview-quote strong { color: #243b36; font: 600 12px Arial, sans-serif; font-variant-numeric: tabular-nums; }
.preview-quote small { align-self: end; color: #b33f40; font: 12px Arial, sans-serif; }
.elegant-sheet .preview-lines { background: repeating-linear-gradient(to bottom, #e1e5df 0 1px, transparent 1px 9px); border-radius: 0; }
html[data-style="modern"][data-appearance="elegant"] .settings-panel { gap: 16px; }
html[data-style="modern"][data-appearance="elegant"] .panel-heading { gap: 14px; padding-bottom: 12px; border-bottom: 1px solid var(--color-border-0); }
html[data-style="modern"][data-appearance="elegant"] .panel-heading h2 { font-size: 20px; line-height: 1.5; font-weight: 600; }
html[data-style="modern"][data-appearance="elegant"] .panel-heading p { font-size: 13px; line-height: 1.7; }
html[data-style="modern"][data-appearance="elegant"] .theme-card { gap: 8px; padding: 16px; text-align: left; }
html[data-style="modern"][data-appearance="elegant"] .theme-card b { font-size: 14px; }
html[data-style="modern"][data-appearance="elegant"] .section-nav .nav-item { padding: 12px 14px; gap: 3px; }
html[data-style="modern"][data-appearance="elegant"] .section-nav .nav-item span { font-size: 14px; }
.theme-card small { color: var(--color-text-secondary); font-family: var(--font-sans); font-size: var(--text-xs); line-height: 1.6; }
.settings-footer { display: flex; align-items: center; justify-content: space-between; color: var(--color-text-tertiary); font-size: var(--text-xs); }
.done-btn { height: 32px; padding: 0 20px; border: 0; border-radius: var(--radius-sm); background: var(--color-accent); color: #fff; font-family: var(--font-sans); cursor: pointer; }
.settings-shell :is(button, input, select, summary):focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
@media (prefers-reduced-motion: reduce) { .settings-panel { animation: none; } .settings-shell * { transition-duration: 0.01ms !important; } }
@keyframes panel-in { from { opacity: 0; transform: translateY(3px); } }
@media (max-width: 680px) {
  .settings-shell { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); }
  .section-nav { flex-direction: row; overflow-x: auto; padding-bottom: 8px; border-right: 0; border-bottom: 1px solid var(--color-border-0); }
  .nav-item { flex: 0 0 auto; min-width: 60px; padding-inline: 8px; align-items: center; }
  .nav-item.active::before { top: auto; right: 12px; bottom: 0; width: auto; height: 2px; }
  .settings-content { padding: 0; }
  .explain-grid { grid-template-columns: 1fr; }
  .history-field, .history-program-row { grid-template-columns: minmax(0, 1fr); align-items: stretch; }
  .history-field .field-btns, .history-program-row .field-btns { justify-content: flex-start; }
  .history-program-row code { overflow-wrap: anywhere; }
}
@media (max-width: 460px) {
  .theme-grid { grid-template-columns: 1fr; }
  .style-grid { grid-template-columns: 1fr; }
  .card-title-row { align-items: flex-start; }
  .settings-footer > span { display: none; }
  .settings-footer { justify-content: flex-end; }
}
</style>

<style>
/* NModal 将 class 透传给卡片根节点，根节点没有本组件的 scoped 属性。 */
.settings-modal.n-card {
  height: min(662px, calc(100dvh - 16px));
  max-height: calc(100dvh - 16px);
  overflow: hidden;
}
html[data-style="modern"] .settings-modal.n-card {
  height: min(782px, calc(100dvh - 16px));
}
.settings-modal.n-card > .n-card-content {
  display: flex;
  min-height: 0;
  overflow: hidden;
}
.settings-modal.n-card .settings-shell {
  flex: 1 1 auto;
  height: auto;
  min-height: 0;
  min-width: 0;
}
</style>
