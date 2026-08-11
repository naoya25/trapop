# TraPoP

ホットキー(⌥⇧⌘P)で翻訳ポップアップが開く macOS アプリ。テキストを ⌘V で貼ると即翻訳(日本語⇔英語の双方向・言語ペアは設定で変更可)。

- 仕様: `~/.agents/docs/2026-08-10_spec_translation-popup.md`
- UI 仕様: `~/.agents/docs/2026-08-10_UI仕様_TraPoP.md`

## 使い方

1. 翻訳したいテキストをコピー(⌘C)
2. ⌥⇧⌘P でポップアップを開く
3. ⌘V で貼り付け → 即翻訳開始

アプリはクリップボードを読まない(貼り付けはユーザー操作)ため、アクセシビリティ等の特別な権限は不要。

## 開発

```sh
npm install
npm run tauri dev
```

## ビルド

```sh
npm run tauri build -- --debug --bundles app
```

API キーはメニューバーアイコン → 設定... から登録(macOS Keychain に保存)。
