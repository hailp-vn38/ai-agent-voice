<script setup lang="ts">
import { CheckCircle2, Database, Download, ShieldAlert, Upload } from '@lucide/vue'
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
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'

const { locale } = useI18n()
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
  <Card data-testid="database-backup-card">
    <CardHeader class="flex flex-row flex-wrap items-center justify-between gap-3">
      <div class="flex items-center gap-3">
        <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-sky-500/10 text-sky-600 dark:text-sky-400">
          <Database class="size-5" />
        </div>
        <div class="space-y-0.5">
          <p class="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
            DATABASE BACKUP &amp; RESTORE
          </p>
          <CardTitle>
            {{ isVi ? 'Sao lưu & Khôi phục Database Trực tuyến' : 'Online Database Backup & Restore' }}
          </CardTitle>
          <CardDescription>
            {{
              isVi
                ? 'Xuất gói cấu hình (.json) hoặc ảnh chụp SQLite (.db) nhất quán và khôi phục trực tuyến qua giao dịch không cần khởi động lại máy chủ.'
                : 'Export a portable JSON Backup Bundle (.json) or consistent SQLite Snapshot (.db) and restore online transactionally without restarting the server.'
            }}
          </CardDescription>
        </div>
      </div>
    </CardHeader>

    <CardContent class="grid gap-6 lg:grid-cols-2">
      <!-- Block 1: Export -->
      <div class="space-y-4 rounded-xl border border-border/70 bg-muted/20 p-4">
        <div class="space-y-1">
          <h3 class="text-sm font-semibold">
            {{ isVi ? '1. Xuất dữ liệu (Export Backup)' : '1. Export Backup' }}
          </h3>
          <p class="text-xs text-muted-foreground">
            {{
              isVi
                ? 'Bí mật đã mã hóa AES-256-GCM được giữ nguyên dạng niêm phong, không bao giờ xuất ra plaintext.'
                : 'AES-256-GCM credentials remain sealed and are never exported in plaintext.'
            }}
          </p>
        </div>

        <div class="grid gap-3 sm:grid-cols-2">
          <label class="space-y-1 text-xs font-medium">
            <span>{{ isVi ? 'Định dạng file' : 'Backup Format' }}</span>
            <select
              v-model="exportFormat"
              data-testid="backup-export-format"
              class="admin-input w-full text-sm"
              :disabled="exporting"
            >
              <option value="json">JSON Bundle (.json)</option>
              <option value="sqlite">SQLite Snapshot (.db)</option>
            </select>
          </label>

          <label class="space-y-1 text-xs font-medium">
            <span>{{ isVi ? 'Phạm vi dữ liệu' : 'Backup Scope' }}</span>
            <select
              v-model="exportScope"
              data-testid="backup-export-scope"
              class="admin-input w-full text-sm"
              :disabled="exporting"
            >
              <option value="full">
                {{ isVi ? 'Toàn bộ (Cấu hình + Lịch sử + Nhắc hẹn)' : 'Full (Config + History + Reminders)' }}
              </option>
              <option value="config_only">
                {{ isVi ? 'Chỉ cấu hình cốt lõi (Config-Only)' : 'Config-Only' }}
              </option>
            </select>
          </label>
        </div>

        <p v-if="exportError" role="alert" class="text-xs text-destructive">
          {{ exportError }}
        </p>

        <Button
          type="button"
          data-testid="backup-export-button"
          :disabled="exporting"
          @click="handleExport"
        >
          <Download class="size-4" />
          <span>
            {{
              exporting
                ? isVi
                  ? 'Đang tạo bản sao lưu...'
                  : 'Exporting...'
                : isVi
                  ? 'Tải xuống bản sao lưu'
                  : 'Download Backup'
            }}
          </span>
        </Button>
      </div>

      <!-- Block 2: Inspect & Import -->
      <div class="space-y-4 rounded-xl border border-border/70 bg-muted/20 p-4">
        <div class="space-y-1">
          <h3 class="text-sm font-semibold">
            {{ isVi ? '2. Nhập / Khôi phục dữ liệu (Inspect & Import)' : '2. Inspect & Restore Backup' }}
          </h3>
          <p class="text-xs text-muted-foreground">
            {{
              isVi
                ? 'Tự động kiểm tra trước (Dry-run Backup Inspection) khi chọn tệp .json hoặc .db.'
                : 'Automatically runs a read-only Backup Inspection when a .json or .db file is selected.'
            }}
          </p>
        </div>

        <div class="space-y-3">
          <label class="block space-y-1 text-xs font-medium">
            <span>{{ isVi ? 'Tệp sao lưu (.json hoặc .db)' : 'Backup File (.json or .db)' }}</span>
            <input
              type="file"
              accept=".json,.db,.sqlite,application/json,application/vnd.sqlite3"
              data-testid="backup-file-input"
              class="admin-input block w-full text-xs file:mr-3 file:rounded-md file:border-0 file:bg-primary/10 file:px-2.5 file:py-1 file:text-xs file:font-semibold file:text-primary"
              :disabled="inspecting || importing"
              @change="onFileChange"
            />
          </label>

          <div class="grid gap-3 sm:grid-cols-2">
            <label class="space-y-1 text-xs font-medium">
              <span>{{ isVi ? 'Chế độ nhập' : 'Import Mode' }}</span>
              <select
                v-model="importMode"
                data-testid="backup-mode-select"
                class="admin-input w-full text-sm"
                :disabled="inspecting || importing"
              >
                <option value="merge">
                  {{ isVi ? 'Merge (Gộp & cập nhật theo khóa)' : 'Merge (Upsert by key)' }}
                </option>
                <option value="replace">
                  {{ isVi ? 'Replace (Đồng bộ theo bản sao lưu)' : 'Replace (Sync to backup)' }}
                </option>
              </select>
            </label>

            <label class="space-y-1 text-xs font-medium">
              <span>{{ isVi ? 'Phạm vi áp dụng' : 'Import Scope' }}</span>
              <select
                v-model="importScope"
                data-testid="backup-import-scope-select"
                class="admin-input w-full text-sm"
                :disabled="inspecting || importing"
              >
                <option value="full">{{ isVi ? 'Toàn bộ (Full)' : 'Full' }}</option>
                <option value="config_only">{{ isVi ? 'Chỉ cấu hình (Config-Only)' : 'Config-Only' }}</option>
              </select>
            </label>
          </div>

          <label class="flex items-center gap-2 text-xs">
            <input
              v-model="clearUnreadableCredentials"
              type="checkbox"
              data-testid="backup-clear-credentials-checkbox"
              :disabled="inspecting || importing"
            />
            <span>
              {{
                isVi
                  ? 'Tự động xóa bí mật không giải mã được (khi nhập từ máy chủ khác khóa mã hóa)'
                  : 'Automatically scrub unreadable credentials (when importing across different master keys)'
              }}
            </span>
          </label>
        </div>

        <!-- Shimmer loading state during Backup Inspection -->
        <div
          v-if="inspecting"
          role="status"
          aria-busy="true"
          data-testid="backup-inspect-shimmer"
          class="card-shimmer space-y-2 rounded-lg border border-border/60 p-3 text-xs text-muted-foreground"
        >
          {{ isVi ? 'Đang phân tích gói sao lưu (Backup Inspection)...' : 'Inspecting backup file...' }}
        </div>

        <p v-if="importError" role="alert" data-testid="backup-import-error" class="text-xs text-destructive">
          {{ importError }}
        </p>

        <!-- Inspection summary -->
        <div
          v-if="inspectionReport && !inspecting"
          data-testid="backup-inspection-summary"
          class="space-y-3 rounded-lg border border-border/80 bg-background/80 p-3 text-xs"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex flex-wrap items-center gap-2">
              <Badge variant="secondary">{{ inspectionReport.detected_format.toUpperCase() }}</Badge>
              <Badge variant="outline">{{ inspectionReport.effective_scope }}</Badge>
              <span class="text-muted-foreground">v{{ inspectionReport.server_version }}</span>
            </div>
            <div class="flex flex-wrap gap-2 font-medium">
              <span class="text-emerald-600 dark:text-emerald-400">
                +{{ inspectionReport.totals.to_create }} {{ isVi ? 'mới' : 'create' }}
              </span>
              <span class="text-sky-600 dark:text-sky-400">
                ~{{ inspectionReport.totals.to_update }} {{ isVi ? 'cập nhật' : 'update' }}
              </span>
              <span v-if="inspectionReport.totals.to_delete > 0" class="text-destructive">
                -{{ inspectionReport.totals.to_delete }} {{ isVi ? 'xóa' : 'delete' }}
              </span>
            </div>
          </div>

          <div class="max-h-44 overflow-y-auto rounded border border-border/60">
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
            class="flex items-start gap-2 rounded-md bg-amber-500/10 p-2 text-amber-700 dark:text-amber-300"
          >
            <ShieldAlert class="mt-0.5 size-4 shrink-0" />
            <ul class="list-disc space-y-1 pl-4">
              <li v-for="(warn, idx) in inspectionReport.warnings" :key="idx">{{ warn }}</li>
            </ul>
          </div>

          <Button
            type="button"
            data-testid="backup-import-button"
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
          class="flex items-center gap-2 rounded-md bg-emerald-500/10 p-2.5 text-xs text-emerald-700 dark:text-emerald-300"
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
    </CardContent>
  </Card>

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
