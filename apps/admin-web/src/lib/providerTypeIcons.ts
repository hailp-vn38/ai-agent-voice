import { Brain, Eye, Mic, Volume2, Waves } from '@lucide/vue'
import type { Component } from 'vue'

import type { ProviderType } from '@/domain/admin'

export const providerTypeIcons: Record<ProviderType, Component> = {
  vad: Waves,
  asr: Mic,
  llm: Brain,
  tts: Volume2,
  vision: Eye,
}
