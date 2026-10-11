# IBM MQ 接続（C クライアント／TCP）

GnuCOBOL の `CALL 'MQCONN'` などを [IBM MQ の C MQI](https://www.ibm.com/docs/en/ibm-mq/9.4.x?topic=clients-how-client-connects-server) へ渡す。`extras/mq/mqbridge.c` は COBOL 用のコンパクトな copybook を IBM の `MQOD`、`MQMD`、`MQGMO`、`MQPMO` に変換し、IBM の `libmqic_r.so` を実行時にロードする。接続は `MQCONNX` と `MQCNO_CLIENT_BINDING`／`MQCD` で TCP クライアントチャネルを指定する。REST API は使用しない。

対応する呼び出しは `MQCONN`、`MQDISC`、`MQOPEN`、`MQCLOSE`、`MQPUT`、`MQPUT1`、`MQGET`。IBM の C ヘッダーやバイナリはリポジトリに含めない。[MQ クライアント本体の再配布条件](https://www.ibm.com/docs/en/ibm-mq/9.4.x?topic=overview-redistributable-mq-clients)は IBM が定め、`mqbridge.c` は [別個の MIT ライセンス](../../extras/mq/LICENSE)で提供する。GnuCOBOL 本体の GPL の適用範囲を MQ クライアント本体へ拡張するものではない。

## MQ サーバー

Docker Desktop と WSL を用意し、PowerShell でテスト用パスワードを作成してサーバーを起動する。

```powershell
New-Item -ItemType Directory -Force extras/mq/secrets | Out-Null
[guid]::NewGuid().ToString('N') | Set-Content -NoNewline extras/mq/secrets/app-password.txt
[guid]::NewGuid().ToString('N') | Set-Content -NoNewline extras/mq/secrets/admin-password.txt
docker compose -p gnucobol-mq -f extras/mq/compose.yaml up -d --build
docker compose -p gnucobol-mq -f extras/mq/compose.yaml ps
```

`QM1` のリスナー 1414/tcp はホストの `127.0.0.1:1415` に公開する。開発用 `DEV.APP.SVRCONN` チャネルと CardDemo 用キューを使う。REST 用ポートは公開しない。Compose の設定は [IBM MQ コンテナー](https://github.com/ibm-messaging/mq-container/blob/master/docs/usage.md)の開発構成に従う。

## C クライアントとビルド

WSL の Linux x86-64 に、[IBM 公式の MQ C redistributable client](https://www.ibm.com/docs/en/ibm-mq/9.4.x?topic=linux-redistributable-clients)を取得して展開する。9.2 以降の再配布クライアントにはビルド用ヘッダーも含まれる。展開先に `inc/cmqc.h`、`inc/cmqxc.h`、`lib64/libmqic_r.so` があることを確認する。IBM のダウンロードと使用条件に従う。

検証で使った 9.3.0.4 の例:

```sh
mkdir -p "$HOME/IBM/MQ"
curl -fL https://public.dhe.ibm.com/ibmdl/export/pub/software/websphere/messaging/mqdev/redist/9.3.0.4-IBM-MQC-Redist-LinuxX64.tar.gz -o /tmp/ibm-mq-c-client.tar.gz
tar -xzf /tmp/ibm-mq-c-client.tar.gz -C "$HOME/IBM/MQ"
```

以下は WSL でリポジトリのルートから実行する例。 `MQ_CLIENT_HOME` は実際の展開先に合わせる。

```sh
export MQ_CLIENT_HOME="$HOME/IBM/MQ"
export MQ_CLIENT_LIB="$MQ_CLIENT_HOME/lib64/libmqic_r.so"
export LD_LIBRARY_PATH="$MQ_CLIENT_HOME/lib64:$PWD/build/libcob/.libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export MQ_PASSWORD_FILE="$PWD/extras/mq/secrets/app-password.txt"
sh extras/mq/build-carddemo.sh
export COB_PRE_LOAD="$PWD/build/mq/libcobmq.so"
build/mq/mqdemo
python3 extras/mq/test_mqi_binary.py build/mq/libcobmq.so extras/mq/secrets/app-password.txt
python3 extras/mq/test_carddemo.py build/mq/CODATE01 build/mq/libcobmq.so build/libcob/.libs extras/mq/secrets/app-password.txt
python3 extras/mq/test_carddemo.py build/mq/COACCT01 build/mq/libcobmq.so build/libcob/.libs extras/mq/secrets/app-password.txt --account --acct-data reference/aws-mainframe-modernization-carddemo/app/data/ASCII/acctdata.txt
```

`COBC_BIN`、`LIBCOB_DIR`、`MQ_BUILD_DIR` でコンパイラと出力先を変更できる。既存ビルドの libtool ラッパーが再リンクできない環境では、`COBC_BIN="$PWD/build/cobc/.libs/cobc"` を指定する。

## 接続設定

| 変数 | 既定値 | 内容 |
| --- | --- | --- |
| `MQ_CONN_NAME` | `localhost(1415)` | TCP ホストとポート |
| `MQ_CHANNEL` | `DEV.APP.SVRCONN` | SVRCONN チャネル |
| `MQ_QMGR_NAME` | `QM1` | キューマネージャー |
| `MQ_USER` | `app` | 接続ユーザー |
| `MQ_PASSWORD_FILE` / `MQ_PASSWORD` | なし | MQCSP に渡すパスワード |
| `MQ_CLIENT_LIB` | `libmqic_r.so` | IBM C クライアントライブラリのパス |

`MQCONN` と `MQOPEN` は実際の MQI を呼ぶため、その時点で接続や権限のエラーを返す。CICS 参照資材のように接続ハンドルが 0 の場合、ブリッジは最初の MQI 呼び出しでクライアント接続を作る。メッセージはバイト列のまま送受信される。 `MQGET` は相関 ID・メッセージ ID・待機時間を扱い、空キューでは 2033 を返す。受信バッファが短い場合は MQI の 2080 を返し、メッセージはキューに残る。

COBOL 参照資材のコンパクトな copybook のみを変換する。CICS の単位作業との連動はまだないため、`MQGMO_SYNCPOINT` と `MQPMO_SYNCPOINT` は `NO_SYNCPOINT` に、コンテキスト継承の `MQOO` フラグは通常のオープンに置き換える。分散トランザクションが必要な処理では別途同期点の実装が必要。

テストの詳細は [MQ テストケース](../testcases/mq-carddemo.md)を参照。
