<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type WatcherStatus = {
  paused: boolean;
};

type WatcherEventPayload = {
  line_text: string;
  source: string;
  current_mode: string | null;
  target_mode: string | null;
  switched: boolean;
  preserved: boolean;
  reason: string;
  error: string | null;
  debug: boolean;
  window_title: string | null;
  process_name: string | null;
  focus_class: string | null;
  ime_error: string | null;
  caret: string | null;
  doc_len: number | null;
  selection_start: number | null;
  selection_end: number | null;
  line_number: number | null;
  cursor_utf16: number | null;
  cursor_chars: number | null;
};

type ClassificationResult = {
  line: string;
  cursor: number;
  target_mode: string;
  reason: string;
};

type AppConfig = {
  poll_interval_ms: number;
  debug_mode: boolean;
  auto_start: boolean;
  blacklist: string[];
  whitelist_mode: boolean;
};

const status = ref<WatcherStatus>({ paused: false });
const imeMode = ref("unknown");
const debugMode = ref(false);
const logFilePath = ref("");
const testLine = ref("hello world");
const testCursor = ref(0);
const testResult = ref<ClassificationResult | null>(null);
const error = ref("");
const startupErrors = ref<string[]>([]);
const liveLogs = ref<WatcherEventPayload[]>([]);
const maxLiveLogs = 100;

const config = ref<AppConfig>({
  poll_interval_ms: 250,
  debug_mode: false,
  auto_start: true,
  blacklist: [],
  whitelist_mode: false,
});
const newBlacklistItem = ref("");

let pollTimer: number | null = null;
let unlistenWatcher: (() => void) | null = null;
let unlistenStartup: (() => void) | null = null;

function formatError(errorLike: unknown): string {
  if (errorLike instanceof Error) {
    return errorLike.message;
  }
  return String(errorLike);
}

async function fetchStatus() {
  try {
    status.value = await invoke<WatcherStatus>("get_watcher_status");
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function fetchImeMode() {
  try {
    imeMode.value = await invoke<string>("get_current_ime_mode");
  } catch {
    imeMode.value = "error";
  }
}

async function fetchDebugMode() {
  try {
    debugMode.value = await invoke<boolean>("get_debug_mode");
  } catch {
    debugMode.value = false;
  }
}

async function fetchConfig() {
  try {
    config.value = await invoke<AppConfig>("get_config");
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function saveConfig() {
  try {
    await invoke("set_config", { config: config.value });
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function resetConfig() {
  try {
    config.value = await invoke<AppConfig>("reset_config");
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

function addBlacklistItem() {
  const name = newBlacklistItem.value.trim();
  if (name && !config.value.blacklist.includes(name)) {
    config.value.blacklist.push(name);
    newBlacklistItem.value = "";
  }
}

function removeBlacklistItem(index: number) {
  config.value.blacklist.splice(index, 1);
}

async function toggleDebug() {
  try {
    const enabled = await invoke<boolean>("set_debug_mode", { enabled: !debugMode.value });
    debugMode.value = enabled;
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function fetchLogFilePath() {
  try {
    logFilePath.value = await invoke<string>("get_log_file_path");
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function openLogFolder() {
  try {
    await invoke("open_log_folder");
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function togglePause() {
  try {
    const paused = await invoke<boolean>("toggle_watcher_pause");
    status.value = { paused };
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
  }
}

async function runTest() {
  try {
    testResult.value = await invoke<ClassificationResult>("test_classify", {
      line: testLine.value,
      cursor: testCursor.value,
    });
    error.value = "";
  } catch (errorLike) {
    error.value = formatError(errorLike);
    testResult.value = null;
  }
}

function clearLiveLogs() {
  liveLogs.value = [];
}

async function refreshDiagnostics() {
  await Promise.all([
    fetchStatus(),
    fetchImeMode(),
    fetchDebugMode(),
    fetchConfig(),
    fetchLogFilePath(),
  ]);
}

onMounted(async () => {
  await refreshDiagnostics();
  pollTimer = window.setInterval(() => {
    void fetchStatus();
    void fetchImeMode();
  }, 1000);

  unlistenWatcher = await listen<WatcherEventPayload>("watcher-event", (event) => {
    liveLogs.value.unshift(event.payload);
    if (liveLogs.value.length > maxLiveLogs) {
      liveLogs.value.pop();
    }
  });

  unlistenStartup = await listen<string[]>("startup-check-failed", (event) => {
    startupErrors.value = event.payload;
  });
});

onUnmounted(() => {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
  }
  if (unlistenWatcher) {
    unlistenWatcher();
  }
  if (unlistenStartup) {
    unlistenStartup();
  }
});
</script>

<template>
  <main class="container">
    <header class="hero">
      <p class="eyebrow">Windows IME watcher</p>
      <h1>Smart Shift</h1>
      <p class="subtitle">
        Auto-switch the IME only when focus or caret location changes.
      </p>
    </header>

    <div v-if="error" class="error">{{ error }}</div>
    <div v-if="startupErrors.length > 0" class="warning">
      <strong>Startup check failed:</strong>
      <ul>
        <li v-for="(msg, idx) in startupErrors" :key="idx">{{ msg }}</li>
      </ul>
    </div>

    <section class="card">
      <div class="section-head">
        <h2>Runtime</h2>
        <button class="btn-secondary" @click="refreshDiagnostics">Refresh</button>
      </div>
      <div class="status-grid">
        <div class="status-tile">
          <span class="label">Watcher</span>
          <span :class="['badge', status.paused ? 'paused' : 'running']">
            {{ status.paused ? "Paused" : "Running" }}
          </span>
        </div>
        <div class="status-tile">
          <span class="label">Current IME</span>
          <span class="badge neutral">{{ imeMode }}</span>
        </div>
      </div>
      <div style="display:flex;gap:10px;margin-top:12px;flex-wrap:wrap;">
        <button class="btn-primary" @click="togglePause">
          {{ status.paused ? "Resume watcher" : "Pause watcher" }}
        </button>
        <button :class="['btn-secondary', debugMode ? 'active-debug' : '']" @click="toggleDebug">
          {{ debugMode ? "Debug: ON" : "Debug: OFF" }}
        </button>
      </div>
    </section>

    <section class="card">
      <div class="section-head">
        <h2>Configuration</h2>
        <div class="actions">
          <button class="btn-secondary" @click="saveConfig">Save</button>
          <button class="btn-secondary" @click="resetConfig">Reset</button>
        </div>
      </div>

      <div class="form-row">
        <label for="poll">Poll interval (ms)</label>
        <input
          id="poll"
          v-model.number="config.poll_interval_ms"
          type="number"
          class="input"
          min="50"
          max="1000"
        />
      </div>
      <div class="form-row">
        <label>Auto-start watcher</label>
        <input v-model="config.auto_start" type="checkbox" />
      </div>
      <div class="form-row">
        <label>Whitelist mode</label>
        <input v-model="config.whitelist_mode" type="checkbox" />
      </div>

      <div class="form-row" style="align-items: flex-start;">
        <label>App list</label>
        <div style="display: grid; gap: 8px; min-width: 0;">
          <div style="display: flex; gap: 8px;">
            <input
              v-model="newBlacklistItem"
              class="input"
              placeholder="Process name, e.g. Wave.exe"
              @keydown.enter="addBlacklistItem"
            />
            <button class="btn-secondary" @click="addBlacklistItem">Add</button>
          </div>
          <div v-if="config.blacklist.length > 0" class="tag-list">
            <span v-for="(item, idx) in config.blacklist" :key="idx" class="tag">
              {{ item }}
              <button class="tag-remove" @click="removeBlacklistItem(idx)">×</button>
            </span>
          </div>
          <p v-else class="hint">{{ config.whitelist_mode ? "No whitelist apps configured." : "No blacklist apps configured." }}</p>
        </div>
      </div>
      <p class="hint">
        Poll interval changes take effect after restart. Blacklist changes are effective immediately.
      </p>
    </section>

    <section class="card">
      <h2>Classifier Test</h2>
      <div class="form-row">
        <label for="line">Line</label>
        <input id="line" v-model="testLine" class="input" />
      </div>
      <div class="form-row">
        <label for="cursor">Cursor</label>
        <input
          id="cursor"
          v-model.number="testCursor"
          type="number"
          class="input"
          min="0"
        />
      </div>
      <button class="btn-primary" @click="runTest">Run classify</button>

      <div v-if="testResult" class="result">
        <div><strong>Target mode:</strong> {{ testResult.target_mode }}</div>
        <div><strong>Reason:</strong> {{ testResult.reason }}</div>
        <div><strong>Line:</strong> {{ testResult.line }}</div>
        <div><strong>Cursor:</strong> {{ testResult.cursor }}</div>
      </div>
    </section>

    <section class="card">
      <div class="section-head">
        <h2>Logs</h2>
        <div class="actions">
          <button class="btn-secondary" @click="clearLiveLogs">Clear live</button>
          <button class="btn-secondary" @click="openLogFolder">Open folder</button>
        </div>
      </div>
      <p class="path-line">
        <span class="label">Today file</span>
        <code>{{ logFilePath || "Unavailable" }}</code>
      </p>

      <div class="log-panel">
        <h3>Event log</h3>
        <div class="log-list">
          <div v-if="liveLogs.length === 0" class="log-empty">No live events yet.</div>
          <div
            v-for="(log, index) in liveLogs"
            :key="`live-${index}`"
            :class="[
              'log-item',
              log.preserved ? 'log-preserved' : '',
              log.switched ? 'log-switched' : '',
            ]"
          >
            <span class="log-source">[{{ log.source }}]</span>
            <span class="log-text">{{ log.line_text || "(empty)" }}</span>
            <span class="log-mode">
              {{ log.current_mode || "?" }} -> {{ log.target_mode || "?" }}
            </span>
            <span class="log-reason">{{ log.reason }}</span>
            <div v-if="log.debug" class="log-debug">
              <div v-if="log.window_title"><strong>Window:</strong> {{ log.window_title }}</div>
              <div v-if="log.process_name"><strong>Process:</strong> {{ log.process_name }}</div>
              <div v-if="log.focus_class"><strong>Focus class:</strong> {{ log.focus_class }}</div>
              <div v-if="log.caret"><strong>Caret:</strong> {{ log.caret }}</div>
              <div v-if="log.doc_len !== null"><strong>Doc len:</strong> {{ log.doc_len }}</div>
              <div v-if="log.selection_start !== null && log.selection_end !== null">
                <strong>Selection:</strong> {{ log.selection_start }} - {{ log.selection_end }}
              </div>
              <div v-if="log.line_number !== null"><strong>Line:</strong> {{ log.line_number }}</div>
              <div v-if="log.cursor_utf16 !== null"><strong>Cursor utf16:</strong> {{ log.cursor_utf16 }}</div>
              <div v-if="log.cursor_chars !== null"><strong>Cursor chars:</strong> {{ log.cursor_chars }}</div>
              <div v-if="log.ime_error"><strong>IME error:</strong> {{ log.ime_error }}</div>
            </div>
          </div>
        </div>
      </div>
    </section>
  </main>
</template>

<style scoped>
:global(body) {
  margin: 0;
  background:
    radial-gradient(circle at top, rgba(255, 255, 255, 0.75), transparent 35%),
    linear-gradient(180deg, #f4efe8 0%, #e7ecf3 100%);
  color: #172033;
  font-family: "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
}

.container {
  max-width: 1040px;
  margin: 0 auto;
  padding: 32px 20px 48px;
}

.hero {
  margin-bottom: 20px;
}

.eyebrow {
  margin: 0 0 8px;
  font-size: 0.78rem;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: #8c4a2f;
}

h1 {
  margin: 0;
  font-size: clamp(2rem, 5vw, 3.25rem);
  line-height: 1;
}

.subtitle {
  max-width: 640px;
  margin: 12px 0 0;
  color: #4b5565;
  font-size: 1rem;
}

.card {
  margin-top: 18px;
  padding: 20px;
  border: 1px solid rgba(23, 32, 51, 0.08);
  border-radius: 20px;
  background: rgba(255, 255, 255, 0.78);
  backdrop-filter: blur(12px);
  box-shadow: 0 16px 40px rgba(23, 32, 51, 0.08);
}

.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

h2,
h3 {
  margin: 0;
}

h2 {
  font-size: 1.15rem;
}

h3 {
  font-size: 0.95rem;
  color: #374151;
}

.status-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
  margin: 16px 0;
}

.status-tile {
  padding: 14px;
  border-radius: 14px;
  background: linear-gradient(135deg, rgba(23, 32, 51, 0.04), rgba(140, 74, 47, 0.08));
}

.label {
  display: block;
  margin-bottom: 8px;
  font-size: 0.82rem;
  font-weight: 700;
  color: #6b7280;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.badge {
  display: inline-flex;
  align-items: center;
  padding: 6px 12px;
  border-radius: 999px;
  font-size: 0.88rem;
  font-weight: 700;
}

.badge.running {
  background: #dff7eb;
  color: #0f6b43;
}

.badge.paused {
  background: #fff1d6;
  color: #9a5a00;
}

.badge.neutral {
  background: #e8eef8;
  color: #24436b;
}

.btn-primary,
.btn-secondary {
  border: none;
  border-radius: 999px;
  cursor: pointer;
  font: inherit;
}

.btn-primary {
  padding: 10px 16px;
  background: linear-gradient(135deg, #1f5d8d, #3f7d58);
  color: #fff;
}

.btn-secondary {
  padding: 8px 14px;
  background: #eef2f7;
  color: #243247;
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.form-row {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
  margin: 12px 0;
}

.input {
  width: 100%;
  padding: 10px 12px;
  border: 1px solid #d7dce5;
  border-radius: 12px;
  background: #fff;
  color: inherit;
  box-sizing: border-box;
}

.result {
  margin-top: 14px;
  padding: 14px;
  border-radius: 14px;
  background: #f5f7fb;
  line-height: 1.7;
}

.path-line {
  margin: 16px 0 0;
}

.path-line code {
  display: inline-block;
  max-width: 100%;
  margin-top: 8px;
  padding: 8px 10px;
  border-radius: 10px;
  background: #f4f6fa;
  word-break: break-all;
}

.log-panel {
  min-width: 0;
  margin-top: 16px;
}

.log-list {
  max-height: 320px;
  margin-top: 10px;
  overflow-y: auto;
  padding: 10px;
  border: 1px solid #e2e8f0;
  border-radius: 16px;
  background: rgba(245, 247, 251, 0.8);
}

.log-empty {
  padding: 20px;
  text-align: center;
  color: #94a3b8;
}

.log-item {
  display: grid;
  grid-template-columns: 1fr;
  gap: 4px;
  margin-bottom: 10px;
  padding: 10px 12px;
  border-radius: 12px;
  background: #fff;
}

.log-item:last-child {
  margin-bottom: 0;
}

.log-switched {
  border-left: 4px solid #1c8c5c;
}

.log-preserved {
  border-left: 4px solid #d47a24;
}

.log-source,
.log-reason {
  color: #64748b;
  font-size: 0.78rem;
}

.log-text {
  color: #172033;
  font-weight: 600;
  word-break: break-word;
}

.log-mode {
  color: #334155;
  font-size: 0.84rem;
}

.btn-secondary.active-debug {
  background: #dff7eb;
  color: #0f6b43;
  border: 1px solid #1c8c5c;
}

.tag-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border-radius: 999px;
  background: #eef2f7;
  font-size: 0.84rem;
  color: #243247;
}

.tag-remove {
  border: none;
  background: transparent;
  color: #64748b;
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
  padding: 0 2px;
}

.tag-remove:hover {
  color: #991b1b;
}

.hint {
  margin: 0;
  font-size: 0.78rem;
  color: #6b7280;
}

.log-debug {
  margin-top: 6px;
  padding: 8px 10px;
  border-radius: 10px;
  background: #f4f6fa;
  font-size: 0.78rem;
  color: #4b5565;
  line-height: 1.6;
}

.warning {
  margin-top: 18px;
  padding: 12px 14px;
  border-radius: 14px;
  background: #fff1d6;
  color: #9a5a00;
}

.warning ul {
  margin: 8px 0 0;
  padding-left: 18px;
}

.error {
  margin-top: 18px;
  padding: 12px 14px;
  border-radius: 14px;
  background: #fee2e2;
  color: #991b1b;
}

@media (max-width: 720px) {
  .container {
    padding-inline: 14px;
  }

  .card {
    padding: 16px;
    border-radius: 16px;
  }

  .section-head {
    align-items: flex-start;
    flex-direction: column;
  }

  .form-row {
    grid-template-columns: 1fr;
    gap: 8px;
  }
}
</style>
