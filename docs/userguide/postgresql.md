# PostgreSQL で `EXEC SQL` を使う

固定形式 COBOL の `EXEC SQL` を `cobc --postgres-sql` で変換し、実行時に `extras/pgsql/pgsql.c` の libpq ブリッジで PostgreSQL に接続する。前処理は既存の Db2 SQL 変換器を共有する。生成される `DB2SQL_*` の呼び出し名と `DB2-SQL-*` の作業領域名は内部 ABI であり、`--postgres-sql` のプログラムは `libpgsqlsql.so` を読み込む。`--db2-sql` と `--postgres-sql` は同時指定できない。

対応するのは `SQLCA` / `SQLDA` とソース copybook の `INCLUDE`、単一行 `SELECT ... INTO`、`INSERT`、`UPDATE`、`DELETE`、`DECLARE` / `OPEN` / `FETCH` / `CLOSE CURSOR`、`COMMIT` / `ROLLBACK`。ホスト変数は英数字、整数、小数、長さ付き `VARCHAR` グループ、NULL 指示変数を扱う。SQLCODE は 0、100、-305、-811 と一部の PostgreSQL エラーを Db2 互換値へ写す。SQLSTATE は PostgreSQL の値を返す。

## Docker PostgreSQL の起動

PowerShell でリポジトリ直下から実行する。パスワードファイルは Git 対象外。Docker は [公式 PostgreSQL イメージ](https://hub.docker.com/_/postgres)の `17-alpine` を使用し、5433 番をホストのループバックだけに公開する。

```powershell
New-Item -ItemType Directory -Force extras/pgsql/secrets | Out-Null
[guid]::NewGuid().ToString('N') | Set-Content -NoNewline extras/pgsql/secrets/password.txt
docker compose -f extras/pgsql/compose.yaml up -d
docker compose -f extras/pgsql/compose.yaml ps
```

初回起動時に `extras/pgsql/init.sql` が `PGSQL_DEMO` を作る。既存のデータボリュームで起動した場合、この初期 SQL は再実行されない。現在の構成は `gcobpg` データベース、`gcob` ユーザーを使用する。

## ビルドと実行

WSL/Linux で `gcc`、`pkg-config`、`libpq-dev`、Python 3 とビルド済みの本リポジトリの GnuCOBOL が必要。`cobc` をソース更新後に再ビルドして `--postgres-sql` を使えるようにする。`build/cobc` 以外のビルド先なら `COBC_BIN` と `LIBCOB_DIR` を指定する。ビルド出力は既定で `build/pgsql`、`PGSQL_BUILD_DIR` で変更できる。

```sh
make -C build/cobc -j4 cobc
export PGSQL_BUILD_DIR="$PWD/build/ci-check/pgsql"
sh extras/pgsql/build-demo.sh
export PGHOST=127.0.0.1 PGPORT=5433 PGDATABASE=gcobpg PGUSER=gcob
export PG_PASSWORD_FILE="$PWD/extras/pgsql/secrets/password.txt"
export COB_PRE_LOAD="$PGSQL_BUILD_DIR/libpgsqlsql.so"
export LD_LIBRARY_PATH="$PWD/build/libcob/.libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
"$PGSQL_BUILD_DIR/pgsql-demo"
COB_HOST_CODESET=UTF-8 "$PGSQL_BUILD_DIR/pgsql-ebcdic-demo"
```

通常デモは再実行でき、成功時に `POSTGRES SQL OK` と表示する。重複キーの試験では PostgreSQL が 23505 を標準エラーへ出力する。EBCDIC デモは `-fdata-codeset=IBM-1399` でビルドされ、日本語を往復して `POSTGRES EBCDIC OK` と表示する。結果は [PostgreSQL テストケース](../testcases/postgresql-exec-sql.md) に記録する。

自分の固定形式ソースをコンパイルする場合:

```sh
COB_DB2_SQL_PREPROCESSOR="$PWD/extras/preprocessors/target/release/db2sql-preprocess" \
  build/cobc/.libs/cobc --postgres-sql -x -Icopy \
  -o build/pgsql/my-program path/to/my-program.cbl
```

接続は [libpq の環境変数](https://www.postgresql.org/docs/17/libpq-envars.html) `PGHOST`、`PGPORT`、`PGDATABASE`、`PGUSER` に従う。`PG_PASSWORD_FILE` はこのブリッジ固有で、ローカルの秘密ファイルを読む。`PG_CONNINFO` に libpq 接続文字列を渡すこともできる。`PG_PASSWORD_FILE` を設定しない場合は標準の `PGPASSWORD` や `.pgpass` を使用できる。

## 実装上の範囲

ホスト変数は libpq の [パラメーター付き実行](https://www.postgresql.org/docs/17/libpq-exec.html)で送る。`OPEN CURSOR` は結果をメモリに取得し、以降の `FETCH` で順番に返す。大量結果のストリーミング、`SCROLL` / `WITH HOLD` のカーソル意味論、位置指定更新、Db2 固有 SQL の PostgreSQL 方言への変換には対応していない。長さ付き `VARCHAR` グループの文字データは現在バイト列として渡す。アプリケーション SQL は PostgreSQL が受け付ける文法で記述する。CICS FILE の VSAM を同じ PostgreSQL 接続に置く場合は [CICS VSAM SQL モード](cics-vsam-sql.md)を参照する。
