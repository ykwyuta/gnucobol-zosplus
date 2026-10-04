# 複数ステップ・PROC・COND の JCL 試験

## 対象

[`multistep.jcl`](../../extras/jclsh/tests/multistep.jcl) を編集せず、登録済み `STEPTEST` を Docker コンテナー内で実行する。`STEPTEST` は `CARD` DD の数値を RC として返し、`PARM` と値を標準出力に記録する。[自動試験](../../extras/jclsh/test-multistep-jcl.ps1)は試験用 C プログラムを Docker の GCC でビルドする。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-multistep-jcl.ps1
```

## 期待結果

| 展開後ステップ | 期待状態 | RC | 確認点 |
| --- | --- | ---: | --- |
| `MAKE` | 実行 | 8 | `CARD=8`、`PARM=FIRST` |
| `CALL.CHECK` | スキップ | ― | PROC 内 `COND=(7,LT,MAKE)` が真 |
| `CALL.RUN` | 実行 | 0 | `&CODE` が `PROCVAL`、`RUN.CARD` の上書きが `0` |
| `AFTER` | 実行 | 0 | `COND=(7,GT,MAKE)` は偽。先行 RC=8 の後も実行 |
| `THEN` | 実行 | 0 | `MAKE.RC=8 AND AFTER.RC=0` が真 |
| `OTHER` | スキップ | ― | IF の ELSE 側 |

ジョブ RC は 8、CLI 終了コードは 1。`result.json` に全 6 ステップの順序、実行状態、RC/null を保存する。`CALL.RUN/PRINT.txt` に `PARM=PROCVAL CARD=0` を保存し、JCL 原本の SHA-256 が実行前後で同じであることを確認する。

別の Rust 単体試験では、JOB レベル COND の複数条件、`EVEN` / `ONLY`、スキップ済み RC、`JOBLIB`、未知の PROC DD 上書き、`DD DATA,DLM=` の本文、Bank of Z 原本の解析を確認する。これらは実行時の ABEND やデータセット台帳を検証したものではない。
