<script setup lang="ts">
import { computed } from 'vue'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const props = defineProps<{
  title: string
  value: string
  detail?: string
  state?: string
  percentage?: number | null
}>()
const gauge = computed(() => props.percentage != null && Number.isFinite(props.percentage)
  ? Math.min(100, Math.max(0, props.percentage)) : null)
</script>

<template>
  <Card class="min-w-0">
    <CardHeader class="pb-3">
      <CardTitle class="text-sm">{{ title }}</CardTitle>
      <CardDescription v-if="state" role="status">{{ state }}</CardDescription>
    </CardHeader>
    <CardContent>
      <p class="text-2xl font-semibold tabular-nums">{{ value }}</p>
      <p v-if="detail" class="mt-2 text-xs text-muted-foreground">{{ detail }}</p>
      <div v-if="gauge !== null" role="progressbar" :aria-label="title" :aria-valuenow="gauge" :aria-valuemin="0" :aria-valuemax="100" class="mt-3 h-2 overflow-hidden rounded-full bg-muted">
        <div class="h-full bg-primary" :style="{ width: `${gauge}%` }" />
      </div>
    </CardContent>
  </Card>
</template>
