import { readonly, ref } from 'vue'

const STORAGE_KEY = 'voice-agent-admin-theme'

const dark = ref(false)
let initialized = false

function applyTheme(value: boolean) {
  dark.value = value
  document.documentElement.classList.toggle('dark', value)
  localStorage.setItem(STORAGE_KEY, value ? 'dark' : 'light')
}

function initTheme() {
  if (initialized) return
  const stored = localStorage.getItem(STORAGE_KEY)
  const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches
  applyTheme(stored ? stored === 'dark' : systemDark)
  initialized = true
}

export function useTheme() {
  initTheme()

  return {
    dark: readonly(dark),
    toggleTheme: () => applyTheme(!dark.value),
  }
}
