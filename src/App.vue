<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const status = ref({ paused: false });
const imeMode = ref("unknown");
const testLine = ref("hello world");
const testCursor = ref(0);
const testResult = ref<any>(null);
const error = ref("");
const logs = ref<any[]>([]);
const maxLogs = 100;

let pollTimer: number | null = null;
let unlistenWatcher: (() => void) | null = null;

async function fetchStatus() {
  try {
    status.value = await invoke("get_watcher_status");
    error.value = "";
  } catch (e: any) {
    error.value = String(e);
  }
}

async function fetchImeMode() {
  try {
    imeMode.value = await invoke("get_current_ime_mode");
  } catch (e: any) {
    imeMode.value = "error";
  }
}

async function togglePause() {
  try {
    const paused = await invoke("toggle_watcher_pause");
    status.value = { paused };
    error.value = "";
  } catch (e: any) {
    error.value = String(e);
  }
}

async function runTest() {
  try {
    testResult.value = await invoke("test_classify", {
      line: testLine.value,
      cursor: testCursor.value,
    });
    error.value = "";
  } catch (e: any) {
    error.value = String(e);
    testResult.value = null;
  }
}

function clearLogs() {
  logs.value = [];
}

onMounted(async () => {
  fetchStatus();
  fetchImeMode();
  pollTimer = window.setInterval(() => {
    fetchStatus();
    fetchImeMode();
  }, 1000);

  unlistenWatcher = await listen("watcher-event", (event) => {
    logs.value.unshift(event.payload);
    if (logs.value.length > maxLogs) {
      logs.value.pop();
    }
  });
});

onUnmounted(() => {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
  }
  if (unlistenWatcher) {
    unlistenWatcher();
  }
});
</script>

<template>
  <main class="container">
    <h1>Smart Shift</h1>
    <p class="subtitle">智能输入法切换器</p>

    <div v-if="error" class="error">{{ error }}</div>

    <section class="card">
      <h2>运行状态</h2>
      <div class="status-row">
        <span class="label">Watcher：</span>
        <span :class="['badge', status.paused ? 'paused' : 'running']">
          {{ status.paused ? "已暂停" : "运行中" }}
        </span>
      </div>
      <div class="status-row">
        <span class="label">当前 IME：</span>
        <span class="badge">{{ imeMode }}</span>
      </div>
      <button class="btn-primary" @click="togglePause">
        {{ status.paused ? "恢复监听" : "暂停监听" }}
      </button>
    </section>

    <section class="card">
      <h2>分类测试</h2>
      <div class="form-row">
        <label>文本：</label>
        <input v-model="testLine" class="input" />
      </div>
      <div class="form-row">
        <label>光标位置：</label>
        <input v-model.number="testCursor" type="number" class="input" min="0" />
      </div>
      <button class="btn-primary" @click="runTest">测试分类</button>

      <div v-if="testResult" class="result">
        <div><strong>目标模式：</strong>{{ testResult.target_mode }}</div>
        <div><strong>原因：</strong>{{ testResult.reason }}</div>
        <div><strong>行内容：</strong>{{ testResult.line }}</div>
        <div><strong>光标：</strong>{{ testResult.cursor }}</div>
      </div>
    </section>

    <section class="card">
      <h2>
        事件日志
        <button class="btn-small" @click="clearLogs">清空</button>
      </h2>
      <div class="log-list">
        <div v-if="logs.length === 0" class="log-empty">暂无事件</div>
        <div
          v-for="(log, index) in logs"
          :key="index"
          :class="['log-item', log.preserved ? 'log-preserved' : '', log.switched ? 'log-switched' : '']"
        >
          <span class="log-source">[{{ log.source }}]</span>
          <span class="log-text">{{ log.line_text || "(empty)" }}</span>
          <span class="log-mode">
            {{ log.current_mode || "?" }} → {{ log.target_mode || "?" }}
          </span>
          <span class="log-reason">({{ log.reason }})</span>
        </div>
      </div>
    </section>
  </main>
</template>

<style scoped>
.container {
  max-width: 560px;
  margin: 0 auto;
  padding: 24px;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}

h1 {
  font-size: 1.8rem;
  margin-bottom: 4px;
  text-align: center;
}

.subtitle {
  text-align: center;
  color: #888;
  margin-top: 0;
  margin-bottom: 24px;
}

.card {
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
  padding: 20px;
  margin-bottom: 20px;
}

.card h2 {
  font-size: 1.1rem;
  margin-top: 0;
  margin-bottom: 14px;
  color: #333;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.status-row {
  display: flex;
  align-items: center;
  margin-bottom: 10px;
  gap: 8px;
}

.label {
  color: #555;
  min-width: 80px;
}

.badge {
  display: inline-block;
  padding: 4px 10px;
  border-radius: 999px;
  background: #eee;
  font-size: 0.85rem;
  font-weight: 600;
  color: #333;
}

.badge.running {
  background: #d4edda;
  color: #155724;
}

.badge.paused {
  background: #fff3cd;
  color: #856404;
}

.btn-primary {
  margin-top: 10px;
  padding: 8px 16px;
  border: none;
  border-radius: 8px;
  background: #2b6cb0;
  color: #fff;
  font-size: 0.95rem;
  cursor: pointer;
  transition: background 0.2s;
}

.btn-primary:hover {
  background: #2c5282;
}

.btn-small {
  padding: 4px 10px;
  border: none;
  border-radius: 6px;
  background: #e2e8f0;
  color: #4a5568;
  font-size: 0.8rem;
  cursor: pointer;
}

.btn-small:hover {
  background: #cbd5e0;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.form-row label {
  min-width: 80px;
  color: #555;
}

.input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-size: 0.95rem;
}

.result {
  margin-top: 14px;
  padding: 12px;
  background: #f8f9fa;
  border-radius: 8px;
  font-size: 0.9rem;
  line-height: 1.6;
}

.log-list {
  max-height: 300px;
  overflow-y: auto;
  border: 1px solid #eee;
  border-radius: 8px;
  padding: 8px;
}

.log-empty {
  color: #aaa;
  text-align: center;
  padding: 20px;
  font-size: 0.9rem;
}

.log-item {
  padding: 6px 8px;
  border-radius: 4px;
  font-size: 0.85rem;
  margin-bottom: 4px;
  background: #f8f9fa;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
}

.log-item:last-child {
  margin-bottom: 0;
}

.log-switched {
  background: #e6fffa;
  border-left: 3px solid #38b2ac;
}

.log-preserved {
  background: #fffaf0;
  border-left: 3px solid #ed8936;
}

.log-source {
  color: #888;
  font-size: 0.75rem;
  min-width: 80px;
}

.log-text {
  flex: 1;
  color: #333;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.log-mode {
  color: #555;
  font-weight: 600;
}

.log-reason {
  color: #888;
  font-size: 0.75rem;
}

.error {
  background: #f8d7da;
  color: #721c24;
  padding: 10px 14px;
  border-radius: 8px;
  margin-bottom: 16px;
}

@media (prefers-color-scheme: dark) {
  .container {
    color: #f0f0f0;
  }
  .card {
    background: #1e1e1e;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  }
  .card h2 {
    color: #f0f0f0;
  }
  .label,
  .form-row label {
    color: #bbb;
  }
  .input {
    background: #2a2a2a;
    border-color: #444;
    color: #f0f0f0;
  }
  .result {
    background: #2a2a2a;
  }
  .badge {
    background: #333;
    color: #eee;
  }
  .log-item {
    background: #2a2a2a;
  }
  .log-switched {
    background: #1a3c3c;
  }
  .log-preserved {
    background: #3c2a1a;
  }
  .log-text {
    color: #eee;
  }
}
</style>
