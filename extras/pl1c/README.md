# pl1c（開発中）

Rust 製の PL/I → C フロントエンドと C ランタイムの初期実装です。`PROCEDURE OPTIONS(MAIN)`、内部 `PROCEDURE`、`DCL` の `CHAR` / `FIXED BIN` / `FIXED DEC` / `BIT(1)`、`INIT`、代入、式、`IF`、`DO`、`DO WHILE`、`CALL`、`RETURN`、ラベルと `GO TO`、一部の組み込み関数、`FILE RECORD INPUT ENV(F RECSIZE(...))`、`ON ENDFILE`、`PUT SKIP LIST` / `PUT SKIP EDIT` / `PUT STRING ... EDIT` / `PUT PAGE` に対応しています。Db2 には `EXEC SQL INCLUDE SQLCA`、`DECLARE` / `OPEN` / `FETCH` / `CLOSE`、`SELECT ... INTO`、`COMMIT WORK` とホスト変数・NULL 指示変数の構文で接続します。

Bank of Z の原本 `BNKSTMT.pli` は無編集で C へ変換し、Db2 CLI とリンクして実行ファイルを作れます。DD パスは環境変数 `DD_DATECARD` / `DD_SORTCODE` で渡します。現在の FILE 入力は改行区切りの短いカードを読み、指定 RECSIZE まで空白埋めする実装です。DD が欠けた場合は、この原本の `ON ENDFILE` フォールバックが動作します。SQL の型検査、EBCDIC、ソース位置マップ、`-c` / `-x` は未実装です。未対応構文はコンパイル時に拒否します。

## Bank of Z 原本をコンパイルする

リポジトリのルートから PowerShell で次を実行します。Docker イメージの準備は [Db2 サンプルの手順](../db2sql/README.md)に従ってください。

```powershell
cargo run --manifest-path extras/pl1c/Cargo.toml -- -C reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli -o build/pl1c/BNKSTMT.c
docker run --rm --entrypoint bash -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && sh extras/pl1c/build-bank.sh'
```

実行ファイルは `build/pl1c/BNKSTMT` です。検証用の `PL1TEST` スキーマには `ACCOUNT` / `CUSTOMER` / `PROCTRAN` と、2026 年 6 月の 2 口座・2 取引を投入します。既存の Bank of Z 画面試験用 `BANKZTEST` とは別のスキーマです。

```powershell
docker run --rm --entrypoint python3 --env-file extras/db2sql/.runtime.env -v D:/workspace/gnucobol:/src -w /src gnucobol-db2-build extras/pl1c/setup-bank-fixture.py
docker run --rm --entrypoint bash --env-file extras/db2sql/.runtime.env -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && sh extras/pl1c/build-bank.sh && python3 extras/pl1c/test-bank-report.py'
```

成功時は `BNKSTMT PL1TEST report passed` と表示します。検査内容は [テストケース](../../docs/testcases/pl1-bnkstmt.md)を参照してください。Docker のボリュームパスはチェックアウト先に合わせて変更してください。

## SQL を使わないプログラム

リポジトリのルートから PowerShell で生成 C を作り、WSL Ubuntu の GCC でビルド・実行できます。

```powershell
cargo run --manifest-path extras/pl1c/Cargo.toml -- -C extras/pl1c/tests/hello.pli -o build/pl1c/hello.c
wsl -d Ubuntu -- bash -lc 'cd /mnt/d/workspace/gnucobol && gcc -std=c11 -Wall -Wextra -Werror -Iextras/pl1c/runtime build/pl1c/hello.c extras/pl1c/runtime/pli_runtime.c -o build/pl1c/hello-wsl && build/pl1c/hello-wsl'
```

期待出力は `HELLO        3` です。WSL のパスはチェックアウト先に合わせて変更してください。

DD と編集出力の小型例は `tests/file-edit.pli` です。`DD_CARD` に `tests/card.txt` を指定すると `CODE=123456    7` と `     12.30`、未指定では `NO CARD` を出力します。

## Db2 に接続するプログラム

[Db2 サンプルの手順](../db2sql/README.md)に従い、Compose の Db2 と `gnucobol-db2-build` イメージ、`extras/db2sql/.runtime.env` を用意します。`.runtime.env` には接続先と資格情報を設定し、Git へ追加しません。次の例は Db2 標準表 `SYSIBM.SYSDUMMY1` を使い、独自の表は作成しません。

```powershell
cargo run --manifest-path extras/pl1c/Cargo.toml -- -C extras/pl1c/tests/db2-select.pli -o build/pl1c/db2-select.c
docker run --rm --entrypoint bash --env-file extras/db2sql/.runtime.env -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && gcc -std=c11 -Wall -Wextra -Werror -Iextras/pl1c/runtime -I/usr/local/lib64/python3.9/site-packages/clidriver/include build/pl1c/db2-select.c extras/pl1c/runtime/pli_runtime.c extras/pl1c/runtime/pli_db2.c -L/usr/local/lib64/python3.9/site-packages/clidriver/lib -Wl,-rpath-link,/usr/local/lib64/python3.9/site-packages/ibm_db.libs -ldb2 -o build/pl1c/db2-select && LD_LIBRARY_PATH=/usr/local/lib64/python3.9/site-packages/clidriver/lib:/usr/local/lib64/python3.9/site-packages/ibm_db.libs build/pl1c/db2-select'
```

期待出力は `42` です。カーソル例の `tests/db2-cursor.pli` は `7` と `EOF 02000`、NULL 指示変数の例 `tests/db2-null.pli` は `-1 9` と `-305 22002` を出力します。Docker のボリュームパスはチェックアウト先に合わせて変更してください。

Rust の単体試験は `cargo test --manifest-path extras/pl1c/Cargo.toml --offline` で実行します。設計と実装順は [提案書](../../docs/proposal/pli-to-c-compiler.md)に記載しています。
