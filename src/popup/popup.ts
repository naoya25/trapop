import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { marked } from "marked";
import { safeStreamPreview } from "./preview";

type TranslationChunk = {
  request_id: number;
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

type LangPairView = {
  lang_a: string;
  lang_b: string;
};

type SourceKind = "pasted" | "typed";

type PendingAttempt = {
  input: string;
  html: string | null;
  kind: SourceKind;
};

marked.use({ breaks: true });

const label = getCurrentWindow().label;
const popupMode =
  new URLSearchParams(window.location.search).get("mode") === "replay" ? "replay" : "paste";

const pasteForm = document.getElementById("paste-form") as HTMLElement;
const pasteInput = document.getElementById("paste-input") as HTMLDivElement;
const translateButton = document.getElementById("translate-button") as HTMLButtonElement;
const stateLoading = document.getElementById("state-loading") as HTMLElement;
const stateError = document.getElementById("state-error") as HTMLElement;
const errorMessage = document.getElementById("error-message") as HTMLElement;
const retryButton = document.getElementById("retry-button") as HTMLButtonElement;
const translation = document.getElementById("translation") as HTMLElement;
const sourceText = document.getElementById("source-text") as HTMLPreElement;
const translatedText = document.getElementById("translated-text") as HTMLPreElement;
const translatedHtml = document.getElementById("translated-html") as HTMLDivElement;
const statusSource = document.getElementById("status-source") as HTMLElement;
const statusState = document.getElementById("status-state") as HTMLElement;
const toggleSourceButton = document.getElementById("toggle-source") as HTMLButtonElement;
const copyButton = document.getElementById("copy-translation") as HTMLButtonElement;
const headerMode = document.querySelector(".header__mode") as HTMLElement;

const SOURCE_LABEL: Record<SourceKind, string> = {
  pasted: "貼り付け",
  typed: "入力",
};

let showingSource = false;
let isHtmlMode = false;
let outputBuffer = "";
let pastedHtml: string | null = null;
let lastAttempt: PendingAttempt | null = null;
let isTranslating = false;
let currentRequestId = 0;
let nextRequestId = 1;

function setTranslating(active: boolean) {
  isTranslating = active;
  translateButton.textContent = active ? "停止" : "翻訳";
}

async function renderMarkdown(source: string): Promise<string> {
  const html = await marked.parse(source);
  return invoke<string>("sanitize_html", { html });
}

async function renderModeLabel() {
  const [engine, langPair] = await Promise.all([
    invoke<string>("engine_name"),
    invoke<LangPairView>("get_lang_pair"),
  ]);
  headerMode.textContent = `${engine} · ${langPair.lang_a}⇔${langPair.lang_b}`;
}

function showState(state: "loading" | "error" | "translation" | "idle") {
  stateLoading.hidden = state !== "loading";
  stateError.hidden = state !== "error";
  translation.hidden = state !== "translation";
}

async function startTranslation(input: string, html: string | null, kind: SourceKind) {
  const requestId = nextRequestId++;
  currentRequestId = requestId;

  lastAttempt = { input, html, kind };
  outputBuffer = "";
  isHtmlMode = Boolean(html && html.trim().length > 0);
  sourceText.textContent = input;
  statusSource.textContent = SOURCE_LABEL[kind];
  translatedText.textContent = "";
  translatedText.hidden = false;
  translatedHtml.hidden = true;
  translatedHtml.innerHTML = "";
  statusState.textContent = "▍生成中";
  toggleSourceButton.disabled = true;
  copyButton.disabled = true;
  setTranslating(true);

  showState("translation");

  try {
    await invoke("start_translation", { label, input, html, requestId });
  } catch (error) {
    if (currentRequestId !== requestId) {
      return;
    }
    setTranslating(false);
    errorMessage.textContent = error instanceof Error ? error.message : String(error);
    showState("error");
  }
}

function currentInputText(): string {
  return pasteInput.innerText.trim();
}

async function handlePaste(event: ClipboardEvent) {
  event.preventDefault();
  const clipboardData = event.clipboardData;
  if (!clipboardData) {
    return;
  }

  const html = clipboardData.getData("text/html");
  const plain = clipboardData.getData("text/plain");

  let sanitizedHtml: string | null = null;
  if (html.trim().length > 0) {
    sanitizedHtml = await invoke<string>("sanitize_html", { html });
    pasteInput.innerHTML = sanitizedHtml;
  } else {
    pasteInput.textContent = plain;
  }
  pastedHtml = sanitizedHtml;

  const plainForTranslation = plain.trim().length > 0 ? plain : pasteInput.innerText;
  if (plainForTranslation.trim().length === 0) {
    return;
  }

  await startTranslation(plainForTranslation, sanitizedHtml, "pasted");
}

function triggerManualTranslate() {
  if (isTranslating) {
    return;
  }
  const text = currentInputText();
  if (text.length === 0) {
    return;
  }
  void startTranslation(text, pastedHtml, pastedHtml ? "pasted" : "typed");
}

pasteInput.addEventListener("paste", (event) => {
  void handlePaste(event);
});

pasteInput.addEventListener("input", () => {
  pastedHtml = null;
});

pasteInput.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
    event.preventDefault();
    triggerManualTranslate();
  }
});

translateButton.addEventListener("click", () => {
  if (isTranslating) {
    void invoke("cancel_translation", { label });
    setTranslating(false);
    statusState.textContent = "■ 停止しました";
    return;
  }
  triggerManualTranslate();
});

async function focusPasteInput() {
  await getCurrentWindow().setFocus();
  pasteInput.focus();
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
  outputBuffer = record.translated_text;
  translatedText.textContent = "";
  translatedHtml.innerHTML = "";

  const rendered = isHtmlMode
    ? await invoke<string>("sanitize_html", { html: record.translated_text })
    : await renderMarkdown(record.translated_text);
  translatedHtml.innerHTML = rendered;
  translatedText.hidden = true;
  translatedHtml.hidden = false;

  statusState.textContent = "✓ 完了";
  toggleSourceButton.disabled = false;
  copyButton.disabled = false;
  showState("translation");
}

function listenForTranslationChunks() {
  getCurrentWindow().listen<TranslationChunk>("translate-chunk", (event) => {
    const chunk = event.payload;
    if (chunk.request_id !== currentRequestId) {
      return;
    }
    if (chunk.done) {
      void finishTranslation(chunk.request_id);
      return;
    }
    outputBuffer += chunk.text;
    translatedText.textContent = safeStreamPreview(outputBuffer);
  });
}

async function finishTranslation(requestId: number) {
  const rendered = isHtmlMode
    ? await invoke<string>("sanitize_html", { html: outputBuffer })
    : await renderMarkdown(outputBuffer);

  if (requestId !== currentRequestId) {
    return;
  }

  translatedText.hidden = true;
  translatedText.textContent = "";
  translatedHtml.innerHTML = rendered;
  translatedHtml.hidden = false;
  statusState.textContent = "✓ 完了";
  toggleSourceButton.disabled = false;
  copyButton.disabled = false;
  setTranslating(false);
}

retryButton.addEventListener("click", () => {
  if (popupMode === "replay") {
    runReplay().catch((error: unknown) => {
      errorMessage.textContent = error instanceof Error ? error.message : String(error);
      showState("error");
    });
    return;
  }
  if (lastAttempt) {
    void startTranslation(lastAttempt.input, lastAttempt.html, lastAttempt.kind);
  }
});

toggleSourceButton.addEventListener("click", () => {
  showingSource = !showingSource;
  sourceText.hidden = !showingSource;
  toggleSourceButton.textContent = showingSource ? "訳文を表示" : "原文を表示";
});

async function copyRichTranslation() {
  const htmlFlavor = translatedHtml.innerHTML;
  const plainFlavor = isHtmlMode ? (translatedHtml.textContent ?? "") : outputBuffer;

  try {
    const item = new ClipboardItem({
      "text/html": new Blob([htmlFlavor], { type: "text/html" }),
      "text/plain": new Blob([plainFlavor], { type: "text/plain" }),
    });
    await navigator.clipboard.write([item]);
  } catch {
    await navigator.clipboard.writeText(plainFlavor);
  }
}

copyButton.addEventListener("click", () => {
  void copyRichTranslation();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    void getCurrentWindow().close();
  }
});

const POPUP_SIZE_SAVE_DEBOUNCE_MS = 500;
let popupSizeSaveTimer: number | undefined;

function schedulePopupSizeSave() {
  if (popupSizeSaveTimer !== undefined) {
    window.clearTimeout(popupSizeSaveTimer);
  }
  popupSizeSaveTimer = window.setTimeout(() => {
    void invoke("save_popup_size", {
      width: window.innerWidth,
      height: window.innerHeight,
    });
  }, POPUP_SIZE_SAVE_DEBOUNCE_MS);
}

window.addEventListener("resize", schedulePopupSizeSave);

if (popupMode === "replay") {
  pasteForm.hidden = true;
  runReplay().catch((error: unknown) => {
    errorMessage.textContent = error instanceof Error ? error.message : String(error);
    showState("error");
  });
} else {
  listenForTranslationChunks();
  void renderModeLabel();
  void focusPasteInput();
}
