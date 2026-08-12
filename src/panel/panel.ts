import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { marked } from "marked";
import { htmlLooksRich, safeStreamPreview } from "../main/preview";

type EngineChoice = "auto" | "openai" | "gemini" | "mock";
type TranslationTarget = "auto" | "lang_a" | "lang_b";

type SettingsView = {
  engine_choice: EngineChoice;
  model_override: string | null;
  custom_prompt: string | null;
  default_prompt: string;
  has_openai_key: boolean;
  has_gemini_key: boolean;
  effective_engine_name: string;
  lang_a: string;
  lang_b: string;
  translation_target: TranslationTarget;
};

type SourceKind = "pasted" | "typed";

type PendingAttempt = {
  input: string;
  html: string | null;
  kind: SourceKind;
};

marked.use({ breaks: true });

const pasteInput = document.getElementById("paste-input") as HTMLDivElement;
const translateButton = document.getElementById("translate-button") as HTMLButtonElement;
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
const viewMeta = document.getElementById("view-meta") as HTMLElement;
const translationTargetSelect = document.getElementById(
  "translation-target-select",
) as HTMLSelectElement;

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
let requestStartedAt = 0;
let firstTokenMs: number | null = null;
// 停止・エラーごとに増える世代。-1 センチネルだけだと
// 「停止→再翻訳→停止」で古い pending レンダリングを区別できない
let partialGeneration = 0;
let currentSettings: SettingsView | null = null;

function setTranslating(active: boolean) {
  isTranslating = active;
  translateButton.textContent = active ? "停止" : "翻訳";
}

async function renderMarkdown(source: string): Promise<string> {
  const html = await marked.parse(source);
  return invoke<string>("sanitize_html", { html });
}

function renderViewMeta() {
  if (!currentSettings) {
    viewMeta.textContent = "";
    return;
  }
  viewMeta.textContent = `${currentSettings.effective_engine_name} · ${currentSettings.lang_a}⇔${currentSettings.lang_b}`;
}

function showState(state: "error" | "translation" | "idle") {
  stateError.hidden = state !== "error";
  translation.hidden = state !== "translation";
}

function cancelInFlightTranslation() {
  // 停止直後に pending の renderPartial(部分訳レンダリング)が残っていることが
  // あるため、翻訳中でなくても世代を進めて遅延描画を無効化する
  partialGeneration++;
  if (!isTranslating) {
    return;
  }
  const stoppedRequestId = currentRequestId;
  currentRequestId = -1;
  void invoke("cancel_translation", { requestId: stoppedRequestId });
  setTranslating(false);
}

function resetToNewTranslation() {
  cancelInFlightTranslation();
  renderViewMeta();
  lastAttempt = null;
  isHtmlMode = false;
  outputBuffer = "";
  pastedHtml = null;
  showingSource = false;
  pasteInput.textContent = "";
  sourceText.hidden = true;
  sourceText.textContent = "";
  toggleSourceButton.textContent = "原文を表示";
  toggleSourceButton.disabled = true;
  copyButton.disabled = true;
  translatedText.textContent = "";
  translatedText.hidden = false;
  translatedHtml.innerHTML = "";
  translatedHtml.hidden = true;
  statusSource.textContent = "";
  statusState.textContent = "";
  showState("idle");
  pasteInput.focus();
}

async function startTranslation(input: string, html: string | null, kind: SourceKind) {
  const requestId = nextRequestId++;
  currentRequestId = requestId;

  lastAttempt = { input, html, kind };
  outputBuffer = "";
  requestStartedAt = performance.now();
  firstTokenMs = null;
  isHtmlMode = Boolean(html && html.trim().length > 0);
  showingSource = false;
  sourceText.hidden = true;
  toggleSourceButton.textContent = "原文を表示";
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
    await invoke("start_translation", { input, html, requestId });
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

  const rawHtml = clipboardData.getData("text/html");
  const plain = clipboardData.getData("text/plain");
  // エディタ/ターミナル由来の「平文を div/span で包んだだけ」の text/html は
  // HTML 扱いしない(モデルに span スープが渡って echo される・markdown が
  // 描画されない)。意味のあるタグを含むリッチ HTML だけを HTML モードに乗せる
  const html = htmlLooksRich(rawHtml) ? rawHtml : "";

  let sanitizedHtml: string | null = null;
  if (html.trim().length > 0) {
    try {
      sanitizedHtml = await invoke<string>("sanitize_html", { html });
      pasteInput.innerHTML = sanitizedHtml;
    } catch {
      // sanitize に失敗しても貼り付けを無かったことにしない(plain で続行)
      sanitizedHtml = null;
      pasteInput.textContent = plain;
    }
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

// 部分訳もコピー・原文トグルできるよう、完了時と同じレンダリングに落とす。
// await 後に世代が変わっていたら(新しい翻訳/停止が始まっていたら)何も触らない。
async function renderPartial(statusText: string) {
  const generation = ++partialGeneration;
  const buffer = outputBuffer;
  const wasHtml = isHtmlMode;
  try {
    const rendered = wasHtml
      ? await invoke<string>("sanitize_html", { html: buffer })
      : await renderMarkdown(buffer);
    if (currentRequestId !== -1 || generation !== partialGeneration) {
      return;
    }
    translatedText.hidden = true;
    translatedText.textContent = "";
    translatedHtml.innerHTML = rendered;
    translatedHtml.hidden = false;
    toggleSourceButton.disabled = false;
    copyButton.disabled = false;
    statusState.textContent = statusText;
  } catch {
    if (currentRequestId === -1 && generation === partialGeneration) {
      statusState.textContent = statusText;
    }
  }
}

translateButton.addEventListener("click", () => {
  if (isTranslating) {
    cancelInFlightTranslation();
    if (outputBuffer.trim().length > 0) {
      void renderPartial("■ 停止(部分訳)");
    } else {
      statusState.textContent = "■ 停止しました";
    }
    return;
  }
  triggerManualTranslate();
});

function listenForTranslationChunks() {
  void getCurrentWindow().listen<{
    request_id: number;
    text: string;
    done: boolean;
    error: boolean;
  }>("translate-chunk", (event) => {
    const chunk = event.payload;
    if (chunk.request_id !== currentRequestId) {
      return;
    }
    if (chunk.error) {
      setTranslating(false);
      // 途中までの訳があるなら消さずに残す(9割できた訳をエラーで失わない)
      if (outputBuffer.trim().length > 0) {
        currentRequestId = -1;
        void renderPartial(`⚠ ${chunk.text || "エラー"}(部分訳)`);
        return;
      }
      errorMessage.textContent = chunk.text || "翻訳中にエラーが発生しました。";
      showState("error");
      return;
    }
    if (chunk.done) {
      void finishTranslation(chunk.request_id);
      return;
    }
    if (firstTokenMs === null) {
      // 目標: first token 1秒以内。実測値を status バーに常時出して検証可能にする
      firstTokenMs = Math.round(performance.now() - requestStartedAt);
    }
    outputBuffer += chunk.text;
    if (isHtmlMode) {
      // HTML はチャンク境界でタグが割れうるため全バッファから安全な範囲を作り直す
      translatedText.textContent = safeStreamPreview(outputBuffer, true);
    } else {
      // 平文はチャンク追記で足りる(全文再セットだと訳文長 n に対して O(n^2))
      translatedText.append(chunk.text);
    }
    followStreamingScroll();
  }).catch((error: unknown) => {
    console.error("translate-chunk の購読に失敗", error);
  });
}

// 生成中は末尾へ追従する。ユーザーが上へスクロールして読み始めたら追従をやめる。
// scrollHeight 読みは同期レイアウトを起こすため rAF で1フレーム1回に間引く
let scrollFollowScheduled = false;

function followStreamingScroll() {
  if (scrollFollowScheduled) {
    return;
  }
  scrollFollowScheduled = true;
  requestAnimationFrame(() => {
    scrollFollowScheduled = false;
    const nearBottom =
      translation.scrollHeight - translation.scrollTop - translation.clientHeight < 48;
    if (nearBottom) {
      translation.scrollTop = translation.scrollHeight;
    }
  });
}

async function finishTranslation(requestId: number) {
  let rendered: string;
  try {
    rendered = isHtmlMode
      ? await invoke<string>("sanitize_html", { html: outputBuffer })
      : await renderMarkdown(outputBuffer);
  } catch (error) {
    // レンダリング失敗を無言の「生成中」のまま放置しない
    if (requestId !== currentRequestId) {
      return;
    }
    setTranslating(false);
    errorMessage.textContent = error instanceof Error ? error.message : String(error);
    showState("error");
    return;
  }

  if (requestId !== currentRequestId) {
    return;
  }

  translatedText.hidden = true;
  translatedText.textContent = "";
  translatedHtml.innerHTML = rendered;
  translatedHtml.hidden = false;
  statusState.textContent =
    firstTokenMs === null ? "✓ 完了" : `✓ 完了 · 初速${firstTokenMs}ms`;
  toggleSourceButton.disabled = false;
  copyButton.disabled = false;
  setTranslating(false);
}

retryButton.addEventListener("click", () => {
  if (lastAttempt) {
    void startTranslation(lastAttempt.input, lastAttempt.html, lastAttempt.kind);
  }
});

toggleSourceButton.addEventListener("click", () => {
  showingSource = !showingSource;
  sourceText.hidden = !showingSource;
  translatedHtml.hidden = showingSource;
  toggleSourceButton.textContent = showingSource ? "訳文を表示" : "原文を表示";
});

async function copyRichTranslation() {
  const htmlFlavor = translatedHtml.innerHTML;
  // sanitize が全部落とした等で textContent が空でも、元バッファへフォールバックして
  // 無言の空コピーを避ける
  const plainFlavor = isHtmlMode
    ? translatedHtml.textContent || outputBuffer
    : outputBuffer;

  if (htmlFlavor.trim().length === 0) {
    await navigator.clipboard.writeText(plainFlavor);
    return;
  }

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

// --- 設定(表示専用。エンジン・言語ペアはメイン設定を共有。翻訳先トグルのみ操作可) ---

function updateTranslationTargetOptions(langA: string, langB: string) {
  const options = translationTargetSelect.options;
  for (const option of options) {
    if (option.value === "lang_a") {
      option.textContent = `${langA}に`;
    } else if (option.value === "lang_b") {
      option.textContent = `${langB}に`;
    }
  }
}

async function loadSettings() {
  const settings = await invoke<SettingsView>("get_settings");
  currentSettings = settings;
  renderViewMeta();
  updateTranslationTargetOptions(settings.lang_a, settings.lang_b);
  translationTargetSelect.value = settings.translation_target;
}

translationTargetSelect.addEventListener("change", () => {
  invoke("set_translation_target", { target: translationTargetSelect.value }).then(
    () => loadSettings(),
    (error: unknown) => {
      console.error("翻訳先の保存に失敗", error);
      void loadSettings();
    },
  );
});

// --- リンク・Esc・起動時初期化 ---

// 訳文内のリンクは既定ブラウザで開く(main.ts の翻訳ビューと同じ挙動)
document.addEventListener("click", (event) => {
  const anchor = (event.target as HTMLElement | null)?.closest("a");
  if (!anchor) {
    return;
  }
  event.preventDefault();
  if (anchor.isContentEditable) {
    return;
  }
  const href = anchor.getAttribute("href") ?? "";
  if (/^https?:\/\//i.test(href)) {
    void invoke("plugin:opener|open_url", { url: href });
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    void invoke("close_panel");
  }
});

listenForTranslationChunks();
showState("idle");
resetToNewTranslation();
void loadSettings();
