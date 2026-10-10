# CICS/BMS 画面を TN3270 で使う

`cobcicsd` は Rust 製の TN3270 ホストである。BMS の物理マップを読み、COBOL ワーカーが返す画面項目と CICS トランザクション ID を端末に表示する。`wc3270` や `c3270` はこのホストに接続するクライアントとして使う。以下は Bank of Z の非 IMS 画面を Windows PowerShell から起動する手順である。

対応する CICS 機能、`EXEC CICS` コマンドと制限は [CICS 対応機能・コマンド一覧](cics-supported-features.md) を参照。
wc3270 の TN3270E、TLS、NVT、IND$FILE、3287 印刷は [wc3270 接続ガイド](wc3270-host.md)を参照。

## 準備とビルド

[Db2 ガイド](db2.md)に従って Db2 を起動し、`.runtime.env` と `gnucobol-db2-build` イメージを用意する。`reference/Bank-of-Z-main/src/base/cics` と、Rust のオフライン依存パッケージ、`rust:1-alpine` イメージも必要である。テスト専用の `BANKZTEST` スキーマと固定データを用意し、画面ワーカーをビルドする。

```powershell
$ws = (Resolve-Path .).Path
$driver = '/usr/local/lib64/python3.9/site-packages'
docker run --rm --env-file extras/db2sql/.runtime.env `
  -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build `
  extras/cobcics/setup-bank-test-db.py
& extras/cobcics/build-bank-screens.ps1
```

ビルドスクリプトは BMS 10 本をシンボリックマップに変換し、IMS 関連を除く COBOL 32 本をコンパイルして、画面ワーカーと `build/bank-business/cobcicsd` を生成する。ビルドログは `build/bank-syntax` に置く。

## ホストを起動して wc3270 で操作する

次の Docker コマンドはポートを Windows の `127.0.0.1:3271` のみに公開する。`--listen 0.0.0.0:3270` はコンテナー内から Docker のポート転送を受けるための指定である。

```powershell
$bankHostArgs = @(
  '--bms', '/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms',
  '--transid', 'OMEN', '--applid', 'BANKAPP',
  '--route', 'OMEN=/src/build/bank-syntax/screens/BNKMENU',
  '--route', 'ODCS=/src/build/bank-syntax/screens/BNKLINK',
  '--map-route', 'ODCS=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1DCM.bms',
  '--route', 'ODAC=/src/build/bank-syntax/screens/BNK1DAC',
  '--map-route', 'ODAC=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1DAM.bms',
  '--route', 'OCCS=/src/build/bank-syntax/screens/BNK1CCS',
  '--map-route', 'OCCS=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1CCM.bms',
  '--route', 'OCAC=/src/build/bank-syntax/screens/BNK1CAC',
  '--map-route', 'OCAC=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1CAM.bms',
  '--route', 'OUAC=/src/build/bank-syntax/screens/BNK1UAC',
  '--map-route', 'OUAC=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1UAM.bms',
  '--route', 'OCRA=/src/build/bank-syntax/screens/BNKLINK',
  '--map-route', 'OCRA=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1CDM.bms',
  '--route', 'OTFN=/src/build/bank-syntax/screens/BNK1TFN',
  '--map-route', 'OTFN=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1TFM.bms',
  '--route', 'OCCA=/src/build/bank-syntax/screens/BNK1CCA',
  '--map-route', 'OCCA=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1ACC.bms',
  '--listen', '0.0.0.0:3270'
)
docker run -d --rm --name bankz-userguide -p 127.0.0.1:3271:3270 `
  --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=BANKZTEST `
  -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" `
  -v "${ws}:/src" -w /src `
  --entrypoint /src/build/bank-business/cobcicsd gnucobol-db2-build @bankHostArgs
docker logs bankz-userguide
& 'C:\Program Files\wc3270\wc3270.exe' +S -model 3279-2 `
  -tn IBM-3279-2 N:127.0.0.1:3271
```

`wc3270` を別の場所にインストールした場合は、実行ファイルのパスを変更する。`N:` と `-tn IBM-3279-2` は、このホストが対応する TN3270 端末タイプを選ぶために使う。メニューでは `1` が顧客照会、`2` が口座照会、`3` が顧客作成、`4` が口座作成、`5` が口座更新、`6` が入出金、`7` が送金、`A` が顧客の口座一覧である。PF3 はメニューへ戻り、PF12 はセッションを終了する。顧客照会画面の PF10 は編集欄を解放する。

終了時に一時ホストを停止する。

```powershell
docker stop bankz-userguide
```

## 画面の自動確認

端末操作と Db2 の行を照合する試験は、専用の `BANKZTEST` データを使い、書き込み後に元へ戻す。ワーカーのビルド後に実行する。

```powershell
docker run --rm --env-file extras/db2sql/.runtime.env `
  -e DB2_SCHEMA=BANKZTEST `
  -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" `
  -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build `
  extras/cobcics/test-bank-screens.py
docker run --rm --env-file extras/db2sql/.runtime.env `
  -e DB2_SCHEMA=BANKZTEST `
  -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" `
  -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build `
  extras/cobcics/test-bank-boundaries.py
```

確認した操作と Db2 の期待値は [画面試験シナリオ](../testcases/bank-of-z-cics-screens.md)を参照する。

## 自分の BMS と COBOL に適用する

`cobcics-mapgen` で BMS からシンボリックマップ `.cpy` を作る。固定形式の COBOL ソースでは `--cics-map=<生成した .cpy>` を指定する。Db2 の埋め込み SQL も使う場合は `--db2-sql` を同時に指定する。変換順は CICS、SQL、通常の COBOL コンパイルである。ソースチェックアウトからコンパイルするときは `COB_CICS_PREPROCESSOR` と `COB_DB2_SQL_PREPROCESSOR` に各 Python スクリプトの絶対パスを設定する。生成したマップ、`extras/cobcics/copy`、アプリケーションの COPY を `-I` で探索させ、ワーカーには `extras/cobcics/bridge.c` と呼び出す COBOL プログラムをリンクする。Db2 を使うワーカーには `libdb2sql` と CLI ドライバーも必要である。具体的なビルド例は `extras/cobcics/build-bank-screens.ps1` にある。

TN3270 ホストには最初の BMS を `--bms` で渡し、トランザクションごとに `--route ID=ワーカー` と `--map-route ID=BMS` を設定する。Enter/Clear/PF キー、Read Modified、Model 2～5 と拡張画面、色・強調表示を扱える。TN3270E や TLS などの起動オプションは [wc3270 接続ガイド](wc3270-host.md)を参照する。IMS、すべての CICS コマンドと BMS オプションには対応しない。日本語の入力欄には SO/SI 入力制御の拡張属性を付ける。未対応構文は変換時にソース行を示してエラーとなる。

## 日本語 EBCDIC の画面入出力

`cobcicsd --charset IBM-1399` を指定すると、画面の UTF-8 文字列を IBM-1399 の SBCS/DBCS 混合データへ変換し、端末からの SO/SI 付き入力を UTF-8 へ戻して COBOL ワーカーに渡す。`IBM-930`、`IBM-939`、`IBM-1390` も選べる。`--charset` の既定値は `IBM-037`。BMS の `INITIAL` は UTF-8 で記述でき、`LENGTH` は変換後の EBCDIC バイト数で検査される。COBOL を使うときはホストの `--charset` と `-fdata-codeset` を一致させ、ワーカーに `COB_HOST_CODESET=UTF-8` を設定する。

次の試験は IBM-1399 の COBOL ワーカーと Rust ホストをビルドし、TN3270 で「日本語」を入力して COBOL が受信・返送した値を照合する。Db2 は使わない。

```powershell
& extras/cobcics/test-japanese-tn3270.ps1
```

成功時は `IBM-1399 Japanese TN3270 -> Rust -> COBOL -> Rust -> TN3270 OK` と表示される。

Windows の `wc3270` を使う試験は IBM-939 を指定する。確認に用いた `wc3270` v4.0ga14 は `cp930` と `cp939` に対応するが `cp1399` は選べない。次のスクリプトは IBM-939 のワーカーと Rust ホストをビルドし、`wc3270` を `-codepage cp939 -utf8` で起動する。スクリプトポートから日本語を入力し、COBOL が返した値を画面で照合する。

```powershell
& extras/cobcics/test-japanese-wc3270.ps1
```

成功時は `wc3270 cp939 Japanese display, input and COBOL echo OK` と表示される。別の場所にインストールしたときは `-Wc3270Path` を指定する。入力欄には Rust ホストが 3270 の SO/SI 自動生成属性を付ける。
