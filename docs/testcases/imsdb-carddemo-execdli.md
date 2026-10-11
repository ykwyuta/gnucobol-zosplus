# CardDemo EXEC DLI 変換とバッチ結合試験

## 対象

CardDemo の固定形式 `EXEC DLI` を `cobc --ims-dli=DBPAUTP0` で変換する。バッチ原本 `CBPAUP0C.cbl`、`PSBPAUTB.PSB`、`DBPAUTP0.dbd` を使う。

## 実行

Linux 版 `libimsdb`、`libcob`、`cobc` をビルドした WSL 環境で、リポジトリのルートから実行する。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-execdli.sh
```

スクリプトは前処理器の4つの単体試験を実行し、原本を `--ims-dli=DBPAUTP0` でコンパイルする。空の DB で `GN` の未修飾 SSA が終端 `GB` を返し、原本の集計と `CHKP` が正常終了することを確認する。期待する表示は `IMS CARDDEMO EXEC DLI BATCH OK`。

原本 JCL と Docker の `DFSRRC00` 経路は次を実行する。

```powershell
./extras/imsdb/test-carddemo-jcl.ps1
```

`CBPAUP0J.jcl` を書き換えずに `jclsh plan` と `jclsh run` へ渡す。`SYSIN DD *` の値が COBOL へ届くこと、集計が出ること、ステップ RC 0 を確認する。プロファイルは `extras/jclsh/tests/ims-carddemo.conf`。この試験では認可 DB は空なので、親子レコードの削除件数は0である。

原本アンロードの親子データを使う業務経路は次を実行する。事前に Linux 版 `imsdbctl` を `build/imsdb-linux/debug` にビルドする。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-unload.sh
```

`DBPAUTP0.jcl` が `DFSURGU0` で作成した `AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat` を `imsdbctl import-dfsurgu0` で投入する。ヘッダー・トレーラー、VB レコード長、DFSURGUF プレフィックス、セグメント名と長さを検証し、22件の親と202件の子を入れる。無修正 `CBPAUP0C.cbl` を IBM-037／ビッグエンディアンモードで起動し、親削除前に同じ PCB で子を読んだため `DJ` となることを確認する。異常終了後は別プロセスで親22件・子202件が残ることを確認する。この原本の呼出順序は [IBM の EXEC DLI DLET 制約](https://www.ibm.com/docs/en/ims/15.5.0?topic=commands-dlet-command)にも反する。

元のロードプログラムを使った成功経路は次を実行する。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-load.sh
```

`split-dfsurgu0` が原本アンロードから100バイトの親と、親の6バイトキーを付けた206バイトの子レコードを作る。無修正 `PAUDBLOD.CBL` を起動して全件を投入し、別のCOBOLプログラムで親22件・子202件を読み戻す。期待する表示は `IMS CARDDEMO LOAD 22 ROOTS 202 CHILDREN OK`。

原本アンロードプログラムの出力は次を実行する。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-roundtrip.sh
```

無修正 `PAUDBUNL.CBL` を `COB_IMS_LINKAGE=DB` で起動し、親と子の固定長出力を元データとバイト単位で比較する。元データの親22件のうち最終1件は packed decimal キーが EBCDIC 空白で、原本の `IS NUMERIC` 条件により出力されない。親21件と子202件が一致する。期待する表示は `IMS CARDDEMO ORIGINAL UNLOAD 21 ROOTS 202 CHILDREN OK`。

元の JCL からアンロードとロードを続けて実行する。

```powershell
./extras/imsdb/test-carddemo-jcl-cycle.ps1
```

`UNLDPADB.JCL` の `IEFBR14` と `DFSRRC00`、続いて `LOADPADB.JCL` の `DFSRRC00` を無修正で `jclsh` から実行する。前者の FB 出力を元データの有効レコードとバイト単位で照合し、後者が親21件・子202件の計223セグメントを別の WAL に確定したことを確認する。期待する表示は `IMS CARDDEMO ORIGINAL JCL UNLOAD OK` と `IMS CARDDEMO ORIGINAL JCL LOAD 223 SEGMENTS OK`。実行には Docker の `gnucobol-ims-integration` イメージ、Linux 版 `libcob`／`cobc`／`libimsdb`／`imsdbctl` が必要。

オンライン／MQ 側の無修正原本3本を CICS と IMS の両方の前処理器に通す。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-online-build.sh
```

`COPAUS0C`、`COPAUS1C`、`COPAUA0C` をそれぞれの BMS マップ、CardDemo の共通 COPY、MQ COPY を指定し、native と IBM-037／ビッグエンディアンの両モードでモジュールにコンパイルする。これは原本の結合コンパイル試験であり、TN3270 画面操作や MQ を含む業務フローの実行試験ではない。

CICS 同期点と IMS WAL の結合試験は次のとおり。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-cics-uow.sh
```

Bank of Z の1169件を投入し、COBOL から `CBLTDLI` で顧客レコードを更新する。CICS 側の `commit_uow` と `rollback_uow` を通してそれぞれ確定・取消した後、別プロセスで再読込して確定値のみが残ることを確認する。あわせて packed decimal 検索キーを含む SSA が ASCII／IBM-037 双方で生バイトを保持すること、IBM-037 の `FORMATTIME YYDDD`／`YYMMDD` が数値文字列を返すことを確認する。期待する表示は `IMS CICS UOW REOPEN OK`。実際の CICS 画面からの IMS 更新は未検証。
