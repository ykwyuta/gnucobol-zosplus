# Rust 製 jclsh で BNKSTMT.jcl を実行する試験

## 対象

[Bank of Z の原本 JCL](../../reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl)を編集せずに `jclsh` へ渡す。`STEP1 EXEC PGM=IKJEFT01` の `SYSTSIN` から `DSN RUN PROGRAM(BNKSTMT)` を読み、[PL/I 原本](../../reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli)由来の実行ファイルを Docker の Db2 環境で起動する。

## 準備と実行

[PL/I 帳票試験](pl1-bnkstmt.md)に従って Compose の Db2、`PL1TEST` スキーマと `build/pl1c/BNKSTMT` を準備する。リポジトリのルートから実行する。

```powershell
cargo test --manifest-path extras/jclsh/Cargo.toml --offline
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-bank-jcl.ps1
```

`bank.conf` は登録プログラム、Db2 スキーマ、許可する `DSN SYSTEM` / `PLAN` / `LIB` を定義する。`STEPLIB` の DSN は計画に記録するが、Docker 内のロードライブラリーとしては使わない。

## 検査項目

| 項目 | 期待値 |
| --- | --- |
| JCL 原本 | 実行前後の SHA-256 が同じ |
| 解析 | 1 JOB、1 STEP、7 DD。STEPLIB は 2 データセットの連結 |
| DD | `DATECARD=202606`、`SORTCODE=123456` を実行ステップへ渡す |
| DSN RUN | `SYSTEM=DBD1`、`PROGRAM=BNKSTMT`、`PLAN=BANKZPLN` を登録内容と照合 |
| 戻りコード | ジョブと `STEP1` が RC=0。`result.json` に保存 |
| スプール | `SYSPRINT` に帳票、`SYSTSPRT` は空。DD ごとに保存 |
| 業務結果 | 2 口座、6 月の取引 2 件、期首・期末残高、2 回の改ページ。期間外取引を除外 |

自動試験は各項目を照合し、`BNKSTMT JCL passed` と出力する。これは `IKJEFT01` / `DSN RUN` の対象サブセットを使った試験であり、z/OS の JES や任意の JCL プロシージャーの実行を確認するものではない。
