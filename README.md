# TraPoP

テキストを貼り付けると即翻訳する macOS アプリ(日本語⇔英語の双方向・言語ペアは設定で変更可)。

普段は起動せず、使うときだけランチャー(Raycast 等)や Dock から起動する。ウィンドウは今いる画面に1枚だけ開き、閉じるとアプリごと終了する。

- 仕様: `docs/spec-single-window-redesign.md`

## 使い方

1. 翻訳したいテキストをコピー(⌘C)
2. TraPoP を起動(Raycast にホットキーを割り当てると速い)
3. ⌘V で貼り付け → 即翻訳開始(手入力の場合は ⌘Enter)

- サイドバーの履歴をクリックすると過去の訳文を即表示(再翻訳なし)
- サイドバー最下部の「設定」で言語ペア・エンジン(OpenAI / Gemini)・API キー・モデル・翻訳プロンプトを変更できる
- API キーは macOS Keychain に保存される

アプリはクリップボードを読まない(貼り付けはユーザー操作)ため、アクセシビリティ等の特別な権限は不要。

## 開発

```sh
npm install
npm run tauri dev
```

動作確認用のデバッグビルドは `npm run tauri build -- --debug --bundles app`。

## ビルド

```sh
npm run tauri build
```

`src-tauri/target/release/bundle/macos/TraPoP.app` が生成される。
