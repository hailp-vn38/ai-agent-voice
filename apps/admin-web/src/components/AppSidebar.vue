<script setup lang="ts">
import SidebarContent from '@/components/SidebarContent.vue'

defineProps<{ collapsed: boolean }>()
defineEmits<{ toggle: [] }>()
</script>

<template>
  <aside
    class="fixed inset-y-0 left-0 z-30 hidden overflow-hidden border-r border-border bg-sidebar transition-[width] duration-300 ease-in-out motion-reduce:transition-none lg:flex"
    :class="collapsed ? 'w-[72px]' : 'w-64'"
    aria-label="Web navigation sidebar"
  >
    <SidebarContent collapsible :collapsed="collapsed" @toggle="$emit('toggle')" />
  </aside>
</template>
