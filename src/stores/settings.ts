// src/stores/settings.ts
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';

/// Broadcast event name used to keep settings in sync across windows.
/// The main window and the ticker bar are separate WebViews with independent
/// Pinia stores, so any setting that affects the ticker MUST be broadcast —
/// writing it to SQLite alone is invisible to the other window.
export const SETTING_CHANGED_EVENT = 'setting-changed';

export interface SettingChangedPayload {
  key: string;
  value: string;
}

/// 0 means "auto" — follow the trading session's recommended interval.
export const REFRESH_INTERVAL_AUTO = 0;

export interface MarketSessionInfo {
  session: string;
  interval_secs: number;
  is_trading: boolean;
}

export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<Record<string, string>>({});
  const datasources = ref<[string, string][]>([]);
  const activeDatasource = ref('tencent');
  const theme = ref<'dark' | 'light'>('light');
  const autoLaunch = ref(false);
  const isPortable = ref(false);
  const tickerHotkey = ref('Alt+Q');
  const tickerOpacity = ref(100);
  const tickerSingleColor = ref(false);
  const tickerTextColor = ref('#9AA5B1');
  const tickerDisplayMode = ref<'carousel' | 'fixed'>('carousel');
  const tickerPageSize = ref(2);
  const quoteScheduleEnabled = ref(false);
  const alertsEnabled = ref(true);
  const notificationDesktopAlways = ref(false);
  const refreshInterval = ref(REFRESH_INTERVAL_AUTO);
  const marketSession = ref<MarketSessionInfo>({
    session: '休市',
    interval_secs: 30,
    is_trading: false,
  });
  const error = ref<string | null>(null);

  function clampOpacity(v: number): number {
    if (Number.isNaN(v)) return 100;
    return Math.min(100, Math.max(5, Math.round(v)));
  }

  function clampInterval(v: number): number {
    if (Number.isNaN(v) || v <= 0) return REFRESH_INTERVAL_AUTO;
    return Math.min(60, Math.max(1, Math.round(v)));
  }

  function clampTickerPageSize(v: number): number {
    if (Number.isNaN(v)) return 2;
    return Math.min(20, Math.max(1, Math.round(v)));
  }

  /// Apply a raw `settings` table value to the matching reactive ref.
  /// Single source of truth for both initial load and remote updates, so a
  /// new setting only needs to be registered here once.
  function applySettingLocally(key: string, value: string) {
    switch (key) {
      case 'active_datasource':
        activeDatasource.value = value || 'tencent';
        break;
      case 'theme':
        applyTheme((value as 'dark' | 'light') || 'light');
        break;
      case 'ticker_hotkey':
        tickerHotkey.value = value || 'Alt+Q';
        break;
      case 'ticker_opacity':
        tickerOpacity.value = clampOpacity(parseInt(value, 10));
        break;
      case 'ticker_single_color':
        tickerSingleColor.value = value === '1';
        break;
      case 'ticker_text_color':
        tickerTextColor.value = value || '#9AA5B1';
        break;
      case 'ticker_display_mode':
        tickerDisplayMode.value = value === 'fixed' ? 'fixed' : 'carousel';
        break;
      case 'ticker_page_size':
        tickerPageSize.value = clampTickerPageSize(parseInt(value, 10));
        break;
      case 'quote_schedule_enabled':
        quoteScheduleEnabled.value = value === '1';
        break;
      case 'alerts_enabled':
        alertsEnabled.value = value !== '0';
        break;
      case 'notification_desktop_always':
        notificationDesktopAlways.value = value === '1';
        break;
      case 'refresh_interval':
        refreshInterval.value = clampInterval(parseInt(value, 10));
        break;
      default:
        break;
    }
  }

  /// Called by the receiving window when a `setting-changed` event arrives.
  /// Updates local state only — never re-broadcasts, which would loop.
  function applyRemoteSetting(key: string, value: string) {
    settings.value[key] = value;
    applySettingLocally(key, value);
  }

  async function fetchSettings() {
    try {
      settings.value = await invoke<Record<string, string>>('get_settings');
      applySettingLocally('active_datasource', settings.value['active_datasource'] || 'tencent');
      applySettingLocally('theme', settings.value['theme'] || 'light');
      applySettingLocally('ticker_hotkey', settings.value['ticker_hotkey'] || 'Alt+Q');
      applySettingLocally('ticker_opacity', settings.value['ticker_opacity'] ?? '100');
      applySettingLocally('ticker_single_color', settings.value['ticker_single_color'] ?? '0');
      applySettingLocally('ticker_text_color', settings.value['ticker_text_color'] || '#9AA5B1');
      applySettingLocally('ticker_display_mode', settings.value['ticker_display_mode'] || 'carousel');
      applySettingLocally('ticker_page_size', settings.value['ticker_page_size'] || '2');
      applySettingLocally('quote_schedule_enabled', settings.value['quote_schedule_enabled'] ?? '0');
      applySettingLocally('alerts_enabled', settings.value['alerts_enabled'] ?? '1');
      applySettingLocally('notification_desktop_always', settings.value['notification_desktop_always'] ?? '0');
      applySettingLocally('refresh_interval', settings.value['refresh_interval'] ?? '0');
      datasources.value = await invoke<[string, string][]>('list_datasources');
      autoLaunch.value = await isEnabled();
      isPortable.value = await invoke<boolean>('get_portable_mode');
    } catch (e) {
      console.error('Failed to fetch settings:', e);
      error.value = `加载设置失败: ${e}`;
    }
  }

  async function toggleAutoLaunch() {
    try {
      // Persist to DB first so that on restart the app knows the desired state.
      const newValue = String(!autoLaunch.value);
      if (!await setSetting('auto_launch', newValue)) return false;
      // Then toggle the OS-level autostart.
      if (autoLaunch.value) {
        await disable();
      } else {
        await enable();
      }
      autoLaunch.value = !autoLaunch.value;
    } catch (e) {
      console.error('[settings] toggleAutoLaunch failed:', e);
      error.value = `自动启动切换失败: ${e}`;
      return false;
    }
    return true;
  }

  /// Persist a setting, apply it locally, then broadcast to every other
  /// window so cross-window settings (e.g. ticker appearance) take effect
  /// immediately without a restart.
  async function setSetting(key: string, value: string) {
    try {
      await invoke('set_setting', { key, value });
      settings.value[key] = value;
      applySettingLocally(key, value);
      await emit(SETTING_CHANGED_EVENT, { key, value }).catch((e) => {
        console.error(`[settings] Failed to broadcast '${key}':`, e);
      });
    } catch (e) {
      console.error(`[settings] setSetting('${key}') failed:`, e);
      error.value = `保存设置失败: ${e}`;
      return false;
    }
    return true;
  }

  async function switchDatasource(name: string) {
    const previous = activeDatasource.value;
    try {
      await invoke('switch_datasource', { name });
      activeDatasource.value = name;
      settings.value['active_datasource'] = name;
      emit(SETTING_CHANGED_EVENT, { key: 'active_datasource', value: name }).catch((e) => {
        console.error('[settings] Failed to broadcast datasource change:', e);
      });
    } catch (e) {
      activeDatasource.value = previous;
      error.value = `数据源切换失败: ${e}`;
      console.error('[settings] switchDatasource failed:', e);
      return false;
    }
    return true;
  }

  async function toggleTheme() {
    const next = theme.value === 'dark' ? 'light' : 'dark';
    return setSetting('theme', next);
  }

  function applyTheme(t: 'dark' | 'light') {
    theme.value = t;
    document.documentElement.setAttribute('data-theme', t);
    // NOTE: does NOT broadcast — setSetting()/toggleTheme() own the broadcast.
    // If applyTheme emitted, the ticker's listener would call applyTheme again,
    // creating an infinite event loop between windows.
  }

  async function setTickerHotkey(hotkey: string) {
    try {
      await invoke('set_ticker_hotkey', { hotkey });
      tickerHotkey.value = hotkey;
      settings.value['ticker_hotkey'] = hotkey;
    } catch (e) {
      console.error('[settings] setTickerHotkey failed:', e);
      error.value = `热键设置失败: ${e}`;
      return false;
    }
    return true;
  }

  async function setTickerOpacity(value: number) {
    const v = clampOpacity(value);
    try {
      await invoke('set_ticker_opacity', { opacity: v });
      tickerOpacity.value = v;
      settings.value['ticker_opacity'] = String(v);
      await emit(SETTING_CHANGED_EVENT, { key: 'ticker_opacity', value: String(v) }).catch(() => {});
    } catch (e) {
      console.error('[settings] setTickerOpacity failed:', e);
      error.value = `透明度设置失败: ${e}`;
      return false;
    }
    return true;
  }

  async function setTickerSingleColor(value: boolean) {
    return setSetting('ticker_single_color', value ? '1' : '0');
  }

  async function setTickerTextColor(value: string) {
    return setSetting('ticker_text_color', value);
  }

  async function setTickerDisplayMode(value: 'carousel' | 'fixed') {
    return setSetting('ticker_display_mode', value);
  }

  async function setTickerPageSize(value: number) {
    return setSetting('ticker_page_size', String(clampTickerPageSize(value)));
  }

  /// secs = 0 → auto (follow trading session), 1–60 → fixed interval.
  async function setRefreshInterval(secs: number) {
    const v = clampInterval(secs);
    try {
      await invoke('set_refresh_interval', { secs: v });
      refreshInterval.value = v;
      settings.value['refresh_interval'] = String(v);
      await emit(SETTING_CHANGED_EVENT, { key: 'refresh_interval', value: String(v) }).catch(() => {});
    } catch (e) {
      console.error('[settings] setRefreshInterval failed:', e);
      error.value = `刷新间隔设置失败: ${e}`;
      return false;
    }
    return true;
  }

  async function fetchMarketSession() {
    try {
      marketSession.value = await invoke<MarketSessionInfo>('get_market_session');
    } catch (e) {
      console.warn('[settings] fetchMarketSession failed:', e);
    }
  }

  /// Effective polling interval in seconds, accounting for auto mode.
  function effectiveInterval(): number {
    return refreshInterval.value === REFRESH_INTERVAL_AUTO
      ? marketSession.value.interval_secs
      : refreshInterval.value;
  }

  return {
    settings, datasources, activeDatasource, theme, autoLaunch, isPortable,
    tickerHotkey, tickerOpacity, tickerSingleColor, tickerTextColor, tickerDisplayMode, tickerPageSize,
    quoteScheduleEnabled, alertsEnabled, notificationDesktopAlways, refreshInterval, marketSession, error,
    fetchSettings, setSetting, switchDatasource, toggleTheme, toggleAutoLaunch,
    applyTheme, applyRemoteSetting, setTickerHotkey, setTickerOpacity,
    setTickerSingleColor, setTickerTextColor, setTickerDisplayMode, setTickerPageSize,
    setRefreshInterval, fetchMarketSession, effectiveInterval,
  };
});
