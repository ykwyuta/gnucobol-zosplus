# CICS FILE の VSAM 実体を Db2 または PostgreSQL にする

`genapp_file.c` が扱うキー付き固定長レコードを、ローカル FB ファイル、Db2 テーブル、PostgreSQL テーブルのいずれかに保存できる。`cobcicsd` の `--vsam-backend local|db2|postgres` で **起動単位に一つ**選ぶ。`local` は `--dataset-profile` で解決した `COBCICS_VSAM_<FILE>` の FB ファイルを使う。SQL モードはローカルファイルへのフォールバックを行わない。フラグを省略した場合は従来どおり、ファイルのパスがあればローカル、それ以外の GenApp 既定ファイルはリンク済みの SQL ドライバーを使う。

## テーブルとレイアウト

SQL モードの各表は `K`（主キー）と `V`（レコード）の二列を持つ。Db2 は `K BINARY(<キー長>) NOT NULL PRIMARY KEY, V BLOB(<レコード長>) NOT NULL`、PostgreSQL は `K BYTEA PRIMARY KEY, V BYTEA NOT NULL` を使用する。格納する値は固定長 COBOL レコードの **元のバイト列**で、EBCDIC、日本語のシフトバイト、パック十進数などを変換しない。`V` のキー位置にあるバイトは `K` と一致する必要がある。

`KSDSCUST`（225 バイト、先頭から 10 バイトのキー）と `KSDSPOLY`（64 バイト、先頭から 21 バイトのキー）はレイアウトが組み込み済み。表名は同名を使う。他の FILE、または別の表名を使う場合は次を追加する。

```text
--vsam-backend postgres
--vsam-table OTHER=public.vsam_other
--vsam-layout OTHER=12,4,2
```

レイアウトは `LRECL,KEYLEN,KEYOFFSET`（キー位置は 0 始まり）。表名は未引用の識別子または `schema.table` に限り、英数字と `_` 以外は受け付けない。FILE 名は最大 8 桁の英数字。既定の GenApp 表は `--vsam-table KSDSCUST=...` のように表名のみ変更できる。`--vsam-layout` で既定値を上書きすることもできる。

Db2 の GenApp 初期データと表作成は `extras/cobcics/setup-genapp-file-test-db.py` を参照する。PostgreSQL の例:

```sql
CREATE TABLE public.vsam_other (
    K BYTEA PRIMARY KEY,
    V BYTEA NOT NULL
);
```

このテーブルは事前に作成する。SQL モードでは `--dataset-profile` を同時指定できない。[jclsh の SQL VSAM モード](jcl-vsam-sql.md)で `DEFINE CLUSTER` と `REPRO` を使えば、JCL から同じ表を作成・投入できる。CICS の表マッピングとレイアウトを一致させる。

## ビルドと起動

Db2 用 GenApp ワーカーは [GenApp ガイド](cics-genapp.md)の `build-genapp-setup.ps1` でビルドし、`cobcicsd` の起動引数に `--vsam-backend db2` を追加する。`--db2-sql` の SQL ランタイムと CICS FILE が同一 Db2 接続を共有する。既定表の接続スキーマは `DB2_SCHEMA` で設定する。

PostgreSQL 用 CICS FILE ランタイムは WSL/Linux で次を実行して作る。Windows の作業ツリーでは、最初に PowerShell で `New-Item -ItemType Directory -Force build/cobcics-postgres` を実行して出力先を作る。PostgreSQL Compose の起動・接続設定は [PostgreSQL ガイド](postgresql.md)を参照する。

```sh
sh extras/cobcics/build-vsam-postgres.sh
```

生成物 `build/cobcics-postgres/libpgsqlsql.so` は `--postgres-sql` の COBOL ワーカーと CICS FILE の両方に使う。ワーカーの `bridge.c` は `-DCOBCICS_DB2SQL` でビルドし、`COB_LDFLAGS` にこのライブラリのディレクトリと `-lpgsqlsql` を指定する。ワーカー実行時は `COB_PRE_LOAD` に共有ライブラリの絶対パスを設定し、`LD_LIBRARY_PATH` にそのディレクトリと `build/libcob/.libs` を含める。`cobcicsd` の起動引数には `--vsam-backend postgres`、必要な `--vsam-table` と `--vsam-layout` を指定する。PostgreSQL 用ワーカーでは、アプリケーションの埋め込み SQL 自体も PostgreSQL が受け付ける文法であることを確認する。

## 作業単位と検証

READ、範囲 READ、READ UPDATE、WRITE、REWRITE、DELETE は選択した表へ実行する。SQL の更新と COBOL の `EXEC SQL` は同じ接続を使い、`EXEC CICS SYNCPOINT` または正常終了で COMMIT、`SYNCPOINT ROLLBACK` や ABEND で ROLLBACK する。PostgreSQL は重複キーの後も作業単位を継続できるよう各更新を SAVEPOINT で囲む。テーブルモードのデータベーストランザクションを、ローカル FB ファイルと混在させない。

回帰試験は `extras/cobcics/test-genapp-file.py`（Db2 既定表）、`extras/cobcics/test-vsam-db2-generic.py`（Db2 任意表）、`extras/cobcics/test-vsam-postgres.py`（PostgreSQL）である。任意表試験では、EBCDIC/バイナリを含む非ゼロ位置キー、更新、削除、COMMIT/ROLLBACK と SQL/ローカルの排他を検証する。PostgreSQL 試験は重複と範囲読み取りも確認する。

PostgreSQL の実 DB 試験例（Compose 起動後、WSL/Linux）:

```sh
export PGHOST=127.0.0.1 PGPORT=5433 PGDATABASE=gcobpg PGUSER=gcob
export PG_PASSWORD_FILE="$PWD/extras/pgsql/secrets/password.txt"
export LD_LIBRARY_PATH="$PWD/build/libcob/.libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
python3 extras/cobcics/test-vsam-postgres.py build/cobcics-postgres/libpgsqlsql.so
```

Db2 の両試験は `extras/cobcics/test-genapp-all.ps1` に含まれる。
