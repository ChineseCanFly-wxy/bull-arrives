<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  NAlert,
  NButton,
  NCard,
  NForm,
  NFormItem,
  NInput,
  NInputNumber,
  NModal,
  NSpace,
  useMessage,
} from 'naive-ui';
import type { WatchItem } from '@/types';
import {
  decimalToScaledPrice,
  MAX_HOLDING_SHARES,
  scaledPriceToDecimal,
  type Holding,
} from '@/utils/holdings';

const props = defineProps<{
  show: boolean;
  item: WatchItem | null;
}>();
const emit = defineEmits<{
  (event: 'update:show', value: boolean): void;
  (event: 'saved', holding: Holding | null): void;
}>();

const message = useMessage();
const costPrice = ref('');
const shares = ref<number | null>(0);
const loading = ref(false);
const saving = ref(false);
const deleting = ref(false);
const error = ref('');
const existing = ref(false);
let loadGeneration = 0;

const title = computed(() => props.item ? `持仓设置 · ${props.item.name}` : '持仓设置');
const busy = computed(() => loading.value || saving.value || deleting.value);

watch(
  () => [props.show, props.item?.id] as const,
  ([show]) => {
    if (show) void loadHolding();
    else loadGeneration += 1;
  },
  { immediate: true },
);

async function loadHolding() {
  const item = props.item;
  const generation = ++loadGeneration;
  costPrice.value = '';
  shares.value = 0;
  existing.value = false;
  error.value = '';
  if (!item) {
    error.value = '未选择自选股票';
    return;
  }

  loading.value = true;
  try {
    const holdings = await invoke<Holding[]>('get_holdings');
    if (generation !== loadGeneration) return;
    const holding = holdings.find(value => value.watch_id === item.id);
    if (holding) {
      existing.value = true;
      costPrice.value = scaledPriceToDecimal(holding.cost_price);
      shares.value = holding.shares;
    }
  } catch (cause) {
    if (generation === loadGeneration) error.value = `读取持仓失败：${String(cause)}`;
  } finally {
    if (generation === loadGeneration) loading.value = false;
  }
}

function validatedShares(): number {
  const value = shares.value;
  if (value === null || !Number.isSafeInteger(value) || value < 0 || value > MAX_HOLDING_SHARES) {
    throw new Error(`持仓股数应为 0 到 ${MAX_HOLDING_SHARES} 之间的整数`);
  }
  return value;
}

async function save() {
  const item = props.item;
  if (!item || busy.value) return;
  error.value = '';
  saving.value = true;
  try {
    const holding: Holding = {
      watch_id: item.id,
      cost_price: decimalToScaledPrice(costPrice.value),
      shares: validatedShares(),
    };
    await invoke('save_holding', {
      watchId: holding.watch_id,
      costPrice: holding.cost_price,
      shares: holding.shares,
    });
    emit('saved', holding);
    emit('update:show', false);
    message.success('持仓已保存');
  } catch (cause) {
    error.value = `保存失败：${cause instanceof Error ? cause.message : String(cause)}`;
  } finally {
    saving.value = false;
  }
}

async function clearHolding() {
  const item = props.item;
  if (!item || busy.value || !existing.value) return;
  error.value = '';
  deleting.value = true;
  try {
    await invoke('delete_holding', { watchId: item.id });
    emit('saved', null);
    emit('update:show', false);
    message.success('持仓记录已清除');
  } catch (cause) {
    error.value = `清除失败：${String(cause)}`;
  } finally {
    deleting.value = false;
  }
}

function cancel() {
  if (!busy.value) emit('update:show', false);
}
</script>

<template>
  <NModal :show="props.show" :mask-closable="!busy" @update:show="$event ? undefined : cancel()">
    <NCard
      :title="title"
      class="holding-dialog"
      :bordered="false"
      closable
      role="dialog"
      aria-modal="true"
      @close="cancel"
    >
      <NForm label-placement="top" :disabled="busy" @submit.prevent="save">
        <NFormItem label="每股成本价" path="costPrice">
          <NInput
            v-model:value="costPrice"
            inputmode="decimal"
            placeholder="留空表示未填写成本"
            clearable
            maxlength="24"
            aria-label="每股成本价"
          >
            <template #suffix>元</template>
          </NInput>
          <template #feedback>最多 4 位小数；清空成本价不会清空持仓股数。</template>
        </NFormItem>

        <NFormItem label="持仓股数" path="shares">
          <NInputNumber
            v-model:value="shares"
            :min="0"
            :max="MAX_HOLDING_SHARES"
            :precision="0"
            :show-button="false"
            placeholder="请输入非负整数"
            aria-label="持仓股数"
          >
            <template #suffix>股</template>
          </NInputNumber>
        </NFormItem>

        <NAlert v-if="error" type="error" :show-icon="true" role="alert" class="holding-error">
          {{ error }}
        </NAlert>

        <div class="holding-actions">
          <NButton
            v-if="existing"
            type="error"
            secondary
            :disabled="busy"
            :loading="deleting"
            @click="clearHolding"
          >
            清除持仓
          </NButton>
          <span class="action-spacer" />
          <NSpace :size="8">
            <NButton :disabled="busy" @click="cancel">取消</NButton>
            <NButton
              attr-type="submit"
              type="primary"
              :disabled="loading || !props.item"
              :loading="saving"
            >
              保存
            </NButton>
          </NSpace>
        </div>
      </NForm>
    </NCard>
  </NModal>
</template>

<style scoped>
.holding-dialog {
  width: min(420px, calc(100vw - 32px));
}

.holding-dialog :deep(.n-input-number) {
  width: 100%;
}

.holding-error {
  margin-bottom: var(--space-4);
}

.holding-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: 34px;
}

.action-spacer {
  flex: 1;
}
</style>
