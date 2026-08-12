# TraPoP

テキストを貼り付けると即翻訳する macOS アプリ(日本語⇔英語の双方向・言語ペアは設定で変更可)。

普段は起動せず、使うときだけランチャー(Raycast 等)や Dock から起動する。ウィンドウは今いる画面に1枚だけ開き、閉じるとアプリごと終了する。

- 仕様: `docs/spec-single-window-redesign.md`・`docs/spec-mini-panel.md`

## 使い方

1. 翻訳したいテキストをコピー(⌘C)
2. TraPoP を起動(Raycast にホットキーを割り当てると速い)
3. ⌘V で貼り付け → 即翻訳開始(手入力の場合は ⌘Enter)

- サイドバーの履歴をクリックすると過去の訳文を即表示(再翻訳なし)
- サイドバー最下部の「設定」で言語ペア・エンジン(OpenAI / Gemini)・API キー・モデル・翻訳プロンプトを変更できる
- API キーは macOS Keychain に保存される

アプリはクリップボードを読まない(貼り付けはユーザー操作)ため、アクセシビリティ等の特別な権限は不要。

## ミニパネル(フルスクリーン重畳)

`trapop://new` を開くと、メイン窓を表示せずに今いる Space(フルスクリーンアプリ上も含む)へ翻訳専用の小さいパネルだけ出す。パネルの赤ボタン/Esc で閉じてもアプリは終了せず、次回は再表示だけで即座に開く。仕様は `docs/spec-mini-panel.md` を参照。

Raycast から呼ぶには Script Command を登録する。

1. `~/.raycast/scripts` などスクリプト用ディレクトリに以下の内容で `trapop-new.sh` を作成し、実行権限を付ける(`chmod +x`)

   ```sh
   #!/bin/bash
   # @raycast.schemaVersion 1
   # @raycast.title TraPoP: New Translation
   # @raycast.mode silent
   # @raycast.icon 🌐

   open -g "trapop://new"
   ```

2. Raycast の Extensions 設定で該当ディレクトリを Script Commands のフォルダとして追加する
3. 好みで "TraPoP: New Translation" コマンドにホットキーを割り当てる

`trapop://` の scheme 登録は macOS の LaunchServices が初回起動時に行うため、`trapop://new` を初めて使う前に一度 TraPoP を通常起動(Dock 等)しておくこと。

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
