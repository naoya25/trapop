# TraPoP 単一ウィンドウ再設計 — Quick Spec

**Created**: 2026-08-12 / **Type**: アーキテクチャ変更(常駐廃止・単一ウィンドウ化) / **Scope**: window 層 + フロント全面・エンジン層は不変

## 背景(なぜ変えるか)

- 現行: メニューバー常駐 + グローバルホットキー + 右下 popup スタック(複数ウィンドウ)
- 問題: 「見えないメイン窓 + 別 Space の popup」構造が Space ジャンプバグの温床。常駐の価値も薄い(起動は Raycast で足りる)
- 決定: 常駐と複数ウィンドウを捨て、普通の単一ウィンドウアプリにする(2026-08-12 Naoya 決定)

## 対応(decided)

### アプリライフサイクル

- **常駐廃止**: トレイアイコン・グローバルショートカット・ActivationPolicy::Accessory をすべて削除。普通の Dock アプリにする
- **起動**: Raycast(等のランチャー)からアプリを起動 → 今いる Space にウィンドウが開く(macOS 標準挙動に任せる。Space 制御コードは書かない)
- **終了**: ウィンドウを閉じたらプロセスごと終了(CloseRequested → exit)。冷起動のラグは許容済み
- 翻訳ウィンドウは常に1枚。複数ウィンドウ・popup スタックは実装しない

### UI(単一ウィンドウ)

```
+----------------+------------------------------+
| sidebar        | content                      |
|  [+ 新規翻訳]  |  (A) 翻訳ビュー               |
|  履歴一覧      |   - 入力エリア(貼り付け/入力) |
|  (新しい順)    |   - 翻訳ボタン/⌘Enter        |
|                |   - ストリーミング訳文(md描画)|
|  ----------    |  (B) 設定ビュー               |
|  [設定]        |   - 言語ペア/エンジン/キー/   |
+----------------+     モデル/プロンプト         |
```

- サイドバー: 上に「新規翻訳」、履歴一覧(クリックで訳文表示・再翻訳なし)、最下部に「設定」
- content は翻訳ビューと設定ビューの切り替え(タブ画面遷移なし・同一ウィンドウ内)
- 翻訳ビューの挙動は現 popup を踏襲: 貼り付けで即翻訳・⌘Enter 手動翻訳・停止・部分訳保持・原文トグル・リッチコピー
- 設定ビューは現メイン窓の設定タブを移植(ホットキー欄だけ削除)
- ウィンドウサイズは記憶(config の window_width/window_height に保存)

### 消すもの

- `window/popup.rs` 全体(スタック・collectionBehavior・Space pin)
- `window/mod.rs` のトレイ・hide 対策(`setup_tray` / `hide_main_window_before_popup` / 閉じる=hide 化)
- `hotkey.rs`・`tauri_plugin_global_shortcut`・ホットキー録音 UI・`config.hotkey`
- `src/popup/` ディレクトリ(ロジックは main へ移植)・vite の popup エントリ
- 履歴 replay 受け渡し(`HistoryReplayState` / `get_history_replay` / `open_history_popup`) — 履歴クリックは main 内で描画

### 変えないもの

- エンジン層(`engine/` 全部・SSE・プロンプト組み立て)。同日に別作業として入ったエンジン改善(モデル選択式+動的取得・SSE CRLF 対応・Gemini 既定モデル変更・カスタムプロンプト)は本再設計のスコープ外で、コミットは再設計と分けて積む
- 履歴 JSONL・config 保存形式(hotkey フィールド削除と、ウィンドウサイズ設定キーの window_width/window_height への改名のみ。旧 config の未知キーは serde が無視し、旧キーの値は既定値 860x600 に置き換わる)
- `start_translation` / `cancel_translation` の契約(呼び出し元ウィンドウ基準なので main からそのまま動く)
- 貼り付けの HTML リッチ判定(`htmlLooksRich`)・safeStreamPreview

## 受け入れ基準

1. `cargo test` / `npx tsc --noEmit` / `npm test` 全パス・警告 0
2. 起動 → 今いる Space にウィンドウ1枚。トレイアイコンなし・ホットキー登録なし
3. 貼り付け → ストリーミング翻訳 → md 描画・履歴保存・サイドバー一覧に反映
4. 履歴クリックで訳文即表示(再翻訳なし)・「新規翻訳」で入力に戻る
5. 設定(言語/エンジン/キー/モデル/プロンプト)が全部操作できる
6. ウィンドウを閉じるとプロセス終了(`ps` で残らない)
