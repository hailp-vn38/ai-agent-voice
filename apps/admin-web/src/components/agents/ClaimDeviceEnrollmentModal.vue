<script setup lang="ts">
import { computed, reactive, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import type { ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'

const props = withDefaults(defineProps<{
  agent: AdminAgent
  templates: AgentTemplateLink[]
  submitting?: boolean
  error?: unknown
}>(), {
  submitting: false,
})

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ submit: [input: ClaimDeviceEnrollmentInput] }>()

const form = reactive({
  code: '',
  name: '',
  templateKey: '',
})

const usableTemplates = computed(() => props.templates.filter((template) => template.enabled))
const codeValid = computed(() => /^[0-9]{6}$/.test(form.code))

watch(open, (visible) => {
  if (!visible) return
  form.code = ''
  form.name = ''
  form.templateKey = ''
})

function submit() {
  if (!codeValid.value || props.submitting) return
  emit('submit', {
    code: form.code,
    agent_key: props.agent.key,
    ...(form.name.trim() ? { name: form.name.trim() } : {}),
    ...(form.templateKey ? { template_key: form.templateKey } : {}),
  })
}
</script>

<template>
  <BaseModal v-model="open" title="Thêm thiết bị" :description="`Liên kết với Agent ${agent.name}`" width-class="max-w-lg">
    <form class="space-y-4" @submit.prevent="submit">
      <p class="text-sm text-muted-foreground">Nhập mã 6 chữ số đang hiển thị trên thiết bị. Thiết bị sẽ kết nối sau khi hoàn tất kích hoạt.</p>
      <p v-if="error" class="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{{ formatApiError(error) }}</p>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">Mã kích hoạt</span>
        <input v-model="form.code" class="admin-input font-mono tracking-[0.25em]" type="text" inputmode="numeric" maxlength="6" pattern="[0-9]{6}" autocomplete="one-time-code" placeholder="000000" aria-describedby="activation-code-hint" required />
        <span id="activation-code-hint" class="text-xs text-muted-foreground">Giữ cả số 0 ở đầu mã.</span>
      </label>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">Tên thiết bị <span class="text-muted-foreground">(tuỳ chọn)</span></span>
        <input v-model="form.name" class="admin-input" maxlength="128" placeholder="Loa phòng khách" />
      </label>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">Template</span>
        <select v-model="form.templateKey" class="admin-input">
          <option value="">Theo Agent</option>
          <option v-for="template in usableTemplates" :key="template.key" :value="template.key">{{ template.name }} · {{ template.language }}</option>
        </select>
      </label>
      <div class="flex justify-end gap-2 pt-2">
        <Button type="button" variant="outline" :disabled="submitting" @click="open = false">Huỷ</Button>
        <Button type="submit" :disabled="!codeValid || submitting">{{ submitting ? 'Đang liên kết…' : 'Liên kết thiết bị' }}</Button>
      </div>
    </form>
  </BaseModal>
</template>
