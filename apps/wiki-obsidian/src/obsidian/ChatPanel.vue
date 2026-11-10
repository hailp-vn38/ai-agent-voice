<script setup lang="ts">
import WikiChatPanel from '@vela/knowledge-ui/components/WikiChatPanel.vue'
import type { ObsidianChatState } from './chatState'

defineEmits<{ openFile: [path: string] }>()
const props = defineProps<{ chat: ObsidianChatState; saveAnswer: () => void; saveConversation: () => void }>()
const suggestions = [
  { label: 'Tổng quan wiki', prompt: 'Tóm tắt các chủ đề chính trong wiki và dẫn nguồn.' },
  { label: 'Khám phá liên kết', prompt: 'Những chủ đề nào trong wiki có liên quan với nhau? Dẫn nguồn cụ thể.' },
]
</script>

<template>
  <div class="wiki-agent-toolbar">
    <label for="wiki-agent-mode">Chế độ</label>
    <select id="wiki-agent-mode" v-model="props.chat.state.mode" :disabled="chat.state.busy">
      <option value="ask">Hỏi (chỉ đọc)</option>
      <option value="agent">Agent (được sửa wiki)</option>
    </select>
    <button type="button" :disabled="chat.state.busy || !chat.lastTurn?.answer" @click="saveAnswer">Lưu vào wiki</button>
    <button type="button" :disabled="chat.state.busy || !chat.state.messages.length" @click="saveConversation">Lưu hội thoại</button>
  </div>
  <WikiChatPanel :messages="chat.state.messages" :busy="chat.state.busy" :suggestions="suggestions"
    :pinned-files="chat.state.pinnedFiles" :open-file="chat.state.openFile"
    :conversation-history="false" :allow-retry="false"
    @send="chat.send($event)" @stop="chat.stop()" @new-conversation="chat.reset()"
    @pin-file="chat.pin($event)" @unpin-file="chat.unpin($event)"
    @focus-citation="$emit('openFile', $event)" />
</template>
