# COBOL 分岐カバレッジの取得

`cobc --cobol-branch-coverage` でコンパイルすると、COBOL ソース上の分岐結果を計測できる。生成された C の分岐を測る `--coverage` とは別の機能で、GCC の `gcov` は不要。各結果が少なくとも一度実行されたかを `bin/cobcov` で集計する。以下では、このリポジトリからビルドした `cobc` と、それに対応する `libcob`、Python 3 を使用する。

## 最小例

リポジトリのルートから作業ディレクトリを作り、その中に次の固定形式ソースを `sample.cbl` として保存する。以後のコマンドは `build/cobol-coverage-demo` で実行する。

```cobol
       IDENTIFICATION DIVISION.
       PROGRAM-ID. SAMPLE.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 X PIC 9 VALUE 0.
       PROCEDURE DIVISION.
           IF X = 0 DISPLAY 'TRUE' END-IF.
           IF X = 1
               DISPLAY 'BAD'
           ELSE
               DISPLAY 'FALSE'
           END-IF.
           STOP RUN.
```

WSL/Linux では次のようにコンパイル・実行・集計する。`cobc` はこのリポジトリのビルドまたはインストール済みのものを `PATH` から使う。

```sh
mkdir -p build/cobol-coverage-demo
cd build/cobol-coverage-demo
# ここで上記ソースを sample.cbl として保存する
cobc -x --cobol-branch-coverage -o sample sample.cbl
COB_COVERAGE_DIR=coverage-data ./sample
python3 ../../bin/cobcov report . coverage-data
```

Windows PowerShell では、同じ作業ディレクトリに `sample.cbl` を保存してから次を実行する。

```powershell
cobc -x --cobol-branch-coverage -o sample sample.cbl
$env:COB_COVERAGE_DIR = 'coverage-data'
./sample.exe
python ../../bin/cobcov report . coverage-data
```

この例では `TRUE` と `FALSE` が表示され、集計の先頭行は `COBOL IF branches: 2/4 (50.0%), 1 counter file(s)` となる。`IF` が2個あり、それぞれ真・偽の2結果を持つため、分母は4である。後続の行には `sample.cbl:行:列`、プログラム名、分岐種別、結果（`true` / `false`）、通過回数が表示される。`0` は、その結果をまだ通っていないことを示す。

## 計測ファイルと複数回の実行

コンパイルすると、**コンパイル時の作業ディレクトリ**に `cobcov-<build-id>.json` という分岐マップができる。実行時には `COB_COVERAGE_DIR` に指定したディレクトリへ、プロセスごとの `*.counts` ファイルができる。未指定時の出力先は実行時の作業ディレクトリにある `cobol-coverage` である。指定先は事前に作成するか、親ディレクトリが存在するパスにする。

`cobcov report <マップファイルまたはディレクトリ> <カウンターファイルまたはディレクトリ>` は、指定した全カウンターファイルの回数を合算する。同じビルドのプログラムを異なる入力で複数回実行してから集計すれば、それらのテストを合わせた分岐カバレッジになる。新しい計測を始める際は、作業ディレクトリとカウンターディレクトリを分けると、古い結果の混入を防げる。マップのディレクトリを指定すると、その中の全 `cobcov-*.json` が分母に入る。単一のビルドを調べる場合は、対象のマップファイルを直接指定できる。対応するマップがないカウンターはエラーになる。

## 出力形式と CI の閾値

```sh
python3 ../../bin/cobcov report . coverage-data --format json > coverage.json
python3 ../../bin/cobcov report . coverage-data --format lcov > coverage.info
python3 ../../bin/cobcov report . coverage-data --fail-under-branches 80
```

既定はテキスト形式。`json` は各結果の位置・回数を含み、`lcov` は COBOL ソースを `SF`、分岐結果を `BRDA` として出力する。`--fail-under-branches 80` はカバレッジ率が80%未満のとき終了コード1を返すため、CI の判定に使える。率は「通過回数が1以上の結果数 ÷ マップ中の結果総数」であり、実行回数そのものの割合ではない。結果が0件のときも閾値判定は失敗する。

## 計測する構文と範囲

現在の対象は `IF` の真・偽、`EVALUATE` の各 `WHEN` と不一致、`PERFORM UNTIL` の継続・終了、`PERFORM TIMES` の0回・1回以上、`SEARCH` / `SEARCH ALL` の選択・未検出、`GO TO ... DEPENDING ON` の転送先・範囲外通過である。また、`ON SIZE ERROR`、`ON OVERFLOW`、動的 `CALL ... ON EXCEPTION`、ファイル操作の `AT END` / `INVALID KEY` / `AT EOP`、`DISPLAY` / `ACCEPT` / XML / JSON の `ON EXCEPTION` で通常側と例外側を計測する。実際のレポートでは、マップに含まれる構文だけが分母になる。

この値は **実装済みの COBOL 分岐結果**のカバレッジであり、COBOL 全構文の網羅率、文カバレッジ、条件式内の `AND` / `OR` ごとの条件カバレッジではない。`COPY` 内の計測対象は元のコピー元ファイルの位置で表示される。プロセスの強制終了や出力先への書き込み失敗では、実行データが残らない場合がある。
