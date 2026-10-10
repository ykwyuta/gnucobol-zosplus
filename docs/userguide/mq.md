# IBM MQ 接続

GnuCOBOL の `CALL 'MQOPEN'` などを `extras/mq/mqbridge.c` の共有ライブラリへ渡し、Docker 上の IBM MQ の [messaging REST API](https://www.ibm.com/docs/en/ibm-mq/9.4.x?topic=messagingqmgrqmgrnamequeuequeuenamemessage-post) で送受信する。対応する呼び出しは `MQCONN`、`MQDISC`、`MQOPEN`、`MQCLOSE`、`MQPUT`、`MQPUT1`、`MQGET`。`extras/mq/copy` に、この対象で必要な最小限の COBOL copybook を置く。IBM の製品 copybook 全体を再配布するものではない。

## 起動

Docker Desktop と WSL、WSL 側の `gcc`、`pkg-config`、`libcurl4-openssl-dev`、`python3`、ビルド済み GnuCOBOL が必要。PowerShell でローカル専用のパスワードを生成する。`secrets` は Git 対象外。

```powershell
New-Item -ItemType Directory -Force extras/mq/secrets | Out-Null
[guid]::NewGuid().ToString('N') | Set-Content -NoNewline extras/mq/secrets/app-password.txt
[guid]::NewGuid().ToString('N') | Set-Content -NoNewline extras/mq/secrets/admin-password.txt
docker compose -p gnucobol-mq -f extras/mq/compose.yaml up -d --build
docker compose -p gnucobol-mq -f extras/mq/compose.yaml ps
```

MQ の 1414 番ポートをホストのループバック 1415 番へ、HTTPS REST の 9443 番をループバック 9444 番へ公開する。キューマネージャーは `QM1`。MQSC で CardDemo 用要求・応答キューとデモキューを作る。コンテナー設定は [IBM MQ container の公式手順](https://github.com/ibm-messaging/mq-container/blob/master/docs/usage.md) と [developer config](https://github.com/ibm-messaging/mq-container/blob/master/docs/developer-config.md) に沿う。

## ビルドと動作確認

WSL でリポジトリのルートから実行する。`COBC_BIN` と `LIBCOB_DIR` はローカルのビルド先に合わせる。`COBC_BIN` を省略すると `build/cobc/cobc`、次に PATH の `cobc` を使う。

```sh
sh extras/mq/build-carddemo.sh
export COB_PRE_LOAD="$PWD/build/mq/libcobmq.so"
export MQ_PASSWORD_FILE="$PWD/extras/mq/secrets/app-password.txt"
export MQ_TLS_INSECURE=1
export LD_LIBRARY_PATH="$PWD/build/libcob/.libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
build/mq/mqdemo
python3 extras/mq/test_carddemo.py build/mq/CODATE01 build/mq/libcobmq.so build/libcob/.libs extras/mq/secrets/app-password.txt
python3 extras/mq/test_carddemo.py build/mq/COACCT01 build/mq/libcobmq.so build/libcob/.libs extras/mq/secrets/app-password.txt --account --acct-data reference/aws-mainframe-modernization-carddemo/app/data/ASCII/acctdata.txt
```

`mqdemo` は `MQPUT`、`MQPUT1`、`MQGET`、空キューの 2033 を確認する。CardDemo の 2 本は参照資材の COBOL を CICS 前処理してコンパイルし、要求キューへ投入したメッセージを取り出して応答キューへ返す。口座照会では参照資材の `acctdata.txt` を固定長レコードとして読み込む。結果の詳細は [MQ テストケース](../testcases/mq-carddemo.md) を参照。

標準では `https://localhost:9444`、`QM1`、`app` を使う。接続先は `MQ_REST_URL`、`MQ_QMGR_NAME`、`MQ_USER`、パスワードは `MQ_PASSWORD_FILE` または `MQ_PASSWORD` で変更できる。ローカルの自己署名証明書を使う場合のみ `MQ_TLS_INSECURE=1` を設定する。それ以外は `MQ_CA_BUNDLE` で信頼する CA 証明書を指定する。

## 対応範囲

このブリッジは REST API が扱う UTF-8 テキストメッセージ用。`MQCONN` と `MQOPEN` はローカルハンドルを用意し、実際の接続・権限・キューのエラーは最初の `MQPUT` / `MQGET` 時に判定される。`MQGET` の待機時間とメッセージ記述子の ID・相関 ID・返信先の一部を扱う。空キューは `MQCC_FAILED` / `MQRC_NO_MSG_AVAILABLE (2033)` を返す。

REST 経由のメッセージ操作には MQI の同期点、rollback、完全な `MQGMO` / `MQPMO`、バイナリ・EBCDIC ペイロード、トリガーモニターの意味論を実装していない。短すぎる受信バッファは 2080 を返すが、REST の破壊的取得後なのでメッセージは戻らない。`COBCICS_MQ_TRIGGER_QUEUE` を指定すると CICS `RETRIEVE` 用に簡略 MQTM を渡す。これらの制約があるアプリケーションには IBM のネイティブ MQI ライブラリを使う必要がある。
