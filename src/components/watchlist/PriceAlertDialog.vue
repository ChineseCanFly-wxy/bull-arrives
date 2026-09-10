<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import {
  NAlert,
  NButton,
  NInputNumber,
  NModal,
  NSelect,
  NSwitch,
  useMessage,
} from 'naive-ui';
import { invoke } from '@tauri-apps/api/core';
import type {
  PriceAlert,
  PriceAlertRepeatMode,
  PriceAlertType,
  WatchItem,
} from '@/types';
import {
  PRICE_ALERT_TYPES,
  buildPriceAlertPayload,
  defaultPriceAlertDraft,
  toPriceAlertDraft,
  validatePriceAlertDraft,
} from '@/utils/priceAlert';
import type { PriceAlertDraft } from '@/utils/priceAlert';
import { formatCode } from '@/utils/format';

const props = defineProps<{
  show: boolean;
  item: WatchItem | null;
}>();

const emit = defineEmits<{
  'update:show': [value: boolean];
}>();

const message = useMessage();
const loading = ref(false);
const saving = ref(false);
const loadError = ref<string | null>(null);
const saveError = ref<string | null>(null);
const drafts = ref<Record<PriceAlertType, PriceAlertDraft>>({
  change_pct: defaultPriceAlertDraft('change_pct'),
  fixed_price: defaultPriceAlertDraft('fixed_price'),
});
let loadGeneration = 0;

const repeatOptions = [
  { label: '每日一次', value: 'daily' },
  { label: '重新穿越', value: 'crossing' },
  { label: '冷却间隔', value: 'cooldown' },
];

const title = computed(() => props.item ? `${props.item.name} · 行情提醒` : '行情提醒');
const hasConfiguredRule = computed(() => PRICE_ALERT_TYPES.some((type) => drafts.value[type].configured));
const validationErrors = computed<Record<PriceAlertType, string | null>>(() => ({
  change_pct: validatePriceAlertDraft(drafts.value.change_pct),
  fixed_price: validatePriceAlertDraft(drafts.value.fixed_price),
}));
const hasValidationError = computed(() => PRICE_ALERT_TYPES.some((type) => validationErrors.value[type]));

watch(
  () => [props.show, props.item?.id] as const,
  ([show]) => {
    if (show && props.item) void loadAlerts(props.item);
    else loadGeneration += 1;
  },
  { immediate: true },
);

function resetDrafts() {
  drafts.value = {
    change_pct: defaultPriceAlertDraft('change_pct'),
    fixed_price: defaultPriceAlertDraft('fixed_price'),
  };
}

async function loadAlerts(item: WatchItem) {
  const generation = ++loadGeneration;
  loading.value = true;
  loadError.value = null;
  saveError.value = null;
  resetDrafts();
  try {
    const alerts = await invoke<PriceAlert[]>('get_price_alerts', {
      code: item.code,
      market: item.market,
    });
    if (generation !== loadGeneration) return;
    drafts.value = {
      change_pct: toPriceAlertDraft(
        'change_pct',
        alerts.find((alert) => alert.alert_type === 'change_pct'),
      ),
      fixed_price: toPriceAlertDraft(
        'fixed_price',
        alerts.find((alert) => alert.alert_type === 'fixed_price'),
      ),
    };
  } catch (error) {
    if (generation !== loadGeneration) return;
    loadError.value = `读取提醒失败：${String(error)}`;
  } finally {
    if (generation === loadGeneration) loading.value = false;
  }
}

function setConfigured(type: PriceAlertType, configured: boolean) {
  const draft = drafts.value[type];
  draft.configured = configured;
  if (configured) {
    draft.enabled = true;
    if (draft.threshold == null) {
      draft.threshold = type === 'change_pct' ? 5 : null;
    }
  } else {
    draft.enabled = false;
  }
  saveError.value = null;
}

function setRepeatMode(type: PriceAlertType, value: string) {
  drafts.value[type].repeat_mode = value as PriceAlertRepeatMode;
}

async function save() {
  const item = props.item;
  if (!item || saving.value || loading.value || hasValidationError.value) return;

  saving.value = true;
  saveError.value = null;
  const failures: string[] = [];
  let successCount = 0;

  for (const type of PRICE_ALERT_TYPES) {
    const draft = drafts.value[type];
    try {
      if (!draft.configured) {
        if (draft.existing) {
          await invoke('delete_price_alert', {
            code: item.code,
            market: item.market,
            alertType: type,
          });
          draft.existing = null;
        }
      } else {
        const alert = buildPriceAlertPayload(draft, item.code, item.market);
        if (!alert) continue;
        await invoke('save_price_alert', { alert });
        // 部分成功后继续编辑时，也必须知道该规则已存在，才能正确清除。
        draft.existing = alert;
      }
      successCount += 1;
    } catch (error) {
      const label = type === 'change_pct' ? '涨跌幅规则' : '价格规则';
      failures.push(`${label}：${String(error)}`);
    }
  }

  saving.value = false;
  if (failures.length > 0) {
    saveError.value = `${successCount > 0 ? '部分规则已保存；' : ''}${failures.join('；')}`;
    return;
  }

  message.success(hasConfiguredRule.value ? '提醒设置已保存' : '提醒设置已清除');
  emit('update:show', false);
}

function close() {
  if (saving.value) return;
  emit('update:show', false);
}
</script>

<template>
  <NModal
    :show="show"
    preset="card"
    :title="title"
    class="price-alert-modal"
    :style="{ width: 'min(680px, 94vw)' }"
    :bordered="false"
    :mask-closable="!saving"
    :close-on-esc="!saving"
    :closable="!saving"
    :segmented="{ content: 'soft', footer: 'soft' }"
    @update:show="(value: boolean) => !value && close()"
  >
    <div v-if="item" class="instrument-strip">
      <div>
        <span class="instrument-code">{{ formatCode(item.code) }}</span>
        <span class="instrument-market">{{ item.market }}</span>
      </div>
      <span class="strip-copy">独立配置，不受其他股票规则影响</span>
    </div>

    <NAlert v-if="loadError" type="error" :show-icon="false" class="feedback-alert">
      <span>{{ loadError }}</span>
      <NButton size="tiny" :loading="loading" class="retry-button" @click="item && loadAlerts(item)">重试</NButton>
    </NAlert>
    <NAlert v-if="saveError" type="error" :show-icon="false" class="feedback-alert">
      {{ saveError }}
    </NAlert>

    <div class="rule-stack" :class="{ busy: loading }" :aria-busy="loading">
      <section class="rule-card" :class="{ active: drafts.change_pct.configured }">
        <header class="rule-head">
          <div class="rule-identity">
            <span class="rule-mark pct">%</span>
            <div>
              <h3>涨跌幅提醒</h3>
              <p>按昨收价计算，每个交易日重新计数</p>
            </div>
          </div>
          <NSwitch
            :value="drafts.change_pct.configured"
            :disabled="loading || saving"
            aria-label="配置涨跌幅提醒"
            @update:value="(value: boolean) => setConfigured('change_pct', value)"
          />
        </header>
        <div v-if="drafts.change_pct.configured" class="rule-controls">
          <label class="field-block">
            <span>触发阈值</span>
            <NInputNumber
              v-model:value="drafts.change_pct.threshold"
              :show-button="false"
              :precision="2"
              placeholder="例如 5 或 -3"
              class="threshold-input"
            >
              <template #suffix>%</template>
            </NInputNumber>
            <small>可填正数或负数，但不能为 0</small>
          </label>
          <label class="field-block">
            <span>重复方式</span>
            <NSelect
              :value="drafts.change_pct.repeat_mode"
              :options="repeatOptions"
              @update:value="(value: string) => setRepeatMode('change_pct', value)"
            />
            <small>{{ drafts.change_pct.repeat_mode === 'daily' ? '当天触发一次，次日重新生效' : drafts.change_pct.repeat_mode === 'crossing' ? '回到阈值内后再次穿越才提醒' : '达到阈值后按冷却时间限制频率' }}</small>
          </label>
          <label v-if="drafts.change_pct.repeat_mode === 'cooldown'" class="field-block cooldown-field">
            <span>冷却分钟</span>
            <NInputNumber v-model:value="drafts.change_pct.cooldown_minutes" :min="1" :max="1440" :precision="0" />
            <small>N 分钟内最多提醒一次</small>
          </label>
          <div class="enable-row">
            <div><b>此规则启用</b><small>关闭会保留阈值，稍后可再次启用</small></div>
            <NSwitch v-model:value="drafts.change_pct.enabled" />
          </div>
          <p v-if="validationErrors.change_pct" class="field-error" role="alert">{{ validationErrors.change_pct }}</p>
        </div>
        <div v-else class="rule-empty">未配置，不会创建占位规则</div>
      </section>

      <section class="rule-card" :class="{ active: drafts.fixed_price.configured }">
        <header class="rule-head">
          <div class="rule-identity">
            <span class="rule-mark price">¥</span>
            <div>
              <h3>固定价格提醒</h3>
              <p>长期有效，直到关闭或删除这条规则</p>
            </div>
          </div>
          <NSwitch
            :value="drafts.fixed_price.configured"
            :disabled="loading || saving"
            aria-label="配置固定价格提醒"
            @update:value="(value: boolean) => setConfigured('fixed_price', value)"
          />
        </header>
        <div v-if="drafts.fixed_price.configured" class="rule-controls">
          <label class="field-block">
            <span>目标价格</span>
            <NInputNumber
              v-model:value="drafts.fixed_price.threshold"
              :show-button="false"
              :min="0.01"
              :precision="3"
              placeholder="输入大于 0 的价格"
              class="threshold-input"
            >
              <template #prefix>¥</template>
            </NInputNumber>
            <small>价格必须大于 0</small>
          </label>
          <label class="field-block">
            <span>重复方式</span>
            <NSelect
              :value="drafts.fixed_price.repeat_mode"
              :options="repeatOptions"
              @update:value="(value: string) => setRepeatMode('fixed_price', value)"
            />
            <small>{{ drafts.fixed_price.repeat_mode === 'crossing' ? '回到目标价格另一侧后再次穿越才提醒' : drafts.fixed_price.repeat_mode === 'daily' ? '当天触发一次，次日重新生效' : '达到目标价格后按冷却时间限制频率' }}</small>
          </label>
          <label v-if="drafts.fixed_price.repeat_mode === 'cooldown'" class="field-block cooldown-field">
            <span>冷却分钟</span>
            <NInputNumber v-model:value="drafts.fixed_price.cooldown_minutes" :min="1" :max="1440" :precision="0" />
            <small>N 分钟内最多提醒一次</small>
          </label>
          <div class="enable-row">
            <div><b>此规则启用</b><small>关闭会保留价格，稍后可再次启用</small></div>
            <NSwitch v-model:value="drafts.fixed_price.enabled" />
          </div>
          <p v-if="validationErrors.fixed_price" class="field-error" role="alert">{{ validationErrors.fixed_price }}</p>
        </div>
        <div v-else class="rule-empty">未配置，不会创建占位规则</div>
      </section>
    </div>

    <template #footer>
      <div class="modal-footer">
        <span class="footer-note">总开关仅暂停提醒，不会覆盖这里的逐票设置</span>
        <div class="footer-actions">
          <NButton :disabled="saving" @click="close">取消</NButton>
          <NButton
            type="primary"
            :loading="saving"
            :disabled="loading || Boolean(loadError) || hasValidationError"
            @click="save"
          >保存设置</NButton>
        </div>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.instrument-strip {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
  padding: 10px 12px;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-surface-1);
}
.instrument-code {
  color: var(--color-text-primary);
  font-family: var(--font-mono);
  font-size: var(--text-md);
  font-variant-numeric: tabular-nums;
  font-weight: var(--font-weight-semibold);
}
.instrument-market {
  margin-left: 8px;
  padding: 2px 6px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  color: var(--color-accent);
  font-size: 10px;
}
.strip-copy,
.footer-note {
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}
.feedback-alert { margin-bottom: var(--space-3); }
.rule-stack {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
  transition: opacity var(--transition-fast);
}
.rule-stack.busy { opacity: 0.45; pointer-events: none; }
.rule-card {
  overflow: hidden;
  border: 1px solid var(--color-border-0);
  border-radius: var(--radius-md);
  background: var(--color-surface-0);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.rule-card.active {
  border-color: color-mix(in srgb, var(--color-accent) 42%, var(--color-border-0));
  box-shadow: 0 8px 24px color-mix(in srgb, var(--color-accent) 7%, transparent);
}
.rule-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: 14px;
}
.rule-identity { display: flex; align-items: center; gap: 10px; min-width: 0; }
.rule-mark {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  flex: 0 0 34px;
  border-radius: 10px;
  font-family: var(--font-mono);
  font-size: var(--text-md);
  font-weight: 700;
}
.rule-mark.pct { color: var(--color-up); background: color-mix(in srgb, var(--color-up) 12%, transparent); }
.rule-mark.price { color: var(--color-accent); background: color-mix(in srgb, var(--color-accent) 12%, transparent); }
.rule-head h3 { margin: 0 0 2px; color: var(--color-text-primary); font-size: var(--text-sm); }
.rule-head p { margin: 0; color: var(--color-text-tertiary); font-size: var(--text-xs); line-height: 1.4; }
.rule-controls {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px;
  border-top: 1px solid var(--color-border-0);
  background: var(--color-surface-1);
}
.field-block { display: flex; flex-direction: column; gap: 6px; color: var(--color-text-secondary); font-size: var(--text-xs); }
.field-block small,
.enable-row small { color: var(--color-text-tertiary); font-size: 10px; line-height: 1.4; }
.threshold-input { width: 100%; }
.cooldown-field { animation: reveal 140ms ease-out; }
.enable-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 2px;
}
.enable-row > div { display: flex; flex-direction: column; gap: 2px; }
.enable-row b { color: var(--color-text-primary); font-size: var(--text-xs); font-weight: var(--font-weight-medium); }
.rule-empty {
  padding: 12px 14px;
  border-top: 1px dashed var(--color-border-0);
  color: var(--color-text-tertiary);
  font-size: var(--text-xs);
}
.field-error { margin: 0; color: var(--color-warning); font-size: var(--text-xs); }
.modal-footer { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
.footer-actions { display: flex; gap: var(--space-2); flex-shrink: 0; }
@keyframes reveal { from { opacity: 0; transform: translateY(-3px); } }
@media (max-width: 620px) {
  .rule-stack { grid-template-columns: 1fr; }
  .instrument-strip,
  .modal-footer { align-items: flex-start; flex-direction: column; }
  .footer-actions { align-self: stretch; justify-content: flex-end; }
}
</style>
