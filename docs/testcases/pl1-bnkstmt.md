# PL/I BNKSTMT 帳票の Db2 通し試験

## 対象と前提

Bank of Z の原本 `reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli` を無編集で C に変換し、Db2 CLI とリンクして実行する。Db2 Compose と `gnucobol-db2-build` イメージ、`extras/db2sql/.runtime.env` を [Db2 ガイド](../../extras/db2sql/README.md)の手順で準備する。

`extras/pl1c/setup-bank-fixture.py` は [Bank 用テーブル定義](../../extras/cobcics/bank-test-db.sql)を専用 `PL1TEST` スキーマに作り、SORTCODE `123456` の顧客 1 件、口座 2 件、取引 3 件を不足分だけ追加する。`BANKZTEST` の既存行は変更しない。

## 手順

リポジトリのルートで実行する。以下のボリュームパスはチェックアウト先に合わせる。

```powershell
cargo run --manifest-path extras/pl1c/Cargo.toml -- -C reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli -o build/pl1c/BNKSTMT.c
docker run --rm --entrypoint python3 --env-file extras/db2sql/.runtime.env -v D:/workspace/gnucobol:/src -w /src gnucobol-db2-build extras/pl1c/setup-bank-fixture.py
docker run --rm --entrypoint bash --env-file extras/db2sql/.runtime.env -v D:/workspace/gnucobol:/src gnucobol-db2-build -lc 'cd /src && sh extras/pl1c/build-bank.sh && python3 extras/pl1c/test-bank-report.py'
```

自動試験は `DB2_SCHEMA=PL1TEST`、`DD_DATECARD=202606`、`DD_SORTCODE=123456` を設定して実行する。通過すると `BNKSTMT PL1TEST report passed: 2 accounts, 2 June transactions, balances and pages` を出力する。

## 期待値

| 項目 | 期待値 |
| --- | --- |
| 対象期間 | 2026-06-01 ～ 2026-06-30 |
| 口座 | `00000001`、`00000002` の 2 件 |
| 口座 1 の 6 月取引 | 入金 150.00、出金 50.00 の 2 件。5 月 31 日の取引は除外 |
| 口座 1 の残高 | 期首 1000.00、入金計 150.00、出金計 50.00、期末 1100.00 |
| 口座 2 | 取引なし、期首・期末とも 500.00 |
| 帳票 | 明細見出し 2 件、改ページ 2 回、SQL/実行時エラーなし |

`test-bank-report.py` は原本の実行結果をこれらの固定期待値と照合する。IBM Enterprise PL/I の帳票書式との比較試験や JCL インタプリタからの起動はこの試験に含まない。
