<script setup lang="ts">
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance } from '@/domain/admin'

defineProps<{ provider: ProviderInstance }>()

const { providerTypeLabel } = useI18n()
</script>

<template>
  <Badge variant="secondary" class="gap-1.5">
    <span class="font-semibold">{{ providerTypeLabel(provider.type) }}</span>
    <span class="font-normal opacity-80">{{ provider.name }}</span>
  </Badge>
</template>
