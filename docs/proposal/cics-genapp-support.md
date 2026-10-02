# CICS GenApp を GnuCOBOL + ローカル CICS 互換環境で動かす作業計画

## 目的と完了条件

`reference/cics-genapp/base/src` の IMS 関連を除く COBOL を、元ソースを変更せずに変換・コンパイル・リンクし、`wc3270` / `c3270` から GenApp の画面と業務処理を実行できるようにする。Db2 は既存の Docker Compose 環境を使う。`reference/cics-genapp` の JCL、データ、BMS、イベント定義、ワークロード用資材は、実行に必要な設定と検証データの原典とする。

完了判定は「コンパイルできる」だけでは行わない。対象の全プログラムについて起動経路と正常系・失敗系を記録し、画面、Db2、キー付きファイル、キュー、カウンターの状態を検査する。未対応構文を空の成功処理へ変換しない。

## 調査時点の資材と差分

| 対象 | 確認した内容 | 計画への影響 |
| --- | --- | --- |
| COBOL | `base/src` に `.cbl` が **31 本**、`.cpy` が **13 本**。現時点の `.cbl` / `.cpy` / `.jcl` に対する `EXEC DLI`、`CBLTDLI`、`DFSDLI`、`IMS`、`PCB` 検索は 0 件 | 現状の候補は **31 本すべて**。IMS 除外は実装前の依存関係監査で確定し、除外する場合はファイル名と根拠を残す |
| BMS | `base/src/ssmap.bms` の 1 マップセットに `SSMAPC1` と `SSMAPP1`～`SSMAPP5` の **6 マップ** | `COPY SSMAP` を 6 マップ分の symbolic map として生成する。`SSMAPP5` も生成・描画テストの対象 |
| 端末トランザクション | `cdef121.jcl` に `LGSE`、`LGCF`、`LGPF`、`SSC1`、`SSP1`～`SSP4`。`cdef125.jcl` に `LGST`、`SSST` | 画面だけでなく初期化、ファイル照会、イベント統計、定期統計も起動経路を用意する |
| Db2 | `db2cre.jcl` に顧客、顧客保護情報、保険契約、保険種別ごとの明細、請求の表と初期データ。COBOL に `INSENSITIVE SCROLL` カーソル、`WITH HOLD`、`IDENTITY_VAL_LOCAL()` がある | z/OS 固有 DDL を Db2 LUW 用に移植し、SQL 前処理と実行時カーソルの互換性を確認する |
| キー付きファイル | `adef121.jcl` は `KSDSCUST` を 225 バイト・キー 10 バイト、`KSDSPOLY` を 64 バイト・キー 21 バイトで定義し、`base/data` からロードする | `READ`、`WRITE`、`REWRITE`、`DELETE` に加え、`GENERIC GTEQ` のキー検索と応答コードを実装する |
| 共通サービス | `LGSETUP` は名前付きカウンターと TS キューを初期化。`LGASTAT1` はチャネルのコンテナーを読み、`LGWEBST5` はカウンター照会と `START ... AFTER MINUTES(1)` を使う | カウンター、コンテナー、TS/TD キュー、タイマー起動を実際に動くサービスとして用意する |

この調査はソース検索と既存ツールの試行に基づく初期結果であり、全構文のコンパイル結果ではない。具体的には、現行 `cobcics-mapgen` に `ssmap.bms` を渡すと **29 行目の `JUSTIFY=(RIGHT,ZERO)`** で停止する。`XINIT` も同じ BMS に含まれる。現在の `cobcicsd --map-route` は BMS ファイルにマップが 1 個だけあることを要求するため、6 マップの動的選択も未対応である。既存の `WRITE FILE('ABNDFILE')` は診断ログへの追記であり、GenApp の VSAM に必要なキー付きファイル操作には使えない。また `cdef121.jcl` の `KSDSCUST Recordsize(82)` と `adef121.jcl` の 225 バイト定義が食い違うため、実データ、COBOL の `LENGTH`、キー位置を照合して採用するレコード形式を決める。

## 実装方針

### 1. 対象と未対応構文の監査

`base/src` の全 31 本を対象とする監査スクリプトを追加する。固定形式のコメント・継続行を解釈した上で、`EXEC CICS`、`EXEC SQL`、`COPY`、`CALL`、リンク先、BMS 名、参照する資源名を抽出し、ソース位置つきの対応表を生成する。コンパイル失敗は COBOL、CICS 変換、SQL 変換、BMS、リンクの段階ごとに記録する。IMS 依存の判定は字句検索だけで終えず、呼び出し先と外部資源の依存も確認する。該当がなければ IMS 除外 0 本として全 31 本を完了条件にする。

JCL は z/OS ジョブとしてコンパイルする対象ではない。CICS 資源定義と Db2/VSAM 初期化処理をローカルの設定・スクリプトへ移す。`wsim` の入力と REXX、Web サービス、イベント定義は用途を監査し、対応するローカル起動・入力・検証経路を明示する。機能が任意構成であることだけを理由に COBOL を対象外にしない。

### 2. BMS と端末セッション

`JUSTIFY`、`XINIT`、`EXTATT=MAPONLY`、`CTRL=(FREEKB)`、継続行、属性の解釈を `cobcics-mapgen` に追加し、1 つの `SSMAP.cpy` に 6 マップの入力・出力領域を生成する。元 BMS のフィールド位置・長さと COBOL が参照する項目名を照合する。`JUSTIFY=(RIGHT,ZERO)` は入力時の右寄せ・ゼロ詰めまで検証し、単にパーサーで受理するだけにはしない。

端末ホストは `SEND MAP` / `RECEIVE MAP` のマップ名を worker から受け、同一マップセット内の該当物理マップを選ぶ。現在の TRANSID と BMS ファイルの静的な 1 対 1 対応に依存しない。`SSC1` と `SSP1`～`SSP4` で入力欄、AID、`ASIS`、エラー表示、PF3 を検査し、未使用の `SSMAPP5` はレンダリングと項目往復を検査する。

### 3. COBOL/CICS/SQL の変換・リンク

Bank of Z の `audit-bank-syntax.py` とビルド手順を一般化し、GenApp 用の再現可能な一括ビルドを用意する。前処理順序は CICS → SQL → GnuCOBOL とし、変換後ソースと個別ログを `build` 配下に保存する。`EXEC SQL INCLUDE` のコピー句、SQLCA、カーソル宣言・操作、ホスト変数と標識変数、`SET :host = IDENTITY_VAL_LOCAL()` を確認する。Db2 LUW と文法差がある場合は GenApp 専用の SQL 変換で吸収し、元 COBOL を書き換えない。

CICS では既存の Bank 対応を再利用し、GenApp が使うオプションを `RESP` / `RESP2` と EIB の値まで含めて追加する。特に `RECEIVE INTO`、`SEND TEXT`、`ASSIGN SYSID/STARTCODE/INVOKINGPROG/APPLID`、`READ/WRITE/REWRITE/DELETE FILE`、`READQ/WRITEQ/DELETEQ TS`、`WRITEQ TD`、`DEFINE/DELETE/GET/QUERY COUNTER`、`GET CONTAINER`、`START AFTER`、`ENQ/DEQ` を対象にする。`HANDLE AID` と `HANDLE CONDITION` の分岐も画面で検証する。リンク先は全 31 本を同じビルド成果物から解決し、実行時に未登録のプログラム名を明確に報告する。

### 4. Db2、キー付きファイル、作業単位

`db2cre.jcl` の業務表、キー、初期データを GenApp 専用スキーマへ移植する。`STOGROUP`、`TABLESPACE`、`CCSID EBCDIC` 等の z/OS 固有の配置指定は Db2 LUW の定義へ変換し、日付・時刻、ID 生成、NULL、SQLCODE の結果を元プログラムに合わせる。Compose 上で初期化を再実行できるようにし、Bank のテスト用スキーマとは分離する。

`KSDSCUST` / `KSDSPOLY` は、ローカルでは **同じ Db2 接続上のキー付きレコード表**として実装する。COBOL から見えるのは CICS FILE 操作であり、キーのバイト列順、固定長レコード、重複キー、未検出、`GENERIC GTEQ`、更新ロック、`RESP` / `RESP2` を再現する。これにより業務表とファイル表を単一の Db2 トランザクションで commit / rollback できる。外部 VSAM と Db2 の真の分散 2 フェーズコミットを実装したと主張せず、GenApp が期待する原子性をローカル実行環境で検証する。`adef121.jcl` と `base/data` に基づくロード時には、225/64 バイトと 10/21 バイトのキー、件数、先頭・末尾レコードを検査する。

`SYNCPOINT`、`SYNCPOINT ROLLBACK`、正常 `RETURN`、`ABEND`、端末切断で業務表とファイル表の作業単位を揃える。ファイル書き込みを意図的に失敗させ、顧客・保険契約の Db2 側変更も取り消されることを確認する。`READ UPDATE` から `REWRITE` までの排他は複数端末で検証する。

### 5. キュー、カウンター、イベント、定期起動

名前付きカウンターは定義・削除・取得・照会・範囲・増分を永続化し、同時起動でも重複番号を発行しない。TS キューは項目番号、上書き、削除、長さ、`QIDERR` などを扱い、`ENQ/DEQ` と組み合わせた競合を検証する。TD キューは `LGSTSQ` のメッセージをローカルの永続ログへ書き、失敗時には正常を返さない。

`LGST` は `cdef125.jcl` のイベント定義に対応するローカル入力経路を用意し、`LGASTAT1` にコンテナー値または COMMAREA を渡す。`SSST` / `LGWEBST5` は `START ... AFTER MINUTES(1)` の予約実行を実装し、カウンターを TS キューの統計値へ反映する。テストでは時計を制御できるようにし、1 分の実待機を不要にする。Web サービス用 JCL が参照する業務プログラムには、元の入出力契約を使うローカル HTTP アダプターを用意し、少なくとも顧客・保険契約の照会/追加と VSAM 参照を試す。CICSPlex やカップリング・ファシリティの運用機能は、COBOL の実行に必要な意味を上記ローカルサービスで提供する。

## 作業順序と成果物

| 段階 | 作業 | 終了条件 |
| --- | --- | --- |
| 0. 棚卸し | 全ソース・資源・リンク関係の監査、IMS 依存判定、現行ビルドの失敗一覧 | 31 本それぞれの対象判定と未対応項目がソース位置つきで再現可能 |
| 1. 画面・ビルド | 複数マップ BMS、GenApp 用変換・一括ビルド、端末経路 | 対象 COBOL 全本がオブジェクト化・リンクでき、`SSC1` と `SSP1`～`SSP4` の初期画面が表示される |
| 2. データ処理 | Db2 DDL/初期データ、SQL カーソル、キー付きファイル、作業単位 | 上流 `Testing.md` の `LGSE`、顧客 1～10 の照会・追加、保険番号 `00000000001`/顧客番号 `0000000002` の照会とロールバックが通る |
| 3. 全経路 | 保険種別ごとの追加・変更・削除、`LGCF`/`LGPF`、キュー・カウンター・イベント・統計・Web 入力 | 31 本に到達する起動/リンク経路があり、各経路の実行結果と副作用を検査済み |
| 4. 回帰・文書 | `wc3270` / `c3270` の対話、自動 TN3270 シナリオ、並行端末、再初期化、EBCDIC 互換モード | CI/ローカルで再実行可能なログ、テストケース、起動手順を `docs/testcases` / `docs/userguide` に記録 |

テストケースは画面ごとに照会・追加・変更・削除・不正入力・未検出・PF3 を整理する。Db2 表とファイル表の前後差、SQLCA、CICS 応答、発行番号、キュー項目も照合する。`base/wsim` にある操作データは端末シナリオの入力候補として照合し、元の `Testing.md` と `Reference.md` のサンプル番号を期待値に採用する。失敗が残った場合は対象プログラム・操作・原因を一覧に残し、全件完了と扱わない。

## 実装時に先に確定する点

- `KSDSCUST` の 82 バイトと 225 バイトの定義差、および `WRITE FILE` で渡す `LENGTH` と実レコードの対応。実際の 225 バイトデータを読み、必要なら読み書き対象の部分長を明文化する。
- BMS の `JUSTIFY` / `XINIT` が GenApp の入力値に与える結果。生成 copybook のフィールドオフセットを COBOL の参照項目と比較する。
- `INSENSITIVE SCROLL`、`WITH HOLD`、`IDENTITY_VAL_LOCAL()` の Db2 LUW 上の動作と既存 SQL 前処理の対応。実クエリーで SQLCODE とカーソル位置を比較する。
- カウンターの採番と `ABEND` 時の戻し方。業務データの rollback と番号の再利用可否を元プログラム・テスト期待値から定める。

関連する原典: [`base/Testing.md`](../../reference/cics-genapp/base/Testing.md)、[`base/Reference.md`](../../reference/cics-genapp/base/Reference.md)、[`ssmap.bms`](../../reference/cics-genapp/base/src/ssmap.bms)、[`adef121.jcl`](../../reference/cics-genapp/base/cntl/adef121.jcl)、[`db2cre.jcl`](../../reference/cics-genapp/base/cntl/db2cre.jcl)。
