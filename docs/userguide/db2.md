# Db2 接続と埋め込み SQL

固定形式 COBOL の `EXEC SQL` を `cobc --db2-sql` で変換し、実行時に Db2 CLI で接続する。ここでは Docker Compose の IBM Db2 Community Edition 12.1.5.0 を使う。Community Edition は無償で利用できるが、オープンソースではない。

## Db2 を起動する

PowerShell でリポジトリのルートから実行する。まず `extras/db2sql/.env.example` を `extras/db2sql/.env` にコピーし、`DB2INST1_PASSWORD` をローカル用の値へ変更する。次に同じパスワードを使って、Git 管理から除外される `extras/db2sql/.runtime.env` を作る。

```text
DB2_HOST=host.docker.internal
DB2_PORT=50001
DB2_DATABASE=GCOBDB
DB2_USER=db2inst1
DB2_PASSWORD=<.env と同じパスワード>
```

```powershell
docker compose --env-file extras/db2sql/.env -f extras/db2sql/compose.yaml up -d
docker logs gnucobol-db2-db2-1
```

初回は初期化に時間がかかる。ログに `Setup has completed.` が出てから次へ進む。ホスト側のポートは既定で `127.0.0.1:50001`、データは Compose volume に保存される。

## デモをビルドして接続を確認する

最初のデモが使う表を一度だけ作成する。既に存在する場合は再作成しない。

```powershell
docker exec gnucobol-db2-db2-1 su - db2inst1 -c "db2 connect to GCOBDB && db2 'CREATE TABLE DB2SQL_DEMO (ID CHAR(8) NOT NULL PRIMARY KEY, AMOUNT INTEGER NOT NULL)'"
$ws = (Resolve-Path .).Path
docker build -f extras/db2sql/Dockerfile.build -t gnucobol-db2-build extras/db2sql
docker run --rm --entrypoint bash -v "${ws}:/src" gnucobol-db2-build -lc 'cd /src && sh extras/db2sql/build-demo.sh'
```

ビルド済みの 3 つのデモを実行する。

```powershell
docker run --rm --entrypoint bash --env-file extras/db2sql/.runtime.env `
  -e COB_HOST_CODESET=UTF-8 -v "${ws}:/src" gnucobol-db2-build -lc `
  'cd /src && export COB_PRE_LOAD=/src/build/db2sql-build/libdb2sql.so LD_LIBRARY_PATH=/src/build/db2sql-build/libcob/.libs:/usr/local/lib64/python3.9/site-packages/clidriver/lib && build/db2sql-build/db2-demo && build/db2sql-build/phase2-demo && build/db2sql-build/ebcdic-demo'
```

成功時は `DB2 SQL OK +000000042`、`DB2 PHASE 2 OK`、`DB2 EBCDIC OK` が出る。最後のデモは IBM-1399 の日本語ホスト変数を Db2 に渡し、読み戻す。

## 自分の COBOL プログラムで使う

固定形式のソースに `EXEC SQL ... END-EXEC` と `EXEC SQL INCLUDE SQLCA END-EXEC` を記述し、`--db2-sql` を指定する。ソースチェックアウトから使う場合は、Rust前処理器の場所を `COB_DB2_SQL_PREPROCESSOR` で指定する。インストール済み環境では実行ファイルを PATH から探す。次はこのブランチの `cobc` をインストールし、ソースを `/src` に配置した Linux 環境の例である。Docker 内でビルドするときは、上記の `build-demo.sh` が同じ設定を行う。

```sh
COB_DB2_SQL_PREPROCESSOR=/src/extras/preprocessors/target/release/db2sql-preprocess \
  cobc --db2-sql -x -Icopy -o db2-demo extras/db2sql/demo.cbl
```

実行時には `libdb2sql.so` を `COB_PRE_LOAD` で読み込み、Db2 CLI と `libcob` を `LD_LIBRARY_PATH` から見つけられるようにする。上の Docker デモはこの設定を含んでいる。接続先は `.runtime.env` の `DB2_HOST`、`DB2_PORT`、`DB2_DATABASE`、`DB2_USER`、`DB2_PASSWORD` を使用する。`DB2_DSN` でデータベース接続文字列を渡すこともできる。`DB2_SCHEMA=BANKZTEST` のように指定すると接続後の既定スキーマを設定する。

現在の変換対象は `INSERT`、単一行 `SELECT ... INTO`、`UPDATE`、`DELETE`、`COMMIT WORK`、`ROLLBACK WORK`、`DECLARE/OPEN/FETCH/CLOSE CURSOR` と `INCLUDE SQLCA/SQLDA` 等である。ホスト変数には整数、`PIC X`、`COMP-3` 小数、NULL 指示変数を使える。未対応の SQL は変換時にエラーになる。実行結果は `SQLCODE` と `SQLSTATE` で確認する。`--save-temps` を使うと SQL 変換後の `.db2.cbl` を調べられる。

接続や SQL の診断が必要なときは `DB2_SQL_TRACE=1` を設定する。SQL 本文が標準エラーへ出るため、機密値を含む実行では使用しない。エラーが出る場合は、Db2 の初期化完了、`.runtime.env` のホスト・ポート・資格情報、`COB_PRE_LOAD`、`LD_LIBRARY_PATH` の順に確認する。
