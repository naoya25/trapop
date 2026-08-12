# TraPoP ミニパネル(フルスクリーン重畳翻訳) — Quick Spec

**Created**: 2026-08-12 / **Type**: 機能追加(第2ウィンドウ=非アクティブ化パネル) / **Scope**: window 層 + パネル用フロント追加。エンジン層・履歴・設定は不変
**前提**: 全設計判断は spike(~/Desktop/trapop-spike-panel・P1/P2/P4/P5/P6 実機 PASS)で確定済み

## 背景

フルスクリーンアプリの上では通常ウィンドウを出せない(Space が切り替わる)。翻訳はフルスクリーン作業中にこそ使いたいため、Spotlight 型の非アクティブ化パネルを追加する。

## 対応(decided)

### 起動導線

- **通常起動**(Dock / Raycast のアプリ起動): 現行どおりメイン窓を表示
- **`trapop://new`(deep link)起動**: メイン窓は非表示のまま、**今いる Space(フルスクリーン含む)にパネルだけ表示**。Raycast には Script Command で `open -g "trapop://new"` を登録(README に手順を書く)
- 起動済みのアプリに `trapop://new` が届いたら: **今いる Space に新しいパネルを1枚出す**。表示中の他パネルには触らない(それぞれ開いた Space に固定されたまま残る=複数パネル可)。実装は「隠れたパネルがあれば再利用(全 Space 参加へ戻して表示→再固定)、無ければ新規生成」のプール方式(destroy 不可制約のため。ウィンドウ数は同時表示の最大数で頭打ち)。メイン窓の表示状態は変えない
- Dock アイコンクリック(Reopen)でメイン窓を表示(spike P6 で確定)

### パネルウィンドウ

- tauri-nspanel(github.com/ahkohd/tauri-nspanel `v2.1` ブランチ・コミット a3122e89 固定)で NSPanel 化
- スタイル: titled + closable(信号機: 赤=有効 / 黄・緑=無効)・nonactivating・floating level・サイズ 480×400・リサイズ可
- Space 挙動: 出現時 canJoinAllSpaces + fullScreenAuxiliary → 表示 350ms 後に managed へ切替(**開いた Space に固定**。spike P4 で managed 固定 PASS)
- **閉じる(赤ボタン/Esc)= hide**。destroy しない(ネイティブ close は tao/wry と衝突してクラッシュする — spike v3 で2経路とも実証済み。`window_should_close` デリゲートで横取りして `panel.hide()` する経路のみ安全)。副次効果として2回目以降の `trapop://new` は再表示だけで速い

### パネル UI(最小構成)

- 貼り付け→即翻訳・⌘Enter 手動翻訳・停止・ストリーミング md 描画・原文トグル・リッチコピー(メイン窓の翻訳ビューと同じ挙動。サイドバー・履歴一覧・設定 UI は持たない)
- エンジン・言語ペア・モデル・カスタムプロンプトはメイン設定を共有(config 読みは既存コマンド経由)
- 翻訳結果は履歴に保存し、メイン窓の一覧へ自動反映(既存の history-appended イベントがそのまま効く)

### ライフサイクル(変更点・要認識)

- メイン窓を閉じる = アプリ終了(現行維持)。このときパネルも道連れになる
- deep link 起動でパネルだけ使った場合、パネルを hide してもアプリは Dock に残り続ける。終了は ⌘Q かメイン窓を開いて閉じる

### 実装メモ(spike からの移植点)

- `tauri_panel!` マクロ+`window_should_close` intercept+`hide()` は spike の main.rs の実装をそのまま踏襲してよい(ハーネス固有の `--panel-only` `--auto-close-test` フラグは持ち込まない)
- deep link は tauri-plugin-deep-link(schemes: `trapop`)。macOS は URL が起動済みインスタンスへ届くので single-instance プラグインは不要
- パネル用フロントは `src/panel/` に新設(vite マルチエントリ復活)。翻訳ロジックはメイン窓 main.ts の翻訳ビュー部と重複するが、初回は素直にコピーで可(共通化は動いてから)

## 受け入れ基準

1. `cargo test` / `npx tsc --noEmit` / `npm test` 全パス・警告 0
2. フルスクリーンアプリ上で `open -g "trapop://new"` → パネルが Space を切り替えずに出る・文字入力できる・貼り付けで翻訳が流れる
3. Space を切り替えてもパネルはついてこない(開いた Space に固定)
4. 赤ボタン/Esc でそのパネルだけ消え、アプリは継続。再度 `trapop://new` で即表示
5. パネルを表示したまま別 Space で `trapop://new` → 新しいパネルが今の Space に出て、前のパネルは元の Space に残る
6. パネルでの翻訳がメイン窓の履歴一覧に出る
7. 通常起動ではメイン窓が出て、従来の全機能(履歴・設定含む)が動く
8. Dock クリックでメイン窓が表示される
