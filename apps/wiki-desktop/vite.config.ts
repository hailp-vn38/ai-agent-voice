import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 5174, strictPort: true },
  build: { target: 'es2022' },
  test: { environment: 'jsdom', include: ['src/**/*.test.ts'] },
})
