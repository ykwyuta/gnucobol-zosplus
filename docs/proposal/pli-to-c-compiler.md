# PL/I を C 経由でコンパイルする提案

## 目的と到達点

GnuCOBOL と同じ **ソース解析 → C 生成 → ホスト C コンパイラでビルド → 実行時ライブラリとリンク** という経路で、z/OS 向け PL/I ソースを実行可能にする。最初の到達点は、Bank of Z の [`BNKSTMT.pli`](../../reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli) を編集せずにコンパイルし、既存の Db2 Compose 環境と JCL の DD 割り当てを使って月次明細を生成することとする。対応外の構文・型・実行時機能は、元ソースの位置を付けてエラーにする。

ここでいう「同じ方式」は C を中間言語に使う構成を指す。`cobc` の COBOL 文法へ PL/I を押し込まず、PL/I 専用フロントエンドを設ける。IBM Enterprise PL/I 全体との互換性は段階的に広げる。C を生成できること、実行できること、z/OS と同じ結果になることは別々に測る。

## 資材と既存実装の調査

- GnuCOBOL の `cobc/` は COBOL を C に変換し、`libcob/` が COBOL 固有の実行時処理を担う。`cobc -C` で生成 C を確認できる。既存の `--db2-sql` は [`bin/db2sql-preprocess.py`](../../bin/db2sql-preprocess.py) が埋め込み SQL を **COBOL の `CALL`** に変換し、[`extras/db2sql/db2sql.c`](../../extras/db2sql/db2sql.c) が Db2 CLI を使う仕組みである。その出力を PL/I の前処理結果として直接流用することはできない。
- Bank of Z の `.pli` は現時点で 2 本。バッチの `BNKSTMT.pli` は Db2 の `ACCOUNT`、`CUSTOMER`、`PROCTRAN` を参照する。もう 1 本の [`IBLOGIN.pli`](../../reference/Bank-of-Z-main/src/base/ims/pli/IBLOGIN.pli) は `PLITDLI`、PCB、IMS セグメントを使うため、コンパイラの構文対応と IMS 実行機能を別々に評価する。
- `BNKSTMT.pli` は `PROCEDURE OPTIONS(MAIN)`、内包 `PROCEDURE`、レベル付き `DCL`、`CHAR(n)`、`FIXED BIN(15/31)`、`FIXED DEC(p,s)`、`BIT(1)`、`INIT`、`IF` / `DO WHILE`、ラベルと `GO TO`、`RETURN`、`CALL`、文字列連結、`SUBSTR` / `TRIM` / `CHAR` / `ABS` / `DATETIME` を使う。帳票には `PUT SKIP LIST`、`PUT SKIP EDIT`、`PUT STRING ... EDIT ... (A, F(...))`、`PUT PAGE` が必要。
- 埋め込み SQL は `INCLUDE SQLCA`、`DECLARE` / `OPEN` / `FETCH` / `CLOSE` カーソル、`SELECT ... INTO`、`COMMIT WORK`。`FETCH` には数値・文字列のホスト変数と NULL 指示変数が混在する。SQL 中の `DATE(...)`、`CHAR(..., ISO)`、`SUBSTR` は PL/I の式ではなく Db2 SQL として扱う。
- [`BNKSTMT.jcl`](../../reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl) は `DATECARD` と `SORTCODE` をインストリーム DD として渡し、`IKJEFT01` の `DSN RUN PROGRAM(BNKSTMT)` を使う。PL/I 側は `FILE RECORD INPUT ENV(F RECSIZE(80))` と `ON ENDFILE` を使う。JCL の実行計画は [Rust 製 JCL シェルの提案](jcl-rust-shell.md)で扱う。

## 先行事例の調査と採用判断（2026-10-04）

| 事例 | 確認した仕組み | この提案への反映 |
| --- | --- | --- |
| [INRIA の PL/1 → C 翻訳事例](https://cadp.inrialpes.fr/ftp/presentations/Garavel-ENS-23.pdf) | INRIA の回顧資料は、SYNTAX システムの約 45,000 行の PL/1 を C に翻訳した事例と、1986 年の翻訳器の報告書を挙げる。元の報告書と実装は今回確認できていない。 | PL/I → C の経路自体には先例がある。ただし z/OS の Db2、JCL、EBCDIC までの互換性を示す根拠には使わず、現行資材で個別に検証する。 |
| [IBM Enterprise PL/I の `PP` オプション](https://www.ibm.com/docs/en/epfz/5.3.0?topic=descriptions-pp) | `INCLUDE`、`MACRO`、`SQL`、`CICS` を独立した前処理段階とし、実行順を指定できる。 | 各段階の入出力と元ソース位置を保存する設計にする。`BNKSTMT` では必要な `SQL INCLUDE` と SQL 解析から始め、後にマクロ・CICS を追加しても順序を固定した一体型パーサーにしない。 |
| [Micro Focus Open PL/I の前処理・ファイル対応](https://www.microfocus.com/documentation/openpli/80/puusng.htm)と[Db2 SQL 対応](https://www.microfocus.com/documentation/openpli/80/pusqlo.htm) | マクロ、CICS、Db2 の前処理を分け、前処理だけの出力も保存できる。論理ファイル名を実ファイルへ解決し、SQL ホスト変数と指示変数の型対応を明示している。 | `--emit=preprocessed` で段階別の結果を検査可能にし、DD 名解決と SQL 型検査を独立した契約にする。資料の版に固有の制限を IBM z/OS の仕様として扱わない。 |
| [Iron Spring PL/I 1.4.1](https://www.iron-spring.com/readme_linux.html)と[Programming Guide](https://www.iron-spring.com/prog_guide.html) | コンパイラと PL/I 実行時ライブラリを分離し、実行時エラーの元行表示を備える。Linux 版は 32 ビット実行で、共有オブジェクト用 PIC を生成できないと記載されている。 | `libpli` と元行マップの考え方を採用する。既存の 64 ビット Db2 ビルドとそのまま結合できる前提にはせず、Iron Spring のコンパイラ／ランタイムを必須依存にしない。 |
| [pl1gcc 0.0.15](https://pl1gcc.sourceforge.net/) | GCC 用 PL/I フロントエンドの公開版は、プロジェクト自身がコード生成未実装と記載している。 | GCC バックエンドへの接続を最初の実装経路に選ばない。構文解析だけで「コンパイル対応」と判定せず、生成物の実行まで受け入れ条件に含める。 |
| [p2c（Pascal → C）](https://github.com/FranklinChen/p2c) | PL/I とは別言語だが、生成 C と専用ランタイムを C コンパイラでビルドする実例。既定の古い C 出力が現行の厳格な C 規格で問題になる点を公開テストで検証している。 | C 出力の方言を固定し、生成 C の厳格コンパイルと実行を CI の必須検査にする。p2c の Pascal 実装を PL/I の意味規則の根拠にはしない。 |

調査した実装から前処理境界、型表、ランタイム、診断、生成 C の検証方法を取り込む。他の PL/I コンパイラや z/OS での実行結果との比較試験は、この計画に含めない。先行事例の記述は実行環境の導入を要求するものではない。

### 検証の基準

PL/I の文字列、固定小数点、条件処理、編集出力は、[IBM の言語資料](https://www.ibm.com/docs/en/epfz/5.3?topic=elements-data-types-attributes)から小型テストの入力と期待値を先に作る。Db2 の取得行、NULL、カーソル終端、`SQLCODE` は固定したテストデータと SQL の直接実行結果で確認する。`BNKSTMT` の帳票は、投入した口座・取引から期間、件数、金額合計、ページ境界を独立に計算した期待値で検証する。生成物から期待値を作らない。

結果は「構文・型検査」「小型テスト」「Db2 結合」「JCL からの通し実行」に分けて報告する。これらの試験で Bank of Z の対象動作を確認する一方、IBM Enterprise PL/I と出力が一致した、あるいは z/OS と完全互換であるという主張はしない。

## 提案する構成

```text
BNKSTMT.pli ── pl1c（字句解析・PL/I AST・型検査・SQL 解析）
                     ├── 生成 C とソース位置マップ
                     └── C コンパイラ ── 実行ファイル BNKSTMT
                                          ├── libpli（PL/I 実行時処理）
                                          └── 共通 Db2 CLI 実行核
                                                   ↑
BNKSTMT.jcl ── jclsh ── DD / Db2 設定 / プログラム登録表
```

1. **コンパイラと前処理:** `extras/pl1c/` に Rust 製 `pl1c`（仮称）を置く。字句解析は PL/I のコメント、文字・ビット定数、セミコロン、ラベル、SQL ブロックを区別し、AST と型付き中間表現を経て C を生成する。前処理は `%INCLUDE` / マクロ / CICS / SQL を拡張可能な段階として表し、元ファイル・行・列と展開元を各段階から引き継ぐ。最初の `BNKSTMT` 対応では使う段階だけ実装し、未対応の段階を要求するソースは診断する。`-C` は C と位置マップ、`-c` はオブジェクト、`-x` は実行ファイルを作り、`-I`、`-o`、`--save-temps`、`--emit=preprocessed` を用意する。生成 C はホストコンパイラと `libpli` でビルドでき、Windows/Linux のホスト差はビルド設定で吸収する。構文を文字列置換で COBOL に翻訳しない。
2. **型と意味:** `DCL` の階層・スコープ・`INIT` を解析し、PL/I の固定長文字列の空白埋め、比較、代入時の切り詰め、1 起点の `SUBSTR`、ビット演算、式の評価順を保つ。`FIXED BIN` の精度から格納幅と範囲を決め、`FIXED DEC` は浮動小数点に置き換えず、精度・小数桁・丸め／切り捨て・桁あふれを型付き演算とランタイムで処理する。集約は C の暗黙のパディングに依存しない。IBM の [データ型・属性](https://www.ibm.com/docs/en/epfz/5.3?topic=elements-data-types-attributes)と[固定小数点の演算結果規則](https://www.ibm.com/docs/en/epfz/5.3?topic=operations-results-arithmetic)を基準に、実装した規則と未実装の規則をテストごとに記録する。
3. **制御と入出力:** 内包 `PROCEDURE` はスコープ付き C 関数または明示的な環境を渡す関数へ下げる。`ON ENDFILE` はファイル別の条件ハンドラとして扱い、`GO TO` の脱出先と後処理を検査する。`DATECARD` / `SORTCODE` は JCL の DD 台帳から固定長 80 バイトのレコードとして読み、`SYSPRINT` への `PUT` の行・ページ・編集書式を実装する。**DD が無い、OPEN に失敗した、EOF に達した** は別の条件として検証し、ソースのフォールバックが実際に有効な場合だけその動作を再現する。ホストの標準入出力で黙って代用しない。
4. **Db2:** PL/I の `EXEC SQL ... ;` を SQL とホスト変数の型付きノードに分ける。SQL 文字列のリテラル・コメント内の `:` を誤認せず、`SELECT ... INTO` と `FETCH ... INTO` の出力先、`:変数 :指示変数`、SQLCA を検査する。Open PL/I の型表を初期の確認項目に使い、`FIXED BIN(15)` は `SMALLINT` 相当の 2 バイト、`FIXED BIN(31)` は `INTEGER` 相当の 4 バイト、`FIXED DEC(p,s)` は `DECIMAL(p,s)` に対応付ける。NULL 指示変数は別の符号付き 2 バイト値として扱い、固定長 `CHAR(n)` と日付文字列は長さを明示する。最終的な受理条件と精度は IBM の PL/I / Db2 仕様と実 Db2 接続試験で決める。SQL は CLI のパラメータ束縛で実行し、各文後に PL/I の SQLCA を更新する。既存 `db2sql.c` の接続・カーソル・診断処理は言語中立の C API と COBOL アダプターへ整理し、PL/I 専用の型変換アダプターを追加する。COBOL の `PIC` レイアウトや `CALL` ABI を PL/I に仮定しない。[IBM の PL/I ホスト変数](https://www.ibm.com/docs/en/db2-for-zos/13.0.0?topic=pli-host-variables-in)、[SQLCA](https://www.ibm.com/docs/en/db2-for-zos/12.0.0?topic=patiss-defining-sql-communications-area-sqlstate-sqlcode-in-pli)、[指示変数](https://www.ibm.com/docs/en/db2-for-zos/13.0.0?topic=variables-indicator-arrays-structures)を照合対象にする。
5. **文字コードと JCL:** `-fsource-codeset` 相当で PL/I ソースを読み、`-fdata-codeset` 相当で `CHAR` の内部バイト表現を決める。DD のレコード、帳票、Db2 CLI の各境界で明示的に変換し、既存 COBOL の CP037 と日本語 EBCDIC の変換表・テストを再利用できる部分だけ共有する。`jclsh` は `BNKSTMT` を登録済み実行ファイルとして起動し、DD、接続プロファイル、戻りコード、帳票を受け渡す。`PLAN(BANKZPLN)` と `LIB(...)` は登録・構成の検証対象とし、z/OS の DBRM バインドを行ったことにはしない。

## 実装順序と受け入れ条件

| 段階 | 実装 | 確認すること |
| --- | --- | --- |
| 0. 構文棚卸しと仕様固定 | 2 本の `.pli` の構文、型、組み込み関数、外部依存を位置付きで一覧化し、`BNKSTMT` を最初の対象とする。 | 対応済み／構文未対応／実行時未対応を機械的に区別できる。 |
| 1. PL/I → C の縦断実装 | `PROCEDURE`、宣言、代入、式、分岐、ループ、文字列と固定小数点、生成 C のビルド。前処理結果と位置マップを保存。 | 小型 PL/I プログラムを `-C`、`-c`、`-x` で扱え、型エラーが元の行で報告される。生成 C を Linux の GCC/Clang と Windows の構成済み C コンパイラで警告をエラーとしてビルドし、実行結果も一致する。 |
| 2. バッチ実行時処理 | DD の固定長入力、条件ハンドラ、`PUT` の書式、ページ制御、文字コード。 | `DATECARD=202606`、`SORTCODE=123456` と EOF、空 DD、欠落 DD を区別し、帳票の行・改ページが再現する。 |
| 3. SQL 結合 | PL/I ホスト変数・NULL 指示変数・SQLCA、2 種のカーソル、`SELECT INTO`、`COMMIT` を Db2 CLI へ接続。 | Compose 上の Db2 で口座、顧客、取引を取得し、成功・未検出・NULL・SQL エラー時の値を照合できる。 |
| 4. 原本と JCL の通し試験 | 無編集の `BNKSTMT.pli` をコンパイルし、無編集の `BNKSTMT.jcl` から `DSN RUN` 経由で起動。 | 月次明細の対象口座数、期間、取引行、金額集計、ページ、RC、SQLCA を期待値と比較する。 |
| 5. 対象拡張 | `IBLOGIN.pli` の文法上の差分を追加し、IMS 呼び出しは別の実行機能として設計。 | `PLITDLI` が未接続なら実行不能を明示し、構文コンパイルの達成と IMS 業務動作を混同しない。 |

通し試験の Db2 表は既存の [`bank-test-db.sql`](../../extras/cobcics/bank-test-db.sql) に `ACCOUNT`、`CUSTOMER`、`PROCTRAN` がある。既存のテスト投入処理を確認したうえで、固定シードの顧客・複数口座・対象月内外の入出金・NULL 値を用意する。期待帳票は SQL の抽出結果と独立に計算して `docs/testcases/` に記録し、単にプログラムが出力した値を正解として複製しない。Db2 LUW と z/OS の `DATE`、`CHAR(..., ISO)`、`SQLCODE` の差があれば SQL 変換規則またはテスト結果として明示する。

## 境界と主なリスク

- PL/I の完全実装、IMS ランタイム、z/OS ロードモジュールの実行はこの最初の到達点に含めない。`IBLOGIN.pli` のコンパイル／リンク／実行の可否は別々に報告する。
- `FIXED DEC` の格納形式と演算精度、`PUT EDIT` の空白・符号・丸め、PL/I 条件処理は C の標準演算や `printf` だけでは一致しない。数値と帳票の境界値を先にテスト化する。
- 既存 Db2 コードの共通化では COBOL 側の ABI と回帰試験を維持する。既存の EBCDIC と Db2 の結合試験を再実行し、PL/I 固有変換の導入で COBOL の動作を変えない。
- Bank of Z の例は 1 本のバッチ PL/I から必要機能を決める。別の PL/I 資材に適用するときは対応構文表を更新し、未対応の属性や暗黙宣言を推測して通さない。
- 生成 C は選択した C 規格と対象コンパイラを明記し、未定義動作に依存する式、暗黙の関数宣言、ホストの整数幅・構造体パディングに依存するコードを出さない。ソース位置マップと `--emit=preprocessed` の成果物も回帰試験の対象にする。

関連提案: [JCL シェル](jcl-rust-shell.md)、[Db2 埋め込み SQL](db2-embedded-sql.md)。既存の利用方法: [Db2 ガイド](../userguide/db2.md)、[EBCDIC ガイド](../userguide/ebcdic.md)。

## 実装状況（2026-10-04）

`extras/pl1c/` に Rust 製フロントエンドと C ランタイムの初期版を追加した。小型 PL/I プログラムの C 生成、WSL Ubuntu の GCC でのビルドと実行を確認した。Db2 では Docker 内の CLI から `SELECT INTO` とカーソルを実行し、パラメーター束縛、行取得、終端時の `SQLCODE=100` / `SQLSTATE=02000` を確認した。現時点の受け入れ結果は [pl1c README](../../extras/pl1c/README.md)に記載する。

段階 1 と 3 の一部に続き、原本 `BNKSTMT.pli` の FILE 宣言、`ON ENDFILE`、`PUT SKIP EDIT`、`PUT STRING ... EDIT` を実装した。原本を無編集で C に変換し、Docker 内の Db2 CLI とリンクして実行ファイルを生成した。WSL で DD カード入力、欠落時の分岐、`A` / `F` 編集出力を小型例で確認した。

追加検証として、専用 `PL1TEST` スキーマに `ACCOUNT`、`CUSTOMER`、`PROCTRAN` を作成した。2026 年 6 月の 2 口座・2 取引と期間外 1 取引を投入し、原本が出力する口座数、期間、取引の抽出、期首・期末残高、改ページを固定期待値と照合した。実行中に PL/I の `TRIM` が両端の空白を除くべき点も修正した。[通し試験](../testcases/pl1-bnkstmt.md)は Docker の Db2 で成功した。

固定長レコードの一般的な読み方、帳票書式の IBM 互換性、EBCDIC、型付き SQL ホスト変数、元ソース位置マップ、JCL 結合は残作業である。`pl1c` の生成 C は現時点で `-C` のみ対応する。
