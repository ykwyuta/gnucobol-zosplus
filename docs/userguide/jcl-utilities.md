# jclsh の IBM ユーティリティー互換機能

## 原本 JCL の呼び出し一覧

`reference/Bank-of-Z-main`、`reference/aws-mainframe-modernization-carddemo`、`reference/cics-genapp` の `.jcl` / `.JCL` にある直接の `EXEC PGM=` を集計した。PROC の中身や動的に呼び出すプログラムは件数に含めない。CardDemo の件数には `app` と `samples` の両方を含む。

| バッチユーティリティー PGM | Bank of Z | CardDemo | GenApp | jclsh での扱い |
|---|---:|---:|---:|---|
| IDCAMS | 0 | 62 | 10 | Rust 内蔵。下記の制御文を実行 |
| IEBGENER | 0 | 11 | 2 | Rust 内蔵。SYSUT1 → SYSUT2 のレコードコピー |
| IEFBR14 | 0 | 10 | 0 | Rust 内蔵。DD の DISP 処理を実行 |
| SORT / DFSORT | 0 | 5 | 0 | Rust 内蔵。下記のソート制御文を実行 |
| DFHCSDUP | 0 | 1 | 5 | Rust 内蔵。下記の CSD 制御文を実行 |

同じ JCL の `EXEC PGM=` には、ユーティリティー以外の IBM 製プログラムもある。呼び出しを見落とさないよう併記する。

| その他の IBM 製 PGM | Bank of Z | CardDemo | GenApp | jclsh での扱い |
|---|---:|---:|---:|---|
| IKJEFT01 | 1 | 8 | 12 | `DSN RUN PROGRAM(...)` は既存の外部プログラム登録で実行。TSO/Db2 コマンド全般は未対応 |
| IKJEFT1B | 0 | 1 | 0 | 未実装。CardDemo の TXT2PDF REXX 実行 |
| SDSF | 0 | 10 | 0 | 未実装。CICS 領域への `/F ... CEMT SET FIL ...` など |
| FTP | 0 | 1 | 0 | 未実装。リモート FTP サーバーとの通信 |
| DFHECP1$ | 0 | 1 | 0 | 未実装。CICS/IMS トランスレート |
| DFSRRC00 | 0 | 5 | 0 | 未実装。IMS 実行制御 |
| IGYCRCTL | 0 | 1 | 1 | 未実装。IBM COBOL コンパイラー |
| ASMA90 | 0 | 0 | 2 | 未実装。HLASM アセンブラー |
| IEWL / HEWL | 0 | 1 | 1 / 1 | 未実装。z/OS バインダー |
| EYU9XDUT | 0 | 0 | 1 | 未実装。CICSPlex SM リポジトリー初期化 |

GenApp の `DFHNCMN`、`DFHSIP`、`DFHXQMN`、`EYU9XECS` は常駐サーバー／領域プログラムであり、ユーティリティーではない。`DFHLS2WS` と `EYUJXBTP` は `EXEC` で呼ぶ IBM 提供 PROC で、PROC 本体はこのリポジトリにない。`ITP*` と `CB*` はアプリケーションのプログラム。未実装の PGM は従来どおり `program.NAME=` で外部アダプターを登録するか、計画段階で明示的に失敗する。

## Rust 内蔵の実行範囲

`jclsh run` は上記の内蔵 5 プログラムをホストの Rust プロセスで実行する。`program.NAME=` 登録、Docker イメージ、Db2 環境ファイルは不要。`dataset_root` と、原本が参照する永続 DSN の `dataset.<DSN>=...` は必要である。書き込み先は `dataset_root` 内に限る。SYSIN の未対応制御文は RC 12 を返す。

| PGM | 実装した制御 |
|---|---|
| IDCAMS | `DELETE`、`DEFINE CLUSTER` / `ALTERNATEINDEX` / `PATH` / `GENERATIONDATAGROUP`、`REPRO INFILE/OUTFILE`、`BLDINDEX INDATASET/OUTDATASET`、`LISTCAT LEVEL`、資材中の `SET MAXCC` / `IF ... THEN SET MAXCC` |
| IEBGENER | 空または DUMMY の SYSIN を使う SYSUT1 → SYSUT2 コピー |
| IEFBR14 | 処理本体は空。jclsh の `DISP` に従い割当・削除 |
| SORT | `SORT FIELDS`、`OPTION COPY`、`INCLUDE COND`、`OUTREC FIELDS`、`SYMNAMES` の資材中で使う形式 |
| DFHCSDUP | `DEFINE`、`ADD`、`REMOVE`、`DELETE GROUP(...) ALL`、`LIST GROUP(...)`。DFHCSD DD に jclsh 独自の TSV CSD カタログを保存 |

IDCAMS の VSAM クラスターは既定でローカルファイル、AIX はキーと元レコード番号の索引ファイル、PATH は索引への参照ファイルとして表現する。`vsam_backend=db2|postgres` と DSN ごとの表マッピングを設定すると、キー付きクラスターのレコード実体を SQL 表に切り替えられる。DEFINE／REPRO／DELETE／BLDINDEX の詳細は [jclsh SQL VSAM モード](jcl-vsam-sql.md)を参照する。GDG の `(+1)` / `(0)` / `(-n)` は `G0001V00` 形式のローカルファイルに割り当て、CATLG 時には LIMIT を超える古い世代を削除する。

`DCB=(RECFM=FB,LRECL=n)` または IDCAMS の `RECORDSIZE(n,n)` を持つデータセットは、改行・長さヘッダーのない n バイト単位の FB ファイルとして保存する。`<データファイル>.jclsh-meta` に `FB<TAB>LRECL<TAB>KEYLEN<TAB>KEYOFFSET` を記録し、IEBGENER、REPRO、SORT、BLDINDEX と GDG のコピー・削除で維持する。メタデータのない投入元は改行区切りのレコードとして読み、指定 LRECL まで空白で埋める。既存の FB 入力は LRECL 単位で検証する。GenApp の元 JCL `adef121.jcl` では `KSDSCUST` が 225 バイト、`KSDSPOLY` が 64 バイトになる。

`cobcicsd --csd` は DFHCSDUP が生成した CSD を読み、`--csd-list` に属する TRANSACTION→PROGRAM 定義を `--program PROGRAM=実行ファイル` に対応付ける。`--dataset-profile` の DSN マッピングと CSD FILE の `DSNAME` が一致すると、その FB ファイルを COBOL ワーカーの `COBCICS_VSAM_<FILE>` に渡す。`genapp_file.c` をリンクしたワーカーは、キー付き FB ファイルをメタデータの LRECL・キー長・キー位置に従って CICS READ、WRITE、READ UPDATE→REWRITE、DELETE で利用できる。GenApp の `KSDSCUST` / `KSDSPOLY` に加え、別名と非ゼロキー位置のファイルも試験している。CardDemo の `ACCTDAT` は専用ブリッジで READ に対応する。詳細は [GenApp の CSD/VSAM 手順](cics-genapp.md#jclsh-の-csd・fb-vsam-を使う)を参照。

この保存形式と CSD TSV は jclsh と cobcicsd 間のローカル形式であり、IBM の VSAM カタログや CSD の物理形式との交換には対応しない。AIX/PATH を CICS ファイル操作へ接続する機能は未実装である。CICS ワーカーが行うローカル VSAM 更新は SYNCPOINT または正常終了まで保留し、SYNCPOINT ROLLBACK・ABEND では破棄する。現在はマッピングしたローカルファイルをすべて回復対象として扱い、CSD の非回復ファイル属性による切り替えは未実装である。Db2 とローカル VSAM の通常の確定・取り消しを同じ境界で行うが、プロセスやホストの障害時まで保証する分散二相コミットと回復ログは未実装である。

## 実行例

リポジトリのルートで実行する。

```powershell
cargo run --locked --manifest-path extras/jclsh/Cargo.toml -- run reference/aws-mainframe-modernization-carddemo/app/jcl/ESDSRRDS.jcl --profile extras/jclsh/tests/carddemo-utilities.conf
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-utilities.ps1
```

回帰試験は内蔵機能に加え、CardDemo の `ESDSRRDS`、`TRANIDX`、`DEFGDGB`、`DEFGDGD`、`COMBTRAN`、`CBADMCDJ` と GenApp の `adef121`、`cdef121`〜`cdef125` の原本を無編集で実行する。バイト単位コピー、連結 DD、並べ替え、AIX キー順、GDG 世代とジョブ内再参照、CSD 定義を確認する。Bank of Z の `BNKSTMT.jcl` は従来の `IKJEFT01` → PL/I アダプターの [専用試験](../../extras/jclsh/test-bank-jcl.ps1)で検証する。

## IBM リファレンス

- [Access Method Services / IDCAMS](https://www.ibm.com/docs/en/zos-basic-skills?topic=utilities-idcams-use-access-method-services-catalogs)
- [IEBGENER](https://www.ibm.com/docs/en/zos/3.1.0?topic=utilities-iebgener-sequential-copygenerate-data-set-program)
- [DFSORT 制御文](https://www.ibm.com/docs/en/zos/3.2.0?topic=jobs-summary-dfsort-control-statements)
- [DFHCSDUP](https://www.ibm.com/docs/en/cics-ts/5.6.0?topic=components-cics-utility-program-dfhcsdup)
