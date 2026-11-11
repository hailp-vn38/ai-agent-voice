<script setup lang="ts">
import { provide } from 'vue'
import SharedWikiChatPanel from '@vela/knowledge-ui/components/WikiChatPanel.vue'
import { KNOWLEDGE_TRANSLATOR_KEY, KNOWLEDGE_DARK_MODE_KEY } from '@vela/knowledge-ui'
import type { WikiChatPanelProps, WikiChatPanelEvents } from '@vela/knowledge-ui/chat'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'

const props = withDefaults(defineProps<WikiChatPanelProps>(), { conversationHistory: true, allowRetry: true })
const emit = defineEmits<WikiChatPanelEvents>()
const { t } = useI18n()
provide(KNOWLEDGE_TRANSLATOR_KEY, (key, params) => t(key as Parameters<typeof t>[0], params))
provide(KNOWLEDGE_DARK_MODE_KEY, useTheme().dark)
</script>

<template>
  <SharedWikiChatPanel v-bind="props" :confirm-dialog="ConfirmDialog"
    @send="emit('send', $event)" @stop="emit('stop')" @retry="emit('retry', $event)"
    @refresh-conversation="emit('refreshConversation')" @show-graph="emit('showGraph', $event)"
    @focus-citation="emit('focusCitation', $event)" @highlight-citation="emit('highlightCitation', $event)"
    @pin-file="emit('pinFile', $event)" @unpin-file="emit('unpinFile', $event)"
    @load-conversations="emit('loadConversations')" @select-conversation="emit('selectConversation', $event)"
    @new-conversation="emit('newConversation')" @rename-conversation="(id, title) => emit('renameConversation', id, title)"
    @delete-conversation="emit('deleteConversation', $event)">
    <template v-if="$slots.title" #title><slot name="title" /></template>
  </SharedWikiChatPanel>
</template>
