<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { NAlert, NModal } from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, REFRESH_INTERVAL_AUTO } from '@/stores/settings';
import { useUniverseStore } from '@/stores/universe';
import QuoteScheduleSettings from './QuoteScheduleSettings.vue';
import WindowSizeSettings from './WindowSizeSettings.vue';
import GroupHotkeySettings from './GroupHotkeySettings.vue';
import { eventToHotkey, formatHotkeyLabel } from '@/utils/hotkey';

const props = defineProps<{ show: boolean }>();
const emit = defineEmits<{ 'update:show': [value: boolean] }>();
const settings = useSettingsStore();
const universeStore = useUniverseStore();

/** 筛选结果分页大小的可选项 */
const universePageSizeOptions = [20, 50, 100];

function onUniversePageSizeChange(event: Event) {
  const value = Number((event.target as HTMLSelectElement).value);
  void universeStore.setPageSize(value);
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
const activeSection = ref<SectionKey>('market');
const actionError = ref<string | null>(null);
const savingKeys = ref(new Set<string>());
interface NotificationIdentityStatus { supported: boolean; registered: boolean; shortcut_path?: string; detail: string }
interface NotificationTestStatus { native: 'accepted' | 'failed'; native_error?: string; desktop: string; desktop_error?: string }
interface BuildInfo { version: string; built_at: string; profile: string; exe_path: string }
interface LocalHistoryStatus {
  state: 'connected' | 'not_started' | 'not_found' | 'unavailable';
  message: string;
  source_format?: string;
  start_date?: string;
  end_date?: string;
  sample_count?: number;
  candidates: string[];
}
interface AgentStatus { installed: boolean; state: string; path: string | null; message: string; guidance: string }
const notificationIdentity = ref<NotificationIdentityStatus | null>(null);
const notificationResult = ref<NotificationTestStatus | null>(null);
const buildInfo = ref<BuildInfo | null>(null);
const localHistoryStatus = ref<LocalHistoryStatus | null>(null);
const localHistoryUrlDraft = ref('http://127.0.0.1:7899');
const agentStatus = ref<AgentStatus | null>(null);
const agentPathDraft = ref('');
const agentTimeoutDraft = ref(90);

const capturing = ref(false);
const capturedCombo = ref<string | null>(null);
const hotkeyError = ref<string | null>(null);
const opacityDraft = ref(100);
const intervalDraft = ref(3);
const intervalAuto = computed(() => settings.refreshInterval === REFRESH_INTERVAL_AUTO);
const tickerOpacitySupported = typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent);

watch(() => props.show, (open) => {
  if (open) {
    opacityDraft.value = settings.tickerOpacity;
    intervalDraft.value = intervalAuto.value
      ? (settings.marketSession.interval_secs || 3)
      : settings.refreshInterval;
    actionError.value = null;
    localHistoryUrlDraft.value = settings.localHistoryUrl;
    agentPathDraft.value = settings.settings['agent_claude_path'] || '';
    agentTimeoutDraft.value = Number(settings.settings['agent_timeout_seconds'] || 90);
    safelyRun('session', () => settings.fetchMarketSession());
    if (settings.localHistoryEnabled) void loadLocalHistoryStatus();
    void settings.fetchStockDbStatus();
    void loadNotificationIdentity();
    void loadBuildInfo();
    void loadAgentStatus();
  } else {
    stopCapture();
  }
}, { immediate: true });

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
  safelyRun('opacity', () => settings.setTickerOpacity(value));
}
function onColorCommit(event: Event) {
  const value = (event.target as HTMLInputElement).value;
  safelyRun('ticker-color', () => settings.setTickerTextColor(value));
}
async function loadNotificationIdentity() {
  try { notificationIdentity.value = await invoke<NotificationIdentityStatus>('get_notification_identity_status'); }
  catch (error) { notificationIdentity.value = { supported: true, registered: false, detail: String(error) }; }
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
async function registerNotificationIdentity() {
  notificationIdentity.value = await invoke<NotificationIdentityStatus>('register_notification_identity');
}
async function sendTestNotification() {
  notificationResult.value = await invoke<NotificationTestStatus>('test_notification');
}
async function loadAgentStatus() {
  try { agentStatus.value = await invoke<AgentStatus>('get_agent_status'); }
  catch (error) { agentStatus.value = { installed: false, state: 'unavailable', path: null, message: String(error), guidance: '请手动检查 Claude Code。' }; }
}
async function saveAgentPath() {
  if (!await settings.setSetting('agent_claude_path', agentPathDraft.value.trim())) return false;
  await loadAgentStatus();
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
function close() {
  stopCapture();
  emit('update:show', false);
}

onBeforeUnmount(stopCapture);
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    title="偏好设置"
    class="settings-modal"
    :style="{ width: 'min(860px, 95vw)' }"
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
          @click="activeSection = section.key"
        >
          <small>{{ section.eyebrow }}</small>
          <span>{{ section.label }}</span>
        </button>
      </nav>

      <main class="settings-content">
        <NAlert v-if="actionError" type="error" :show-icon="false" closable class="settings-error" @close="actionError = null">
          保存失败：{{ actionError }}。设置未被视为成功，请重试。
        </NAlert>

        <section v-if="activeSection === 'market'" class="settings-panel">
          <header class="panel-heading"><span>01</span><div><h2>行情节奏</h2><p>平衡实时性与请求频率。</p></div></header>
          <article class="setting-card hero-card">
            <div class="card-title-row">
              <div><h3>自动调节刷新间隔</h3><p>根据交易时段使用推荐频率，休市时自动降频。</p></div>
              <button class="switch" :class="{ on: intervalAuto }" role="switch" :aria-checked="intervalAuto" :disabled="isSaving('interval')" @click="setIntervalAuto(!intervalAuto)"><span /></button>
            </div>
            <div v-if="intervalAuto" class="session-status">
              <i :class="{ trading: settings.marketSession.is_trading }" />
              <span>{{ settings.marketSession.session }}</span><b>{{ settings.marketSession.interval_secs }} 秒</b>
            </div>
            <div v-else class="slider-block">
              <div><span>固定间隔</span><b>{{ intervalDraft }}s</b></div>
              <input type="range" min="1" max="60" step="1" :value="intervalDraft" @input="onIntervalInput" @change="onIntervalCommit" />
              <p>固定模式不会跟随交易时段自动变化。</p>
            </div>
            <p class="session-calendar">{{ settings.marketSession.calendar }}</p>
          </article>
          <QuoteScheduleSettings />
          <article class="setting-card">
            <h3>筛选结果每页条数</h3>
            <p class="card-desc">全市场筛选器结果表的分页大小，范围 1–100 条，默认 20 条。</p>
            <div class="inline-setting">
              <div><b class="page-size-label">每页显示</b><p>在筛选结果表底部也可以随时改。</p></div>
              <select
                class="page-size-select"
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
              <button class="switch" :class="{ on: settings.localHistoryEnabled }" role="switch" :aria-checked="settings.localHistoryEnabled" :disabled="isSaving('local-history-enabled') || settings.stockDbStatus?.busy" @click="safelyRun('local-history-enabled', () => settings.setLocalHistoryEnabled(!settings.localHistoryEnabled))"><span /></button>
            </div>
            <div class="history-status" :class="settings.stockDbStatus?.state">
              <b>{{ stockDbStateLabel() }}</b>
              <span>{{ settings.stockDbStatus?.message || '正在检测…' }}</span>
              <small v-if="settings.stockDbStatus?.owned">此进程由 Bull Arrives 管理，托盘真正退出时会一并关闭。</small>
              <small v-else-if="settings.stockDbStatus?.state === 'running_external'">外部进程不会被 Bull Arrives 停止或更新。</small>
            </div>
            <template v-if="settings.stockDbStatus?.platformSupported !== false">
              <div class="history-program-row">
                <span>stockdb 程序</span>
                <code :title="settings.localHistoryEnginePath || '尚未选择'">{{ settings.localHistoryEnginePath || '尚未选择 stockdb.exe' }}</code>
                <button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('stockdb-engine')" @click="safelyRun('stockdb-engine', browseStockDbEngine)">浏览选择</button>
              </div>
              <div class="history-program-row">
                <span>数据更新程序</span>
                <code :title="settings.localHistoryUpdaterPath || '尚未找到'">{{ settings.localHistoryUpdaterPath || '尚未找到 数据更新.exe' }}</code>
                <button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('stockdb-updater')" @click="safelyRun('stockdb-updater', browseStockDbUpdater)">浏览选择</button>
              </div>
              <div class="history-actions">
                <button class="minor-btn" :disabled="settings.stockDbStatus?.busy || isSaving('local-history-scan')" @click="safelyRun('local-history-scan', scanLocalHistory)">自动查找</button>
                <button v-if="settings.localHistoryEnabled" class="minor-btn" :disabled="isSaving('local-history-test')" @click="safelyRun('local-history-test', testLocalHistory)">测试连接</button>
              </div>
              <div v-if="settings.stockDbStatus?.candidates.length" class="candidate-list">
                <button v-for="candidate in settings.stockDbStatus.candidates" :key="candidate.enginePath" class="minor-btn candidate-btn" :title="candidate.enginePath" @click="safelyRun('stockdb-candidate', () => chooseStockDbCandidate(candidate.enginePath))">{{ candidate.source }} · {{ candidate.enginePath }}</button>
              </div>
              <details v-if="settings.localHistoryEnabled" class="history-advanced">
                <summary>高级连接设置</summary>
                <label class="history-field"><span>服务地址</span><input v-model="localHistoryUrlDraft" type="url" placeholder="http://127.0.0.1:7899" /><button class="minor-btn" :disabled="isSaving('local-history-url')" @click="safelyRun('local-history-url', saveLocalHistoryUrl)">保存并测试</button></label>
                <small v-if="localHistoryStatus?.start_date && localHistoryStatus?.end_date">样本区间 {{ localHistoryStatus.start_date }} → {{ localHistoryStatus.end_date }} · {{ localHistoryStatus.sample_count }} 根 · {{ localHistoryStatus.source_format }}</small>
              </details>
            </template>
            <p class="card-desc">关闭时不会启动或读取本地数据库。更新时仅停止由 Bull Arrives 启动的 stockdb，显示“数据更新.exe”窗口，完成后自动重启。</p>
          </article>
        </section>

        <section v-else-if="activeSection === 'alerts'" class="settings-panel">
          <header class="panel-heading"><span>02</span><div><h2>行情提醒</h2><p>总开关只控制提醒是否运行，不覆盖逐票规则。</p></div></header>
          <article class="setting-card accent-card">
            <div class="card-title-row">
              <div><h3>启用行情提醒</h3><p>关闭后所有提醒暂停，逐票阈值和开关保持不变。</p></div>
              <button class="switch" :class="{ on: settings.alertsEnabled }" role="switch" :aria-checked="settings.alertsEnabled" :disabled="isSaving('alerts')" @click="safelyRun('alerts', () => settings.setSetting('alerts_enabled', settings.alertsEnabled ? '0' : '1'))"><span /></button>
            </div>
            <div class="alert-guidance">
              <b>逐票设置入口</b>
              <p>在自选表格双击股票，或右键选择“设置行情提醒”。涨跌幅按昨收每日重新计数；固定价格规则长期有效。</p>
              <p>仅监控当前分组中的股票，切换分组后其他股票暂停监控。</p>
            </div>
          </article>
          <article class="setting-card">
            <div class="card-title-row">
              <div><h3>资讯摘要与时序盯盘</h3><p>默认关闭。交易日按规则检查关注池资讯；命中后尝试 Agent 摘要，失败回退原文。盘前、盘后和夜间卡片统一进入提醒记录，额外休市日不触发。</p></div>
              <button class="switch" :class="{ on: settings.newsNotificationsEnabled }" role="switch" :aria-checked="settings.newsNotificationsEnabled" :disabled="isSaving('news-notifications')" @click="safelyRun('news-notifications', () => settings.setSetting('news_notifications_enabled', settings.newsNotificationsEnabled ? '0' : '1'))"><span /></button>
            </div>
            <p class="card-desc">来源：东方财富上市公司快讯与公司公告。首次开启只建立当前水位，不推送历史内容。</p>
          </article>
          <article class="setting-card compact-card">
            <h3>Windows 通知身份</h3>
            <p>绿色版需要当前用户开始菜单快捷方式声明本应用 AUMID。只会在您点击后注册，不需要管理员权限，也不会修改勿扰或系统策略。</p>
            <p v-if="notificationIdentity">{{ notificationIdentity.detail }}</p>
            <button v-if="notificationIdentity?.supported" class="minor-btn notification-action" :disabled="isSaving('identity')" @click="safelyRun('identity', registerNotificationIdentity)">{{ notificationIdentity.registered ? '修复 Windows 通知身份' : '启用 Windows 通知' }}</button>
            <h3 style="margin-top: 16px">通知通道</h3>
            <p>默认 Windows 优先，真实发送失败自动用独立桌面提醒兜底。系统返回“已受理”不代表横幅一定可见。</p>
            <div class="inline-setting"><div><h3>同时显示桌面提醒</h3><p>适合勿扰或企业策略会隐藏 Windows 横幅的电脑。</p></div><button class="switch" :class="{ on: settings.notificationDesktopAlways }" role="switch" :aria-checked="settings.notificationDesktopAlways" :disabled="isSaving('desktop-toast')" @click="safelyRun('desktop-toast', () => settings.setSetting('notification_desktop_always', settings.notificationDesktopAlways ? '0' : '1'))"><span /></button></div>
            <button class="minor-btn notification-action" :disabled="isSaving('notification')" @click="safelyRun('notification', sendTestNotification)">发送测试通知</button>
            <p v-if="notificationResult">Windows：{{ notificationResult.native === 'accepted' ? '已受理' : `失败（${notificationResult.native_error || '未知错误'}）` }}；桌面：{{ notificationResult.desktop === 'queued' ? '已排队' : notificationResult.desktop === 'not-requested' ? '未启用' : `失败（${notificationResult.desktop_error || '未知错误'}）` }}</p>
            <button v-if="notificationResult?.native === 'accepted' && !settings.notificationDesktopAlways" class="minor-btn notification-action" @click="safelyRun('desktop-toast', () => settings.setSetting('notification_desktop_always', '1'))">没看到系统通知，启用桌面兜底</button>
            <h3 style="margin-top: 16px">重复方式说明</h3>
            <div class="explain-grid"><div><b>重新穿越</b><span>回到阈值另一侧，再次穿越才提醒</span></div><div><b>冷却间隔</b><span>N 分钟内最多提醒一次</span></div><div><b>每日一次</b><span>当天触发一次，次日重新生效</span></div></div>
          </article>
        </section>

        <section v-else-if="activeSection === 'ai'" class="settings-panel">
          <header class="panel-heading"><span>03</span><div><h2>AI / 量化智能</h2><p>总开关统一管理所有自动运行的智能功能。</p></div></header>
          <article class="setting-card accent-card">
            <div class="card-title-row">
              <div><h3>AI 智能总开关</h3><p>关闭后，后台自动运行的 AI / 量化功能全部停止；手动点击的分析与推荐仍可照常使用。</p></div>
              <button class="switch" :class="{ on: settings.aiEnabled }" role="switch" :aria-checked="settings.aiEnabled" :disabled="isSaving('ai')" @click="safelyRun('ai', () => settings.setSetting('ai_enabled', settings.aiEnabled ? '0' : '1'))"><span /></button>
            </div>
            <div v-if="!settings.aiEnabled" class="alert-guidance">
              <b>已全部停用</b>
              <p>智能监控已暂停，不会再产生任何后台计算与提醒。逐票的涨跌幅 / 固定价格提醒属于「提醒」分类，不受此处影响。</p>
            </div>
          </article>
          <article class="setting-card">
            <h3>自动运行的功能</h3>
            <p class="card-desc">这些功能在后台持续运行，因此可单独开关，并统一受上面的总开关约束。</p>
            <div class="inline-setting">
              <div>
                <h3>智能监控 · 量化止损止盈</h3>
                <p>按 ATR14 自动计算止损 / 止盈位，盘中价格触及即提醒；价格和阈值全部由模型算出，无需手填。</p>
              </div>
              <button
                class="switch"
                :class="{ on: settings.aiEnabled && settings.aiMonitorEnabled }"
                role="switch"
                :aria-checked="settings.aiEnabled && settings.aiMonitorEnabled"
                :disabled="!settings.aiEnabled || isSaving('ai-monitor')"
                @click="safelyRun('ai-monitor', () => settings.setSetting('ai_monitor_enabled', settings.aiMonitorEnabled ? '0' : '1'))"
              ><span /></button>
            </div>
          </article>
          <article class="setting-card">
            <h3>本地 Agent · Claude Code</h3>
            <p class="card-desc">手动生成解读或多角色研判；开启资讯摘要、动态筛选时也会按对应规则调用。模型和登录沿用 Claude Code 配置，应用不保存凭据。</p>
            <div class="history-status" :class="agentStatus?.state === 'ready' ? 'connected' : 'not_found'">
              <b>{{ agentStatus?.state === 'ready' ? '连接正常' : agentStatus?.state === 'failed' ? '连接失败' : agentStatus?.installed ? '已安装 · 待验证' : '不可用' }}</b>
              <span>{{ agentStatus?.message || '正在检测…' }}</span>
              <small v-if="agentStatus?.path">{{ agentStatus.path }}</small>
            </div>
            <label class="history-field"><span>可执行文件</span><input v-model="agentPathDraft" type="text" placeholder="留空自动检测 claude.cmd / claude.exe" /><button class="minor-btn" :disabled="isSaving('agent-path')" @click="safelyRun('agent-path', saveAgentPath)">保存并检测</button></label>
            <label class="history-field"><span>单次超时（秒）</span><input v-model.number="agentTimeoutDraft" type="number" min="15" max="300" step="1" /><button class="minor-btn" :disabled="isSaving('agent-timeout')" @click="safelyRun('agent-timeout', saveAgentTimeout)">保存超时</button></label>
            <div class="history-actions">
              <button class="minor-btn" :disabled="isSaving('agent-scan') || isSaving('agent-test')" @click="safelyRun('agent-scan', loadAgentStatus)">重新检测</button>
              <button class="minor-btn" :disabled="!agentStatus?.installed || isSaving('agent-test')" @click="safelyRun('agent-test', testAgentConnection)">{{ isSaving('agent-test') ? '测试连接中…' : '测试连接' }}</button>
              <button v-if="isSaving('agent-test')" class="minor-btn" @click="safelyRun('agent-cancel', () => invoke('cancel_agent_analysis'))">中止</button>
            </div>
            <p class="card-desc">{{ agentStatus?.guidance }}。测试连接会进行一次简短模型调用，可能产生服务商费用。单并发；每次调用默认 90 秒，支持 15–300 秒；多角色研判依次运行，可中断。失败保留纯量化结果。</p>
          </article>
          <article class="setting-card compact-card">
            <h3>手动触发，无需开关</h3>
            <p>下面这些只有你主动点击时才会运行，不会产生后台请求，所以不占用这里的开关。</p>
            <div class="explain-grid">
              <div><b>量化评分</b><span>双击个股打开「分析」时才计算</span></div>
              <div><b>推荐榜</b><span>在筛选器里点「生成推荐榜」时才扫描</span></div>
              <div><b>涨跌幅 / 价格提醒</b><span>原有逐票规则，请在「提醒」中管理</span></div>
            </div>
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
            <p class="card-desc">轮播可自定义每页数量；固定模式会一次展示当前分组全部股票。</p>
            <div class="ticker-mode-options" role="radiogroup" aria-label="悬浮窗展示方式">
              <label><input type="radio" name="ticker-mode" value="carousel" :checked="settings.tickerDisplayMode === 'carousel'" :disabled="isSaving('ticker-mode')" @change="onTickerModeChange" /><span><b>轮播</b><small>每 3 秒翻页，鼠标悬停暂停</small></span></label>
              <label><input type="radio" name="ticker-mode" value="fixed" :checked="settings.tickerDisplayMode === 'fixed'" :disabled="isSaving('ticker-mode')" @change="onTickerModeChange" /><span><b>固定</b><small>默认展示全部，不自动翻页</small></span></label>
            </div>
            <label v-if="settings.tickerDisplayMode === 'carousel'" class="ticker-count-row">
              <span><b>每页展示数量</b><small>可设置 1–20 只，超过当前分组数量时自动按实际数量展示。</small></span>
              <input type="number" min="1" max="20" step="1" :value="settings.tickerPageSize" :disabled="isSaving('ticker-page-size')" @change="onTickerPageSizeChange" />
            </label>
          </article>
          <article class="setting-card">
            <div v-if="tickerOpacitySupported" class="slider-block"><div><span>透明度</span><b>{{ opacityDraft }}%</b></div><input type="range" min="5" max="100" step="1" :value="opacityDraft" @input="onOpacityInput" @change="onOpacityCommit" /><p>数值越低，悬浮窗越隐蔽。</p></div>
            <p v-else class="card-desc">当前平台暂不支持悬浮窗整体透明度。</p>
            <div class="inline-setting"><div><h3>单色显示</h3><p>统一文字颜色，代替红涨绿跌。</p></div><button class="switch" :class="{ on: settings.tickerSingleColor }" role="switch" :aria-checked="settings.tickerSingleColor" @click="safelyRun('single-color', () => settings.setTickerSingleColor(!settings.tickerSingleColor))"><span /></button></div>
            <div v-if="settings.tickerSingleColor" class="color-row"><span>字体颜色</span><label><input type="color" :value="settings.tickerTextColor" @change="onColorCommit" /><code>{{ settings.tickerTextColor }}</code></label></div>
          </article>
        </section>

        <section v-else-if="activeSection === 'appearance'" class="settings-panel">
          <header class="panel-heading"><span>05</span><div><h2>外观</h2><p>选择适合环境的界面明暗。</p></div></header>
          <article class="theme-grid">
            <button class="theme-card light" :class="{ active: settings.theme === 'light' }" :disabled="isSaving('theme')" @click="settings.theme !== 'light' && safelyRun('theme', () => settings.toggleTheme())"><i><span /><span /><span /></i><b>浅色</b><small>清晰明快</small></button>
            <button class="theme-card dark" :class="{ active: settings.theme === 'dark' }" :disabled="isSaving('theme')" @click="settings.theme !== 'dark' && safelyRun('theme', () => settings.toggleTheme())"><i><span /><span /><span /></i><b>深色</b><small>专注低光</small></button>
          </article>
        </section>

        <section v-else class="settings-panel">
          <header class="panel-heading"><span>06</span><div><h2>系统</h2><p>配置启动行为与运行方式。</p></div></header>
          <article class="setting-card">
            <div class="inline-setting"><div><h3>开机自启</h3><p>登录 Windows 时自动启动 Bull Arrives。</p></div><button class="switch" :class="{ on: settings.autoLaunch }" role="switch" :aria-checked="settings.autoLaunch" :disabled="isSaving('autostart')" @click="safelyRun('autostart', () => settings.toggleAutoLaunch())"><span /></button></div>
            <div class="system-line"><span>运行模式</span><b>{{ settings.isPortable ? '便携模式' : '标准安装' }}</b></div>
            <div class="system-line"><span>版本</span><b v-if="buildInfo">v{{ buildInfo.version }} · {{ buildInfo.profile }}</b><b v-else>读取中…</b></div>
            <div class="system-line"><span>构建时间</span><b v-if="buildInfo">{{ buildInfo.built_at }}</b><b v-else>—</b></div>
            <div v-if="buildInfo" class="system-line exe-line"><span>程序位置</span><code>{{ buildInfo.exe_path }}</code></div>
            <p v-if="buildInfo" class="build-hint">排查问题时请核对这里的构建时间 —— 如果它不是最近一次构建的时间，说明启动的是旧副本。</p>
          </article>
          <WindowSizeSettings />
        </section>
      </main>
    </div>
    <template #footer><div class="settings-footer"><span>修改会立即生效</span><button class="done-btn" @click="close">完成</button></div></template>
  </NModal>
</template>

<style scoped>
.settings-shell { display: grid; grid-template-columns: 150px minmax(0, 1fr); height: min(480px, calc(100vh - 240px)); min-height: 0; gap: 18px; }
.section-nav { display: flex; flex-direction: column; gap: 5px; padding: 4px; border-right: 1px solid var(--color-border-0); }
.nav-item { position: relative; display: flex; flex-direction: column; align-items: flex-start; gap: 1px; padding: 10px 12px; border: 0; border-radius: var(--radius-md); background: transparent; color: var(--color-text-secondary); text-align: left; cursor: pointer; transition: background var(--transition-fast), color var(--transition-fast); }
.nav-item small { color: var(--color-text-tertiary); font-family: var(--font-mono); font-size: 9px; letter-spacing: .12em; }
.nav-item span { font-size: var(--text-sm); font-weight: var(--font-weight-medium); }
.nav-item:hover { background: var(--color-surface-1); color: var(--color-text-primary); }
.nav-item.active { background: color-mix(in srgb, var(--color-accent) 12%, var(--color-surface-1)); color: var(--color-accent); }
.nav-item.active::before { position: absolute; left: 0; top: 10px; bottom: 10px; width: 2px; border-radius: 2px; background: var(--color-accent); content: ''; }
.settings-content { min-width: 0; min-height: 0; overflow-y: auto; padding: 4px 8px 4px 0; }
.settings-error { margin-bottom: 12px; }
.settings-panel { display: flex; flex-direction: column; gap: 12px; animation: panel-in 150ms ease-out; }
.panel-heading { display: flex; align-items: flex-start; gap: 10px; margin-bottom: 2px; }
.panel-heading > span { padding-top: 3px; color: var(--color-accent); font-family: var(--font-mono); font-size: 10px; }
.panel-heading h2 { margin: 0; color: var(--color-text-primary); font-size: 18px; letter-spacing: -.02em; }
.panel-heading p, .card-desc { margin: 3px 0 0; color: var(--color-text-tertiary); font-size: var(--text-xs); }
.setting-card { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-0); box-shadow: var(--shadow-sm); }
.hero-card { background: linear-gradient(145deg, color-mix(in srgb, var(--color-accent) 6%, var(--color-surface-0)), var(--color-surface-0) 58%); }
.accent-card { border-top: 2px solid var(--color-accent); }
.compact-card { padding: 14px 16px; }
.setting-card h3 { margin: 0; color: var(--color-text-primary); font-size: var(--text-sm); }
.setting-card p { margin: 4px 0 0; color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.55; }
.card-title-row, .inline-setting { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.switch { position: relative; width: 38px; height: 22px; flex: 0 0 38px; padding: 0; border: 0; border-radius: 999px; background: var(--color-border-1); cursor: pointer; transition: background var(--transition-fast); }
.switch span { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,.28); transition: transform var(--transition-fast); }
.switch.on { background: var(--color-accent); }
.switch.on span { transform: translateX(16px); }
.switch:disabled { opacity: .55; cursor: wait; }
.session-status { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 8px; margin-top: 16px; padding: 10px 12px; border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-secondary); font-size: var(--text-xs); }
.session-status i { width: 7px; height: 7px; border-radius: 50%; background: var(--color-text-tertiary); }
.session-status i.trading { background: #3fb950; box-shadow: 0 0 0 4px rgba(63,185,80,.12); }
.session-status b, .slider-block b { color: var(--color-accent); font-family: var(--font-mono); }
.session-calendar { margin-top: 8px; color: var(--color-text-tertiary); font-size: 10px; line-height: 1.5; }
.slider-block { margin-top: 16px; }
.slider-block > div { display: flex; justify-content: space-between; color: var(--color-text-secondary); font-size: var(--text-xs); }
.slider-block input[type='range'] { width: 100%; height: 4px; margin: 14px 0 5px; appearance: none; border-radius: 999px; background: var(--color-border-1); }
.slider-block input::-webkit-slider-thumb { width: 16px; height: 16px; appearance: none; border-radius: 50%; background: var(--color-accent); box-shadow: 0 1px 4px rgba(0,0,0,.3); cursor: pointer; }
.alert-guidance { margin-top: 16px; padding: 12px; border-left: 2px solid var(--color-accent); border-radius: 0 var(--radius-sm) var(--radius-sm) 0; background: var(--color-surface-1); }
.alert-guidance b { color: var(--color-text-secondary); font-size: var(--text-xs); }
.explain-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin-top: 12px; }
.explain-grid div { display: flex; flex-direction: column; gap: 3px; padding: 9px; border-radius: var(--radius-sm); background: var(--color-surface-1); }
.explain-grid b { color: var(--color-text-secondary); font-size: var(--text-xs); }
.explain-grid span { color: var(--color-text-tertiary); font-size: 10px; line-height: 1.4; }
.ticker-mode-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; margin-top: 12px; }
.ticker-mode-options label { display: flex; min-width: 0; gap: 8px; padding: 10px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); cursor: pointer; }
.ticker-mode-options label:has(input:checked) { border-color: var(--color-accent); background: color-mix(in srgb, var(--color-accent) 8%, transparent); }
.ticker-mode-options input { margin-top: 2px; accent-color: var(--color-accent); }
.ticker-mode-options span, .ticker-count-row > span { display: flex; min-width: 0; flex-direction: column; gap: 3px; }
.ticker-mode-options b, .ticker-count-row b { color: var(--color-text-primary); font-size: var(--text-xs); }
.ticker-mode-options small, .ticker-count-row small { color: var(--color-text-tertiary); font-size: 10px; line-height: 1.45; }
.ticker-count-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--color-border-0); }
.ticker-count-row input { width: 72px; min-height: 32px; padding: 0 8px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-mono); }
.ticker-count-row input:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.hotkey-row { display: flex; gap: 8px; margin-top: 12px; }
.hotkey-box { display: flex; flex: 1; min-height: 42px; align-items: center; justify-content: center; flex-direction: column; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-mono); cursor: pointer; }
.hotkey-box.capturing { border-color: var(--color-accent); color: var(--color-accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 10%, transparent); }
.hotkey-box small { color: var(--color-text-tertiary); font-family: var(--font-sans); font-size: 9px; }
.minor-btn { min-height: 32px; padding: 0 12px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-secondary); font-family: var(--font-sans); cursor: pointer; }
.notification-action { margin-top: 10px; }
.inline-setting { margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--color-border-0); }
.color-row, .system-line { display: flex; align-items: center; justify-content: space-between; margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--color-border-0); color: var(--color-text-secondary); font-size: var(--text-xs); }
.color-row label { display: flex; align-items: center; gap: 8px; }
.color-row input { width: 34px; height: 24px; padding: 1px; border: 1px solid var(--color-border-1); border-radius: 4px; background: none; }
.color-row code, .system-line b { color: var(--color-text-primary); font-family: var(--font-mono); }
.exe-line { align-items: flex-start; }
.exe-line code { max-width: 62%; overflow-wrap: anywhere; text-align: right; color: var(--color-text-secondary); font-size: 10px; }
.build-hint { margin-top: 8px; color: var(--color-text-tertiary); font-size: 10px; line-height: 1.5; }
.page-size-label { color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-status { display: flex; flex-direction: column; gap: 3px; margin-top: 12px; padding: 10px 12px; border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-tertiary); font-size: var(--text-xs); }
.history-status.connected b { color: #3fb950; }
.history-status.unavailable b, .history-status.not_started b { color: #d29922; }
.history-status small { font-family: var(--font-mono); font-size: 10px; }
.history-field { display: grid; grid-template-columns: 72px minmax(0, 1fr) auto; align-items: center; gap: 8px; margin-top: 10px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-field input { min-width: 0; min-height: 32px; padding: 0 9px; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); }
.history-actions, .candidate-list { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
.candidate-list button { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.history-program-row { display: grid; grid-template-columns: 108px minmax(0, 1fr) auto; align-items: center; gap: 8px; margin-top: 10px; font-size: var(--text-xs); color: var(--color-text-secondary); }
.history-program-row code { min-width: 0; padding: 7px 9px; overflow-wrap: anywhere; border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); background: var(--color-surface-1); color: var(--color-text-primary); font-family: var(--font-mono); }
.history-advanced { margin-top: 12px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.history-advanced summary { cursor: pointer; user-select: none; }
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
.theme-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.theme-card { display: flex; flex-direction: column; align-items: flex-start; gap: 5px; padding: 14px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); background: var(--color-surface-0); color: var(--color-text-primary); cursor: pointer; transition: transform var(--transition-fast), border-color var(--transition-fast); }
.theme-card:hover { transform: translateY(-2px); border-color: var(--color-border-1); }
.theme-card.active { border-color: var(--color-accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 12%, transparent); }
.theme-card > i { display: grid; grid-template-columns: 28% 1fr; grid-template-rows: repeat(2, 1fr); gap: 4px; width: 100%; height: 110px; margin-bottom: 6px; padding: 8px; border-radius: var(--radius-sm); background: #f4f6f8; box-shadow: inset 0 0 0 1px #d9dee5; }
.theme-card > i span { border-radius: 3px; background: #d8dee7; }
.theme-card > i span:first-child { grid-row: 1 / 3; background: #c6cfdb; }
.theme-card.dark > i { background: #111820; box-shadow: inset 0 0 0 1px #2b3541; }
.theme-card.dark > i span { background: #26313d; }
.theme-card.dark > i span:first-child { background: #1d2732; }
.theme-card b { font-size: var(--text-sm); }
.theme-card small { color: var(--color-text-tertiary); }
.settings-footer { display: flex; align-items: center; justify-content: space-between; color: var(--color-text-tertiary); font-size: var(--text-xs); }
.done-btn { height: 32px; padding: 0 20px; border: 0; border-radius: var(--radius-sm); background: var(--color-accent); color: #fff; font-family: var(--font-sans); cursor: pointer; }
@keyframes panel-in { from { opacity: 0; transform: translateY(3px); } }
@media (max-width: 680px) {
  .settings-shell { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); }
  .section-nav { flex-direction: row; overflow-x: auto; padding-bottom: 8px; border-right: 0; border-bottom: 1px solid var(--color-border-0); }
  .nav-item { min-width: 84px; align-items: center; }
  .nav-item.active::before { top: auto; right: 12px; bottom: 0; width: auto; height: 2px; }
  .settings-content { padding: 0; }
  .explain-grid { grid-template-columns: 1fr; }
}
@media (max-width: 460px) {
  .theme-grid { grid-template-columns: 1fr; }
  .card-title-row { align-items: flex-start; }
  .settings-footer > span { display: none; }
  .settings-footer { justify-content: flex-end; }
}
</style>
