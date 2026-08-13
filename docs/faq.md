

### 日誌中大量類似於 `Create telegraph page error: FLOOD_WAIT_7` 的提示。  

原因是建立 Telegraph 頁面請求過快觸發了 API 限制，可嘗試在設定檔中新增多個 Telegraph token。 


### 如何申請 Telegraph Token？ 

如果要使用應用內即時預覽，必須在設定檔中填寫 `telegraph_token` 設定項目，Telegraph Token 申請指令如下：  
```bash
curl https://api.telegra.ph/createAccount?short_name=flowerss&author_name=flowerss&author_url=https://github.com/indes/flowerss-bot
```

回傳的 JSON 中 access_token 欄位值即為 Telegraph Token。


### 如何取得我的 Telegram ID？
可以參考這個網頁取得：https://botostore.com/c/getmyid_bot/