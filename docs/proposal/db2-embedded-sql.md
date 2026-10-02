# Db2 向け埋め込み SQL 対応の実装提案

## 実装状況（2026-10-01）

最初の縦断実装を `bin/db2sql-preprocess.py`、`extras/db2sql/` に追加した。固定形式の `INCLUDE SQLCA`、`INSERT`、単一行の `SELECT ... INTO` と、英数字・整数のホスト変数を扱う。Compose 上の Db2 Community Edition 12.1.5.0 に、別のビルドコンテナーから IBM CLI で接続し、挿入、検索、該当なし（`SQLCODE=+100`）、複数行の単一行検索（`-811`）、重複キー（`SQLCODE=-803`、`SQLSTATE=23505`）を確認した。手順は `extras/db2sql/README.md` に記載する。

`cobc --db2-sql` を追加し、固定形式ソースを通常の COBOL 前処理より先に SQL 変換する経路を実装した。変換スクリプトはインストール対象であり、ソースチェックアウトでは `COB_DB2_SQL_PREPROCESSOR` でその場所を指定できる。`--save-temps` で変換結果を残せる。Db2実行時処理は引き続き任意の共有ライブラリとして提供し、`libcob` への組み込みは今後の課題とする。CICS文はまだ変換できない。

第2段階では、一般のSQLコピー（`CUSTDB2`など）、スキーマ修飾付き`DECLARE TABLE`、`SQLDA`、`UPDATE`、`DELETE`、COMMIT/ROLLBACK、カーソル、`COMP-3`小数とNULL指示変数を追加した。Bank-of-ZのSQLを含む15個の`.cbl`はすべて事前変換に成功した。Db2の結合試験で更新・削除、カーソルの複数行FETCHと終端、コミット後の参照、ロールバック、NULL入出力を確認した。CICS文を含む元プログラム全体のコンパイル・実行は未実施である。

EBCDIC対応との結合試験では、IBM-1399の日本語ホスト変数をDb2に渡すと、当初はEBCDICの生バイト列が保存される問題が見つかった。`libcob` のCOBOL-C文字列書き込みでDBCSを含む変換を行い、Db2出力の長さ判定を変換後のデータ長に合わせた。`COB_HOST_CODESET=UTF-8` の環境で、IBM-930/939/1390/1399の日本語入力・取得とDb2側のUTF-8値を確認した。IBM-037では英数字のDb2操作を確認した。

## 目的と到達点

GnuCOBOL のソースにある `EXEC SQL ... END-EXEC` をコンパイルし、Docker Compose で起動した Db2 Community Edition に対して SQL を実行できるようにする。最初の到達点は、CICS に依存しない固定形式の COBOL プログラムから、ホスト変数を使った `INSERT` と `SELECT ... INTO` を実行し、結果と `SQLCODE` を確認できることとする。その後、Bank-of-Z の Db2 使用箇所に必要な文を追加する。

この段階では Db2 for z/OS と Db2 Community Edition の SQL 方言・トランザクション・文字コードの完全一致は保証しない。Bank-of-Z の CICS プログラム全体を動かすための、Rust 製 TN3270 ホストと COBOL の統合方針は [CICS 互換機能の提案](cics-rust-tn3270.md)に記す。

## 現状調査

- 本リポジトリは `cobc/` が COBOL を前処理・C へ変換し、`libcob/` が実行時処理を担う。`EXEC SQL` の構文処理・実行経路は現状見当たらない。
- `copy/sqlca.cpy` には `SQLCODE`、`SQLSTATE` などの定義が既にある。ただし SQL 実行後に更新する仕組みはない。
- `reference/Bank-of-Z-main/src/base/cics/` の `.cbl` と `.cpy` から `EXEC SQL` を 121 ブロック確認した。内訳は `INCLUDE` 40、`INSERT` 20、`SELECT` 13、`UPDATE` 11、`DECLARE` 10、`OPEN` 6、`FETCH` 6、`CLOSE` 6、`DELETE` 5、`COMMIT` 4。`INCLUDE CUSTDB2` などは SQL の `DECLARE ... TABLE` を含むコピーであり、実行文ではない。
- 例として `INQCUST.cbl` は `INCLUDE SQLCA` と `SELECT ... INTO :HV-* ... WHERE ... = :HV-*` を使う。`CREACC.cbl` は `INSERT` と packed decimal のホスト変数を使う。ほとんどの対象プログラムは `EXEC CICS` も含む。

## 設計

### 1. コンパイル経路

`cobc` に明示的な Db2 埋め込み SQL オプション `--db2-sql` を追加した。このオプションが有効な場合だけ、通常の COBOL 前処理・解析より前に SQL 変換段階を通す。通常のコンパイル挙動は維持する。

SQL 変換段階は、固定形式の列規則、継続行、コメント、引用符を認識し、`EXEC SQL` ブロックを COBOL の宣言または `CALL` に変換する。単純な文字列置換は使わない。`EXEC SQL INCLUDE SQLCA` は既存の `copy/sqlca.cpy` を取り込む。`INCLUDE <member>` は `-I` などのコピー探索規則に従って展開し、SQL の `DECLARE ... TABLE` は静的検査用として扱い、実行コードを生成しない。変換後のソースと元のファイル・行の対応を保ち、エラー時に元の位置を表示する。

SQL 文中の入力ホスト変数を `?` パラメーターに変え、`SELECT ... INTO` や `FETCH ... INTO` の出力変数は SQL 文字列から外して順序付きの受け取り先にする。SQL リテラル・コメント中の `:` はホスト変数として扱わない。変数名、`PIC`、`USAGE`、長さ、符号、小数桁、NULL 指示変数を検査し、未対応型はコンパイル時に明示的なエラーとする。SQL 文をホスト変数の文字列連結で組み立てない。

生成コードは専用の COBOL 呼び出し API を通じて実行時ライブラリへ、SQL テキスト・文 ID・入出力変数のアドレスと型情報を渡す。生成コードを COBOL 文として残すため、既存の `cobc` の C コード生成器へ SQL 固有の AST を最初から追加する必要はない。`--save-temps` で変換結果を検査できる。

### 2. Db2 実行時処理

`libcob` 側に独立した SQL 実行モジュールを追加し、IBM Data Server Driver for ODBC and CLI を使って Db2 に接続する。ODBC/CLI の `SQLPrepare`、`SQLBindParameter`、`SQLExecute`、`SQLFetch`、`SQLEndTran` と診断 API を利用する。Db2 サーバーは Community Edition、クライアントドライバーは IBM の配布物という別の依存物として扱い、リポジトリには同梱しない。ビルド時にこの機能を任意に有効化できるようにする。

接続先・データベース・ユーザー・パスワードは実行時の環境変数または DSN から受け取り、生成ソースや Compose ファイルに資格情報を埋め込まない。初期段階はプロセス内で接続を管理し、自動コミットは無効にする。`COMMIT WORK` は明示的にコミットする。`ROLLBACK` も同じ API で扱う。CICS の `SYNCPOINT` と接続の所有関係は CICS 対応時に決める。

COBOL の `PIC X`、表示形式数値、`COMP`、`COMP-3` と Db2 の `CHAR`、`VARCHAR`、整数、`DECIMAL` の間に型変換を設ける。桁あふれ、文字コード変換、NULL 指示変数、切り捨ては黙って処理せず、診断を返す。最初の結合試験は ASCII/UTF-8 の英数字データに限定し、EBCDIC 互換性は別途評価する。

各文の後に既存の SQLCA の `SQLCODE` と `SQLSTATE` を更新する。成功は `SQLCODE=0`、該当行なしは `SQLCODE=+100` を必須とする。エラーは CLI 診断から SQLSTATE、ネイティブコード、メッセージを取得する。Db2 for z/OS の全 SQLCODE が同じ値になるとは仮定せず、値の差は結合試験で記録する。

### 3. Docker Compose の検証環境

`icr.io/db2_community/db2` を使う `compose.yaml` と、資格情報を Git 管理しないための `.env.example`、起動・停止手順を追加する予定。Db2 Community Edition は無料のエディションだが、オープンソース製品ではない。ここでは依頼の「オープン版」を、この Community Edition と解釈する。設定には `LICENSE=accept`、`DB2INSTANCE=db2inst1`、`DB2INST1_PASSWORD`、`DBNAME`、ポート `50000`、`/database` 用の永続ボリュームを含める。IBM の手順に従いコンテナーには `privileged: true` が必要となるため、開発・テスト用途に限定する。実装時に動作確認済みのイメージタグまたは digest を固定し、初期化完了をログまたは接続試行で確認してからテストを開始する。

ホスト側で GnuCOBOL をビルドして CLI ドライバーから Compose 上の Db2 へ接続する構成を基本とする。Windows では Docker Desktop 上の Linux コンテナーを使用する。CI では特権コンテナーが利用できる環境に限って結合試験を実行し、通常のコンパイラ試験と分離する。

## 実装順序と受け入れ条件

1. **最小の縦断実装:** `EXEC SQL INCLUDE SQLCA`、単一行 `INSERT`、`SELECT ... INTO`、入力ホスト変数、`PIC X` と整数に対応する。Compose の Db2 へ挿入・検索し、成功時の `SQLCODE=0` と未検出時の `+100` を確認する。固定形式と自由形式の通常ソースが既存どおりコンパイルできることも確認する。
2. **Bank-of-Z の主要文:** `UPDATE`、`DELETE`、`COMMIT WORK`、`ROLLBACK`、`DECLARE CURSOR`、`OPEN`、`FETCH`、`CLOSE`、`SQL INCLUDE` 内の `DECLARE TABLE`、`COMP-3`、NULL 指示変数を追加する。`INQCUST.cbl` と `CREACC.cbl` から SQL 部分を取り出した独立テストで、元の SQL とホスト変数宣言を極力変更せずに実行する。
3. **互換性検証:** Bank-of-Z の SQL ブロックを一括変換し、未対応構文を一覧化する。Db2 Community Edition 上で表定義・サンプルデータを用意し、カーソル、更新、コミット・ロールバック、エラー診断を結合試験する。CICS と共存する実プログラムの実行試験は CICS 実装後に行う。

最初のマイルストーンの完了条件は、クリーンな Compose 起動後に COBOL サンプルのビルドと実行が再現可能で、ホスト変数を使った SQL が Db2 上のデータを実際に変更・取得できることとする。接続失敗、SQL エラー、未対応型も明確な診断と非ゼロの `SQLCODE` を返す。

## 主な設計上の注意

- Db2 Community Edition は Db2 for z/OS の代用品ではあるが同一製品ではない。SQL 方言、SQLCODE、照合順序、日付形式、コードページは差分を確認する。
- 固定形式の桁位置、`EXEC SQL INCLUDE` と通常の `COPY` の探索順、SQL 内のコメント・引用符は最初の実装からテストする。
- 接続と文ハンドルは正常終了・エラー終了とも解放する。更新系はパラメーター束縛を必須にし、資格情報をログへ出さない。
- Db2 の Docker イメージは IBM が非本番・無サポート用途として提供している。サーバーイメージと CLI ドライバーの取得条件を実装時に明記する。

## 参照資料

- リポジトリ内: `reference/Bank-of-Z-main/src/base/cics/cobol/INQCUST.cbl`、`CREACC.cbl`、`reference/Bank-of-Z-main/src/base/cics/copy/CUSTDB2.cpy`、`copy/sqlca.cpy`
- [IBM: Db2 Community Edition for Docker](https://www.ibm.com/docs/en/db2/12.1.x?topic=deployments-db2-community-edition-docker)
- [IBM: Linux への Db2 Community Edition コンテナー導入](https://www.ibm.com/docs/en/db2/12.1.x?topic=system-linux)
- [IBM: Db2 CLI/ODBC の関数一覧](https://www.ibm.com/docs/en/db2/12.1.x?topic=odbc-cli-functions)
- [IBM: CLI の診断情報取得](https://www.ibm.com/docs/en/db2/12.1.x?topic=cof-sqlgetdiagrec-function-get-multiple-fields-settings-diagnostic-record)
