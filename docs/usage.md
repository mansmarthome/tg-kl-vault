## 使用

命令：

```
/sub [url] 订阅（url 为可选）
/unsub [url] 取消订阅（url 为可选）
/list 查看当前订阅
/set 设置订阅
/check 检查当前订阅
/setfeedtag [sub id] [tag1] [tag2] 设置订阅标签（最多设置三个Tag，以空格分隔）
/setinterval [interval] [sub id] 设置订阅刷新频率（可设置多个sub id，以空格分隔）
/activeall 开启所有订阅
/pauseall 暂停所有订阅
/import 导入 OPML 文件
/export 导出 OPML 文件
/unsuball 取消所有订阅
/help 帮助
```

### 书签（Bookmarks）

以聊天室为单位的书签库。每则推播讯息下方会出现 🔖 按钮，一键收藏；也可用指令收藏任意网址。收藏后会立即回覆，背景 worker 会自动补上分类标签（预设走 Gemini 免费层，无 API key 时退回本地关键字启发式）后再编辑讯息。

```
/bm [url] 收藏网址（不带参数时，回覆一则含连结的讯息即可收藏该连结）
/bookmarks 分页浏览书签（每页 5 笔，可进详细页编辑／删除／改标签）
/bmsearch [关键字] 关键字搜寻（标题／网址／备注，前 10 笔）
/bmnote [id] [文字] 为书签加备注（也可从详细页的 📝 按钮进入）
/bmtag [id] [slug…] 手动设定标签（标签为固定英文分类，空格分隔）
/bmdel [id] 删除书签（详细页的 🗑 按钮有确认步骤）
```

- **标签为固定英文 slug 分类表**，AI 只能从表中挑选；手动标签可在详细页的「🏷 标签」网格中点选切换。
- **归属为每聊天室**：群组成员共用同一个书签库，任何成员可读取／新增；删除／改标签需为建立者或群组管理员。
- 搜寻使用 SQLite `LIKE`：**仅 ASCII 不分大小写**（中日韩字元区分大小写），且 `%`、`_` 会被当成字面字元。
- 网址正规化会移除常见追踪参数（`utm_*`、`fbclid` 等，但**保留** `ref` 与 `si`），不会移除 `www.` 或结尾斜线 — 因此 `www.x.com/a` 与 `x.com/a` 会是两笔不同书签。
- 在 `/settings → 🔖 书签` 可开关每则推播的 🔖 按钮、开关 AI 自动标签，以及汇出书签（Markdown，依标签分组）。

### Channel 订阅使用方法

1. 将 Bot 添加为 Channel 管理员
2. 发送相关命令给 Bot

Channel 订阅支持的命令：

```
/sub @ChannelID [url] 订阅
/unsub @ChannelID [url] 取消订阅
/list @ChannelID 查看当前订阅
/check @ChannelID 检查当前订阅
/unsuball @ChannelID 取消所有订阅
/activeall @ChannelID 开启所有订阅
/setfeedtag @ChannelID [sub id] [tag1] [tag2]  设置订阅标签（最多设置三个Tag，以空格分隔）
/import 导入 OPML 文件
/export @ChannelID 导出 OPML 文件
/pauseall @ChannelID 暂停所有订阅
```

**ChannelID 只有设置为 Public Channel 才有。如果是 Private Channel，可以暂时设置为 Public，订阅完成后改为 Private，不影响 Bot 推送消息。**

例如要给 t.me/debug 频道订阅 [阮一峰的网络日志](http://www.ruanyifeng.com/blog/atom.xml) RSS 更新：

1. 将 Bot 添加到 debug 频道管理员列表中
2. 给 Bot 发送 `/sub @debug http://www.ruanyifeng.com/blog/atom.xml` 命令
