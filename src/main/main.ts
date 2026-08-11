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

type EngineChoice = "auto" | "openai" | "gemini" | "mock";

type SettingsView = {
  hotkey: string;
  hotkey_error: string | null;
  engine_choice: EngineChoice;
  model_override: string | null;
  has_openai_key: boolean;
  has_gemini_key: boolean;
  effective_engine_name: string;
  lang_a: string;
  lang_b: string;
};

// メイン窓は普段非表示。翻訳のたびに全件再読みしないよう、非表示中は dirty フラグだけ
// 立てて、表示時・履歴タブ切替時にまとめて読み直す
let historyDirty = false;
let windowVisible = false;

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

tabHistory.addEventListener("click", () => {
  activate("history");
  if (historyDirty) {
    historyDirty = false;
    void loadHistory();
  }
});
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

const clearHistoryModal = document.getElementById("clear-history-modal") as HTMLElement;
const clearHistoryCancelButton = document.getElementById(
  "clear-history-cancel-button",
) as HTMLButtonElement;
const clearHistoryConfirmButton = document.getElementById(
  "clear-history-confirm-button",
) as HTMLButtonElement;

function openClearHistoryModal() {
  clearHistoryModal.hidden = false;
}

function closeClearHistoryModal() {
  clearHistoryModal.hidden = true;
}

clearHistoryButton.addEventListener("click", () => openClearHistoryModal());
clearHistoryCancelButton.addEventListener("click", () => closeClearHistoryModal());

clearHistoryConfirmButton.addEventListener("click", () => {
  void invoke("clear_history").then(() => {
    closeClearHistoryModal();
    return loadHistory();
  });
});

clearHistoryModal.addEventListener("click", (event) => {
  if (event.target === clearHistoryModal) {
    closeClearHistoryModal();
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !clearHistoryModal.hidden) {
    event.preventDefault();
    event.stopPropagation();
    closeClearHistoryModal();
  }
});

getCurrentWindow().listen("history-appended", () => {
  // 窓が見えていて履歴タブ表示中なら即反映、それ以外は dirty だけ立てる
  // (既定タブが履歴なので、タブ状態だけで判定すると非表示の窓でも毎回全件再読みが走る)
  if (windowVisible && !panelHistory.hidden) {
    void loadHistory();
    return;
  }
  historyDirty = true;
});

// --- settings ---

const hotkeyDisplay = document.getElementById("hotkey-display") as HTMLElement;
const hotkeyChangeButton = document.getElementById("hotkey-change-button") as HTMLButtonElement;
const hotkeyHint = document.getElementById("hotkey-hint") as HTMLElement;
const langASelect = document.getElementById("lang-a-select") as HTMLSelectElement;
const langBSelect = document.getElementById("lang-b-select") as HTMLSelectElement;
const engineSelect = document.getElementById("engine-select") as HTMLSelectElement;
const openaiApiKeyInput = document.getElementById("openai-api-key-input") as HTMLInputElement;
const openaiApiKeySaveButton = document.getElementById(
  "openai-api-key-save-button",
) as HTMLButtonElement;
const openaiApiKeyStatus = document.getElementById("openai-api-key-status") as HTMLElement;
const geminiApiKeyInput = document.getElementById("gemini-api-key-input") as HTMLInputElement;
const geminiApiKeySaveButton = document.getElementById(
  "gemini-api-key-save-button",
) as HTMLButtonElement;
const geminiApiKeyStatus = document.getElementById("gemini-api-key-status") as HTMLElement;
const modelInput = document.getElementById("model-input") as HTMLInputElement;
const modelSaveButton = document.getElementById("model-save-button") as HTMLButtonElement;
const modelStatus = document.getElementById("model-status") as HTMLElement;
const langStatus = document.getElementById("lang-status") as HTMLElement;
const engineStatus = document.getElementById("engine-status") as HTMLElement;

// loadSettings が engineStatus を毎回書き直すため、エラーは変数に持って
// 描画時に優先表示する(catch で直接書くと数 ms で上書きされて消える)
let engineError = "";

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
  langASelect.value = settings.lang_a;
  langBSelect.value = settings.lang_b;
  engineSelect.value = settings.engine_choice;
  modelInput.value = settings.model_override ?? "";
  openaiApiKeyStatus.textContent = settings.has_openai_key ? "登録済み: ●●●●●●●●" : "未登録";
  geminiApiKeyStatus.textContent = settings.has_gemini_key ? "登録済み: ●●●●●●●●" : "未登録";
  engineStatus.textContent = engineError || `実効エンジン: ${settings.effective_engine_name}`;
  if (settings.hotkey_error) {
    hotkeyHint.hidden = false;
    hotkeyHint.textContent = `ホットキー登録に失敗しています: ${settings.hotkey_error}`;
  }
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

function saveLangPair() {
  invoke("set_lang_pair", {
    langA: langASelect.value,
    langB: langBSelect.value,
  })
    .then(() => {
      langStatus.textContent = "";
      engineError = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      langStatus.textContent = String(error);
      // 保存されなかったので表示を実際の設定値へ戻す
      void loadSettings();
    });
}

langASelect.addEventListener("change", saveLangPair);
langBSelect.addEventListener("change", saveLangPair);

engineSelect.addEventListener("change", () => {
  invoke("set_engine_choice", { choice: engineSelect.value })
    .then(() => {
      engineError = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      engineError = String(error);
      void loadSettings();
    });
});

openaiApiKeySaveButton.addEventListener("click", () => {
  const key = openaiApiKeyInput.value;
  if (!key.trim()) {
    return;
  }
  invoke("save_api_key", { provider: "openai", key })
    .then(() => {
      openaiApiKeyInput.value = "";
      engineError = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      openaiApiKeyStatus.textContent = `保存に失敗しました: ${String(error)}`;
    });
});

geminiApiKeySaveButton.addEventListener("click", () => {
  const key = geminiApiKeyInput.value;
  if (!key.trim()) {
    return;
  }
  invoke("save_api_key", { provider: "gemini", key })
    .then(() => {
      geminiApiKeyInput.value = "";
      engineError = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      geminiApiKeyStatus.textContent = `保存に失敗しました: ${String(error)}`;
    });
});

modelSaveButton.addEventListener("click", () => {
  const model = modelInput.value.trim();
  invoke("set_model_override", { model: model.length > 0 ? model : null })
    .then(() => {
      modelStatus.textContent = "";
      engineError = "";
      return loadSettings();
    })
    .catch((error: unknown) => {
      modelStatus.textContent = String(error);
    });
});

getCurrentWindow().listen("main-shown", () => {
  windowVisible = true;
  historyDirty = false;
  void loadHistory();
  void loadSettings();
});

getCurrentWindow().listen("main-hidden", () => {
  windowVisible = false;
});

void loadHistory();
void loadSettings();
