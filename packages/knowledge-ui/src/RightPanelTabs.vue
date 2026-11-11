<script lang="ts">
export type RightTab = 'outline' | 'chat'
</script>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { ListTree, Sparkles } from '@lucide/vue'
import { useI18n } from './integration'
import { movedOrder, useDragReorder } from './useDragReorder'

const props = withDefaults(defineProps<{ active: RightTab; order: RightTab[]; capabilities?: { chat: boolean } }>(), {
  capabilities: () => ({ chat: true }),
})
const emit = defineEmits<{ select: [tab: RightTab]; reorder: [order: RightTab[]] }>()
const { t } = useI18n()

const list = ref<HTMLElement | null>(null)
const visibleOrder = computed(() => props.capabilities.chat ? props.order : props.order.filter(tab => tab !== 'chat'))
const reorder = useDragReorder<RightTab>({ container: list, order: () => props.order, onReorder: order => emit('reorder', order) })

function onClick(tab: RightTab) {
  if (reorder.consumeClick()) return
  if (tab !== props.active) emit('select', tab)
}

function moveWithKeyboard(tab: RightTab, step: -1 | 1) {
  const next = movedOrder(props.order, tab, step)
  if (next) emit('reorder', next)
}
</script>

<template>
  <div ref="list" class="a-right-tabs" role="tablist" :aria-label="t('knowledgeV3.rightPanel')"
    :class="{ 'is-reordering': reorder.draggingKey.value, 'is-settling': reorder.settling.value, 'is-instant': reorder.instant.value }">
    <button v-for="tab in visibleOrder" :key="tab" type="button" role="tab" class="a-right-tab"
      :class="{ active: tab === active, 'is-dragging': tab === reorder.draggingKey.value }" :data-tab="tab" :data-reorder-key="tab"
      :style="reorder.itemStyle(tab)" :aria-selected="tab === active ? 'true' : 'false'" :title="t('knowledgeV3.rightTabReorderHint')"
      v-on="reorder.listeners(tab)" @click="onClick(tab)"
      @keydown.alt.left.prevent="moveWithKeyboard(tab, -1)" @keydown.alt.right.prevent="moveWithKeyboard(tab, 1)">
      <template v-if="tab === 'outline'"><ListTree :size="13" aria-hidden="true" /> {{ t('knowledgeV3.rightOutline') }}</template>
      <template v-else><Sparkles :size="13" aria-hidden="true" /> {{ t('knowledgeV3.chatTitle') }}</template>
    </button>
  </div>
</template>
