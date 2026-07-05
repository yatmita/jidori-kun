# jidori-kun

Webカメラでカウントダウン撮影し、生成画像の**ポーズ参照(ref)**として使える写真を撮るデスクトップアプリ（Tauri v2）。**MCP サーバを内蔵**しており、Claude から `snap`（撮影）を呼べます。

- **ローカル (stdio MCP)**: `jidori-kun.exe --mcp` を Claude に登録
- **ネットワーク (HTTP MCP)**: アプリ内の設定でトークン付き HTTP サーバを起動し、別マシンの Claude から接続

## 機能
- Webカメラ・プレビュー（鏡像）／カメラ選択
- **カウントダウン撮影**（0/3/5/10秒）＋連写（1/3/5枚）
- 保存ダイアログでPNG書き出し
- **MCP `snap`**: Claude が撮影を要求 → 窓がカウントダウン → 撮影 → 画像を返却（stdio / HTTP 両対応、HTTPはBearerトークン認証）
- 手動更新通知（バージョンだけ確認し、DL/インストールは手動）

## 開発
```sh
pnpm install
pnpm tauri dev      # 開発ウィンドウ
pnpm tauri build    # インストーラ (NSIS/MSI) を生成
```
必要環境: Rust/Cargo, Node 20+, pnpm。Windows は WebView2 と MSVC ビルドツール。

## MCP の使い方
### ローカル（stdio）
Claude Desktop / Claude Code の MCP 設定:
```json
{ "mcpServers": { "jidori-kun": {
    "command": "C:\\path\\to\\jidori-kun.exe", "args": ["--mcp"] } } }
```
Claude が `snap` を呼ぶとカメラ窓がカウントダウンして撮影、画像が返ります。

### ネットワーク（HTTP）
アプリ右上「⚙ 設定」→ トークン生成 → 公開範囲(LAN) → 開始。表示された URL とトークンを、別マシンの Claude に登録:
```json
{ "mcpServers": { "jidori-kun": {
    "url": "http://<LAN-IP>:8790/mcp",
    "headers": { "Authorization": "Bearer <token>" } } } }
```
> ⚠️ カメラを操作できるエンドポイントです。**必ずトークンを設定**し、LAN 内 or 認証付きトンネルに限定してください。

## Claude Code スキル（任意）

snap で撮った実写を「ポーズ参照」にして画像生成でポーズ転写する一連の手順を、Claude Code の **スキル**として同梱しています: [`skills/jidori-pose-ref/SKILL.md`](skills/jidori-pose-ref/SKILL.md)。

使うには **`~/.claude/skills/` にコピー**してください。Claude Code はユーザー全体の `~/.claude/skills/`、または「いま開いているプロジェクトの」`.claude/skills/` しか読みません。**このリポジトリに置いてあるだけでは有効になりません**（jidori-kun repo を開いて作業することはまず無いため）。

```sh
mkdir -p ~/.claude/skills/jidori-pose-ref
cp skills/jidori-pose-ref/SKILL.md ~/.claude/skills/jidori-pose-ref/
```

以後 Claude Code で「jidori で撮って ref に」「実写ポーズから生成」等と言うと発火します。要点は、snap の応答画像を **curl で `/mcp` を直叩きして base64 として取り出す**手順（MCP ホストは返却画像を表示用に変換し、生バイトを渡さないため）。詳細は SKILL.md を参照。

## 更新
自動更新は行いません。起動時に `latest.json`（`UPDATE_MANIFEST_URL` を `src/main.js` で設定）を見て、新版があれば通知するだけです。DL/インストールは手動。書式は `latest.json.example` 参照。

## 配布メモ
- インストーラは **未署名**のため、初回に Windows SmartScreen 警告が出ます（「詳細情報」→「実行」）。
- 署名して警告を消す場合は Certum(OSS) / Azure Trusted Signing 等を検討。

## 構成
- `src/` — フロント（getUserMedia 撮影・設定UI・更新通知）
- `src-tauri/src/` — `mcp.rs`（stdio/HTTP MCP・`snap`）, `lib.rs`（コマンド/HTTP制御）, `settings.rs`
- `index.html` / `vite.config.js` / `package.json`
- `skills/` — Claude Code スキル（`~/.claude/skills/` にコピーして使う。任意・配付用）

## リリース / CI
- タグ `vX.Y.Z` を push すると `.github/workflows/release.yml` が `windows-latest` で
  インストーラ（NSIS `*_x64-setup.exe` / MSI）をビルドし、同名の GitHub Release に添付します。
- 手動実行（Actions の workflow_dispatch）も可能です。

## ライセンス
MIT License（[LICENSE](LICENSE) 参照）。
