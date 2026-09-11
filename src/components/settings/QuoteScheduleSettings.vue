<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { NButton, NInput, useMessage } from 'naive-ui';
import { useSettingsStore } from '@/stores/settings';
const settings = useSettingsStore();
const message = useMessage();
const windows = ref([{ start: '09:15', end: '11:30' }, { start: '13:00', end: '15:00' }]);
const closedDates = ref('');
const busy = ref(false);

async function toggleEnabled() {
  busy.value = true;
  const enabled = !settings.quoteScheduleEnabled;
  try {
    const saved = await settings.setSetting('quote_schedule_enabled', enabled ? '1' : '0');
    if (!saved) throw new Error(settings.error || '行情时间限制切换失败');
    message.success(enabled ? '行情时间限制已开启' : '行情时间限制已关闭，休市期间将自动降频');
  } catch (error) {
    message.error(String(error));
  } finally {
    busy.value = false;
  }
}

onMounted(() => {
  try {
    const raw = settings.settings.quote_schedule;
    if (!raw) return;
    const config = JSON.parse(raw);
    if (Array.isArray(config.sessions)) windows.value = config.sessions.map((w: string | { start: string; end: string }) => typeof w === 'string' ? { start: w.split('-')[0], end: w.split('-')[1] } : w);
    if (Array.isArray(config.closed_dates)) closedDates.value = config.closed_dates.join('\n');
  } catch { message.error('保存的行情时段格式不正确，请重新设置。'); }
});
async function save() {
  busy.value = true;
  try {
    const sessions = windows.value.map(w => ({ ...w })).sort((a,b) => a.start.localeCompare(b.start));
    if (!sessions.length || sessions.some(w => !/^\d{2}:\d{2}$/.test(w.start) || !/^\d{2}:\d{2}$/.test(w.end))) throw new Error('请填写完整时间段');
    const closed_dates = closedDates.value.split(/[\s,，]+/).filter(Boolean);
    const saved = await settings.setSetting('quote_schedule', JSON.stringify({ sessions, closed_dates }));
    if (!saved) throw new Error(settings.error || '行情时段保存失败');
    message.success('行情获取时段已保存并生效');
  } catch (error) { message.error(String(error)); }
  finally { busy.value = false; }
}
</script>

<template>
  <section class="schedule-section">
    <div class="schedule-heading">
      <div>
        <h3>限制行情获取时间</h3>
        <p>{{ settings.quoteScheduleEnabled ? '仅在下方配置的北京时间内请求行情，其他时间展示最后有效行情。' : '全天允许获取行情；盘前、午休和休市期间会自动降低轮询频率。' }}</p>
      </div>
      <button class="schedule-switch" :class="{ on: settings.quoteScheduleEnabled }" role="switch" :aria-checked="settings.quoteScheduleEnabled" :disabled="busy" @click="toggleEnabled"><span /></button>
    </div>
    <fieldset :disabled="busy || !settings.quoteScheduleEnabled">
      <div v-for="(window, index) in windows" :key="index" class="schedule-row">
        <input v-model="window.start" type="time" :aria-label="`时段${index + 1}开始`" />
        <span>至</span>
        <input v-model="window.end" type="time" :aria-label="`时段${index + 1}结束`" />
        <NButton size="small" quaternary :disabled="windows.length === 1" @click="windows.splice(index, 1)">删除</NButton>
      </div>
      <NButton size="small" dashed :disabled="windows.length >= 8" @click="windows.push({ start: '15:00', end: '15:30' })">添加时段</NButton>
      <label class="dates-label">额外休市日期</label>
      <NInput v-model:value="closedDates" type="textarea" placeholder="YYYY-MM-DD，每行一个日期" :autosize="{ minRows: 2, maxRows: 4 }" />
      <p>目前内置周末规则；交易所节假日请补充在此，不会通过联网探测是否休市。</p>
      <NButton size="small" type="primary" :loading="busy" @click="save">保存行情时段</NButton>
    </fieldset>
  </section>
</template>

<style scoped>
.schedule-section { padding: 16px; border: 1px solid var(--color-border-0); border-radius: var(--radius-md); }
.schedule-section h3 { margin: 0 0 8px; font-size: 14px; }
.schedule-section p { color: var(--color-text-secondary); font-size: 12px; line-height: 1.7; }
.schedule-heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.schedule-heading > div { min-width: 0; }
.schedule-switch { position: relative; width: 38px; height: 22px; flex: 0 0 38px; padding: 0; border: 0; border-radius: 999px; background: var(--color-border-1); cursor: pointer; }
.schedule-switch span { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0,0,0,.28); transition: transform var(--transition-fast); }
.schedule-switch.on { background: var(--color-accent); }
.schedule-switch.on span { transform: translateX(16px); }
.schedule-switch:disabled { opacity: .55; cursor: wait; }
.schedule-switch:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }
.schedule-section fieldset { min-width: 0; margin: 8px 0 0; padding: 0; border: 0; }
.schedule-section fieldset:disabled { opacity: .5; }
.schedule-row { display: flex; align-items: center; gap: 8px; margin: 8px 0; }
.schedule-row input { min-width: 0; color: var(--color-text-primary); background: var(--color-surface-1); border: 1px solid var(--color-border-1); border-radius: var(--radius-sm); padding: 6px; }
.dates-label { display: block; margin: 16px 0 8px; font-size: 13px; }
</style>
