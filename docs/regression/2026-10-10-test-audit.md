# 回帰テストの棚卸し（2026-10-10）

## 判定と対象

対象は `feature/sql-db2` の `295723b` を基点とした今回の変更。直近の Rust 製 CICS/BMS ホストと wc3270 対応を重点に、Db2 前処理、COBOL カバレッジ、PL/I、JCL の既存テストも確認した。

**判定: 外部サービス不要の高速回帰試験を CI に追加し、端末の主要な異常系と接続組み合わせも補った。ただし、マージ・リリース時の全体ゲートとしてはまだ不十分。** 実際の 3287 印刷先の出力、Docker 上の COBOL/Db2 業務経路の現行コミットでの再実行、Windows 端末統合試験の CI 実行は残る。

以下の「通過」はこの日に Windows PowerShell から再実行した結果。`cobcicsd.exe` は先に `cargo build --manifest-path extras/cobcics/Cargo.toml --locked --offline` でビルドした。Docker デーモンは停止しており、Docker を要する試験は実行できなかった。

## 今回再実行したテスト

| テスト | 検証内容 | 結果 |
| --- | --- | --- |
| `cargo test --manifest-path extras/cobcics/Cargo.toml --locked --offline` | BMS 解析・描画、EBCDIC、日本語 SO/SI とフィールド境界、Telnet/TN3270E、モデル/属性、ワーカー応答、DFT 中断・不正レコード・欠落ファイル・同時転送、TDQ/SCS の再接続カーソルと単一リース | ライブラリー 49 件、CLI 1 件通過 |
| `python extras/cobcics/test_cics_preprocess.py` | CICS 前処理と構文診断 | 20 件通過 |
| [`test-wc3270-models.py`](../../extras/cobcics/test-wc3270-models.py) | 実際の wc3270 で Model 2～5、従来 TN3270、TN3270E、100×50 の拡張画面、色・下線をトレースで確認 | 6 経路通過 |
| [`test-wc3270-transport.py`](../../extras/cobcics/test-wc3270-transport.py) | wc3270 の TLS/STARTTLS、未信頼証明書の拒否と信頼済み証明書の検証、STARTTLS 拒否、NVT から 3270 への切替、pr3287 の LU/SCS 交渉、模擬プリンターへの TDQ/SCS と PRINT-EOJ。IBM-939 日本語の TLS 配送と 3287 再接続後の重複送信防止も確認 | 13 経路通過 |
| [`test-indfile-client.py`](../../extras/cobcics/test-indfile-client.py) | s3270 の DFT GET/PUT、6004 バイトと 128 KiB 超の分割バイナリ、既存ファイル上書き、ASCII、UTF-8/IBM-939 日本語バイナリ、TLS/STARTTLS 上の日本語送受信。wc3270 の Transfer 操作による日本語ファイルの送受信 | 15 経路通過 |
| [`test-japanese-wc3270-host.py`](../../extras/cobcics/test-japanese-wc3270-host.py) | IBM-939 の日本語画面表示と入力の往復を、通常接続、TLS、STARTTLS、NVT 切替で確認 | 4 経路通過 |
| `wsl -d Ubuntu -- ... test-extended-worker.py build/wc3270-models/extended-attributes-worker` | COBOL ワーカーが CCO7 で赤色・下線属性を返すことを確認 | 通過。ただし既存のビルド済みワーカーを使用 |
| `python tests/test_db2sql_preprocess.py` / `python tests/test_cobcov.py` / `python tests/test_cics_genapp_preprocess.py` | Db2 SQL 前処理、COBOL 分岐カバレッジ、GenApp CICS 前処理 | 9 / 2 / 5 件通過 |
| `cargo test --manifest-path extras/jclsh/Cargo.toml --offline -q` / `cargo test --manifest-path extras/pl1c/Cargo.toml --offline -q` | JCL インタプリタと PL/I コンパイラの単体試験 | 16 / 9 件通過 |

外部サービス不要の Rust と前処理の試験は [zosplus-regression.yml](../../.github/workflows/zosplus-regression.yml) に登録した。Ubuntu と Windows の両ジョブで push、pull request、手動起動時に実行する。ジョブに含む全コマンドは Windows でローカル実行済みだが、この変更の GitHub Actions 実行結果はまだない。試験出力と端末トレースの生成先は `build/` 以下で、Git には含めない。

## 用意されているが今回未実施の試験

| 試験群 | 内容 | 未実施の理由・範囲 |
| --- | --- | --- |
| [Bank of Z 画面試験](../testcases/bank-of-z-cics-screens.md) | メニュー、口座照会、送金、境界値、Db2 更新と取消、wc3270 操作 | Docker/Db2 と COBOL ワーカーが必要。Docker デーモン停止のため現行コミットで未実施 |
| [GenApp 統合試験](../testcases/cics-genapp-current.md) / [`test-genapp-all.ps1`](../../extras/cobcics/test-genapp-all.ps1) | ファイル、キュー、ロック、画面、顧客・保険更新、ロールバック、HTTP、IBM-037 EBCDIC | Docker/Db2 と Linux 版 Rust ホストのビルドが必要。同じ理由で未実施 |
| [日本語 COBOL 通し試験](../testcases/cics-japanese-ebcdic.md) | IBM-1399 の TN3270 と IBM-939 の wc3270 → Rust → COBOL 往復 | Docker 用スクリプトは今回未実施。前回は WSL ワーカーを Windows ホストへ中継して手動で通過したが、その中継手順はチェックインされた回帰ランナーではない |
| [JCL 業務・複数ステップ試験](../testcases/jcl-bnkstmt.md) と [PL/I 帳票試験](../testcases/pl1-bnkstmt.md) | 原本 JCL、Db2、帳票の期待値 | 単体試験のみ再実行。Docker/Db2 を使う通し試験は未実施 |
| GnuCOBOL 本体の `make check` / `make test` | Autotest と COBOL85 を含む本体の回帰試験 | 今回は実行していない。今回の wc3270 試験とは別の試験群 |

## 残る回帰リスク

1. **端末統合試験は CI に未登録。** 新しい wc3270/DFT/3287 テストは [接続ガイド](../userguide/wc3270-host.md)の手動コマンドで実行できる。CI の高速試験は端末実行ファイルを用いない。Windows に wc3270 一式と OpenSSL を用意して端末統合試験も自動化する必要がある。
2. **DFT の実クライアントによる異常系が薄い。** 中断後の一時ファイル破棄、重複レコード、不正長、欠落ファイル、同時転送は Rust 単体試験で確認した。s3270/wc3270 による送受信と大容量・上書きは通過したが、実クライアントを転送途中に強制終了した時の後片付けは未確認。日本語の `Mode=ascii` は文字化けするため、[利用ガイド](../userguide/wc3270-host.md)は `Mode=binary` を指定している。
3. **実際の 3287 印刷結果まで検証していない。** pr3287 は交渉まで、SCS データと PRINT-EOJ、再接続後のカーソル復元は模擬ソケットで照合した。pr3287 が印刷先へ書き出した結果、送信途中の失敗、複数プリンター接続は未確認。
4. **TLS の途中切断と全組み合わせは未確認。** 未信頼証明書の拒否、信頼済み証明書、STARTTLS 拒否、TLS/STARTTLS 上の DFT、TLS 上の日本語 SCS 印刷は通過した。TLS ハンドシェイク途中切断と 3287/DFT の全文字セット組み合わせは残る。
5. **日本語と業務画面の組み合わせが限定的。** IBM-939 の画面・フィールド境界、短い印刷・転送データは確認したが、IBM-930/1390/1399 と wc3270 の全組み合わせ、長文・改行、Bank of Z/GenApp の現行コミットでの日本語業務操作は確認していない。

次の優先順位は **(1) Windows 端末統合試験の CI 化、(2) Docker/Db2 の通し試験を定期実行、(3) 実 pr3287 の印刷出力と転送中断、(4) 残る日本語文字セットの業務画面** とする。現在の結果を「wc3270 互換性全体の保証」と解釈しない。
