<script setup lang="ts">
import { ArrowRight, CheckCircle2, Download, File, FolderOpen, ShieldAlert, ShieldCheck, Upload } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import { isApiError } from '@/api/errors'
import { systemApi } from '@/api/system'
import type {
  BackupFormat,
  BackupImportReport,
  BackupInspectionReport,
  BackupScope,
  ImportMode,
} from '@/api/types/system'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'

const { locale, t } = useI18n()
const admin = useAdminStore()

const isVi = computed(() => locale.value === 'vi')

// Export state
const exportFormat = ref<BackupFormat>('json')
const exportScope = ref<BackupScope>('full')
const exporting = ref(false)
const exportError = ref('')

// Import / Inspect state
const selectedFile = ref<File | null>(null)
const importMode = ref<ImportMode>('merge')
const importScope = ref<BackupScope>('full')
const clearUnreadableCredentials = ref(true)
const inspecting = ref(false)
const importing = ref(false)
const importError = ref('')
const inspectionReport = ref<BackupInspectionReport | null>(null)
const importReport = ref<BackupImportReport | null>(null)
const confirmOpen = ref(false)

async function handleExport() {
  exporting.value = true
  exportError.value = ''
  try {
    const { blob, filename } = await systemApi.exportBackup(exportFormat.value, exportScope.value)
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = filename
    document.body.appendChild(anchor)
    anchor.click()
    document.body.removeChild(anchor)
    URL.revokeObjectURL(url)
  } catch (err) {
    exportError.value = isApiError(err) ? err.code : 'export_failed'
  } finally {
    exporting.value = false
  }
}

async function runInspection() {
  if (!selectedFile.value) {
    inspectionReport.value = null
    return
  }
  inspecting.value = true
  importError.value = ''
  importReport.value = null
  try {
    const buffer = await selectedFile.value.arrayBuffer()
    inspectionReport.value = await systemApi.inspectBackup(buffer, {
      mode: importMode.value,
      scope: importScope.value,
      clear_unreadable_credentials: clearUnreadableCredentials.value,
    })
  } catch (err) {
    inspectionReport.value = null
    importError.value = isApiError(err) ? err.code : 'inspect_failed'
  } finally {
    inspecting.value = false
  }
}

function onFileChange(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0] ?? null
  selectedFile.value = file
  void runInspection()
}

watch([importMode, importScope, clearUnreadableCredentials], () => {
  if (selectedFile.value) {
    void runInspection()
  }
})

function requestConfirmImport() {
  if (!selectedFile.value || !inspectionReport.value) return
  confirmOpen.value = true
}

async function executeImport() {
  if (!selectedFile.value) return
  importing.value = true
  importError.value = ''
  try {
    const buffer = await selectedFile.value.arrayBuffer()
    importReport.value = await systemApi.importBackup(buffer, {
      mode: importMode.value,
      scope: importScope.value,
      clear_unreadable_credentials: clearUnreadableCredentials.value,
    })
    await admin.loadAll()
  } catch (err) {
    importError.value = isApiError(err) ? err.code : 'import_failed'
  } finally {
    importing.value = false
  }
}
</script>

<template>
  <div class="backup-workspace" data-testid="database-backup-card">
    <section class="sys-panel backup-export">
      <header class="sys-panel-header"><span class="sys-section-number">01</span>
        <div><h2>{{ t('system.detail.export') }}</h2><p>{{ t('system.detail.exportDescription') }}</p></div>
      </header>
      <div class="backup-art" aria-hidden="true">
        <span class="backup-art-coordinate">VELA / ARCHIVE</span>
        <div class="backup-paper-back" /><div class="backup-paper"><File class="size-7" /><span>{{ exportFormat === 'json' ? '.json' : '.db' }}</span><i /><i /><i /></div>
        <span class="backup-art-index">01 — {{ exportFormat.toUpperCase() }}</span>
      </div>
      <div class="backup-export-body">
        <div class="backup-format-description"><span class="sys-kicker">{{ t(exportFormat === 'json' ? 'system.detail.portable' : 'system.detail.snapshot') }}</span>
          <p>{{ t(exportFormat === 'json' ? 'system.detail.jsonDescription' : 'system.detail.sqliteDescription') }}</p>
        </div>
        <div class="sys-form-grid">
          <label class="sys-field"><span>{{ t('system.detail.format') }}</span>
            <select v-model="exportFormat" data-testid="backup-export-format" class="admin-input" :disabled="exporting">
              <option value="json">JSON (.json)</option><option value="sqlite">SQLite (.db)</option>
            </select>
          </label>
          <label class="sys-field"><span>{{ t('system.detail.scope') }}</span>
            <select v-model="exportScope" data-testid="backup-export-scope" class="admin-input" :disabled="exporting">
              <option value="full">{{ isVi ? 'Toàn bộ dữ liệu' : 'Full workspace' }}</option>
              <option value="config_only">{{ isVi ? 'Chỉ cấu hình' : 'Configuration only' }}</option>
            </select>
          </label>
        </div>
        <p v-if="exportError" role="alert" class="sys-alert">{{ exportError }}</p>
        <Button type="button" data-testid="backup-export-button" class="sys-primary w-full" :disabled="exporting" @click="handleExport">
          <Download class="size-4" aria-hidden="true" />{{ exporting ? (isVi ? 'Đang tạo bản sao lưu…' : 'Creating backup…') : (isVi ? 'Tải xuống bản sao lưu' : 'Download backup') }}
        </Button>
        <p class="backup-sealed"><ShieldCheck class="size-3.5" aria-hidden="true" />{{ t('system.detail.sealed') }}</p>
      </div>
    </section>

    <section class="sys-panel backup-restore">
      <header class="sys-panel-header"><span class="sys-section-number">02</span>
        <div><h2>{{ t('system.detail.restore') }}</h2><p>{{ t('system.detail.restoreDescription') }}</p></div>
      </header>
      <ol class="restore-steps" :aria-label="t('system.detail.restore')">
        <li :aria-current="!selectedFile ? 'step' : undefined" :data-complete="!!selectedFile"><span>1</span>{{ t('system.detail.choose') }}<ArrowRight class="size-3" aria-hidden="true" /></li>
        <li :aria-current="selectedFile && !importReport ? 'step' : undefined" :data-complete="!!inspectionReport"><span>2</span>{{ t('system.detail.inspect') }}<ArrowRight class="size-3" aria-hidden="true" /></li>
        <li :aria-current="importReport ? 'step' : undefined" :data-complete="!!importReport"><span>3</span>{{ t('system.detail.apply') }}</li>
      </ol>
      <div class="backup-restore-body">
        <label class="backup-file-picker" :data-selected="!!selectedFile">
          <FolderOpen class="size-6" aria-hidden="true" />
          <strong>{{ selectedFile?.name ?? t('system.detail.chooseFile') }}</strong>
          <span>{{ selectedFile ? t('system.detail.replaceFile') : t('system.detail.fileHint') }}</span>
          <input type="file" accept=".json,.db,.sqlite,application/json,application/vnd.sqlite3" data-testid="backup-file-input"
            :aria-label="t('system.detail.chooseFile')" :disabled="inspecting || importing" @change="onFileChange" />
        </label>
        <div class="sys-form-grid">
          <label class="sys-field"><span>{{ isVi ? 'Chế độ phục hồi' : 'Restore mode' }}</span>
            <select v-model="importMode" data-testid="backup-mode-select" class="admin-input" :disabled="inspecting || importing">
              <option value="merge">{{ isVi ? 'Gộp & cập nhật (Merge)' : 'Merge & update' }}</option>
              <option value="replace">{{ isVi ? 'Thay thế (Replace)' : 'Replace from backup' }}</option>
            </select>
          </label>
          <label class="sys-field"><span>{{ isVi ? 'Phạm vi áp dụng' : 'Restore scope' }}</span>
            <select v-model="importScope" data-testid="backup-import-scope-select" class="admin-input" :disabled="inspecting || importing">
              <option value="full">{{ isVi ? 'Toàn bộ dữ liệu' : 'Full workspace' }}</option>
              <option value="config_only">{{ isVi ? 'Chỉ cấu hình' : 'Configuration only' }}</option>
            </select>
          </label>
        </div>
        <label class="backup-scrub-option">
          <input v-model="clearUnreadableCredentials" type="checkbox" data-testid="backup-clear-credentials-checkbox" :disabled="inspecting || importing" />
          <span>{{ isVi ? 'Xóa thông tin bí mật không giải mã được khi bản sao lưu dùng khóa mã hóa khác.' : 'Clear unreadable credentials when the backup uses a different encryption key.' }}</span>
        </label>
        <div v-if="inspecting" role="status" aria-busy="true" data-testid="backup-inspect-shimmer" class="card-shimmer sys-loading">
          {{ isVi ? 'Đang kiểm tra bản sao lưu…' : 'Inspecting backup…' }}
        </div>
        <p v-if="importError" role="alert" data-testid="backup-import-error" class="sys-alert">{{ importError }}</p>
        <div v-if="!selectedFile" class="backup-empty-preview"><File class="size-5" aria-hidden="true" />
          <h3>{{ t('system.detail.awaitingFile') }}</h3><p>{{ t('system.detail.awaitingFileDescription') }}</p>
        </div>
        <!-- Inspection summary -->
        <div
          v-if="inspectionReport && !inspecting"
          data-testid="backup-inspection-summary"
          class="backup-preview space-y-3 rounded-lg border border-border/80 bg-background/80 p-4 text-xs"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex flex-wrap items-center gap-2">
              <Badge variant="secondary">{{ inspectionReport.detected_format.toUpperCase() }}</Badge>
              <Badge variant="outline">{{ inspectionReport.effective_scope }}</Badge>
              <span class="text-muted-foreground">v{{ inspectionReport.server_version }}</span>
            </div>
            <div class="flex flex-wrap gap-2 font-medium">
              <span class="text-success-foreground">
                +{{ inspectionReport.totals.to_create }} {{ isVi ? 'mới' : 'create' }}
              </span>
              <span class="text-studio-cyan">
                ~{{ inspectionReport.totals.to_update }} {{ isVi ? 'cập nhật' : 'update' }}
              </span>
              <span v-if="inspectionReport.totals.to_delete > 0" class="text-danger-foreground">
                -{{ inspectionReport.totals.to_delete }} {{ isVi ? 'xóa' : 'delete' }}
              </span>
            </div>
          </div>

          <div class="max-h-60 overflow-auto rounded border border-border/60">
            <table class="w-full text-left text-[11px]">
              <thead class="bg-muted/50 text-muted-foreground">
                <tr>
                  <th class="px-2 py-1 font-medium">{{ isVi ? 'Tài nguyên' : 'Resource' }}</th>
                  <th class="px-2 py-1 text-right font-medium">{{ isVi ? 'Trong file' : 'In Backup' }}</th>
                  <th class="px-2 py-1 text-right font-medium">{{ isVi ? 'Tạo mới' : 'Create' }}</th>
                  <th class="px-2 py-1 text-right font-medium">{{ isVi ? 'Cập nhật' : 'Update' }}</th>
                  <th class="px-2 py-1 text-right font-medium">{{ isVi ? 'Xóa' : 'Delete' }}</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-border/50">
                <tr v-for="row in inspectionReport.resources" :key="row.resource">
                  <td class="px-2 py-1 font-mono">{{ row.resource }}</td>
                  <td class="px-2 py-1 text-right">{{ row.in_backup }}</td>
                  <td class="px-2 py-1 text-right">{{ row.to_create }}</td>
                  <td class="px-2 py-1 text-right">{{ row.to_update }}</td>
                  <td class="px-2 py-1 text-right">{{ row.to_delete }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <div
            v-if="inspectionReport.warnings.length > 0"
            data-testid="backup-inspection-warnings"
            class="flex items-start gap-2 rounded-md bg-warning/10 p-2 text-warning-foreground"
          >
            <ShieldAlert class="mt-0.5 size-4 shrink-0" />
            <ul class="list-disc space-y-1 pl-4">
              <li v-for="(warn, idx) in inspectionReport.warnings" :key="idx">{{ warn }}</li>
            </ul>
          </div>

          <Button
            type="button"
            data-testid="backup-import-button"
            class="sys-primary"
            :disabled="importing || (!clearUnreadableCredentials && inspectionReport.credentials.unreadable > 0)"
            @click="requestConfirmImport"
          >
            <Upload class="size-4" />
            <span>
              {{
                importing
                  ? isVi
                    ? 'Đang nhập dữ liệu...'
                    : 'Importing...'
                  : isVi
                    ? 'Xác nhận Nhập dữ liệu'
                    : 'Confirm Import'
              }}
            </span>
          </Button>
        </div>

        <!-- Import completion banner -->
        <div
          v-if="importReport"
          role="status"
          data-testid="backup-import-success"
          class="flex items-center gap-2 rounded-md bg-success/10 p-2.5 text-xs text-success-foreground"
        >
          <CheckCircle2 class="size-4 shrink-0" />
          <span>
            {{
              isVi
                ? `Đã nhập thành công (+${importReport.totals.to_create} tạo mới, ~${importReport.totals.to_update} cập nhật, -${importReport.totals.to_delete} xóa).`
                : `Import completed (+${importReport.totals.to_create} created, ~${importReport.totals.to_update} updated, -${importReport.totals.to_delete} deleted).`
            }}
          </span>
        </div>
      </div>
    </section>
  </div>

  <ConfirmDialog
    v-model="confirmOpen"
    :tone="importMode === 'replace' ? 'danger' : 'default'"
    :title="
      isVi
        ? importMode === 'replace'
          ? 'Xác nhận khôi phục chế độ Replace'
          : 'Xác nhận nhập dữ liệu (Merge)'
        : importMode === 'replace'
          ? 'Confirm Replace Restore'
          : 'Confirm Merge Import'
    "
    :description="
      isVi
        ? importMode === 'replace'
          ? 'Chế độ Replace sẽ xóa các tài nguyên không nằm trong gói sao lưu và cập nhật toàn bộ tài nguyên khớp khóa trong một giao dịch SQLite.'
          : 'Chế độ Merge sẽ thêm mới hoặc cập nhật các tài nguyên theo khóa nghiệp vụ và giữ nguyên các bản ghi hiện hữu khác.'
        : importMode === 'replace'
          ? 'Replace mode removes resources not present in the backup and updates matching resources atomically.'
          : 'Merge mode upserts resources by business key while keeping other existing records.'
    "
    @confirm="executeImport"
  />
</template>
