import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type HistoryRecord = {
  id: string;
  timestamp_ms: number;
  source_preview: string;
  translated_text: string;
  is_html: boolean;
  engine: string;
};

type EngineChoice = "auto" | "openai" | "mock";

type SettingsView = {
  hotkey: string;
  engine_choice: EngineChoice;
  model_override: string | null;
  has_api_key: boolean;
  effective_engine_name: string;
};

const tabHistory = document.getElementById("tab-history") as HTMLButtonElement;
const tabSettings = document.getElementById("tab-settings") as HTMLButtonElement;
const panelHistory = document.getElementById("panel-history") as HTMLElement;
const panelSettings = document.getElementById("panel-settings") as HTMLElement;

function activate(tab: "history" | "settings") {
  const isHistory = tab === "history";

  tabHistory.classList.toggle("tab--active", isHistory);
  tabHistory.setAttribute("aria-selected", String(isHistory));
  panelHistory.hidden = !isHistory;

  tabSettings.classList.toggle("tab--active", !isHistory);
  tabSettings.setAttribute("aria-selected", String(!isHistory));
  panelSettings.hidden = isHistory;
}

tabHistory.addEventListener("click", () => activate("history"));
tabSettings.addEventListener("click", () => activate("settings"));

// --- history ---

const historyList = document.getElementById("history-list") as HTMLUListElement;
const historyEmpty = document.getElementById("history-empty") as HTMLElement;
const historyCount = document.getElementById("history-count") as HTMLElement;
const clearHistoryButton = document.getElementById("clear-history-button") as HTMLButtonElement;

function formatTimestamp(ms: number): string {
  return new Date(ms).toLocaleString("ja-JP", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function stripTagsPreview(html: string): string {
  return html.replace(/<[^>]+>/g, "");
}

function translationPreview(record: HistoryRecord): string {
  const text = record.is_html
    ? stripTagsPreview(record.translated_text)
    : record.translated_text;
  return text.length > 60 ? `${text.slice(0, 60)}…` : text;
}

function renderHistory(records: HistoryRecord[]) {
  historyList.innerHTML = "";
  historyCount.textContent = records.length > 0 ? `${records.length}件` : "";
  historyEmpty.hidden = records.length > 0;

  for (const record of records) {
    const item = document.createElement("li");
    item.className = "history-item";
    item.tabIndex = 0;
    item.dataset.id = record.id;

    const meta = document.createElement("div");
    meta.className = "history-item__meta";
    const engine = document.createElement("span");
    engine.textContent = record.engine;
    const time = document.createElement("span");
    time.textContent = formatTimestamp(record.timestamp_ms);
    meta.append(engine, time);

    const source = document.createElement("div");
    source.className = "history-item__source";
    source.textContent = record.source_preview;

    const translated = document.createElement("div");
    translated.className = "history-item__translated";
    translated.textContent = translationPreview(record);

    item.append(meta, source, translated);
    item.addEventListener("click", () => {
      void invoke("open_history_popup", { id: record.id });
    });

    historyList.appendChild(item);
  }
}

async function loadHistory() {
  const records = await invoke<HistoryRecord[]>("list_history");
  renderHistory(records);
}

clearHistoryButton.addEventListener("click", () => {
  if (!window.confirm("翻訳履歴をすべて削除しますか？この操作は取り消せません。")) {
    return;
  }
  void invoke("clear_history").then(() => loadHistory());
});

getCurrentWindow().listen("history-appended", () => {
  void loadHistory();
});

// --- settings ---

const hotkeyDisplay = document.getElementById("hotkey-display") as HTMLElement;
const hotkeyChangeButton = document.getElementById("hotkey-change-button") as HTMLButtonElement;
const hotkeyHint = document.getElementById("hotkey-hint") as HTMLElement;
const engineSelect = document.getElementById("engine-select") as HTMLSelectElement;
const apiKeyInput = document.getElementById("api-key-input") as HTMLInputElement;
const apiKeySaveButton = document.getElementById("api-key-save-button") as HTMLButtonElement;
const apiKeyStatus = document.getElementById("api-key-status") as HTMLElement;
const modelInput = document.getElementById("model-input") as HTMLInputElement;
const modelSaveButton = document.getElementById("model-save-button") as HTMLButtonElement;

function acceleratorToLabel(accelerator: string): string {
  return accelerator
    .split("+")
    .map((token) => token.trim())
    .map((token) => (token.startsWith("Key") ? token.slice(3) : token))
    .join(" + ");
}

async function loadSettings() {
  const settings = await invoke<SettingsView>("get_settings");
  hotkeyDisplay.textContent = acceleratorToLabel(settings.hotkey);
  engineSelect.value = settings.engine_choice;
  modelInput.value = settings.model_override ?? "";
  apiKeyStatus.textContent = settings.has_api_key ? "登録済み: ●●●●●●●●" : "未登録";
}

const MODIFIER_CODES = new Set([
  "ShiftLeft",
  "ShiftRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

function keyEventToAccelerator(event: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(event.code)) {
    return null;
  }
  const modifiers = [
    event.shiftKey && "Shift",
    event.ctrlKey && "Control",
    event.altKey && "Alt",
    event.metaKey && "Super",
  ].filter((value): value is string => Boolean(value));
  return [...modifiers, event.code].join("+");
}

let recordingHotkey = false;

function onHotkeyKeydown(event: KeyboardEvent) {
  event.preventDefault();
  event.stopPropagation();

  if (event.key === "Escape") {
    stopRecordingHotkey();
    return;
  }

  const accelerator = keyEventToAccelerator(event);
  if (!accelerator) {
    return;
  }

  stopRecordingHotkey();
  invoke("set_hotkey", { accelerator })
    .then(() => loadSettings())
    .catch((error: unknown) => {
      hotkeyHint.hidden = false;
      hotkeyHint.textContent = `変更に失敗しました: ${String(error)}`;
    });
}

function stopRecordingHotkey() {
  recordingHotkey = false;
  hotkeyHint.hidden = true;
  document.removeEventListener("keydown", onHotkeyKeydown, true);
}

hotkeyChangeButton.addEventListener("click", () => {
  if (recordingHotkey) {
    return;
  }
  recordingHotkey = true;
  hotkeyHint.hidden = false;
  hotkeyHint.textContent = "キーを押してください(Escでキャンセル)";
  document.addEventListener("keydown", onHotkeyKeydown, true);
});

engineSelect.addEventListener("change", () => {
  void invoke("set_engine_choice", { choice: engineSelect.value }).then(() => loadSettings());
});

apiKeySaveButton.addEventListener("click", () => {
  const key = apiKeyInput.value;
  if (!key.trim()) {
    return;
  }
  invoke("save_api_key", { key })
    .then(() => {
      apiKeyInput.value = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      apiKeyStatus.textContent = `保存に失敗しました: ${String(error)}`;
    });
});

modelSaveButton.addEventListener("click", () => {
  const model = modelInput.value.trim();
  void invoke("set_model_override", { model: model.length > 0 ? model : null }).then(() =>
    loadSettings(),
  );
});

getCurrentWindow().listen("main-shown", () => {
  void loadHistory();
  void loadSettings();
});

void loadHistory();
void loadSettings();
