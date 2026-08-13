## 使用

指令：

```
/sub [url] 訂閱（url 為可選）
/unsub [url] 取消訂閱（url 為可選）
/list 查看目前訂閱
/set 設定訂閱
/check 立刻抓取所有訂閱並推播新文章
/feedcheck 檢查訂閱清單裡的 feed 是否還有效
/setfeedtag [sub id] [tag1] [tag2] 設定訂閱標籤（最多設定三個Tag，以空格分隔）
/setinterval [interval] [sub id] 設定訂閱刷新頻率（可設定多個sub id，以空格分隔）
/activeall 開啟所有訂閱
/pauseall 暫停所有訂閱
/import 匯入 OPML 檔案
/export 匯出 OPML 檔案
/unsuball 取消所有訂閱
/help 幫助
```

**`/check` 和 `/feedcheck` 不一樣**：`/check` 是「立刻去抓，有新文章就推給我」，`/feedcheck` 是「這些訂閱還活著嗎」——只探測、只回報，不會寫入任何東西，也不會推播文章。

`/feedcheck` 會並發探測每個訂閱源，把有問題的排在最前面：

- ❌ 連不上／HTTP 錯誤（404、403、逾時……）
- 🧩 抓得到但不是有效的 RSS/Atom（例如網站改版後返回 HTML）
- 📭 能解析但一篇文章都沒有
- 🪦 還能抓，但最新一篇超過 180 天——可能已停更
- ⏸ 已被暫停抓取（排程器會略過這類源，`/list` 裡也會標出來）

每一列同時並列**排程器記錄的歷史**（連續失敗次數、上次錯誤、上次成功抓取時間）與**當下探測結果**。兩者不一致就是最有用的訊息：「記錄顯示壞掉、現在探測正常」是暫時性故障，「記錄正常、現在 404」則是剛壞。

### 為什麼 `/check` 會推播很久以前的文章

判斷「新文章」靠的是 `contents` 去重帳本（feed 網址 + GUID 的雜湊）。帳本一旦破洞，舊文章就會被當成新的：feed 換了 GUID（換部落格引擎、http→https、網址結尾斜線變動）、帳本被 `retention_days` 清掉、或來源被暫停很久後才重新抓取，都會觸發。

`[fetch] max_item_age_days`（預設 30 天）是第二道閘門：發布日期比這個久的文章一律不推播，但仍會寫進帳本標記為已讀，所以只會被判斷一次。沒有日期的文章無從判斷，維持照送。設 0 可關閉這道閘門。

### 書籤（Bookmarks）

以聊天室為單位的書籤庫。每則推播訊息下方會出現 🔖 按鈕，一鍵收藏；也可用指令收藏任意網址。收藏後會立即回覆，背景 worker 會自動補上分類標籤（預設走 Gemini 免費層，無 API key 時退回本地關鍵字啟發式）後再編輯訊息。

```
/bm [url] 收藏網址（不帶參數時，回覆一則含連結的訊息即可收藏該連結）
/bookmarks 分頁瀏覽書籤（每頁 5 筆，可進詳細頁編輯／刪除／改標籤）
/bmsearch [關鍵字] 關鍵字搜尋（標題／網址／備註，前 10 筆）
/bmnote [id] [文字] 為書籤加備註（也可從詳細頁的 📝 按鈕進入）
/bmtag [id] [slug…] 手動設定標籤（標籤為固定英文分類，空格分隔）
/bmdel [id] 刪除書籤（詳細頁的 🗑 按鈕有確認步驟）
```

- **標籤為固定英文 slug 分類表**，AI 只能從表中挑選；手動標籤可在詳細頁的「🏷 標籤」網格中點選切換。
- **歸屬為每聊天室**：群組成員共用同一個書籤庫，任何成員可讀取／新增；刪除／改標籤需為建立者或群組管理員。
- 搜尋使用 SQLite `LIKE`：**僅 ASCII 不分大小寫**（中日韓字元區分大小寫），且 `%`、`_` 會被當成字面字元。
- 網址正規化會移除常見追蹤參數（`utm_*`、`fbclid` 等，但**保留** `ref` 與 `si`），不會移除 `www.` 或結尾斜線 — 因此 `www.x.com/a` 與 `x.com/a` 會是兩筆不同書籤。
- 在 `/settings → 🔖 書籤` 可開關每則推播的 🔖 按鈕、開關 AI 自動標籤，以及匯出書籤（Markdown，依標籤分組）。
- **AI 後端可選 MCP 遠端 agent**：在 `[bookmark.ai]` 設 `provider = "mcp"` 並填好 `[bookmark.ai.mcp]`（[pi-mcp-bridge](https://github.com/siygle/pi-mcp-bridge) 端點），標籤就改由你自己的 agent 產生；連不上時自動退回本地啟發式。
- **文章摘要**：只要設定了 MCP 端點，每則推播就會多一個 **📝** 按鈕。點下去會請遠端 agent 去抓該文章內容並彙整，稍候以回覆訊息帶出摘要（非同步，長任務也能等）。此按鈕同樣可在 `/settings → 🔖 書籤` 內開關。

### Channel 訂閱使用方法

1. 將 Bot 新增為 Channel 管理員
2. 發送相關指令給 Bot

Channel 訂閱支援的指令：

```
/sub @ChannelID [url] 訂閱
/unsub @ChannelID [url] 取消訂閱
/list @ChannelID 查看目前訂閱
/check @ChannelID 立刻抓取訂閱並推播新文章
/unsuball @ChannelID 取消所有訂閱
/activeall @ChannelID 開啟所有訂閱
/setfeedtag @ChannelID [sub id] [tag1] [tag2]  設定訂閱標籤（最多設定三個Tag，以空格分隔）
/import 匯入 OPML 檔案
/export @ChannelID 匯出 OPML 檔案
/pauseall @ChannelID 暫停所有訂閱
```

**ChannelID 只有設定為 Public Channel 才有。如果是 Private Channel，可以暫時設定為 Public，訂閱完成後改為 Private，不影響 Bot 推送訊息。**

例如要給 t.me/debug 頻道訂閱 [阮一峰的網路日誌](http://www.ruanyifeng.com/blog/atom.xml) RSS 更新：

1. 將 Bot 新增到 debug 頻道管理員列表中
2. 給 Bot 發送 `/sub @debug http://www.ruanyifeng.com/blog/atom.xml` 指令
