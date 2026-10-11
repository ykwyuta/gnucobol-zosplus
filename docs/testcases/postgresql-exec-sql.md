# PostgreSQL `EXEC SQL` の実行テスト

2026-10-11、Docker Compose の `postgres:17-alpine` と WSL 上の `libpq` を使用。コンテナー `gnucobol-pgsql-postgres-1` は healthy、`init.sql` の `PGSQL_DEMO` を確認した。

| ID | 操作 | 期待値 | 結果 |
| --- | --- | --- | --- |
| P01 | `--postgres-sql` で固定形式デモをコンパイル | `EXEC SQL` が既存の SQL 呼び出し ABI へ変換される | 通過 |
| P02 | INSERT、単一行 SELECT、UPDATE、DELETE | ホスト変数と SQLCODE 0、対象値が一致 | 通過 |
| P03 | 2 行カーソルの OPEN/FETCH/CLOSE | 42、21、終了時 100 / 02000 | 通過 |
| P04 | COMMIT 後の UPDATE と ROLLBACK | 更新前の 21 に戻る | 通過 |
| P05 | NULL 指示変数、指示変数なし NULL | -1 と -305 / 22002 | 通過 |
| P06 | 0 行、複数行、重複キー | 100 / 02000、-811 / 21000、-803 / 23505 | 通過 |
| P07 | SQL 文字列内の `?` とホスト変数 | リテラル `it's ?` を保持 | 通過 |
| P08 | IBM-1399 の日本語ホスト変数 | `日本語` の往復、UTF-8 `e697a5e69cace8aa9e`、短い出力欄の 22001 | 通過 |
| P09 | 既存 `--db2-sql` デモを新コンパイラで変換し、両 SQL オプションを同時指定 | Db2 の C 生成成功、同時指定は明確なエラー | 通過 |
| P10 | 存在しない `PG_PASSWORD_FILE` を指定 | 終了コード 1、SQLSTATE 08001 | 通過 |
| P11 | 通常デモを再実行して DB の確定データを照合 | 2 行、金額 42 と 21 | 通過 |
| P12 | COMP-3 小数と 49 レベル長さ付き VARCHAR グループを入出力 | 123.45 と長さ 5 の `HELLO` が一致 | 通過 |
| P13 | `--save-temps` と `--postgres-sql` を併用 | `demo.pg.cbl` を保存し C 生成に成功 | 通過 |

`extras/pgsql/demo.cbl` と `ebcdic-demo.cbl` が各期待値を COBOL 側で判定する。実行コマンドは [利用ガイド](../userguide/postgresql.md#ビルドと実行) を参照。
