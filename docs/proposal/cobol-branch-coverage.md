# COBOL の分岐を基準にしたカバレッジ測定の提案

## 実装状況

`--cobol-branch-coverage` を追加し、ソース由来の `IF` の真・偽、`EVALUATE` の各 `WHEN`、`PERFORM UNTIL` の継続・終了、`PERFORM TIMES` の0回・1回以上、`SEARCH` / `SEARCH ALL` の各 `WHEN` と未検出、`GO TO ... DEPENDING ON` の各転送先と範囲外通過を個別に数える。さらに `ON SIZE ERROR`・`ON OVERFLOW`、動的 `CALL ... ON EXCEPTION`、ファイル操作の `AT END`・`INVALID KEY`・`AT EOP`、`DISPLAY`・`ACCEPT`・XML・JSON の `ON EXCEPTION` の通常側とハンドラ側を数える。`ELSE` のない `IF` は偽経路を、`WHEN OTHER` のない `EVALUATE` は暗黙の不一致経路を記録する。複数の `WHEN` が文群を共有する場合も、選ばれた最初の `WHEN` だけを増やす。実行時には `COB_COVERAGE_DIR`（既定 `./cobol-coverage`）へカウンターファイルを出し、`bin/cobcov report <マップ> <実行データ>` でテキスト、JSON、LCOV の集計を得られる。マップは現段階では実行した `cobc` の作業ディレクトリに `cobcov-<build-id>.json` として生成する。

現時点の数値は **上記の実装済み構文の結果だけ**を分母にする。`IF` の位置は構文解析で保持するため、ネストした文と `COPY` 内の文を元ファイルに対応付けられる。Autotest では `ELSE` あり／なし、ネスト、`COPY`、同一行に複数ある `IF`、`EVALUATE` の共有文群・`ALSO`・`THRU`・`WHEN OTHER`・暗黙の不一致・`EVALUATE TRUE`、`PERFORM` の前判定・後判定・単独／多重 `VARYING`・0回/複数回、`SEARCH` の各 `WHEN`・`AT END`・暗黙の未検出、`SEARCH ALL` の成功・未検出、`GO TO DEPENDING ON` の全転送先・範囲外、`ON SIZE ERROR`・`ON OVERFLOW`、動的 `CALL ON EXCEPTION`、ファイルの `AT END`・`INVALID KEY`・`AT EOP` の両経路を確認済み。`AT END` では別種のファイルエラーをハンドラ実行として数えないことも確認した。XML/JSON の Autotest は追加したが、現在の Docker イメージでは実行用の開発ライブラリがないためスキップされる。画面の `DISPLAY`・`ACCEPT` ハンドラも実行検証は残る。定数条件と最適化で除去された判断をマップへ明示する処理、列番号、並列スレッドでのカウンター更新、動的ロード／`CANCEL`、複数プラットフォームでの検証は次の作業として残る。

利用例：

```sh
cobc --cobol-branch-coverage -x program.cbl
COB_COVERAGE_DIR=coverage-data ./program
cobcov report . coverage-data --fail-under-branches=80
cobcov report . coverage-data --format lcov > cobol.info
```

ディレクトリを指定すると、その中の全マップを集計対象にする。古いビルドのマップも分母に入るため、ビルドごとに作業ディレクトリを分けるか、対象のマップファイルを明示する。

## 目的

COBOL のソースに書かれた分岐の各結果が、テスト中に少なくとも一度選ばれたかを測る。生成 C コードの `if` やランタイム内部の分岐は分母に入れない。結果は元の COBOL ファイル、プログラム、行・列、分岐の種類に結び付ける。

この提案でいう分岐カバレッジは、**到達可能な COBOL 分岐結果のうち、実行された結果の割合**である。条件式内の個々の `AND` / `OR`、MC/DC、文カバレッジは別の指標として扱う。例えば `ELSE` のない `IF` も真・偽の2結果を持ち、偽で通過した実行を記録する。

## 現状と実装上の根拠

- `cobc --coverage` は生成 C のコンパイル・リンクへ `--coverage` を渡し、COBOL の行情報を C の `#line` に反映する。`Makefile.am` の `CODE_COVERAGE_BRANCH_COVERAGE=1` も GnuCOBOL 自身の C コードのテストに用いる設定であり、COBOL の分岐結果を定義しない。
- `-fprof` と `libcob/profiling.c` は COBOL の節・段落・呼び出しの回数と時間を記録する。分岐結果の一覧は持たない。
- `cobc/tree.h` の `cb_tree_common` には `source_file`、`source_line`、`source_column` がある。`cb_if` は `STMT_IF`、`STMT_WHEN`、`STMT_PRESENT_WHEN` を共用し、`cb_statement` には例外・ファイル操作のハンドラがある。
- `EVALUATE` は `cobc/typeck.c` の `build_evaluate` で内部の `IF` 列とジャンプへ展開される。`SEARCH` と `PERFORM` も `cobc/codegen.c` で複数の C の条件へ展開される。生成 C の分岐数から元の COBOL 分岐数を復元する方式は採らない。
- `codegen.c` の `output_source_reference` はソース位置を出力し、`output_if`、`output_perform`、`output_search` および `cb_statement` の出力箇所が計測コードを置く候補になる。ただし内部生成の `IF` やエラーチェックは除外する必要がある。

## 利用者向けの仕様

### 起動と出力

- `cobc --cobol-branch-coverage -x program.cbl` で明示的に有効化する。`--coverage` の C カバレッジ、`-fprof` のプロファイルとは独立させ、併用可能にする。計測に GCC の `gcov` を要求しない。
- コンパイル時に、分岐結果の静的な一覧とソース位置を記したマップを生成 C と共に出力する。既定の場所は通常のコンパイル成果物と同じディレクトリの `<出力名>.cobcov-map.json` とする。`-c` / 複数モジュールではモジュールごとのマップを作る。
- 実行時には `COB_COVERAGE_DIR`（既定 `./cobol-coverage`）に、プロセスごとに一意のカウンターファイルを書き出す。並列テストのプロセスは同じファイルを上書きしない。正常終了・`STOP RUN`・`GOBACK`・`cob_tidy` での保存を対象とし、強制終了時の未保存データは保証しない。
- `cobcov report <マップまたはディレクトリ> <実行データのディレクトリ>` で複数実行をマージする。標準出力の集計、機械向け JSON、LCOV の `BRDA` を出力可能にする。表示例は `IF 42: 真 3 / 偽 0`、`EVALUATE 77: WHEN 'A' 2 / WHEN OTHER 0` とする。LCOV では COBOL ソースを `SF` とし、同じ行の複数分岐を別の block/branch 番号で区別する。
- `cobcov report --fail-under-branches=80` のように、CI 用の閾値を指定できるようにする。分母に入れていない構文がある段階では、レポートに計測対象の種類と未対応の種類を明示し、「COBOL 全体で100%」とは表示しない。

例として、`ELSE` のない `IF` と、2つの `WHEN` を持ち `WHEN OTHER` のない `EVALUATE` は、合計5結果（真、偽、各 `WHEN`、暗黙の不一致）を持つ。テストで `IF` の真と2番目の `WHEN` だけを通れば、結果は `2/5 = 40%` となる。

### 分岐結果の定義

| COBOL の構文 | 数える結果 | 注意 |
| --- | --- | --- |
| `IF` | 真、偽 | `ELSE` がなくても偽を数える。ネストした `IF` は別の判断。 |
| `EVALUATE` | 選ばれた各 `WHEN` 選択肢、`WHEN OTHER` または暗黙の不一致 | 1回の `EVALUATE` で結果を1つだけ増やす。同じ文群へ続く複数 `WHEN` も、元の各選択肢に位置を持たせる。 |
| `PERFORM ... UNTIL` / `VARYING` | 継続、終了 | `TEST BEFORE` / `TEST AFTER` の評価位置を保つ。多重 `VARYING` は各 `UNTIL` 判断を別に数える。 |
| `PERFORM ... TIMES` | 0回で通過、1回以上実行 | 反復回数の大きさでは分母を増やさない。 |
| `SEARCH` / `SEARCH ALL` | 選ばれた `WHEN`、`AT END` または暗黙の未検出 | 内部の添字境界・二分探索の C 条件は数えない。 |
| `GO TO ... DEPENDING ON` | 各転送先、範囲外の通過 | 無条件の `GO TO` は分岐ではない。 |
| `ON EXCEPTION`、`ON SIZE ERROR`、`AT END`、`INVALID KEY` など | 明示ハンドラ、通常経路、必要なら未処理経路 | 文の種類とハンドラ種別を明示して分類する。ランタイム内部の再試行や検査は除外する。 |
| Report Writer の `PRESENT WHEN`、JSON/XML の `SUPPRESS WHEN` | 表示・生成する、抑止する | 専用の判断として分類し、内部で生成された `IF` は数えない。 |

定数条件などでコンパイラが除去した結果は「到達不能／最適化で除去」としてマップに残し、分母から除外する。判断そのものが消えた場合も、レポートに理由を示す。条件式の各項目や短絡評価を数える「条件カバレッジ」はこの機能の数値に混ぜない。

## 実装方針

### 1. フロントエンドでソース由来の判断を識別する

構文解析・型検査時に、元の COBOL 文に対応する判断記述子を作る。記述子は `program-id`、構文の種類、元ファイルの位置、選択肢の位置とラベル、到達可能性、生成元がユーザー記述か内部生成かを持つ。分岐 ID はマップ内で一意に振り、同じ行に複数の文がある場合は列と連番を用いる。コピー句由来の文はコピー元ファイルの位置を記録する。

特に `EVALUATE` は `build_evaluate` が複数の `WHEN` を論理和に畳む前に選択肢の位置と ID を保存する。計測付きの展開では元の評価順・短絡評価を維持し、選ばれた選択肢のカウンターだけを増やす。`STMT_WHEN` という型だけでは `EVALUATE` と `SEARCH` の内部生成文を区別できないため、AST に明示的な生成元と対象分岐 ID を追加する。既存の `source_file` / `source_line` / `source_column` を使い、位置を後から推測しない。

### 2. コード生成で結果側に計測点を置く

生成 C に `cob_cov_hit(module_handle, branch_id)` 相当の呼び出しを分岐結果の入口へ挿入する。`IF` では真ブロックと偽ブロック（空の `ELSE` を含む）、`EVALUATE` では選択された選択肢、`PERFORM` では継続・終了の確定箇所に置く。条件式の中へ副作用のある計測式を混ぜず、各条件を元どおり1回だけ評価する。`EXIT PERFORM`、`GO TO`、例外脱出、再帰、`CALL` がある場合も、通過していない結果を記録しない。

`-fremove-unreachable` などの最適化が有効な場合は、到達不能判定を計測点とマップの双方へ反映する。`--cobol-branch-coverage` を指定しない生成 C には計測コードもマップ生成も加えない。`--coverage` と `-fprof` を同時に指定した場合の計測点の順序も固定する。

### 3. libcob で回数を保持して保存する

`libcob` に小さなカバレッジモジュールを追加し、生成コードからモジュール単位の結果一覧を登録する。各結果は飽和する64ビットカウンターで保持し、並列実行時に更新を失わない方法を使う。`CANCEL` / 動的ロードでモジュールが解放されても集計を失わないよう、ランタイムが記述子とカウンターを所有する。プロファイル用の時間計測とは独立させる。

実行データには形式バージョン、コンパイル時のマップ ID、プログラム／モジュール ID、結果 ID と回数を含める。マップ ID は前処理済みソース、コンパイル条件、結果一覧から決定的に算出し、異なるビルドの実行データを誤って混ぜない。保存は一時ファイルからの原子的なリネームで行い、同一ディレクトリへの並列書き込みとレポート中の部分ファイル読み取りを防ぐ。パスやソース本文以外の機密データは出力しない。

### 4. 集計ツールで COBOL の結果を表示する

標準ライブラリだけで動く `bin/cobcov` を追加する。マップと実行データの形式・IDを検査し、対応しない組み合わせは黙って加算せずエラーにする。カウンターは実行回数も表示するが、カバレッジ率の分子は `count > 0` の結果数とする。ファイル別・プログラム別・構文別の集計と、未実行結果の一覧を出す。生成 C の `gcov` レポートとは別名・別ディレクトリで管理する。

## 実装順序と受け入れ条件

1. **基礎と `IF`:** オプション、記述子、実行時カウンター、マップ、テキスト／JSONレポートを実装する。`ELSE` なし、同一行の複数 `IF`、ネスト、`COPY` 内の文、定数条件、再帰・並列実行で期待どおりの分母と回数を確認する。
2. **`EVALUATE` と反復:** 複数 `WHEN`、`WHEN OTHER` なし、`ALSO`、`THRU`、`PERFORM UNTIL` の前判定・後判定、多重 `VARYING`、`PERFORM TIMES` の0回を確認する。生成 C の分岐数に引きずられないことを検証する。
3. **探索・例外・転送:** `SEARCH` / `SEARCH ALL`、`GO TO DEPENDING ON`、ファイル・算術・`CALL` 等の条件付きハンドラ、Report Writer / JSON / XML の `WHEN` 条件を追加する。各構文の正常側と異常側、未指定側をテストする。
4. **運用と互換性:** 複数プロセスのマージ、動的ロード／`CANCEL`、LCOV出力、閾値、出力失敗時の診断を試す。Linux GCC、Windowsのサポート対象Cコンパイラ、`--coverage`・`-fprof`・EBCDICモードとの併用を確認する。

各段階で、計測あり・なしのプログラム出力、終了コード、データ更新を比較し、観測可能なCOBOLの動作が変わらないことを受け入れ条件とする。テストは `tests/testsuite.src/` のAutotestに置き、レポートの期待値をソース上の各分岐結果に対して固定する。

## 既知の境界と検討事項

- `--db2-sql` の変換後ソースで生成された文を、元のSQL文や後続のCOBOL文へ正確に対応付けるには、変換スクリプトからのソースマップが必要になる。最初の実装では生成文を「generated」と明示し、元のCOBOLに由来しない内部条件を分母へ混ぜない。CICS/BMSを含む元プログラム全体の計測は、それらの構文に対応してから評価する。
- コピー句や複数モジュールで同名ファイル・同じ行が現れるため、ファイル名と行番号だけをキーにしない。マップ ID とプログラム ID、列、連番を併用する。
- `PERFORM FOREVER` や無条件の転送には二択の結果がない。無理に偽側を作らない。最適化で到達不能とされた結果は率の分母から外す一方、利用者には存在と理由を示す。
- 強制終了、共有ライブラリのアンロード、カウンターあふれ、書き込み不可の出力先では欠測または診断があり得る。実行データの欠落を「未実行」と区別してレポートする。
