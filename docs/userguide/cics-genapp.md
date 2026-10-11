# CICS GenApp をローカルで動かす

`reference/cics-genapp` の IMS 以外の COBOL 31 本を、Docker Compose の Db2 と Rust の TN3270 ホストで動かす手順。原典は変更せず、変換・実行物を `build/genapp` に作る。以下はリポジトリ直下の PowerShell で実行する。

## 準備と一括検証

[Db2 接続ガイド](db2.md)に従い Compose の Db2、`extras/db2sql/.runtime.env`、`gnucobol-db2-build` イメージを準備する。GenApp 専用の `GENAPPTEST` スキーマに JCL の業務8表・初期40行と、元データの KSDS を投入し、ビルドから画面操作まで一括検証する。

新しくクローンした作業ツリーでは、先に `git submodule update --init reference/cics-genapp` を実行して原典を取得する。

```powershell
& extras/cobcics/test-genapp-all.ps1
```

成功時は `GenApp integrated suite passed` と `GenApp IBM-037 screen, SQL and FILE suite passed` が出る。試験は専用スキーマの顧客・保険・KSDS データを初期状態へ復元する。各操作と期待値は [GenApp テストケース](../testcases/cics-genapp-current.md)に記録している。

## wc3270 で画面を開く

次のホストは1ポートで `SSC1` と `SSP1`～`SSP4` を受け付ける。最初の接続画面は顧客メニュー `SSC1`。保険画面を最初に開きたい場合は、`--transid` と `--map` をそれぞれ `SSP1` と `SSMAPP1` などへ変更する。

```powershell
$ws = (Resolve-Path .).Path
$driver = '/usr/local/lib64/python3.9/site-packages'
$runtime = "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/genapp:$driver/clidriver/lib:$driver/ibm_db.libs"
docker run -d --rm --name genapp-terminal `
  --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=GENAPPTEST `
  -e $runtime -p 127.0.0.1:3272:3270 `
  -v "${ws}:/src" -w /src --entrypoint build/genapp/cobcicsd `
  gnucobol-db2-build `
  --bms build/genapp/ssmap-normalized.bms --map SSMAPC1 `
  --transid SSC1 --applid GENAAPP --listen 0.0.0.0:3270 `
  --route SSC1=build/genapp/lgtestc1 `
  --route SSP1=build/genapp/lgtestp1 `
  --route SSP2=build/genapp/lgtestp2 `
  --route SSP3=build/genapp/lgtestp3 `
  --route SSP4=build/genapp/lgtestp4 `
  --map-route 'SSP1=build/genapp/ssmap-normalized.bms#SSMAPP1' `
  --map-route 'SSP2=build/genapp/ssmap-normalized.bms#SSMAPP2' `
  --map-route 'SSP3=build/genapp/ssmap-normalized.bms#SSMAPP3' `
  --map-route 'SSP4=build/genapp/ssmap-normalized.bms#SSMAPP4'
docker logs genapp-terminal
& 'C:\Program Files\wc3270\wc3270.exe' +S -model 3279-2 `
  -tn IBM-3279-2 N:127.0.0.1:3272
```

`SSC1` で顧客番号 `1`、選択 `1` を送ると Andrew Pandy を照会できる。`SSP1` では保険番号 `1`、顧客番号 `2`、選択 `1` で FORD の自動車保険を照会できる。`SSP2` は保険4/顧客8、`SSP3` は保険6/顧客4、`SSP4` は保険9/顧客5が初期データにある。終了後は `docker stop genapp-terminal` を実行する。

## jclsh の CSD・FB VSAM を使う

`jclsh` で原本 JCL を実行すると、CSD の FILE/TRANSACTION 定義と固定長 FB の KSDS を `cobcicsd` と共有できる。次の回帰試験は `cdef121`～`cdef125` と `adef121` を無編集で実行し、CSD の `SSC1`→`LGTESTC1`、FILE 2 件と、225/64 バイトのレコードを検証する。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-utilities.ps1
```

上記の `genapp-terminal` 起動例で、`--route SSC1=build/genapp/lgtestc1` の代わりに以下を指定できる。`--route` で明示した経路は CSD より優先する。`--map-route` は従来どおり画面切り替えに指定する。

```text
--csd build/jclsh/genapp-csd/catalog.dat
--csd-list GENALIST
--dataset-profile extras/jclsh/tests/genapp-vsam.conf
--program LGTESTC1=build/genapp/lgtestc1
--program LGTESTP1=build/genapp/lgtestp1
--program LGTESTP2=build/genapp/lgtestp2
--program LGTESTP3=build/genapp/lgtestp3
--program LGTESTP4=build/genapp/lgtestp4
```

`--check-csd` を付けると待受けせずに解決結果の件数を表示する。起動時には CSD の FILE `DSNAME` とプロファイルの `dataset.<DSN>` を照合し、`dataset_root` 内の実ファイルだけをワーカーに渡す。GenApp の CICS FILE ブリッジは `KSDSCUST` / `KSDSPOLY` に加え、`COBCICS_VSAM_<FILE>` と FB メタデータを持つ任意のキー付きファイルを扱う。非ゼロのキー位置にも対応する。CICS ワーカー間の書き込みは作業単位の終わりまでファイル単位で排他する。更新は同じ作業単位の読み取りにだけ反映し、`EXEC CICS SYNCPOINT` または正常終了で Db2 とともに確定する。`SYNCPOINT ROLLBACK`・ABEND・Db2 確定失敗ではファイル変更を破棄する。現行のローカルバックエンドはファイル全体をメモリへ読み込み、64 MiB を上限とする。Db2 確定後のファイル置換失敗やプロセス・ホスト障害をまたぐ原子的な回復はまだ保証しない。分散二相コミットと回復ログは未実装である。

JCL 生成物を用いた CICS FILE ABI の試験は、`gnucobol-db2-build` イメージで `libgenapp_file.so` をビルドした後に `python3 extras/cobcics/test-genapp-local-vsam.py build/genapp/libgenapp_file.so` を実行する。元データは一時ディレクトリにコピーし、未確定データの読み取り、取り消し、確定を検証する。`test-vsam-db2-uow.c` は Db2 確定失敗時のファイル破棄を、`test-vsam-db2-real.c` は Docker Compose の実 Db2 と FB ファイルを同時に更新する確定・取り消しを検証する。

これらの作業単位試験は、上記の jclsh 回帰試験で FB ファイルを生成した後にまとめて実行できる。

```powershell
$ws = (Resolve-Path .).Path
docker compose --env-file extras/db2sql/.env -f extras/db2sql/compose.yaml up -d
docker run --rm --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=DB2INST1 `
  -v "${ws}:/src" -w /src --entrypoint sh gnucobol-db2-build `
  extras/cobcics/test-local-vsam-uow.sh
docker compose --env-file extras/db2sql/.env -f extras/db2sql/compose.yaml stop
```

## HTTP から業務 COBOL を呼ぶ

HTTP アダプターは `GENAPP-API` ワーカーに元の32,500バイト COMMAREA を渡し、顧客と自動車・生命・住宅・商業保険の元 COBOL を呼び出す。既定はループバック待受け。Docker のポートを公開する場合もホスト側を `127.0.0.1` に限定する。

```powershell
docker run -d --rm --name genapp-http `
  --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=GENAPPTEST `
  -e $runtime -p 127.0.0.1:8088:8088 `
  -v "${ws}:/src" -w /src --entrypoint python3 `
  gnucobol-db2-build extras/cobcics/genapp_http.py `
  --listen 0.0.0.0:8088
Invoke-RestMethod http://127.0.0.1:8088/genapp/customers/1
Invoke-RestMethod 'http://127.0.0.1:8088/genapp/policies/1?customer=2&type=M'
```

POST は `/genapp/customers` と `/genapp/policies/{motor,endowment,house,commercial}`。保険照会の `type` はそれぞれ `M/E/H/C`。生命保険の POST は省略可能な `padding_data` を3,900 ASCII バイトまで受け取る。必須 JSON 項目と期待値は `extras/cobcics/test-genapp-http.py` を参照する。存在しない顧客は404、業務エラーとワーカー失敗は502。HTTP アダプターは認証を持たないローカル検証用である。終了後は `docker stop genapp-http` を実行する。

## IBM-037 データモード

一括検証で `build/genapp-ebcdic/GENAPP` と `lgtestc1`～`lgtestp4` を生成する。個別に再ビルドする場合は、通常ビルド後に次を実行する。

```powershell
$ws = (Resolve-Path .).Path
docker run --rm -e COB_CONFIG_DIR=/src/config -e COB_COPY_DIR=/src/copy `
  -e 'COB_CFLAGS=-I/src -I/usr/local/lib64/python3.9/site-packages/clidriver/include -DCOBCICS_DB2SQL -DCOBCICS_GENAPP' `
  -e 'COB_LDFLAGS=-L/src/build/libcob/.libs -L/src/build/genapp -L/usr/local/lib64/python3.9/site-packages/clidriver/lib -ldb2sql -ldb2 -Wl,-rpath-link,/usr/local/lib64/python3.9/site-packages/ibm_db.libs' `
  --entrypoint sh -v "${ws}:/src" -w /src gnucobol-db2-build `
  extras/cobcics/build-genapp-ebcdic.sh
```

上の TN3270 起動例を IBM-037 版に切り替えるには、`-e COB_HOST_CODESET=UTF-8` を追加し、5つの `--route` の実行ファイルを `build/genapp-ebcdic/lgtestc1`～`lgtestp4` に変更する。Rust ホストと BMS ファイルは通常版と共用する。画面とワーカー間の文字列、Db2 のホスト変数、FILE のキーとレコードは境界で変換される。顧客・保険の照会、追加、削除、更新と競合検出を E01 で確認する。`GENAPP-API` と HTTP 経路はこの IBM-037 ビルドの対象外である。
