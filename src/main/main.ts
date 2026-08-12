import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { marked } from "marked";
import { htmlLooksRich, safeStreamPreview } from "./preview";

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
  engine_choice: EngineChoice;
  model_override: string | null;
  custom_prompt: string | null;
  default_prompt: string;
  has_openai_key: boolean;
  has_gemini_key: boolean;
  effective_engine_name: string;
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

// --- 画面切り替え(サイドバー) ---

const newTranslationButton = document.getElementById(
  "new-translation-button",
) as HTMLButtonElement;
const settingsButton = document.getElementById("settings-button") as HTMLButtonElement;
const viewTranslate = document.getElementById("view-translate") as HTMLElement;
const viewSettings = document.getElementById("view-settings") as HTMLElement;

function showView(view: "translate" | "settings") {
  viewTranslate.hidden = view !== "translate";
  viewSettings.hidden = view !== "settings";
}

newTranslationButton.addEventListener("click", () => {
  showView("translate");
  resetToNewTranslation();
  setActiveHistoryItem(null);
});

settingsButton.addEventListener("click", () => {
  showView("settings");
  // 直近の読み込み結果(in-flight 含む)を待ってから分岐する。
  // 失敗していたら読み直し、モデル一覧の取得は読み込み成功後だけ行う
  // (失敗表示と候補入り select の矛盾を防ぐ)
  void settingsReady.then((ok) => {
    if (ok) {
      ensureDynamicModels();
      return;
    }
    // 再読込に入る前に前回失敗の表示を消し、進行中であることを出す
    engineError = "";
    engineStatus.textContent = "設定を読み込み中…";
    void refreshSettings().then((retryOk) => {
      if (retryOk) {
        ensureDynamicModels();
      }
    });
  });
});

// --- 翻訳ビュー ---

const pasteForm = document.getElementById("paste-form") as HTMLElement;
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

const SOURCE_LABEL: Record<SourceKind, string> = {
  pasted: "貼り付け",
  typed: "入力",
};

let showingSource = false;
let isHtmlMode = false;
let outputBuffer = "";
let pastedHtml: string | null = null;
let lastAttempt: PendingAttempt | null = null;
let lastHistoryRecord: HistoryRecord | null = null;
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

// サイドバーの「新規翻訳」・履歴クリックはどちらも翻訳ビューを差し替える。
// 進行中の翻訳があれば打ち切ってから空の入力状態に戻す。
function resetToNewTranslation() {
  cancelInFlightTranslation();
  renderViewMeta();
  lastAttempt = null;
  lastHistoryRecord = null;
  isHtmlMode = false;
  outputBuffer = "";
  pastedHtml = null;
  showingSource = false;
  pasteForm.hidden = false;
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
    return;
  }
  // 履歴表示のレンダリング失敗から来た場合は同じ履歴を描画し直す
  if (lastHistoryRecord) {
    void showHistoryRecord(lastHistoryRecord);
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

const WINDOW_SIZE_SAVE_DEBOUNCE_MS = 500;
let windowSizeSaveTimer: number | undefined;

function scheduleWindowSizeSave() {
  if (windowSizeSaveTimer !== undefined) {
    window.clearTimeout(windowSizeSaveTimer);
  }
  windowSizeSaveTimer = window.setTimeout(() => {
    void invoke("save_window_size", {
      width: window.innerWidth,
      height: window.innerHeight,
    });
  }, WINDOW_SIZE_SAVE_DEBOUNCE_MS);
}

window.addEventListener("resize", scheduleWindowSizeSave);

// --- 履歴(サイドバー一覧・クリックで即表示) ---

const historyList = document.getElementById("history-list") as HTMLUListElement;
const historyEmpty = document.getElementById("history-empty") as HTMLElement;
const historyError = document.getElementById("history-error") as HTMLElement;
const historyCount = document.getElementById("history-count") as HTMLElement;
const clearHistoryButton = document.getElementById("clear-history-button") as HTMLButtonElement;

let activeHistoryId: string | null = null;

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

function setActiveHistoryItem(id: string | null) {
  activeHistoryId = id;
  for (const item of historyList.children) {
    item.classList.toggle("history-item--active", (item as HTMLElement).dataset.id === id);
  }
}

function renderHistory(records: HistoryRecord[]) {
  historyList.innerHTML = "";
  historyCount.textContent = records.length > 0 ? `履歴 ${records.length}件` : "履歴";
  historyEmpty.hidden = records.length > 0;
  // 描画に成功したら過去のエラー表示を消す
  historyError.hidden = true;

  for (const record of records) {
    const item = document.createElement("li");
    item.className = "history-item";
    // role="button" を付けると listitem セマンティクスが消えて「N 項目のリスト」と
    // 読み上げられなくなるため付けない(キーボード操作は keydown で担保)
    item.tabIndex = 0;
    item.dataset.id = record.id;
    item.classList.toggle("history-item--active", record.id === activeHistoryId);

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
    // tabIndex を振る以上、キーボードでも開けるようにする
    item.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        showView("translate");
        void showHistoryRecord(record);
      }
    });
    item.addEventListener("click", () => {
      showView("translate");
      void showHistoryRecord(record);
    });

    historyList.appendChild(item);
  }
}

async function loadHistory() {
  const records = await invoke<HistoryRecord[]>("list_history");
  renderHistory(records);
}

// 履歴クリックは list_history で既に手元にある訳文をそのまま描画する(再翻訳なし)。
async function showHistoryRecord(record: HistoryRecord) {
  cancelInFlightTranslation();
  setActiveHistoryItem(record.id);

  pasteForm.hidden = true;
  showState("translation");
  viewMeta.textContent = `${record.engine} · 履歴`;

  showingSource = false;
  sourceText.hidden = true;
  sourceText.textContent = record.source_preview;
  statusSource.textContent = "履歴から再表示";
  toggleSourceButton.textContent = "原文を表示";

  isHtmlMode = record.is_html;
  outputBuffer = record.translated_text;
  lastAttempt = null;
  lastHistoryRecord = record;
  translatedText.textContent = "";
  translatedHtml.innerHTML = "";

  try {
    const rendered = isHtmlMode
      ? await invoke<string>("sanitize_html", { html: record.translated_text })
      : await renderMarkdown(record.translated_text);
    if (activeHistoryId !== record.id) {
      return;
    }
    translatedHtml.innerHTML = rendered;
    translatedText.hidden = true;
    translatedHtml.hidden = false;
    statusState.textContent = "✓ 完了";
    toggleSourceButton.disabled = false;
    copyButton.disabled = false;
  } catch (error) {
    if (activeHistoryId !== record.id) {
      return;
    }
    errorMessage.textContent = error instanceof Error ? error.message : String(error);
    showState("error");
  }
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
  invoke("clear_history")
    .then(() => {
      closeClearHistoryModal();
      if (activeHistoryId !== null) {
        showView("translate");
        resetToNewTranslation();
        setActiveHistoryItem(null);
      }
      return loadHistory();
    })
    .catch((error: unknown) => {
      closeClearHistoryModal();
      showHistoryError("履歴の更新に失敗しました", error);
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

// 空表示(historyEmpty)とは別枠に出す。履歴が残っている状態の失敗で
// 「履歴はまだありません」枠を誤って開かないため
function showHistoryError(prefix: string, error: unknown) {
  // 「まだありません」と並ぶと矛盾して見えるため空表示は畳む
  historyEmpty.hidden = true;
  historyError.hidden = false;
  historyError.textContent = `${prefix}: ${String(error)}`;
}

void getCurrentWindow()
  .listen("history-appended", () => {
    loadHistory().catch((error: unknown) => {
      showHistoryError("履歴の更新に失敗しました", error);
    });
  })
  .catch((error: unknown) => {
    console.error("history-appended の購読に失敗", error);
  });

// --- 設定ビュー ---

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
const modelSelect = document.getElementById("model-select") as HTMLSelectElement;
const modelStatus = document.getElementById("model-status") as HTMLElement;
const promptInput = document.getElementById("prompt-input") as HTMLTextAreaElement;
const promptSaveButton = document.getElementById("prompt-save-button") as HTMLButtonElement;
const promptResetButton = document.getElementById("prompt-reset-button") as HTMLButtonElement;
const promptStatus = document.getElementById("prompt-status") as HTMLElement;
const langStatus = document.getElementById("lang-status") as HTMLElement;
const engineStatus = document.getElementById("engine-status") as HTMLElement;

// loadSettings が engineStatus を毎回書き直すため、エラーは変数に持って
// 描画時に優先表示する(catch で直接書くと数 ms で上書きされて消える)
let engineError = "";

const MODEL_STATUS_LOAD_FAILED = "設定の読み込みに失敗したため、モデル一覧を表示できません";

function reportSettingsLoadFailure(error: unknown) {
  // 表示エラーの持ち方は engineError 変数の規約に合わせる
  engineError = `設定を読み込めませんでした: ${String(error)}`;
  engineStatus.textContent = engineError;
  modelStatus.textContent = MODEL_STATUS_LOAD_FAILED;
}

// 読み込みの成否を値で返す(reject させない)。失敗表示はここで済ませるので、
// 呼び出し側の then が設定読み込み以外のエラーを誤って拾うことがない
function loadSettingsSafely(): Promise<boolean> {
  return loadSettings().then(
    () => {
      // 読み込み失敗時に自分で書いた文言だけ回収する(モデル保存エラー等は残す)
      if (modelStatus.textContent === MODEL_STATUS_LOAD_FAILED) {
        modelStatus.textContent = "";
      }
      return true;
    },
    (error: unknown) => {
      reportSettingsLoadFailure(error);
      return false;
    },
  );
}

// 直近の設定読み込み(起動時・保存後の再読み込み含む)の結果。
// モジュール初期化が途中で失敗しても TDZ にならないよう安全側(false)で初期化する
let settingsReady: Promise<boolean> = Promise.resolve(false);

// loadSettings の呼び出しはすべてここを通す(reject させない不変条件と
// settingsReady の鮮度を1箇所で守る)
function refreshSettings(): Promise<boolean> {
  settingsReady = loadSettingsSafely();
  return settingsReady;
}

// API から取得できたときはそちらを使い、未取得・失敗時はこの静的リストに
// フォールバックする。既定値は src-tauri/src/engine/{openai,gemini}.rs の
// DEFAULT_MODEL と一致させること
const MODEL_CHOICES: Record<string, string[]> = {
  openai: ["gpt-4.1-mini", "gpt-4.1", "gpt-4.1-nano", "gpt-4o-mini", "gpt-4o", "gpt-5", "gpt-5-mini", "gpt-5-nano"],
  gemini: [
    "gemini-flash-latest",
    "gemini-flash-lite-latest",
    "gemini-3.6-flash",
    "gemini-3.5-flash",
    "gemini-3.5-flash-lite",
  ],
};

function providerLabel(provider: string): string {
  return provider === "openai" ? "OpenAI" : "Gemini";
}

// ListModels API の取得結果(プロバイダ別)。null は未取得/失敗で静的リストを使う
const dynamicModels: Record<string, string[] | null> = { openai: null, gemini: null };
let lastEngineChoice: EngineChoice = "auto";
let lastModelOverride: string | null = null;

function modelChoicesFor(provider: string): string[] {
  return dynamicModels[provider] ?? MODEL_CHOICES[provider];
}

// 取得は設定ビューを最初に開いたときの一度だけ(閉じる=終了のライフサイクルで
// 起動のたびに ListModels を叩かない)。失敗してもエラー表示はせず静的リストで
// 動き続ける(一覧 API に載っていても実行時に拒否されるモデルはあるため、
// どのみち最終判定は翻訳実行時のエラー表示に委ねる)
let dynamicModelsRequested = false;

function ensureDynamicModels() {
  if (dynamicModelsRequested) {
    return;
  }
  dynamicModelsRequested = true;
  void fetchDynamicModels();
}

async function fetchDynamicModels() {
  await Promise.all(
    ["openai", "gemini"].map(async (provider) => {
      try {
        const models = await invoke<string[]>("list_available_models", { provider });
        if (models.length > 0) {
          dynamicModels[provider] = models;
        }
      } catch {
        // キー未設定・オフライン時は静的リストのまま
      }
    }),
  );
  rebuildModelOptions(lastEngineChoice, lastModelOverride);
}

// エンジン選択に連動してモデル候補を組み直す。openai/gemini 選択中はそのエンジンの
// モデルだけ、auto/mock は両方を optgroup で出す。手入力時代の値など候補に無い
// override は先頭に補って選択状態を保つ
function rebuildModelOptions(engine: EngineChoice, override: string | null) {
  lastEngineChoice = engine;
  lastModelOverride = override;
  modelSelect.innerHTML = "";

  const defaultOption = document.createElement("option");
  defaultOption.value = "";
  defaultOption.textContent = "既定値(エンジンにおまかせ)";
  modelSelect.appendChild(defaultOption);

  const providers = engine === "openai" || engine === "gemini" ? [engine] : ["openai", "gemini"];
  const known = new Set<string>();
  for (const provider of providers) {
    const group = document.createElement("optgroup");
    group.label = providerLabel(provider);
    for (const model of modelChoicesFor(provider)) {
      known.add(model);
      const option = document.createElement("option");
      option.value = model;
      option.textContent = model;
      group.appendChild(option);
    }
    modelSelect.appendChild(group);
  }

  if (override && !known.has(override)) {
    const custom = document.createElement("option");
    custom.value = override;
    custom.textContent = `${override}(カスタム)`;
    modelSelect.insertBefore(custom, modelSelect.children[1] ?? null);
  }

  modelSelect.value = override ?? "";
}

async function loadSettings() {
  const settings = await invoke<SettingsView>("get_settings");
  currentSettings = settings;
  langASelect.value = settings.lang_a;
  langBSelect.value = settings.lang_b;
  engineSelect.value = settings.engine_choice;
  rebuildModelOptions(settings.engine_choice, settings.model_override);
  promptInput.value = settings.custom_prompt ?? "";
  promptInput.placeholder = settings.default_prompt;
  openaiApiKeyStatus.textContent = settings.has_openai_key ? "登録済み: ●●●●●●●●" : "未登録";
  geminiApiKeyStatus.textContent = settings.has_gemini_key ? "登録済み: ●●●●●●●●" : "未登録";
  engineStatus.textContent = engineError || `実効エンジン: ${settings.effective_engine_name}`;
  if (activeHistoryId === null) {
    renderViewMeta();
  }
}

function saveLangPair() {
  invoke("set_lang_pair", {
    langA: langASelect.value,
    langB: langBSelect.value,
  })
    .then(() => {
      langStatus.textContent = "";
      engineError = "";
      return refreshSettings();
    })
    .catch((error: unknown) => {
      langStatus.textContent = String(error);
      // 保存されなかったので表示を実際の設定値へ戻す
      void refreshSettings();
    });
}

langASelect.addEventListener("change", saveLangPair);
langBSelect.addEventListener("change", saveLangPair);

engineSelect.addEventListener("change", () => {
  invoke("set_engine_choice", { choice: engineSelect.value })
    .then(() => {
      engineError = "";
      return refreshSettings();
    })
    .catch((error: unknown) => {
      engineError = String(error);
      void refreshSettings();
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
      return refreshSettings();
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
      return refreshSettings();
    })
    .catch((error: unknown) => {
      geminiApiKeyStatus.textContent = `保存に失敗しました: ${String(error)}`;
    });
});

modelSelect.addEventListener("change", () => {
  const model = modelSelect.value;
  invoke("set_model_override", { model: model.length > 0 ? model : null })
    .then(() => {
      modelStatus.textContent = "";
      engineError = "";
      return refreshSettings();
    })
    .catch((error: unknown) => {
      modelStatus.textContent = String(error);
      // 保存されなかったので表示を実際の設定値へ戻す
      void refreshSettings();
    });
});

function saveCustomPrompt(prompt: string | null, savedMessage: string) {
  invoke("set_custom_prompt", { prompt })
    .then(() => {
      promptStatus.textContent = savedMessage;
      engineError = "";
      return refreshSettings();
    })
    .catch((error: unknown) => {
      promptStatus.textContent = String(error);
    });
}

promptSaveButton.addEventListener("click", () => {
  const prompt = promptInput.value.trim();
  saveCustomPrompt(
    prompt.length > 0 ? prompt : null,
    prompt.length > 0 ? "保存しました" : "既定のプロンプトに戻しました",
  );
});

promptResetButton.addEventListener("click", () => {
  promptInput.value = "";
  saveCustomPrompt(null, "既定のプロンプトに戻しました");
});

// --- 起動時初期化 ---

// リンクは main ウィンドウを遷移させず既定ブラウザで開く(単一ウィンドウなので
// 遷移すると戻る手段が無い)。訳文だけでなく貼り付け欄の HTML 内リンクも通るよう
// document で委譲する
document.addEventListener("click", (event) => {
  const anchor = (event.target as HTMLElement | null)?.closest("a");
  if (!anchor) {
    return;
  }
  // 貼り付け欄(contenteditable)内も含め常に preventDefault する: WebKit は編集可能
  // 領域内のリンクも follow するため、相対リンクを踏むと単一ウィンドウごと別パスへ
  // 遷移して戻れなくなる(キャレット移動は mousedown 由来なので click 抑止では妨げない)
  event.preventDefault();
  // 外部ブラウザで開くのは貼り付け欄の外(訳文など)のリンクだけ。編集中の誤クリックで
  // ブラウザが開くのを避ける(isContentEditable なら属性の書き方によらず判定できる)
  if (anchor.isContentEditable) {
    return;
  }
  const href = anchor.getAttribute("href") ?? "";
  if (/^https?:\/\//i.test(href)) {
    void invoke("plugin:opener|open_url", { url: href });
  }
});

listenForTranslationChunks();
showState("idle");
loadHistory().catch((error: unknown) => {
  showHistoryError("履歴を読み込めませんでした", error);
});
void refreshSettings();
pasteInput.focus();
