import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type CaptureFlavor = {
  flavor: string;
  content: string;
};

type CaptureSource = "selection" | "clipboard";

type CaptureResult = {
  source: CaptureSource;
  plain_text: string | null;
  html: string | null;
  flavors: CaptureFlavor[];
};

type CaptureOutcome =
  | { status: "ok"; result: CaptureResult }
  | { status: "error"; message: string; is_accessibility_error: boolean };

type TranslationChunk = {
  text: string;
  done: boolean;
};

type HistoryRecord = {
  id: string;
  timestamp_ms: number;
  source_preview: string;
  translated_text: string;
  is_html: boolean;
  engine: string;
};

const label = getCurrentWindow().label;
const popupMode = new URLSearchParams(window.location.search).get("mode") === "replay"
  ? "replay"
  : "capture";

const stateLoading = document.getElementById("state-loading") as HTMLElement;
const stateError = document.getElementById("state-error") as HTMLElement;
const errorMessage = document.getElementById("error-message") as HTMLElement;
const retryButton = document.getElementById("retry-button") as HTMLButtonElement;
const openSettingsButton = document.getElementById("open-settings-button") as HTMLButtonElement;
const translation = document.getElementById("translation") as HTMLElement;
const sourceText = document.getElementById("source-text") as HTMLPreElement;
const translatedText = document.getElementById("translated-text") as HTMLPreElement;
const translatedHtml = document.getElementById("translated-html") as HTMLDivElement;
const statusSource = document.getElementById("status-source") as HTMLElement;
const statusState = document.getElementById("status-state") as HTMLElement;
const toggleSourceButton = document.getElementById("toggle-source") as HTMLButtonElement;
const copyButton = document.getElementById("copy-translation") as HTMLButtonElement;
const headerMode = document.querySelector(".header__mode") as HTMLElement;

const SOURCE_LABEL: Record<CaptureSource, string> = {
  selection: "選択テキストから翻訳",
  clipboard: "クリップボードから翻訳",
};

let showingSource = false;
let isHtmlMode = false;
let outputBuffer = "";

function stripTagsPreview(html: string): string {
  return html.replace(/<[^>]+>/g, "");
}

async function renderEngineName() {
  const name = await invoke<string>("engine_name");
  headerMode.textContent = `${name} · 和訳`;
}

function showState(state: "loading" | "error" | "translation") {
  stateLoading.hidden = state !== "loading";
  stateError.hidden = state !== "error";
  translation.hidden = state !== "translation";
}

async function waitForCapture(): Promise<CaptureOutcome> {
  const maxAttempts = 40;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    const outcome = await invoke<CaptureOutcome | null>("get_capture", { label });
    if (outcome) {
      return outcome;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  return {
    status: "error",
    message: "選択テキストの取得がタイムアウトしました。",
    is_accessibility_error: false,
  };
}

async function startTranslation(input: string, html: string | null) {
  outputBuffer = "";
  isHtmlMode = Boolean(html && html.trim().length > 0);
  translatedText.textContent = "";
  translatedText.hidden = false;
  translatedHtml.hidden = true;
  translatedHtml.innerHTML = "";
  statusState.textContent = "▍生成中";
  toggleSourceButton.disabled = true;
  copyButton.disabled = true;

  await invoke("start_translation", { label, input, html });
}

async function runCapture() {
  showState("loading");

  const outcome = await waitForCapture();

  if (outcome.status === "error") {
    errorMessage.textContent = outcome.message;
    openSettingsButton.hidden = !outcome.is_accessibility_error;
    showState("error");
    return;
  }

  const { result } = outcome;
  const input = result.plain_text ?? "";
  sourceText.textContent = input;
  statusSource.textContent = SOURCE_LABEL[result.source];

  showState("translation");
  await startTranslation(input, result.html);
}

async function waitForReplay(): Promise<HistoryRecord> {
  const maxAttempts = 40;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    const record = await invoke<HistoryRecord | null>("get_history_replay", { label });
    if (record) {
      return record;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error("履歴の再表示がタイムアウトしました。");
}

async function runReplay() {
  showState("loading");

  const record = await waitForReplay();

  headerMode.textContent = `${record.engine} · 履歴`;
  sourceText.textContent = record.source_preview;
  statusSource.textContent = "履歴から再表示";

  isHtmlMode = record.is_html;
  translatedText.textContent = "";
  translatedHtml.innerHTML = "";

  if (isHtmlMode) {
    const sanitized = await invoke<string>("sanitize_html", { html: record.translated_text });
    translatedHtml.innerHTML = sanitized;
    translatedText.hidden = true;
    translatedHtml.hidden = false;
  } else {
    translatedText.textContent = record.translated_text;
    translatedText.hidden = false;
    translatedHtml.hidden = true;
  }

  statusState.textContent = "✓ 完了";
  toggleSourceButton.disabled = false;
  copyButton.disabled = false;
  showState("translation");
}

function listenForTranslationChunks() {
  getCurrentWindow().listen<TranslationChunk>("translate-chunk", (event) => {
    const chunk = event.payload;
    if (chunk.done) {
      void finishTranslation();
      return;
    }
    outputBuffer += chunk.text;
    translatedText.textContent = isHtmlMode ? stripTagsPreview(outputBuffer) : outputBuffer;
  });
}

async function finishTranslation() {
  if (isHtmlMode) {
    const sanitized = await invoke<string>("sanitize_html", { html: outputBuffer });
    translatedHtml.innerHTML = sanitized;
    translatedText.hidden = true;
    translatedHtml.hidden = false;
  }
  statusState.textContent = "✓ 完了";
  toggleSourceButton.disabled = false;
  copyButton.disabled = false;
}

retryButton.addEventListener("click", () => {
  if (popupMode === "replay") {
    runReplay().catch((error: unknown) => {
      errorMessage.textContent = error instanceof Error ? error.message : String(error);
      showState("error");
    });
    return;
  }
  void runCapture();
});

openSettingsButton.addEventListener("click", () => {
  void invoke("open_accessibility_settings");
});

toggleSourceButton.addEventListener("click", () => {
  showingSource = !showingSource;
  sourceText.hidden = !showingSource;
  toggleSourceButton.textContent = showingSource ? "訳文を表示" : "原文を表示";
});

copyButton.addEventListener("click", () => {
  const text = isHtmlMode ? translatedHtml.textContent : translatedText.textContent;
  void navigator.clipboard.writeText(text ?? "");
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void getCurrentWindow().close();
  }
});

if (popupMode === "replay") {
  runReplay().catch((error: unknown) => {
    errorMessage.textContent = error instanceof Error ? error.message : String(error);
    openSettingsButton.hidden = true;
    showState("error");
  });
} else {
  listenForTranslationChunks();
  void renderEngineName();
  void runCapture();
}
