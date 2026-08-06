# tg-kl-vault 書籤功能 + AI 自動標籤

## 背景

`tg-kl-vault` 是 Go 版 `flowerss-bot` Telegram RSS 閱讀器的 Rust 移植。目前只能訂閱與推播，
看到想留下來的文章沒有任何地方可以收藏。

這次要加入一套**以聊天室為單位的書籤庫**：一批管理用的 slash command、每則推播訊息下方一鍵收藏的
🔖 按鈕，以及由可插拔 tagger 自動歸類到**固定英文 slug 分類表**的能力。Tagger 預設走
**Google Gemini 免費層**，偵測不到 API key 時自動退回本地關鍵字啟發式。目標是讓「讀到」與「收好」
只差一次點擊，而且書籤庫不用手動歸檔也維持得住。

成本是明確的設計約束：走 Gemini 免費層，預設型號 `gemini-3.1-flash-lite`。**Google 已不再於
官方 rate limits 頁公布逐型號的免費層數字**（該頁改為導向 AI Studio 的個人額度頁），
所以設計上不依賴任何寫死的額度數字：每日軟計數與 RPM 間隔只是保守護欄，**429 latch 才是權威**，
最終還有不可能失敗的啟發式 fallback。維運者應自行到 AI Studio 確認實際額度後調整那兩個數字。

### 已與使用者確認的決策
| # | 決策 |
|---|---|
| 1 | 可插拔 tagger，預設 Gemini 免費層，無 API key 時自動退回本地啟發式 |
| 2 | 先存書籤立即回覆，背景 worker 補標籤後再編輯訊息 |
| 3 | 歸屬為**每聊天室**（`chat_id`）— 群組共享同一個書籤庫 |
| 4 | 固定分類表，AI 只能從表中挑選；標籤一律英文 slug |
| 5 | 分頁卡片，一頁 5 筆，上/下頁，每筆可進詳細頁編輯／刪除 |
| 6 | 每則推播加 🔖 按鈕，可在 `/settings` 開關，預設開 |
| 7 | 另需：關鍵字搜尋、從 `/settings` 匯出、手動備註 |

### 規劃過程中發現的非顯而易見約束
- `handle_check`（`runtime.rs:420-603`）是 scheduler ingest pipeline 的**第二份複製**。
  任何加在送出路徑上的東西都得同時改兩邊，否則就要先抽共用。
- 兩套互不相容的 callback data 慣例並存。舊的 telebot 二進位格式（`bot/callback.rs`，有 golden
  vector 測試）是**凍結的**。書籤只用純冒號慣例。
- `bot/render.rs` **完全不做 escape** — 這是刻意的 Go byte-parity。書籤渲染屬於新介面，
  **必須** escape，否則標題裡一個 `&` 就會讓訊息無聲消失（`sender.rs:99-103` 只是 log 而已）。
- `repo.prune_contents`（`repo.rs:477`）與 `delete_source_and_contents`（`repo.rs:187`）會刪除
  `contents` 資料列。書籤必須是**自帶快照**，`content_hash_id` 只是可能懸空的麵包屑。
- `Repo::pool()` 已經是 public（`repo.rs:32`），所以在同層模組再開一個 `impl Repo` 區塊不需要改可見性。
- `config.apply_env_overrides`（`config.rs:72-91`）是手寫的 — 新欄位不補一行就**完全沒有**環境變數支援。
  `Config` derive 了 `Eq`，所以不能有 `f32` 欄位。
- `Config` derive 了 `Debug` 又被到處 clone。**絕對不要** `{:?}` 整個 `Config`，會把 API key 印進 log。

---

## Step 0 — 前置作業（還沒有書籤程式碼）

**`crates/bot/src/db/mod.rs`** — 改用 `SqliteConnectOptions` 建立連線池
（`.journal_mode(Wal).busy_timeout(5s).synchronous(Normal).foreign_keys(true)`），並刪掉 `:21-24`
那四行 `pool.execute("PRAGMA …")`。那些 pragma 目前只作用在 `max_connections(4)` 裡的**一條**連線上，
而 `synchronous` 是 per-connection 的。標籤 worker 是第二個並行寫入者，正好就是這件事開始造成
偶發 `SQLITE_BUSY` 的時機。現有 repo 測試就是這一步的回歸網。

**`crates/bot/Cargo.toml`** — 加入 `serde_json.workspace = true`，並為 `reqwest` 加上 `"json"`
feature（目前是 `default-features = false` 且沒有它，所以 `.json()` 根本不存在）。

**`crates/bot/src/ratelimit.rs`** — 把私有的 `wait_slot`（`:45`）抽成
`pub struct MinIntervalLimiter { next: Mutex<Instant>, spacing: Duration }`，再用它重寫
`SendRateLimiter`。Gemini client 需要同一個原語來做 RPM 間隔。

**`crates/bot/src/bot/callbacks.rs:35-39`** — 把 `query.regular_message()` 換成 `query.message` +
`MaybeInaccessibleMessage::{chat(), id()}`。訊息被刪除時 `regular_message()` 會回 `None`，
所以現在點那種按鈕會直接 return 而**從未呼叫 `answerCallbackQuery`**，用戶端就一直轉圈到過期。
順手也修好了舊的 feed 按鈕，且不動任何 wire format。

**`crates/bot/src/bot/runtime.rs:154-159`** — 改用 derive 出來的 `Command::bot_commands()`，
不再用手維護的 `COMMANDS` 表；然後把 `COMMANDS` 轉型為 **Go-parity golden**，並把
`commands.rs:59-82` 改寫成斷言「derive 出的清單開頭恰好是凍結的那 14 個」。這樣新增指令就不再是
「改三個地方、順便弄壞一個測試」。

---

## Step 1 — Schema：`migrations/0004_bookmarks.sql`

純新增；時間戳用 INTEGER unix，沿用 `0002` 的前例（而非 `0001` 的 TEXT）。

```sql
CREATE TABLE IF NOT EXISTS bookmarks (
  -- AUTOINCREMENT 是必要的：id 會進到 inline keyboard 的 callback_data，
  -- 而那些按鈕留在使用者的對話紀錄裡。沒有它，SQLite 會重用 rowid，
  -- 於是一顆過期按鈕會改到另一筆書籤。
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id             INTEGER NOT NULL,          -- 歸屬
  created_by          INTEGER NOT NULL,          -- 建立者（群組刪除規則用）
  url                 TEXT    NOT NULL,          -- 已正規化；同時是去重鍵
  title               TEXT    NOT NULL DEFAULT '',
  note                TEXT    NOT NULL DEFAULT '',
  source_title        TEXT    NOT NULL DEFAULT '', -- 快照，因為 `sources` 可能被刪
  content_hash_id     TEXT,                      -- 僅麵包屑，可能懸空，絕不用來 JOIN
  telegraph_url       TEXT,
  tag_state           INTEGER NOT NULL DEFAULT 0, -- 0 待處理, 1 完成
  tag_attempts        INTEGER NOT NULL DEFAULT 0,
  tag_next_attempt_at INTEGER NOT NULL DEFAULT 0,
  notify_message_id   INTEGER,                   -- worker 要編輯的訊息；NULL = 沒有可編輯的
  notify_kind         INTEGER NOT NULL DEFAULT 0, -- 0 = 編輯文字+鍵盤, 1 = 只換鍵盤標籤
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_chat_url ON bookmarks(chat_id, url);
CREATE INDEX IF NOT EXISTS idx_bookmarks_chat_id_desc   ON bookmarks(chat_id, id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_pending
  ON bookmarks(tag_next_attempt_at, id) WHERE tag_state = 0;

CREATE TABLE IF NOT EXISTS bookmark_tags (
  bookmark_id INTEGER NOT NULL,
  tag         TEXT    NOT NULL,
  origin      INTEGER NOT NULL DEFAULT 0,  -- 0 = ai, 1 = 手動
  PRIMARY KEY (bookmark_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_bookmark_tags_tag ON bookmark_tags(tag);
```

刻意不做的幾件事，各有理由：
- **`bookmark_tags` 不宣告 FK。** 少了 `ON DELETE CASCADE`，一旦 `foreign_keys` 對每條連線都真的生效
  （Step 0 會讓它成真），刪書籤就會開始失敗。改為在同一個 transaction 裡明確刪掉 tag 資料列。
  也不加孤兒清掃工作。
- **不用 FTS5。** `MIGRATOR.run()` 在**每次啟動**都會執行（`db/mod.rs:26`），所以一個在非 bundled
  sqlite 上會失敗的 `CREATE VIRTUAL TABLE` 代表的是整個 bot 開不起來，而不是搜尋降級。
  在這個資料量下，`LIKE` 搭配 `chat_id` 索引是微秒級的。
- **不放 `source_id`。** 與 `content_hash_id` 重複而且會懸空；改為快照 `source_title`。
- **不要 `tag_state = 2/3`。** `bookmark_tags.origin` 已經記錄了手動 vs AI，而啟發式 fallback
  不可能失敗，所以每一列都會走到 `1`。手動標籤直接設 `1`，又因為 claim 查詢只讀 `tag_state = 0`，
  「AI 永遠不會覆蓋手動標籤過的書籤」是自然推論出來的結果。
- **索引裡不寫 `DESC`。** SQLite 反向掃 ASC 索引本來就沒問題。

---

## Step 2 — Repo 層：`crates/bot/src/db/bookmarks.rs`（新增）

第二個 `impl Repo` 區塊，raw SQL 加 `?` bind，比照 `repo.rs` 的慣例。
`crates/bot/src/db/models.rs` 新增 `Bookmark` 與 `BookmarkTag`（`FromRow`）。

方法：`upsert_bookmark`、`set_bookmark_notify`、`count_bookmarks`、`bookmarks_page`、
`count_bookmarks_by_tag`、`bookmarks_page_by_tag`、`search_bookmarks`、`tags_for_bookmarks`、
`tag_counts`、`replace_bookmark_tags`、`set_bookmark_note`、`delete_bookmark`、
`claim_pending_bookmarks`、`finish_bookmark_tagging`、`bump_bookmark_attempt`、
`bookmarks_for_export`，以及 `chat_ids_with_option_off(prefix)`（放在 `repo.rs`，Step 6 會用）。

三條絕對不能漏的正確性規則：

1. **去重 upsert 絕不能用 `last_insert_rowid()`。** SQLite 只在真的 insert 時更新它，所以走
   `ON CONFLICT … DO UPDATE` 分支時回傳的是先前某次 insert 的舊 id — handler 會把鍵盤綁到
   別人的書籤上。改用 `RETURNING id, created_at`（需 sqlite ≥3.35，bundled amalgamation 滿足），
   再比較 `created_at == now` 判斷「本來就存在」。
2. **`LIKE` pattern 必須轉義。** `fn like_pattern(q) -> String` 轉義 `\`、`%`、`_`，查詢帶
   `ESCAPE '\'`；否則 `/bmsearch 100%` 會撈出全部。查詢字串上限 100 字元，純空白直接拒絕。
   不要 `COLLATE NOCASE`、也不要 `lower()` — SQLite 的 `LIKE` 不看 collation，本身已對 ASCII
   大小寫不敏感；文件註明非 ASCII 會區分大小寫（對 CJK 無意義）。
3. **在同一個 transaction 裡刪 tag** 和書籤（`let mut tx = self.pool().begin()`，
   然後 `.execute(&mut *tx)` — 注意那個 deref）。

**分頁用 offset/limit 搭配一個 `COUNT(*)`**，不用 keyset。UI 需要 `第 2/7 頁`、需要從詳細頁返回
第 N 頁、也需要夾住過期的頁碼 — keyset 三件都表達不出來，而在有 `chat_id` 索引的情況下兩個查詢
都是次毫秒級。

測試（inline，比照 `repo.rs:507` 用 `tempfile::tempdir()` + `db::connect`）：upsert 第二次呼叫
回傳**同一個** id（`last_insert_rowid` 那個回歸）；每個讀取都做 `chat_id` 隔離；標題為
`100% pure` 的書籤搜 `100%` 找得到，但單獨一個 `%` 不會撈出全部；`claim_pending_bookmarks`
遵守 `tag_next_attempt_at` 與 `LIMIT`；刪除會連帶清掉 tag 資料列。

---

## Step 3 — 標籤模組：`crates/bot/src/tagging/`（新增）

`taxonomy.rs` — **單一**表格，讓 slug 清單、啟發式關鍵字、顯示標籤不可能各自漂移：

```rust
pub struct Category {
    pub slug: &'static str,          // 英文，存進 bookmark_tags.tag，也直接顯示在 UI
    pub keywords: &'static [&'static str],
}
pub const TAGS: &[Category] = &[ /* 約 16-20 個，含 `other` */ ];
pub fn normalize(raw: &str) -> Option<&'static str>;   // 轉小寫、`_`→`-`、別名，否則 None
pub fn idx_of(slug) -> Option<usize>;  pub fn slug_of(idx) -> Option<&'static str>;
```

**`TAGS` 的順序是一種 wire format** — 索引會進到比 process 活得更久的 callback_data。
只能追加、可以改名，永遠不能重排或刪除。用一個 golden test 釘住，精神同 `callback.rs:239`。

`mod.rs` — `TagInput { title, url, excerpt }`，以及：

```rust
#[allow(async_fn_in_trait)]                 // 必要，否則 -D warnings 會失敗
pub trait Tagger: Send + Sync {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>>;
}
pub enum AnyTagger { Gemini(GeminiTagger), Heuristic(HeuristicTagger) }
pub fn build_tagger(cfg: &Config) -> AnyTagger;
```

`Box<dyn Tagger>` **無法編譯** — trait 裡的 `async fn` 會 desugar 成 RPITIT，不是 dyn-compatible。
用 enum dispatch + 泛型消費端，正好對應現有的兩個前例：`MessageSender`（`sender.rs:24`）和
`PreviewPublisher`（`preview.rs:19`），兩者都是以 `Scheduler<P, S>` 泛型消費、從不用 `dyn`。

`heuristic.rs` — 拿 title + excerpt + URL host 去比對 `Category::keywords` 計分，取前 `max_tags` 個，
**永遠至少回傳 `["other"]`**。「不會失敗且不會空」這個性質，正是它能當終點 fallback、
以及 `tag_state = 2` 可以不存在的原因。

`gemini.rs` — `POST {endpoint}/v1beta/models/{model}:generateContent`，header 用 `x-goog-api-key`，
`generationConfig.responseMimeType = "application/json"` 加上結構化輸出 schema
`{"type":"ARRAY","items":{"type":"STRING","enum":[…slugs…]}}`（proto-JSON 的 type 名稱是**大寫**），
在 `normalize` 之外再加一層結構性約束。`temperature` 寫死 `0.0`（做成 config 欄位會弄壞 `Config`
的 `Eq` derive），`maxOutputTokens: 64`。回應的 `candidates[0].content.parts[0].text` 是一個 JSON
**字串** — 要解兩層。`endpoint` 可設定，測試才能指向本地一次性伺服器。

- **實作時要先驗證的兩件事**（文件在 2026 年間有變動，別憑記憶寫）：
  (a) 這個型號在 REST 上吃的是舊的 `responseSchema`（OpenAPI 子集）還是新的 `responseJsonSchema`
  （完整 JSON Schema）— `gemini-3.1-flash-lite` 的官方範例用的是 `response_json_schema`；
  兩者只需選一個，用一次真實呼叫確認。
  (b) 官方文件現在把 `generateContent` 標為 legacy、另有一組 Interactions API。
  對一個「丟標題回分類」的呼叫，`generateContent` 仍是最簡單的選擇且仍可用，
  但這件事要在 code 註解裡寫明，日後才知道為何選它。
- **不要 `thinkingConfig`。** Gemini 3.x 的 Flash-Lite 系列 `thinkingLevel` **預設就是 `minimal`**，
  正是分類這種高吞吐任務要的設定，所以省略它就是正解；而 `thinkingBudget` 已被 `thinkingLevel`
  取代（僅為相容而保留），寫死它只會在換型號時出問題。
- **依 HTTP status 分類，不看 body 結構**（官方錯誤 schema 一直在變）：
  `429` → 設冷卻 latch，盡力掃 body 找每日額度的標記以 latch 到隔天；
  `400/401/403/404` → `error!` 一次並設一個 **process 生命週期的 `AtomicBool` 停用 Gemini**
  （沒有這個，一個打錯的 key 會讓每筆書籤都發一次註定失敗的請求，永遠）；
  `5xx`/timeout → 視為暫時性，交給 worker 退避。
- **`MinIntervalLimiter`，`spacing = 60s / max_rpm`。** 免費層的 RPM 是個位數到十幾，
  不限速的 worker（每 5 秒一批 3 筆 ≈ 36 RPM）會自己撞 429。
  不做請求批次化 — 每筆書籤一次呼叫，量級上完全不需要。
- Gemini client 自己**不做** fallback — 它回 `Err`/`Ok(vec![])`，由 **worker** 決定。
  這讓 HTTP client 保持純粹、可單元測試。

`quota.rs` — **單一** `options` 資料列 `tg-kl-vault:ai:quota`，值為 `"YYYY-MM-DD:count"`，
日期不同就重設。一天一列會無限成長（沒有任何東西會清 `options`）。Google 是太平洋時間午夜重設；
不要為此引入 `chrono-tz` — 這個計數器只是軟性預算護欄，429 latch 才是權威。

`metadata.rs` — **必要**，不是可選：貼進聊天室的 URL，`title` 是空的，書籤會渲染成裸網址，
AI 也只能靠網址猜。做法是硬上限 128 KB 的串流抓取、看到 `</head>` 就提早跳出，
完全照 `feed/fetch.rs:70-79` 現成的模式；非 `text/html` 直接跳過；手寫 `<title>` /
`meta description` / `og:description` 抽取（約 40 行，純函式易測 — `quick-xml` 不容錯 HTML，
而 `scraper` 不該進到 bot crate）。重用 `preview::decode_entities`（`preview.rs:180`，改成
`pub(crate)`）。來自 feed 的書籤直接取 `contents` 的標題，完全跳過抓取。

*安全備註*：這會讓 bot 去抓使用者提供的任意 URL（SSRF：link-local、loopback）。
這個曝險**已經存在**於 `/sub` → `create_source` → `Fetcher`，所以不是退步，
但跟隨轉址會讓天真的前置檢查失效。相關且值得順手處理的一點：`config.allowed_users` 有解析卻
**完全沒有任何地方強制執行** — 在書籤指令上套用它（清單非空時），免得陌生人耗掉你的 Gemini 額度。

`url_norm.rs` — `normalize_url(raw) -> anyhow::Result<String>`：trim、去掉 `<>`、超過 2048 字元拒絕；
`Url::parse`，遇 `RelativeUrlWithoutBase` 補 `https://` 前綴重試；**除 http/https 外一律拒絕**
（這是安全控制不是整潔問題 — 存進去的 `javascript:` URL 會原封不動落在 `href="…"` 裡）；
小寫化、IDNA、預設 port、路徑正規化全部交給 `url` crate；`set_fragment(None)`；
移除一份保守的追蹤參數名單（`utm_*`、`gclid`、`fbclid`、`msclkid`、`yclid`、`mc_cid`、`mc_eid`、
`igshid`、`_hsenc`、`_hsmi`）— **不含** `ref` 與 `si`，這兩個在真實網站上是有作用的。
**不要**排序 query 參數、不要小寫化路徑、不要去掉結尾斜線、不要去掉 `www.`：這四件都會弄壞真實網址。
文件註明後果：`www.x.com/a` 和 `x.com/a` 會是兩筆書籤。

`worker.rs` — `TagWorker<T: Tagger, E: MessageEditor>`，有 `run_once()` 與
`run_until_shutdown(watch::Receiver<bool>)`，比照 `scheduler.rs:61-79`。每 5 秒 claim
`tag_state = 0 AND tag_next_attempt_at <= now ORDER BY id LIMIT 3`（不需要 lease 欄位 —
單一 process、單一 task）。

**先提交標籤，再編輯訊息。** 編輯失敗時標籤已經落地、該列已是終態；反過來做的話，
crash 後會重新標籤並白燒額度。**編輯失敗絕不重試** — 不要第二個狀態機。

重試階梯 — 有界且必然終止：
```
5xx/timeout 第 1 次 → attempts=1, next = now + 30s
5xx/timeout 第 2 次 → attempts=2, next = now + 120s
5xx/timeout 第 3 次 → 啟發式（不會失敗）→ 寫入標籤, state = 1
429 latch / 4xx latch → 立即啟發式 → state = 1
Ok(vec![]) 或所有 slug 都不在表內 → 啟發式 → state = 1（絕不留在待處理）
```

Insert/回覆的競態：handler 必須先 insert（才能拿到 id 做鍵盤），此時還不知道訊息 id。
三個便宜的雙重保險，不新增欄位 — insert 時設 `tag_next_attempt_at = now + 3`；
worker 提交後重讀 `notify_message_id`，若仍為 `NULL` 就跳過編輯；
handler 存好訊息 id 後重讀 `tag_state`，若 worker 已完成就自己直接渲染最終文字。

測試 — 這個切片裡價值最高的部分：待處理列 → 寫入標籤且有記錄到編輯；`Gone` → 該列仍為終態且
`notify_message_id` 被清空；`Ok(vec![])` → 以 `other` 終結而非留在待處理；tagger 連錯 3 次 →
退回啟發式、終結、且**恰好** 3 次嘗試（防無限迴圈）；`notify_message_id IS NULL` → 寫入標籤、
零次編輯、不報錯；連跑兩次 `run_once` → 第二輪不標任何東西（重啟冪等）。Gemini 測試沿用
`scheduler.rs:300` 的 `spawn_single_response_server` 模式（複製過去，不要新增 mock-HTTP dev-dependency），
其中一個要驗證 `400` 會設 latch，使第二次呼叫**完全不發** HTTP 請求；另加一個純測試驗證序列化後的
body 含 `responseMimeType` 與 `enum` 陣列、且**不含** `thinkingBudget`。

---

## Step 4 — 設定

一個嵌套 struct，`[bookmark.ai]`。不做 `[bookmark] enabled` 旗標（一個「你要嘛就做了要嘛就沒做」的
功能不需要全域開關）。

```toml
[bookmark.ai]
provider    = "auto"    # "auto" | "gemini" | "heuristic"
api_key     = ""
model       = "gemini-3.1-flash-lite"
endpoint    = "https://generativelanguage.googleapis.com"
daily_quota = 200       # 保守軟護欄，非官方數字；請到 AI Studio 查自己的額度後調整
max_rpm     = 10        # 同上，保守值
max_tags    = 3
page_size   = 5
```

預設 `provider = "auto"`（有 `api_key` 就走 gemini，否則啟發式）而不是預設 `"gemini"` —
一個設成 `"gemini"` 卻默默變成啟發式的設定，是會說謊的設定。
`AiProvider` 需要 `FromStr` 才能配合 `set_parse`，比照 `MessageMode`（`config.rs:152-162`）。
這個 struct 需要 `Serialize, Deserialize, PartialEq, Eq, Clone, Debug` + `#[serde(default)]`
加上手寫的 `Default`，比照 `FetchConfig`。

**每個欄位都要在 `apply_env_overrides` 裡自己補一行**
（`FLOWERSS_BOOKMARK_AI_PROVIDER`、`_API_KEY`、`_MODEL`、`_ENDPOINT`、`_DAILY_QUOTA`、`_MAX_RPM`、
`_MAX_TAGS`、`_PAGE_SIZE`），另外在 `FLOWERSS_` 那個沒設時支援裸的 `GEMINI_API_KEY`。
現有的 `env_overrides_cover_all_config_keys_without_file` 測試（`config.rs:228`）**必須擴充**，
否則它的名字就成了謊言。

**型號選擇（2026-08-06 查證）：**

| 型號 | 狀態 | 判斷 |
|---|---|---|
| `gemini-3.1-flash-lite` | Stable / GA，2026-05-07 釋出，官方 deprecation 頁「無停用日期」 | **預設** |
| `gemini-3.5-flash-lite` | 2026-07-21 釋出，無停用日期 | 可選的更新選擇，但太新，先不當預設 |
| `gemini-2.5-flash-lite` | 官方 deprecation 頁仍寫「無停用日期」，但已落後一個世代，且 2026-07-09 有 `generativelanguage.googleapis.com` 回 404「no longer available」的社群回報 | **不當預設**；仍可透過設定指定 |
| `gemini-2.0-flash*` | 2026-06-01 前停用，建議換 `gemini-3.6-flash` | 不要用 |

Flash-Lite 系列是正確的等級：這是一個「讀標題輸出 1-3 個分類」的任務，不需要 Flash 或 Pro，
而 Flash-Lite 的免費額度在各代都是最寬的。**注意官方 rate limits 頁已不再列出逐型號的免費層數字**，
所以上面 config 的 `daily_quota` / `max_rpm` 是保守猜測而非官方值 —
README 要寫明請到 AI Studio 查詢後調整，並說明 429 latch 才是真正的保護。

---

## Step 5 — i18n：`crates/bot/src/bot/i18n.rs`（新增）

把 `Lang`（`runtime.rs:31-135`）原封不動搬出已經 768 行的 `runtime.rs`；從 `runtime` 用 `pub use`
re-export，讓 `keyboard.rs:3` 和 `callbacks.rs:17` 繼續編譯通過，之後再用一個 follow-up commit
修掉那兩個 import 並移除 re-export（永久留一個別名，正是 `runtime.rs` 變這麼大的原因）。

書籤需要約 35 個字串。維持「一個字串一個方法」— 每個 `match` 對 `Lang` 都是窮盡的，
所以加第三種語言會在每個字串處變成編譯錯誤，這正是我們要的性質，而 HashMap／JSON 目錄會把它丟掉 —
但加一個 `strings!` macro，把每個字串 4 行變 1 行。帶參數的那幾個
（`interval_updated` 與兩個新的 `format!` 字串）保持手寫。同時把新指令加進 `Lang::help` 的兩個分支
（help 本來就已超出 Go 範圍，不屬凍結介面）。

測試 — `Lang` 的第一批測試：`from_value(None) == ZhTw`、`from_value(Some("de")) == ZhTw`、
往返轉換，以及一個 smoke test 驗證兩種語言下所有書籤字串都不是空的。

---

## Step 6 — 推播路徑：🔖 按鈕

**`sender.rs`** — 在唯一那個方法上加第 4 個參數：
```rust
async fn send_text(&self, chat_id: i64, text: &str, options: SendOptions,
                   reply_markup: Option<InlineKeyboardMarkup>) -> anyhow::Result<SendOutcome>;
```
不做成 `SendOptions` 的欄位（它在 `sender.rs:6-11` 是 `Copy`，而 `InlineKeyboardMarkup` 不是），
也不新增第二個 `send_text_with_markup` — 一個有預設實作的變體必然導致有人呼叫舊的、🔖 就無聲消失，
而這正是這個 codebase 已經有的 bug 類型。影響面是 6 處機械式修改：trait、`NoopSender`、
`TeloxideSender`（`send` closure 在 429 重試時會跑第二次，所以要 **clone** markup）、
`RecordingSender`、`scheduler.rs:215`、`runtime.rs:557`。沒有任何測試斷言需要改 —
`RecordedSend` 只是多一個欄位，而 `scheduler.rs:466-471` 的測試只用 `.len()` 和逐欄位存取。

**`sender.rs`** — 另外加一個**獨立**的編輯 trait，讓 worker 保持可測：
```rust
pub struct EditOptions { parse_mode, disable_web_page_preview, reply_markup }  // Clone，不需要 Copy
pub enum EditOutcome { Edited, NotModified, Gone }
#[allow(async_fn_in_trait)]
pub trait MessageEditor: Send + Sync {
    async fn edit_text(&self, chat_id: i64, message_id: i32, text: &str, options: EditOptions)
        -> anyhow::Result<EditOutcome>;
    async fn edit_markup(&self, chat_id: i64, message_id: i32,
                         markup: InlineKeyboardMarkup) -> anyhow::Result<EditOutcome>;
}
```
`reply_markup` 必須放在 `EditOptions` 上，因為 `editMessageText` 不重送鍵盤就會**把它清掉**。
`MessageId` 是 `MessageId(i32)`，所以在 DB 邊界要轉換。實作在 `TeloxideSender` 上
（重用它的 `SendRateLimiter` — 編輯同樣計入 Telegram 限制），並用**型別化**的 `ApiError` 比對，
而不是舊的 `message.contains("Forbidden")` 字串檢查（那是凍結的 Go parity，新程式碼不受其約束）：
`MessageToEditNotFound` / `MessageCantBeEdited` / `BotBlocked` → `Gone`；
`MessageNotModified` → `NotModified`（算成功）；`RetryAfter` → sleep 後重試一次。
`ApiError` 是 `#[non_exhaustive]`，所以必須有 `_` 分支。加上 `test_support::RecordingEditor`。

**`crates/bot/src/bot/broadcast.rs`（新增）** — 把 `broadcast_item`（`scheduler.rs:191-230`）
的逐訂閱者主體抽成 `send_item_to_chat(sender, config, chat_id, &ItemForChat, sub, bookmark_button)`。
scheduler 和 `handle_check` 都呼叫它，🔖 就不可能只掛在其中一邊 — 這是約 30 行的真實去重，
也是「怎麼涵蓋 `handle_check` 而不加深重複」的答案。`broadcast_item` 保留名稱與簽章形狀，
所以 `scheduler.rs:429-475` 的測試繼續通過。**不要**試圖把整個 180 行的 `handle_check`
pipeline 和 `run_once` 合併 — 那是另一個重構，風險真實而對書籤沒有收益。

**每聊天室開關** — 在 `run_once` 開頭做一次查詢，不用 memo：
```rust
// 每輪一次讀取，而不是每 (訂閱者 × 項目) 一次。只有 opt-out 的資料列，
// 所以集合很小；沒有資料列就代表啟用。
let bm_off: HashSet<i64> = self.repo.chat_ids_with_option_off("tg-kl-vault:bmbtn:").await?;
```
`HashMap` memo 得穿過兩個 `&self` 方法傳 `&mut`（會改掉 `broadcast_item` 的簽章與它的測試）
或藏一個 `Mutex` 欄位。這個旗標是 opt-out 且預設開，所以真正要記的只有寥寥幾個聊天室。

新測試：`broadcast_item_attaches_bookmark_button_unless_chat_opted_out` — 兩個訂閱者、一個已關閉，
斷言 `RecordedSend.reply_markup` 一邊是 `Some` 一邊是 `None`。

---

## Step 7 — Callback 協定

純冒號慣例、`bm:` 命名空間，完全不碰凍結的二進位格式。**標籤身分在 wire 上走索引，
在 DB 與 UI 走 slug** — slug 走 wire 的話，標籤切換鍵的 payload 會是 65 bytes，超過上限 1 byte；
用索引同時讓 slug 可以改名而不使對話紀錄裡的按鈕失效。

| callback_data | 最壞 bytes |
|---|---|
| `bm:add:<hash8>` | 15 |
| `bm:list:<scope>:<page>`（`scope` = `a` \| `t<idx>`） | 16 |
| `bm:view:<id>:<scope>:<page>` | 36 |
| `bm:del:…` / `bm:delok:…` / `bm:retag:…` / `bm:note:…` | ≤ 37 |
| `bm:tt:<id>:<idx>:<scope>:<page>` | 37 |
| `bm:tags:<page>` / `bm:export` | ≤ 13 |

每個字串都由 `bookmarks.rs` 裡一個私有的 `mod cb` 產生（`cb_view(id, scope, page)` 等）—
絕不 inline `format!`，否則位元組測試就被繞過了。兩個測試：所有最壞情況 payload 都 ≤ 64 bytes
且為 ASCII；以及分類表只可追加的 golden。`bm:noop` **砍掉** — 頁碼指示按鈕要嘛是未回應的 query
（轉圈）、要嘛是看得見的空操作，而標頭已經帶了 `第 2/7 頁`。

Dispatch 用**切片比對**，結構性地消滅 `callbacks.rs:174` 那個前綴順序陷阱：
```rust
match rest.split(':').collect::<Vec<_>>().as_slice() {
    ["list", scope, page] => …,  ["view", id, scope, page] => …,  ["tt", id, idx, scope, page] => …,
    _ => respond_toast(bot, query, lang.bm_bad_action()).await,
}
```

---

## Step 8 — 授權

> **書籤的 callback_data 不攜帶任何擁有者與聊天室 id。擁有的聊天室一律從
> `query.message.chat().id` 推導，每一次讀寫在 SQL 裡都是 `WHERE chat_id = ?`。**

這比舊機制**更強**：被轉發或偽造的按鈕只能作用在該訊息實體所在的聊天室，而要作用還得能在那裡發言。
wire 上沒有任何識別資訊，所以沒有東西可以被偽造。

| 情境 | 讀取／新增 | 刪除／重新標籤／備註 |
|---|---|---|
| 私訊 | 允許（`chat_id == user_id`，退化成每使用者） | 允許 |
| 群組／超級群組 | 任何成員 — 這正是決策 #3 的用意 | `query.from.id == created_by`，否則呼叫一次 `get_chat_member` 允許 `Administrator`/`Owner`，再否則 toast |
| 頻道 | 僅作為推播目標；v1 不在頻道貼文上掛 `bm:` 按鈕 | 不適用 |

`get_chat_member` 只在非建立者按下破壞性操作時才發 — 一次往返，遠在
`answerCallbackQuery` 約 15 秒的預算之內。這就是 `callbacks.rs:112-116` 當初延後的
`getChatAdministrators` 路徑；在新介面上做，是刻意且有文件記載的改進。

`is_authorized` 與 `handle_unsub_feed_item` 缺少的檢查**維持不動**（凍結的 Go parity），
並在新的 `bookmark_auth` 上加註解說明為何不同，免得有人去「統一」它們。
順帶一提：`feed_item_list_keyboard` 傳的是 `msg.chat.id.0`（`runtime.rs:311,405`），而
`is_authorized` 比的是 `from.id`，所以在群組裡兩者永遠不相等，整個 `/set` 樹在群組裡其實早就死了。
不在本次範圍內；這也正是書籤要自己定規則的理由。

---

## Step 9 — 畫面

新增 `crates/bot/src/bot/pagination.rs`：
```rust
pub struct Page { pub index: usize, pub per_page: usize, pub total: usize }
impl Page { pub fn clamped(requested, per_page, total) -> Self; /* 另有 offset/limit/total_pages/has_prev/has_next */ }
pub fn nav_row(page: &Page, cb: impl Fn(usize) -> String, lang: Lang) -> Vec<InlineKeyboardButton>;
```
**讀取時就夾住範圍，是唯一能一次消滅所有過期頁面問題的規則**：過期的 `bm:list:a:47` 會落在
最後一個真實頁面；刪掉最後一頁的最後一筆會退回上一頁；`total == 0` 則渲染空狀態。
`nav_row` 只渲染有去處的箭頭。純函式、無 async、不碰 `Bot` — 完全可單元測試，
這在一個 `runtime.rs`、`callbacks.rs`、`keyboard.rs` 零測試的 codebase 裡很重要。

`feed_item_list_keyboard` 同樣缺分頁的 bug（它在 Telegram 的 100 顆按鈕上限就爆，
而那是很現實的訂閱數量）**不在本次範圍**：它是 Go-parity 畫面，凍結的
`Attachment{user_id, source_id}` 沒有空間放頁碼。在 `nav_row` 上註明後續很好接：
該頁切片仍發舊格式的逐來源按鈕，另外附一列純字串的導覽列由新分支處理，
`callback.rs` 零改動。

所有書籤訊息都用 `ParseMode::Html` + `no_preview()`（`runtime.rs:738`）— **編輯時也要**，
否則每次翻頁都會重新觸發連結預覽卡。`edit_plain`（`callbacks.rs:128`）兩者都沒設，
所以書籤需要自己的 `edit_page` helper，同時吞掉良性的編輯錯誤：

```rust
/// 編輯結果與原內容逐位元組相同時 Telegram 會回 400。對連點兩次
/// 或重複投遞的 update 而言那是空操作，不是錯誤。
fn is_benign_edit_error(err) -> bool  // MessageNotModified | MessageToEditNotFound | MessageCantBeEdited
```
這在今天就是個實際 bug：連點兩次 `settings:opml` 會產生完全相同的內容 → `MessageNotModified`
→ handler 報錯，而因為 `settings:` 的導覽分支從不呼叫 `answerCallbackQuery`
（`callbacks.rs:149-198`），使用者只會看到轉圈。

**列表頁** — `[17]` 是 DB id，不是 1-5 的序號：絕對序號每次新增／刪除都會位移，
而 id 才是使用者打進 `/bmnote 17 …` 的那個 token。它也符合本專案慣例
（`/list` 渲染成 `[[3]]`，`render.rs:102`），視覺上又不會和 `#tech` 撞。
```
🔖 <b>書籤</b> · 共 34 筆 · 第 2/7 頁

[17] <a href="…">Designing Data-Intensive Applications 的十年之後</a>
martinkleppmann.com · #tech #ai · 07-28
📝 第三章值得重讀

[18] <a href="…">A tour of Rust's async runtimes</a>
without.boats · #tech · 07-27
```
```
第 1 列：[17] [18] [19] [20] [21]      → bm:view:<id>:<scope>:<page>
第 2 列：[◀ 上一頁]      [下一頁 ▶]      → bm:list:<scope>:<page∓1>（只有一頁時整列省略）
第 3 列：[🏷 標籤]  [⬇ 匯出]             → bm:tags:<page> / bm:export
```
標題 escape 後截到 70 字元，且必須切在 **`char_indices` 邊界**上；空標題 → 本地化的
`未命名`。Host 取 `Url::host_str` 去掉開頭的 `www.`，無法解析就連分隔符一起省略。
標籤加 `#` 前綴、最多 3 個、沒有則 `未分類`。備註列僅在有值時出現、60 字元。日期用
`get(5..10)` 取 `MM-DD`。5 張卡約 1 KB，遠低於 4096。空狀態：提示文字 + 只留第 3 列。

**詳細頁** — 完整標題；`🔗 host`、`🏷 標籤`（`tag_state = 0` 時顯示 `⏳ 標籤處理中…`，
這是讓使用者不必多一則訊息就理解標籤是非同步的方式）、有備註才有 `📝`、`🕘 日期`、
`📰 source_title` 僅來自 feed 的書籤才有。按鍵列：`[🏷 標籤] [📝 備註]` /
`[🗑 刪除]`（自己一列，降低誤觸 標籤 旁邊的機率）/ `[◀ 返回列表]`。

**刪除確認** — 就地編輯詳細頁訊息（跟隨 `render_and_edit_setting`，而不是
`unsuball_confirm_keyboard` 那種另發一則訊息的形狀）：`[確認刪除] [取消]`。確認後：刪除 →
toast → 以 `Page::clamped(page, per_page, new_total)` 重新渲染列表。

**標籤切換頁**（`bm:retag:…`）— 整個分類表以每列 3 顆的網格呈現，已選的加 `✅`，
各自為 `bm:tt:<id>:<idx>:<scope>:<page>`，切換後就地重繪。標籤文字就是原始英文 slug，
依決策 #4。**因為分類表固定而且小，標籤永遠不需要打字。**

**標籤索引頁**（`bm:tags:<page>`）— 每個標籤的計數加上 `未分類`，只有計數 > 0 才給按鈕，
各自 → `bm:list:t<idx>:0`。

**`/settings` → 書籤 子選單** — `settings_keyboard` 加第 4 列 → `settings:bm`，
整個子樹保留在 `settings:` 前綴之下但委派進 `bookmarks.rs`：
`[🔖 推送書籤按鈕：開]` → `settings:bm:btn`、`[🤖 AI 自動標籤：開]` → `settings:bm:ai`、
`[⬇ 匯出書籤]` → `settings:bm:export`、`[返回]`。設定鍵沿用現有慣例（`runtime.rs:718`）：
`tg-kl-vault:bmbtn:{chat_id}`、`tg-kl-vault:bmai:{chat_id}`，沒有資料列時都預設開。
切換要就地重繪**並且回應 callback query**。匯出比照 `export_chat_opml`（`runtime.rs:656`）—
`InputFile::memory(...).file_name("bookmarks_<ts>.md")`，**只有 Markdown**，依標籤分組，
`- [title](url) — #tech #ai — note`。沒有重新匯入的需求，所以不做 JSON、不做格式選項。

---

## Step 10 — 指令與 handler

`crates/bot/src/bot/bookmarks.rs`（新增）放指令 handler、`bm:` dispatch，以及 `settings:bm*`
的委派。`crates/bot/src/bookmark/{mod,render}.rs` 放純渲染。

| 指令 | 選單 | 說明 |
|---|---|---|
| `/bm [url]` | 顯示 | 主要新增路徑；沒帶參數但是**回覆**某則訊息時，從該訊息抽出 URL |
| `/bookmarks` | 顯示 | 主要閱讀路徑 |
| `/bmsearch <q>` | 顯示 | 前 10 筆，不分頁 |
| `/bmnote <id> <text>` | 隱藏 | 自由文字無法避免；由 📝 按鈕引導發現 |
| `/bmtag <id> <slug…>` | 隱藏 | 標籤切換頁才是真正的 UI；保留給進階使用者 |
| `/bmdel <id>` | 隱藏 | 🗑 按鈕加確認才是真正的 UI；保留給進階使用者 |

隱藏 = `#[command(description = "")]`，沿用 `/ping` 的前例（`commands.rs:30`）—
打得出來但不進 `/` 選單，所以 14 + 3 個顯示項目仍然掃得完。
`/bmtags` **砍掉**：純導覽，永遠從列表底部進入。

**從回覆抽 URL 必須掃 message entity**，不能對文字下 regex：這個 bot 自己的推播是用
`<a href>` 渲染連結的，所以看得見的文字是**標題**，URL 只存在於
`MessageEntityKind::TextLink` 裡。按 offset 取第一個，`TextLink` 或 `Url` 皆可，
退而對 text/caption 做樸素的 `https?://` 掃描。掃 entity 也順便避開了經典的結尾 `)` 被吞進網址的 bug。

**一鍵 🔖 流程**：點擊 → 以 `hash_id` 解析 `contents` → 插入書籤
（`notify_kind = 1`、`notify_message_id` = 該推播訊息）→ **先回應 toast**，再把按鈕改標為
`🔖 已收藏` 並指向 `bm:view:<id>:a:0`。worker 完成後再透過 `edit_markup` 改標一次為
`🔖 #tech #ai` — 標籤文字要上限約 30 字元，否則用戶端會從標籤中間截斷。
callback handler 內**絕不** `await` AI 呼叫（約 15 秒的 `InvalidQueryId` 預算），
也絕不讓正確性依賴它：DB 資料列才是事實來源，按鈕文字只是裝飾。

因為 `prune_contents` 會刪 `contents`，在一個月前的推播上按 🔖 可能什麼都找不到。
誠實處理 — toast `此項目已過期，請改用 /bm <網址>` 並且不動那顆按鈕 —
並且**在插入時就把 `title`/`url`/`source_title` 反正規化存下來**，讓任何書籤都不需要
`contents` 才能渲染。（可選的後續：讓 `prune_contents` 保留已被收藏的 hash。）

`main.rs` — 建立 tagger、在 `shutdown` receiver 的**第三**個 clone 上 spawn
`TagWorker::run_until_shutdown`，並把 `try_join!`（`main.rs:78`）擴成三個 task
（解構會跟著改）。`--dry-run` 時整個跳過 worker（`main.rs:36-47`）。

測試：從手工建構的 entity 陣列抽 URL（只有 TextLink／只有裸 Url／兩者都有 → 依 offset 取第一個／
都沒有 → `None`／用 caption 而非 text）；scope 與 page 的往返解析；畸形 callback data 落到
fallback 分支；`is_benign_edit_error` 的分類；以及一個 `handle_callback` 路由測試驗證
`bm:` 前綴的字串永遠不會被送進 `decode_telebot_callback` — 那個檔案有史以來第一個測試。

---

## Step 11 — 渲染與 escape

`crates/bot/src/bookmark/render.rs` 開頭放：
```rust
//! 與 `bot/render.rs` 不同 —— 那裡的樣板為了 Go parity 而逐位元組凍結、
//! 且刻意不做 escape —— 這裡是新介面，必須 escape 每一個來自 feed 的字串，
//! 包含用在 href 屬性裡的 URL。
```
使用 `teloxide::utils::html::escape`。注意 `html::link(url, text)` 會 escape *text* 但**不會**
escape *url*，而 feed 的 URL 常常含 `&` — href 也要 escape。單一個未 escape 的 `&` 就會觸發
`can't parse entities`，而 `sender.rs:99-103` 只是 log，也就是訊息無聲遺失。
`note` 上限約 1000 字元，讓渲染後的書籤維持在 4096 之下。

比照本專案風格的精確字串測試（`render.rs:134-189`）：完整卡片、無備註、無標籤、無法解析的 URL、
CJK 在字元邊界截斷，以及標題含 `<script>&"` — 那個 `render.rs` 不可能有的 escape 測試。

Clippy 帶 `-D warnings`：`too_many_arguments`（門檻 7）會在 `render_list_page` 和
`send_item_to_chat` 上觸發，所以一開始就用 `ListPageData` / `ItemForChat` struct 設計，
不要伸手去拿 `#[allow]`。

---

## 驗證方式

1. **每一步**之後 `cargo test --workspace` 與 `cargo clippy --workspace --all-targets -- -D warnings`
   都要乾淨（CI 跑的正是這兩個，warning 等於錯誤）。
2. 對一份真實 `data.db` 的副本跑 `cargo run -- --dry-run -c config.toml` — 必須仍然報告沒有
   重複推播，以此證明 migration 0004 沒有干擾 ingest 路徑。
3. Migration 檢查：開一份 0004 之前的 `data.db` 副本，確認能遷移且 `/list` 仍正常。
   *陷阱*：`sqlx::migrate!` 是 proc macro，新增檔案可能不會觸發重新展開 —
   如果 0004 看起來沒跑，touch 一下 `crates/bot/src/db/mod.rs`。
4. 實機測試，**先跑啟發式模式**（`provider = "heuristic"`，不給 key）：`/bm <url>` →
   確認訊息立刻出現 → 約 10 秒內該訊息長出標籤。`/bookmarks` 能分頁；
   詳細頁／刪除確認／標籤切換／標籤索引全部就地編輯；連點導覽箭頭兩次確認不會轉圈
   （`MessageNotModified` 那條路徑）。
5. **在寫 `gemini.rs` 主體之前**，先用一次 `curl` 打真實 API 定案兩件事（見 Step 3）：
   `gemini-3.1-flash-lite` 在 REST 上接受的是 `responseSchema` 還是 `responseJsonSchema`，
   以及 `generateContent` 是否仍正常回應。把成功的那份 request body 當成實作的依據，
   並在 `gemini.rs` 註解裡記下查證日期。
6. `provider = "auto"` 加上真的 key：同樣流程，然後看 log 確認每筆書籤只有一次 Gemini 呼叫，
   並確認 `options` 表裡的 `tg-kl-vault:ai:quota` 有累加。
7. 反向路徑：故意填錯 API key，確認只 log 一次 `error!` 且之後每筆書籤都走啟發式、
   **不再發任何 HTTP 請求**；標籤處理中途刪掉確認訊息，確認該列仍會走到 `tag_state = 1`。
8. 🔖 整合：`/check` 與 scheduler 都要掛上按鈕；在 `/settings → 書籤` 關掉後確認新推播沒有按鈕、
   而舊按鈕仍然可用。
9. 群組測試：以非管理員成員新增一筆書籤，確認其他成員讀得到、刪不掉，而管理員刪得掉。
10. 從 `/settings → 書籤 → 匯出書籤` 匯出，人工看一眼 Markdown。

## 交付物

- `docs/superpowers/specs/2026-08-06-bookmarks-ai-tagging-design.md` — 本設計，最先 commit。
- `config.example.toml` + `README.md` — `[bookmark.ai]` 區塊、環境變數名稱、
  搜尋僅 ASCII 不分大小寫的注意事項，以及 `www.`／結尾斜線正規化的權衡。
- `docs/usage.md` — 新指令。**不要**動 `CHANGELOG.md`（那是上游 Go 的 changelog，最後一筆 2019）
  或 `Makefile`（還是 Go 的建置）。

## 明確不在範圍內

修 `feed_item_list_keyboard` 的分頁；把 `handle_check` 與 `Scheduler::run_once` 在送出路徑之外
進一步合併；修 `/set` 在群組壞掉的授權；FTS5 搜尋；搜尋分頁（後續做法是把查詢字串以訊息身分為鍵
存進 `options`，再用 `bm:sq:<page>`）；備註的 force-reply 狀態機；JSON 匯出；重新匯入；
群組內的每使用者書籤範圍。
