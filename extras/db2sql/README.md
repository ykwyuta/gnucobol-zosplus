# Db2 埋め込み SQL の試験実装

固定形式 COBOL の `EXEC SQL INCLUDE SQLCA/SQLDA`、SQLの表定義コピー、`INSERT`、単一行 `SELECT ... INTO`、`UPDATE`、`DELETE`、`COMMIT WORK`、`ROLLBACK WORK`、`DECLARE/OPEN/FETCH/CLOSE CURSOR` を実行できます。ホスト変数は整数、`PIC X`、`COMP-3` の小数、およびNULL指示変数に対応します。`cobc --db2-sql` は通常の COBOL 前処理より先に `extras/preprocessors/target/release/db2sql-preprocess` を起動して SQL を `CALL` に変換し、`db2sql.c` が Db2 CLI で実行します。未対応の SQL は変換時にエラーになります。

Db2 サーバーには IBM Db2 Community Edition 12.1.5.0、クライアントには `ibm_db` 3.3.0 の wheel に含まれる CLI ドライバーを使用します。どちらも IBM の配布物であり、リポジトリには同梱しません。Community Edition は無料で使用できるエディションですが、オープンソースではありません。

## Db2 を起動する

リポジトリのルートで実行します。`.env` と `.runtime.env` には資格情報が入るため、Git の管理対象から除外しています。

1. `extras/db2sql/.env.example` を `extras/db2sql/.env` にコピーし、`DB2INST1_PASSWORD` にローカル専用のパスワードを設定します。
2. `extras/db2sql/.runtime.env` を次の内容で作ります。パスワードは `.env` と同じ値にします。

   ```text
   DB2_HOST=host.docker.internal
   DB2_PORT=50001
   DB2_DATABASE=GCOBDB
   DB2_USER=db2inst1
   DB2_PASSWORD=<設定したパスワード>
   ```

3. Db2 を起動し、`docker logs gnucobol-db2-db2-1` に `Setup has completed.` が出るまで待ちます。初回起動には数分かかります。

   ```sh
   docker compose --env-file extras/db2sql/.env -f extras/db2sql/compose.yaml up -d
   ```

4. サンプル表を作成します。

   ```sh
   docker exec gnucobol-db2-db2-1 su - db2inst1 -c "db2 connect to GCOBDB && db2 'CREATE TABLE DB2SQL_DEMO (ID CHAR(8) NOT NULL PRIMARY KEY, AMOUNT INTEGER NOT NULL)'"
   ```

ポートは既定でホストの `127.0.0.1:50001` に公開されます。データは Compose volume に残ります。表が既にある場合は再作成不要です。

## ビルドと実行

以下の `docker run` は Windows PowerShell での例です。`D:/workspace/gnucobol` をこのリポジトリの絶対パスに置き換えてください。Linux/macOS では同じパスをその環境の絶対パスに置き換えます。

```sh
docker build -f extras/db2sql/Dockerfile.build -t gnucobol-db2-build extras/db2sql
docker run --rm --entrypoint bash -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && sh extras/db2sql/build-demo.sh'
docker run --rm --entrypoint bash --env-file extras/db2sql/.runtime.env -e COB_HOST_CODESET=UTF-8 -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && export COB_PRE_LOAD=/src/build/db2sql-build/libdb2sql.so LD_LIBRARY_PATH=/src/build/db2sql-build/libcob/.libs:/usr/local/lib64/python3.9/site-packages/clidriver/lib && build/db2sql-build/db2-demo && build/db2sql-build/phase2-demo && build/db2sql-build/ebcdic-demo'
```

成功すると `DB2 SQL OK +000000042`、`DB2 PHASE 2 OK`、`DB2 EBCDIC OK` が出ます。最初のサンプルはINSERT、検索、該当なし（`+100`）、複数行の単一行検索（`-811`）、重複キー（`-803`）と各SQLSTATEを確認します。重複キーの診断は標準エラーへ出ます。2つ目はBank-of-Zと同じ形式のSQL表定義コピー、UPDATE/DELETE、カーソル、COMMIT/ROLLBACK、`COMP-3`小数、NULL指示変数を確認し、試験行を最後に削除します。最後はIBM-1399の日本語ホスト変数をDb2へ渡し、Db2側のUTF-8バイト値、COBOLへの読み戻し、短すぎる出力項目の診断を確認します。日本語の試験には、ホスト側のコードセットを `COB_HOST_CODESET=UTF-8` として明示する必要があります。自動コミットは無効です。

`build-demo.sh` は両サンプルを `cobc --db2-sql` でコンパイルします。ソースチェックアウトから直接使う場合はRust前処理器の場所を指定します。インストール後は `db2sql-preprocess` を `PATH` から探します。ビルド方法は [Rust前処理器ガイド](../../docs/userguide/rust-preprocessors.md) を参照してください。`--save-temps` を併用すると変換後の `.db2.cbl` を確認できます。

```sh
COB_DB2_SQL_PREPROCESSOR=/src/extras/preprocessors/target/release/db2sql-preprocess \
  cobc --db2-sql -x -Icopy -o db2-demo extras/db2sql/demo.cbl
```

Bank-of-Zのソースを変換する場合は、コピー探索パスを指定します。以下はSQL変換のみの例です。元ソースにはCICS文もあるため、このままではプログラム全体をコンパイルできません。

```sh
extras/preprocessors/target/release/db2sql-preprocess -I reference/Bank-of-Z-main/src/base/cics/copy reference/Bank-of-Z-main/src/base/cics/cobol/INQCUST.cbl build/INQCUST-db2.cbl
```

現時点でBank-of-ZのSQLを含む15個の`.cbl`は変換できました。元ソースのCICS文は変換対象外のため、これらのプログラム全体をコンパイル・実行した確認ではありません。

ランタイムは `DB2_HOST`、`DB2_PORT`、`DB2_DATABASE`、`DB2_USER`、`DB2_PASSWORD` を読みます。代わりに `DB2_DSN` も指定できます。`DB2_SCHEMA=BANKZTEST` のように指定すると、接続後にそのスキーマを既定に設定します。`DB2_SQL_TRACE=1` を設定すると SQL と接続段階の診断を標準エラーへ出します。SQL 文に機密情報を含む場合は有効にしないでください。

日本語のホスト変数は、COBOLのデータコードセットと `COB_HOST_CODESET` の間で変換してからCLIへ渡します。IBM-930/939/1390/1399で日本語値の往復を、IBM-037で英数字のDb2操作を確認済みです。IBM-037は日本語のソース定数を表現できません。Db2 for z/OSとのコードページ・照合順序の完全一致は未検証です。完全なSQLCA、BMS、CICSのトランザクション管理は未対応です。NULL指示変数は入力の`-1`（NULL）と`0`（非NULL）、出力の`-1`と`0`を扱います。指示変数なしでNULLを受けると`SQLCODE=-305`、`SQLSTATE=22002`を返します。カーソルはCOMMIT/ROLLBACK時に閉じます。
