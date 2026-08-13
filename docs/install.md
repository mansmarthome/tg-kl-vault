# 部署

## 二進位部署

從 [Releases](https://github.com/indes/flowerss-bot/releases) 頁面下載對應的版本解壓後執行即可。

## Docker 部署

1.下載設定檔
在專案目錄下新建 `config.yml` 檔案


```bash
mkdir ~/flowerss &&\
wget -O ~/flowerss/config.yml \
    https://raw.githubusercontent.com/indes/flowerss-bot/master/config.yml.sample
```


2.修改設定檔

```bash
vim ~/flowerss/config.yml
```

修改設定檔中的 sqlite 路徑（如果使用 sqlite 作為資料庫）：
```yaml
sqlite:
  path: /root/.flowerss/data.db
```

3.執行

```shell script
docker run -d -v ~/flowerss:/root/.flowerss indes/flowerss-bot
```

## 原始碼編譯部署

```shell script
git clone https://github.com/indes/flowerss-bot && cd flowerss-bot
make build
./flowerss-bot
```



## 設定

根據以下模板，新建 `config.yml` 檔案。

```yml
bot_token: XXX
#多個 telegraph_token 可採用陣列格式：
# telegraph_token:
#  - token_1
#  - token_2
telegraph_token: xxxx
user_agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36
preview_text: 0
disable_web_page_preview: false
socks5: 127.0.0.1:1080
update_interval: 10
error_threshold: 100
telegram:
  endpoint: https://xxx.com/
mysql:
  host: 127.0.0.1
  port: 3306
  user: user
  password: pwd
  database: flowerss
sqlite:
  path: ./data.db
allowed_users:
  - 123
  - 234
```

設定說明：

| 設定項目                     | 含義                                      | 是否必填                                       |
| --------------------------| ----------------------------------------- | ------------------------------------------ |
| bot_token                 | Telegram Bot Token                        | 必填                                       |
| telegraph_token           | Telegraph Token, 用於轉存原文到 Telegraph   | 可忽略（不轉存原文到 Telegraph ）          |
| preview_text              | 純文字預覽字數（不借助Telegraph）            |可忽略（預設0, 0為禁用）                    |
| user_agent                | User Agent                                |可忽略                                     |
| disable_web_page_preview  | 是否禁用 web 頁面預覽                       | 可忽略（預設 false, true 為禁用）          |
| update_interval           | RSS 源掃描間隔（分鐘）                      | 可忽略（預設 10）                          |
| error_threshold           | 源最大出錯次數                              |可忽略（預設 100）                          |
| socks5                    | 用於無法正常 Telegram API 的環境            | 可忽略（能正常連線上 Telegram API 伺服器） |
| mysql                     | MySQL 資料庫設定                           | 可忽略（使用 SQLite ）                     |
| sqlite                    | SQLite 設定                               | 可忽略（已設定mysql時，該項失效）          |
| telegram.endpoint         | 自定義telegram bot api url                | 可忽略（使用預設api url）          |
| allowed_users             | 允許使用bot的使用者telegram id，                        | 可忽略，為空時所有使用者都能使用bot          |