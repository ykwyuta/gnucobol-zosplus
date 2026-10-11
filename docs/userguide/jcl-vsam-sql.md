# jclsh の IDCAMS／REPRO で SQL テーブルを使う

`jclsh` のプロファイルで VSAM の実体を `local`、`db2`、`postgres` から選択する。SQL モードでは、`DEFINE CLUSTER` が `K`／`V` 表を作り、`REPRO` が表と固定長 FB データを相互に転送し、`DELETE` が表を削除する。`BLDINDEX` は SQL 表の元レコードを読み込む。ローカルにはデータセットの存在と FB レイアウトを示す空のカタログファイルと `.jclsh-meta` だけを置く。レコード本体は DB 表に保存する。

```text
vsam_backend=postgres
vsam_table.TEST.GENAPP.KSDSCUST=public.ksdscust
vsam_postgres_container=gnucobol-pgsql-postgres-1
vsam_postgres_user=gcob
vsam_postgres_database=gcobpg
```

Db2 の場合は `vsam_backend=db2` とし、`vsam_table.<DSN>=SCHEMA.TABLE`、`vsam_db2_image=gnucobol-db2-build` を指定する。Db2 接続設定は既存の `env_file` を Docker コンテナに渡す。補助プログラムの既定パスは `extras/jclsh/vsam_db2.py`。ZIP 配布物から配置する場合は `vsam_db2_helper=jclsh/vsam_db2.py` のように指定する。SQL モードでは、`DEFINE CLUSTER` の DSN に表マッピングが必須。`local` モードと `vsam_table.*` の同時指定はエラーになる。表名は英数字と `_` の未引用識別子、または `schema.table` に限る。

PostgreSQL は Docker Compose の `extras/pgsql/compose.yaml` を起動する。jclsh は設定したコンテナ内の `psql` を呼ぶ。Db2 は `extras/db2sql/compose.yaml` と `gnucobol-db2-build` イメージを使い、`extras/jclsh/vsam_db2.py` をそのイメージ内で実行する。両モードとも、レコードとキーをバイト列のまま格納し、キー位置が 0 以外の FB、EBCDIC、日本語シフトバイト、パック／バイナリ項目を変換しない。

`REPRO` の DB 更新は一回の文を単一トランザクションとして実行する。重複キーやレコード長不一致では更新を確定しない。DB からローカルへ書き出す場合はキー順となる。通常の DD 割り当てと DISP はローカルのカタログファイルで処理するため、SQL 表に対する変更を含む**ジョブ全体**の原子的な取り消しは行わない。SQL モードの表と CICS FILE の表を共有する場合は、CICS 側の `--vsam-table` と `--vsam-layout` を同じ表・レイアウトに設定する。実行中の表に対する REPRO／DELETE は避ける。

SQL 表に割り当てた DD を IDCAMS 以外のプログラムで直接読み書きする実行経路はエラーにする。`IEFBR14` の `DISP=DELETE` は表とカタログを削除できる。バッチ COBOL の VSAM ファイル I/O や SORT の入出力を SQL 表へ直結する機能は、このモードの対象外である。

通し試験は [JCL SQL VSAM テストケース](../testcases/jcl-vsam-sql.md)に記載する。既存の IDCAMS JCL 構文は変更しない。
