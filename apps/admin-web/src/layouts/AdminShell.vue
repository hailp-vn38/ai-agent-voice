<script setup lang="ts">
import { AudioWaveform, Menu } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

import AppSidebar from '@/components/AppSidebar.vue'
import SidebarContent from '@/components/SidebarContent.vue'
import ThemeToggle from '@/components/ThemeToggle.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()

const drawerOpen = ref(false)

watch(drawerOpen, (open) => {
  document.body.style.overflow = open ? 'hidden' : ''
})

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') drawerOpen.value = false
}

function onResize() {
  if (window.matchMedia('(min-width: 64rem)').matches) drawerOpen.value = false
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  window.addEventListener('resize', onResize)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('resize', onResize)
  document.body.style.overflow = ''
})
</script>

<template>
  <div class="min-h-screen bg-background text-foreground">
    <AppSidebar />

    <div class="lg:pl-64">
      <header class="sticky top-0 z-20 border-b bg-background/90 backdrop-blur supports-[backdrop-filter]:bg-background/70 lg:hidden">
        <div class="flex h-14 items-center gap-2 px-4 sm:px-6">
          <Button variant="ghost" size="icon" class="-ml-2" :aria-label="t('nav.open')" @click="drawerOpen = true">
            <Menu class="size-5" />
          </Button>
          <AudioWaveform class="size-5 shrink-0" />
          <span class="truncate text-sm font-semibold">{{ t('app.brand') }}</span>
          <ThemeToggle class="ml-auto" />
        </div>
      </header>

      <main class="mx-auto w-full max-w-[1600px] p-4 sm:p-6 lg:p-8">
        <slot />
      </main>
    </div>

    <Teleport to="body">
      <Transition
        enter-active-class="transition-opacity duration-200 ease-out"
        enter-from-class="opacity-0"
        leave-active-class="transition-opacity duration-150 ease-in"
        leave-to-class="opacity-0"
      >
        <button v-if="drawerOpen" class="fixed inset-0 z-50 bg-black/55 lg:hidden" :aria-label="t('nav.close')" @click="drawerOpen = false" />
      </Transition>

      <Transition
        enter-active-class="transition-transform duration-200 ease-out"
        enter-from-class="-translate-x-full"
        leave-active-class="transition-transform duration-150 ease-in"
        leave-to-class="-translate-x-full"
      >
        <div v-if="drawerOpen" class="fixed inset-y-0 left-0 z-50 w-72 max-w-[85vw] border-r shadow-2xl lg:hidden">
          <SidebarContent @navigate="drawerOpen = false" />
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
