---
name: jidori-pose-ref
description: jidori-kun (webカメラ自撮り MCP) で実写ポーズを撮り、その写真を「ポーズ参照 (ref)」にして画像生成でポーズ転写した絵を作るフロー。キモは「MCP の snap 結果は Claude 側で生バイトが取り出せない → curl で /mcp を直叩きして応答 JSON の base64 を取る」点。ユーザーが「jidori で撮って」「ポーズ撮って ref に」「実写ポーズから生成」「自撮りポーズで生成」「jidori-kun でポーズ参照」と言ったときに使う。
---

<!--
配付用スキル。正本 (source of truth) は jidori-kun プロジェクトディレクトリ
(github.com/yatmita/jidori-kun) で管理する。
配付されたコピーは直接編集せず、更新は正本側で行って再配付すること。
-->

# jidori-pose-ref — 実写ポーズ → ref → ポーズ転写生成

webカメラで撮った実写ポーズを参照画像にして、キャラをそのポーズに置き換えた絵を生成する。教科書ポーズでない動きのあるコマを作れる。

jidori-kun は webカメラでカウントダウン撮影する Tauri 製デスクトップアプリ + MCP サーバ。snap ツール 1 つを持ち、stdio / HTTP (Bearer 認証) の両 transport に対応する。

## 前提

- **jidori-kun MCP が登録済み**であること（HTTP transport 想定）。未登録なら:
  ```bash
  claude mcp add --transport http --scope local jidori-kun \
    http://<LAN-IP>:8790/mcp --header "Authorization: Bearer <token>"
  ```
  （URL・token は jidori-kun アプリの「設定」でトークン生成 → HTTP サーバ開始したときに表示される）
- 生成に使う画像バックエンドが使えること

## 手順

### 1. ポーズ撮影 — curl 直叩きで base64 を取る（最重要）

⚠️ **MCP ツール `snap` を普通に呼ぶと、ホスト（Claude Code 等）が返却画像を「表示用画像」に変換してしまい、生 base64 テキストが手元に来ない。** snap が返す保存先パスも撮影マシンの一時ディレクトリ（Windows なら `%TEMP%\jidori-kun\pose-N.png`）で、別マシン/コンテナからは読めないことが多い。

→ **MCP ツール経由をやめ、自分で HTTP エンドポイントを curl して応答 JSON から base64 を取り出す**。snap の応答は JSON-RPC で `result.content = [{type:"text",...}, {type:"image", data:"<base64 PNG>", mimeType:"image/png"}]` の形。

```bash
# URL と token は登録済み MCP 設定から取得（スクリプトに直書きしない）
eval "$(claude mcp get jidori-kun | awk '/URL:/{print "URL="$2} /Authorization:/{print "TOK=\""$2" "$3"\""}')"
curl -s --max-time 130 -X POST "$URL" \
  -H "Authorization: $TOK" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"snap","arguments":{"countdown":5}}}' \
  -o /tmp/snap_resp.json
python3 - <<'PY'
import json, base64
d = json.load(open("/tmp/snap_resp.json"))
img = next(c for c in d["result"]["content"] if c.get("type") == "image")
open("/tmp/pose.png", "wb").write(base64.b64decode(img["data"]))
print("saved /tmp/pose.png")
PY
```

- **カメラ窓が出てカウントダウン撮影される。** 撮る前に一言「ポーズを取ってください」と伝える。動きのあるポーズほど転写の価値が出る
- `initialize` ハンドシェイク不要、bare `tools/call` で通る
- 撮った写真は一度目視し、ポーズ（腕の角度・重心・接触・頭の向き）を言語化しておく（後段プロンプトに使う）

> 一般化した教訓: **MCP が画像を返すが、その生バイトが必要**なときは、MCP ツール呼び出しに頼らず、その MCP の HTTP エンドポイントを自分で叩いて raw JSON から base64 を取るのが定石。
>
> 恒久策: jidori-kun の保存先を、生成側から読める共有ディレクトリ（ホスト↔コンテナの bind mount 等）に向けられれば、curl 不要でファイルを直接読める。

### 2. ポーズ転写を生成する

`/tmp/pose.png` を **1 枚目の参照画像**にして、キャラをそのポーズに置き換える。プロンプトの骨:

```
Replace the person in the reference photo with a character,
keeping the EXACT same pose: <ポーズの言語化>.
Copy the limb angles, weight distribution, hand/foot placement,
and overall composition precisely.

Render in <目的の画風> (NOT photographic).
Do NOT copy the clothing from the photo; draw <キャラの衣装>.
Background: <差し替えたい背景>.
```

必須の明示指示（落とし穴）:
- **「写真の服をコピーするな」** を書く（放置すると実写の服が漏れる）＋ キャラ衣装を強制
- **「NOT photographic / 目的の画風」** を強く書く（写実に寄るのを防ぐ）
- 接触ポーズは `hands clasped` / `feet pressed together` 等を明示（書かないと接触が弱まる）

バックエンドは環境依存。ポーズ転写は写真 ref を強く効かせられるモデル（例: OpenAI gpt-image-2 の edits）が向く。1 枚ずつ生成し、結果を目視でポーズ転写を確認する。リテイクは確認を取ってから（無駄打ち防止）。

## 落とし穴まとめ

| 症状 | 原因 / 対処 |
|---|---|
| snap の画像が生成に渡せない | ツール経由では生バイトが取れない。curl 直叩きで base64 取得（手順1） |
| ポーズは合うが服が実写のまま | プロンプトに `do NOT copy clothing from the photo` + キャラ衣装を明示 |
| photo-realistic に寄る | 目的の画風 + `NOT photographic` を強く書く |
| 撮影ファイルが別マシンから読めない | 保存先が撮影機の一時ディレクトリ。curl 経路を使うか、共有ディレクトリに保存先を変更 |
