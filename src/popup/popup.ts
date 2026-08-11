import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type CaptureFlavor = {
  flavor: string;
  content: string;
};

type CaptureResult = {
  used_fallback_clipboard: boolean;
  plain_text: string | null;
  html: string | null;
  flavors: CaptureFlavor[];
};

type CaptureOutcome =
  | { status: "ok"; result: CaptureResult }
  | { status: "error"; message: string };

type TranslationChunk = {
  text: string;
  done: boolean;
};

const label = getCurrentWindow().label;

const stateLoading = document.getElementById("state-loading") as HTMLElement;
const stateEmpty = document.getElementById("state-empty") as HTMLElement;
const stateError = document.getElementById("state-error") as HTMLElement;
const errorMessage = document.getElementById("error-message") as HTMLElement;
const retryButton = document.getElementById("retry-button") as HTMLButtonElement;
const translation = document.getElementById("translation") as HTMLElement;
const sourceText = document.getElementById("source-text") as HTMLPreElement;
const translatedText = document.getElementById("translated-text") as HTMLPreElement;
const statusState = document.getElementById("status-state") as HTMLElement;
const toggleSourceButton = document.getElementById("toggle-source") as HTMLButtonElement;
const copyButton = document.getElementById("copy-translation") as HTMLButtonElement;
const headerMode = document.querySelector(".header__mode") as HTMLElement;

let showingSource = false;

async function renderEngineName() {
  const name = await invoke<string>("engine_name");
  headerMode.textContent = `${name} · 和訳`;
}

function showState(state: "loading" | "empty" | "error" | "translation") {
  stateLoading.hidden = state !== "loading";
  stateError.hidden = state !== "error";
  translation.hidden = state !== "translation";
  if (state !== "translation") {
    stateEmpty.hidden = state !== "empty";
  }
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
  return { status: "error", message: "選択テキストの取得がタイムアウトしました。" };
}

async function startTranslation(input: string) {
  translatedText.textContent = "";
  statusState.textContent = "▍生成中";
  toggleSourceButton.disabled = true;
  copyButton.disabled = true;

  await invoke("start_translation", { label, input });
}

async function runCapture() {
  showState("loading");

  const outcome = await waitForCapture();

  if (outcome.status === "error") {
    errorMessage.textContent = outcome.message;
    showState("error");
    return;
  }

  const { result } = outcome;
  const input = result.plain_text ?? "";
  sourceText.textContent = input;

  if (!input) {
    showState("empty");
    stateEmpty.textContent =
      "選択テキストもクリップボードも空だったため、翻訳できませんでした。";
    return;
  }

  showState("translation");
  stateEmpty.hidden = !result.used_fallback_clipboard;
  await startTranslation(input);
}

function listenForTranslationChunks() {
  getCurrentWindow().listen<TranslationChunk>("translate-chunk", (event) => {
    const chunk = event.payload;
    if (chunk.done) {
      statusState.textContent = "✓ 完了";
      toggleSourceButton.disabled = false;
      copyButton.disabled = false;
      return;
    }
    translatedText.textContent += chunk.text;
  });
}

retryButton.addEventListener("click", () => {
  void runCapture();
});

toggleSourceButton.addEventListener("click", () => {
  showingSource = !showingSource;
  sourceText.hidden = !showingSource;
  toggleSourceButton.textContent = showingSource ? "訳文を表示" : "原文を表示";
});

copyButton.addEventListener("click", () => {
  void navigator.clipboard.writeText(translatedText.textContent ?? "");
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void getCurrentWindow().close();
  }
});

listenForTranslationChunks();
void renderEngineName();
void runCapture();
