# z/OS 互換機能の利用ガイド

このガイドは、このリポジトリで追加した EBCDIC データ、Db2 埋め込み SQL、CICS/BMS 画面、COBOL 分岐カバレッジの使い方を説明する。コマンドは特記がなければリポジトリのルートで実行する。

| 目的 | ガイド | 入口 |
| --- | --- | --- |
| COBOL のデータを IBM EBCDIC で保持する | [EBCDIC 互換モード](ebcdic.md) | `-fdata-codeset=IBM-1399` など |
| `EXEC SQL` から Docker Compose の Db2 に接続する | [Db2 接続](db2.md) | `--db2-sql`、`extras/db2sql/compose.yaml` |
| BMS 画面を TN3270 端末に表示し、Bank of Z を操作する | [CICS/BMS](cics-bms.md) | `--cics-map`、`cobcicsd` |
| 対応 CICS 機能と `EXEC CICS` コマンドの範囲を調べる | [CICS 対応機能・コマンド一覧](cics-supported-features.md) | IBM 公式リファレンス付きの対応表 |
| GenApp の顧客・保険画面と HTTP 業務経路を試す | [CICS GenApp](cics-genapp.md) | `test-genapp-all.ps1`、`cobcicsd` |
| COBOL の MQI 呼び出しで Docker 上の IBM MQ と送受信する | [IBM MQ 接続](mq.md) | `extras/mq/compose.yaml`、`libcobmq.so` |
| 生成 C ではなく COBOL ソースの分岐結果を測る | [COBOL 分岐カバレッジ](cobol-branch-coverage.md) | `--cobol-branch-coverage`、`bin/cobcov` |

EBCDIC と Db2 を組み合わせた日本語ホスト変数の往復は、`ebcdic-demo.cbl` で確認できる。Bank of Z の CICS 画面は Db2 と組み合わせて動作確認している。Rust ホストの日本語 EBCDIC 入出力と COBOL との往復は [CICS/BMS ガイド](cics-bms.md#日本語-ebcdic-の画面入出力) の IBM-1399 デモで確認できる。

Docker を使う手順では、PowerShell、Python、Rust/Cargo、Docker Desktop と、このリポジトリのソースが必要になる。Db2 Community Edition と CLI ドライバーは IBM の配布物であり、このリポジトリには同梱しない。
