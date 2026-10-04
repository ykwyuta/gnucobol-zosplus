# JCL カタログド PROC・データセット・ABEND 試験

## 実行

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-datasets-jcl.ps1
```

試験は [`datasets.jcl`](../../extras/jclsh/tests/datasets.jcl) と [`abend.jcl`](../../extras/jclsh/tests/abend.jcl) を無編集で投入する。[`datasets.conf`](../../extras/jclsh/tests/datasets.conf) が `TEST.PROCLIB` と永続データセットのパスを登録する。Docker 内でコンパイルした `STEPTEST` が DD ファイルを読み書きする。

## 期待結果

| 検査 | 期待値 |
| --- | --- |
| `JCLLIB ORDER` / `INCLUDE MEMBER` | `INC1` の `TOKEN=CATALOG` を `LIBPROC` へ渡す |
| `PASS` | `CALL.MAKE` の `&&TEMP` を `COPY` が `INPUT=CATALOG` として読む |
| `CATLG` | `COPY` が作った `TEST.OUT` を `CHECK` が `INPUT=PERSIST` として読む |
| `DELETE` | `DROP` 後に `TEST.OUT` のローカルファイルが存在しない |
| `UNCATLG` | `TEST.ARCH` の対応先を消し、内容 `ARCHIVE` を台帳領域の `.uncataloged` に保存する |
| ABEND | `FAIL` は `U1234`、通常の `SKIP` は未実行、`ONLY`・`EVEN`・`IF ABEND` 側の `BRANCH` は実行 |
| 異常時 DISP | `FAIL` が作った `&&ABTMP` は第3引数 `DELETE` に従い削除 |

`DATAJOB` は全 6 ステップ RC=0。`ABJOB` はジョブ RC=16、CLI 終了コード 1、先頭ステップの `result.json` は `status=ABEND, abend=U1234, rc=null`。両 JCL 原本の SHA-256 が実行前後で同じことを確認する。

IBM の [PROC 検索順](https://www.ibm.com/docs/en/zos/3.2.0?topic=procedures-using-procedure)、[JCLLIB の例](https://www.ibm.com/docs/en/zos/3.2.0?topic=description-examples-jcllib-statement)、[DISP の処理](https://www.ibm.com/docs/en/zos/3.1.0?topic=parameter-summary-disposition-processing)、[ABEND 後の COND](https://www.ibm.com/docs/en/zos/3.2.0?topic=execution-bypassing-executing-steps-based-return-codes)を条件の基準とする。ローカルファイルの内容はバイト列であり、z/OS の物理レコード形式を再現したことは意味しない。
