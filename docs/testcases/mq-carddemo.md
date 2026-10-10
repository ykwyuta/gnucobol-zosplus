# IBM MQ: 資材監査と実行テスト

## 資材監査

`MQOPEN`、`MQGET`、`MQPUT`、`MQPUT1`、`MQCLOSE`、`EXEC MQ`、JMS、MQTM を対象ソースから検索した結果:

| 資材 | 直接 MQ を使うソース | 判定 |
| --- | --- | --- |
| `reference/Bank-of-Z-main/src` | なし | MQ 実行経路なし。既存の Bank of Z 画面テストを回帰確認する |
| `reference/cics-genapp/base/src` | なし | TD/TS キューは CICS 機能であり IBM MQ 呼び出しではない。既存 GenApp テストを回帰確認する |
| `reference/aws-mainframe-modernization-carddemo/app/app-vsam-mq/cbl` | `CODATE01.cbl`、`COACCT01.cbl` | 実 MQ 要求・応答を検証 |
| `reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/cbl` | `COPAUA0C.cbl` | `MQPUT1` を使うが、認可業務本体は IMS/DLI に依存する。MQ 呼び出し自体は `mqdemo` で検証、業務全体は未検証 |

## ケース

| ID | 操作 | 期待値 | 2026-10-11 結果 |
| --- | --- | --- | --- |
| MQ01 | Docker Compose で `QM1` 起動、MQSC 実行 | 対象キュー生成、MQSC エラー 0 | 通過 |
| MQ02 | `mqdemo` で `MQCONN` → `MQOPEN` → `MQPUT` → `MQGET` → `MQPUT1` → `MQGET` → 空キュー → `MQCLOSE` → `MQDISC` | 本文一致、空キュー 2033 | 通過。`MQ ROUNDTRIP OK` |
| MQ03 | `CODATE01` を参照ソースから変換・コンパイルし、DATE 要求を送信 | 応答に `SYSTEM DATE` と `SYSTEM TIME` | 通過。実 MQ の応答キューから取得 |
| MQ04 | `COACCT01` を参照ソースから変換・コンパイルし、口座 `00000000001` を照会 | `acctdata.txt` の固定長レコードを読み、応答に対象口座番号 | 通過。実 MQ の応答キューから取得 |
| MQ05 | `COPAUA0C` の MQ 送信経路 | IMS/DLI を含む認可業務の完走 | 未実施。MQI の `MQPUT1` 単体は MQ02 で確認済み |
| MQ06 | Bank of Z の既存 CICS ソースを再変換・コンパイル | 32 本が通過、CICS ブリッジ単体試験通過 | 通過。`audit-bank-syntax.py` |
| MQ07 | GenApp の既存 CICS ソースを再変換・コンパイル・リンク | 31 本と LGSE/LGST/SSST の単体試験が通過 | 通過。`build-genapp-setup.ps1` |

MQ03/MQ04 は `extras/mq/test_carddemo.py` で相関 ID を付けた要求を投入し、オリジナル COBOL の実行後に返信キューの本文を確認する。CardDemo の端末トランザクション起動は CICS テストフレームで与える。MQTM を返す `RETRIEVE` と `ACCTDAT` の固定長読み取りには、このテスト対象を動かす補助処理を使う。実行コマンドは [利用ガイド](../userguide/mq.md#ビルドと動作確認) にある。
