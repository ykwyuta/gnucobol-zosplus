# jclsh SQL VSAM テストケース

対象 JCL: `extras/jclsh/tests/vsam-sql.jcl`。レコード長 12、キー長 4、キー位置 2 の FB を使用する。入力はキー順と逆に配置した 2 件で、EBCDIC／DBCS 風のバイト、NUL、改行を含む。

| 操作 | 期待値 |
| --- | --- |
| IDCAMS DEFINE CLUSTER | 指定した SQL 表と空のカタログ／FB メタデータを作成する |
| REPRO ローカル→SQL | 2 件を元のバイト列のまま登録する |
| REPRO SQL→ローカル | キー順の 2 件を固定長 24 バイトとして書き出す |
| DEFINE ALTERNATEINDEX / BLDINDEX | SQL 表の元レコードからローカル AIX 索引を作成する |
| IDCAMS DELETE | SQL 表とカタログ／メタデータを削除する |
| IEFBR14 DISP=(OLD,DELETE) | SQL 表とカタログ／メタデータを削除する |
| SQL 表マッピング欠落 | DEFINE CLUSTER を失敗させ、ローカル VSAM に切り替わらない |
| REPRO 中の重複キー | RC=12 とし、先行 INSERT も取り消して空の表を保持する |
| 既存回帰 | local モードの jclsh ユニットテストが通る |

実行例:

```powershell
cargo build --locked --manifest-path extras/jclsh/Cargo.toml --bin jclsh
python extras/jclsh/test-vsam-sql.py postgres extras/jclsh/target/debug/jclsh.exe
python extras/jclsh/test-vsam-sql.py db2 extras/jclsh/target/debug/jclsh.exe
```

PostgreSQL Compose と Db2 Compose をそれぞれ起動してから実行する。Db2 では `extras/db2sql/.runtime.env` と `gnucobol-db2-build` が必要。試験は一意の一時表を使用し、正常終了時に削除する。
