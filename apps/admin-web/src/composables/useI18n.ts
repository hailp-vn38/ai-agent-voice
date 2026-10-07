import { readonly, ref } from 'vue'

import {
  en,
  vi,
  type MessageKey,
  type MessageValue,
  type PluralMessage,
} from '@/i18n/messages'
import type { DeviceStatus, ProviderStatus, ProviderType } from '@/domain/admin'
import { formatRelativeTime } from '@/lib/formatRelativeTime'

const STORAGE_KEY = 'voice-agent-admin-locale'

export const localeOptions = [
  { value: 'en', label: 'English' },
  { value: 'vi', label: 'Tiếng Việt' },
] as const

export type Locale = (typeof localeOptions)[number]['value']

const catalogs: Record<Locale, Record<MessageKey, MessageValue>> = { en, vi }

const providerTypeKeys: Record<ProviderType, MessageKey> = {
  vad: 'providerType.vad',
  asr: 'providerType.asr',
  speaker: 'providerType.speaker',
  llm: 'providerType.llm',
  tts: 'providerType.tts',
  vision: 'providerType.vision',
}

const providerStatusKeys: Record<ProviderStatus, MessageKey> = {
  ready: 'status.provider.ready',
  disabled: 'status.provider.disabled',
  error: 'status.provider.error',
}

const deviceStatusKeys: Record<DeviceStatus, MessageKey> = {
  online: 'status.device.online',
  offline: 'status.device.offline',
}

function isLocale(value: unknown): value is Locale {
  return value === 'en' || value === 'vi'
}

function loadLocale(): Locale {
  if (typeof localStorage === 'undefined') return 'en'
  const stored = localStorage.getItem(STORAGE_KEY)
  if (isLocale(stored)) return stored
  const browser = typeof navigator === 'undefined' ? undefined : navigator.language.slice(0, 2)
  return isLocale(browser) ? browser : 'en'
}

const locale = ref<Locale>(loadLocale())

function applyLocale(value: Locale) {
  locale.value = value
  if (typeof document !== 'undefined') document.documentElement.lang = value
  if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, value)
}

if (typeof document !== 'undefined') document.documentElement.lang = locale.value

function resolve(key: MessageKey, params?: Record<string, string | number>): string {
  const message = catalogs[locale.value][key]
  if (typeof message !== 'string') {
    const plural = message as PluralMessage
    const count = params?.count
    return typeof count === 'number' && count === 1 ? plural.one : plural.other
  }
  return message
}

/** `{placeholder}` interpolation over the active catalog. */
export function translate(key: MessageKey, params?: Record<string, string | number>) {
  const message = resolve(key, params)
  if (!params) return message
  return message.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match,
  )
}

export function setLocale(value: Locale) {
  applyLocale(value)
}

export function useI18n() {
  return {
    locale: readonly(locale),
    locales: localeOptions,
    t: translate,
    setLocale,
    providerTypeLabel: (type: ProviderType) => translate(providerTypeKeys[type]),
    providerStatusLabel: (status: ProviderStatus) => translate(providerStatusKeys[status]),
    deviceStatusLabel: (status: DeviceStatus) => translate(deviceStatusKeys[status]),
    formatDateTime: (value: string | Date) =>
      new Date(value).toLocaleString(locale.value === 'vi' ? 'vi-VN' : 'en-US'),
    formatRelative: (value: string) => formatRelativeTime(value, locale.value),
  }
}
