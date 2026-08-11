# TraPoP

英語テキストを選択してホットキー(⌥⇧⌘P)を押すと、その場に翻訳ポップアップが出る macOS アプリ。

- 仕様: `~/.agents/docs/2026-08-10_spec_translation-popup.md`
- UI 仕様: `~/.agents/docs/2026-08-10_UI仕様_TraPoP.md`

## 開発

```sh
npm install
npm run tauri dev
```

## ビルド

```sh
npm run tauri build -- --debug --bundles app
```

初回はアクセシビリティ権限(システム設定 → プライバシーとセキュリティ → アクセシビリティ)の許可が必要。
