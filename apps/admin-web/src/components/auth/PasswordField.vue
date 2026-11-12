<script setup lang="ts">
import { Eye, EyeOff, LockKeyhole } from '@lucide/vue'
import { computed, ref, useId } from 'vue'
import { useI18n } from '@/composables/useI18n'

const props = withDefaults(defineProps<{
  label: string
  name?: string
  autocomplete?: 'current-password' | 'new-password'
  placeholder?: string
  disabled?: boolean
  invalid?: boolean
  describedBy?: string
  hint?: string
}>(), { name: 'password', autocomplete: 'current-password' })
const value = defineModel<string>({ required: true })
const emit = defineEmits<{ submit: [] }>()
const { t } = useI18n()
const id = useId()
const visible = ref(false)
const capsLock = ref(false)
const descriptions = computed(() => [props.describedBy, props.hint ? `${id}-hint` : '', capsLock.value ? `${id}-caps` : ''].filter(Boolean).join(' ') || undefined)
function checkCapsLock(event: KeyboardEvent) { capsLock.value = event.getModifierState('CapsLock') }
function onKeydown(event: KeyboardEvent) {
  checkCapsLock(event)
  if (event.key === 'Enter' && !event.isComposing && event.keyCode !== 229) {
    event.preventDefault()
    emit('submit')
  }
}
</script>

<template>
  <div class="password-field">
    <label :for="id">{{ label }}</label>
    <div class="password-input" :data-invalid="invalid || undefined">
      <LockKeyhole :size="16" class="password-lock" aria-hidden="true" />
      <input :id="id" v-model="value" :name="name" :type="visible ? 'text' : 'password'" :autocomplete="autocomplete" :placeholder="placeholder" :disabled="disabled" :aria-invalid="invalid || undefined" :aria-describedby="descriptions" autocapitalize="off" :spellcheck="false" required @keydown="onKeydown" @keyup="checkCapsLock" @blur="capsLock = false" />
      <button type="button" :disabled="disabled" :aria-label="t(visible ? 'auth.hidePassword' : 'auth.showPassword')" :title="t(visible ? 'auth.hidePassword' : 'auth.showPassword')" :aria-controls="id" :aria-pressed="visible" @click="visible = !visible">
        <EyeOff v-if="visible" :size="17" aria-hidden="true" /><Eye v-else :size="17" aria-hidden="true" />
      </button>
    </div>
    <p v-if="hint" :id="`${id}-hint`" class="password-hint">{{ hint }}</p>
    <p v-if="capsLock" :id="`${id}-caps`" class="password-caps" role="status">{{ t('auth.capsLock') }}</p>
  </div>
</template>

<style scoped>
.password-field > label { display: block; margin-bottom: 8px; color: var(--foreground); font-size: 12px; font-weight: 550; }
.password-input { position: relative; display: flex; align-items: center; height: 48px; border: 1px solid var(--input); border-radius: 12px; background: var(--background); transition: border-color .2s, box-shadow .2s; }
.password-input:focus-within { border-color: var(--studio-violet); box-shadow: 0 0 0 3px color-mix(in oklab, var(--studio-violet) 12%, transparent); }
.password-input[data-invalid] { border-color: var(--danger); }
.password-lock { flex-shrink: 0; margin-left: 14px; color: var(--muted-foreground); opacity: .7; }
.password-input input { width: 100%; min-width: 0; height: 100%; padding: 0 11px; background: transparent; color: var(--foreground); font-size: 14px; outline: none; border-radius: inherit; }
.password-input input::placeholder { color: var(--muted-foreground); opacity: .65; }
.password-input input:disabled { opacity: .6; }
.password-input button { display: grid; place-items: center; width: 36px; height: 36px; flex-shrink: 0; margin-right: 5px; border-radius: 9px; color: var(--muted-foreground); cursor: pointer; }
.password-input button:hover { color: var(--foreground); background: var(--accent); }
.password-input button:focus-visible { outline: 2px solid var(--ring); outline-offset: -2px; }
.password-input button:disabled { cursor: default; opacity: .4; }
.password-hint { margin-top: 8px; font-size: 11px; line-height: 1.7; color: var(--muted-foreground); }
.password-caps { margin-top: 8px; font-size: 11px; color: var(--warning-foreground); }
@media (prefers-reduced-motion: reduce) { .password-input { transition: none; } }
</style>
