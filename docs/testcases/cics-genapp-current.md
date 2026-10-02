# CICS GenApp 全体動作のテストケースと実行記録

## 対象、原典、フィクスチャ

対象は `reference/cics-genapp/base/src` の COBOL 31 本と `ssmap.bms` の6マップである。IMS/DLI 依存の COBOL は監査上0本。端末経路は `cdef121.jcl` / `cdef125.jcl` の `SSC1`、`SSP1`～`SSP4`、`LGSE`、`LGCF`、`LGPF`、`LGST`、`SSST` に合わせる。業務手順と番号は `base/Testing.md` と `base/Reference.md` を原典とする。

Db2 Community Edition の隔離スキーマ `GENAPPTEST` に、`base/cntl/db2cre.jcl` の8表・40行を移植する。投入後の件数は CUSTOMER 10、CUSTOMER_SECURE 10、POLICY 10、ENDOWMENT 2、HOUSE 3、MOTOR 3、COMMERCIAL 2、CLAIM 0。`base/data/ksdscust.txt` と `ksdspoly.txt` の固定長レコードを、同じスキーマの `KSDSCUST` と `KSDSPOLY` に各10件投入する。画面で追加する顧客・保険は試験後に削除または元データへ復元する。JCL の `COMMERCIAL.Status` は SMALLINT だが初期行は `'ACTIVE'` を使うため、ローカル表だけ VARCHAR(16) とする。大きな `paddingData` は CLOB にする。

## ケース一覧

| ID | 入口・操作 | 期待する画面／資源 | 2026-10-02 結果 |
| --- | --- | --- | --- |
| G00 | 全31本を CICS→SQL 変換、オブジェクト化、リンク | 未解決構文・リンク先がない | 通過。5画面は専用入口、残りは共通ワーカーへリンク |
| G01 | `LGSE` 既定値・入力42・再入力77 | カウンター再初期化、TS キュー3項目 | 通過 |
| G02 | `LGST` に通常文・`Q=TEST` 指定文 | TD `CSMT` と指定 TS キューへ記録 | 通過 |
| G03 | 16並列の GET COUNTER | 重複しない番号 | 通過 |
| G04 | `LGCF` の既存・未登録顧客キー | `KSDSCUST` による照会、未検出応答 | 既存顧客を確認。未登録側は低レベル FILE 試験で確認 |
| G05 | `LGPF` の既存・未登録保険キー | `KSDSPOLY` による照会、未検出応答 | 通過 |
| G06 | FILE の READ/GTEQ/GENERIC/WRITE/REWRITE/DELETE | キー順、重複、未検出、更新、rollback | 通過 |
| G07 | `SSST` 実行、時計を60秒進める | TS 統計と次回 `START`、Rust ホストが期限実行 | 通過。1060秒のタスクを実行して1120秒へ再予約 |
| G08 | `49` レベルの長さ付き SQL VARCHAR を入力・出力 | 長さ5と `HELLO` が Db2 往復後も一致 | 通過 |
| C01 | `SSC1` 初期接続 | `SSMAPC1` と顧客入力欄 | 通過 |
| C02 | 顧客1、選択1 | `Andrew Pandy`、JCL の顧客項目 | 通過。画面内の氏名を照合 |
| C03 | 顧客未登録、選択1 | 「データなし」、Db2 不変 | 通過 |
| C04 | 顧客項目入力、選択2 | CUSTOMER、CUSTOMER_SECURE、KSDSCUST が同じ番号で増加 | 通過。顧客11を照合し後で削除 |
| C05 | KSDSCUST に顧客11のキーを先に置き選択2 | FILE 重複で ABEND、先行 CUSTOMER INSERT を rollback | 通過。顧客・保護情報0件、既存キー1件 |
| C06 | 顧客1を選択4、編集し確定 | CUSTOMER と KSDSCUST の対象列だけ更新 | 通過。Andrew→Andy を両資源で照合し復元 |
| C07 | 不正選択9、PF3 | 入力エラーと終了文面 | 通過 |
| P01 | `SSP1` 初期接続、保険1/顧客2で照会 | `SSMAPP1`、`FORD` 等の MOTOR 項目 | 通過 |
| P02 | `SSP1` 新規モーター保険を追加 | POLICY、MOTOR、KSDSPOLY に同一番号を作成 | 通過。採番1000001以降を照合し復元 |
| P03 | 直前の保険を選択3で削除 | POLICY、MOTOR、KSDSPOLY の追加行が消える | 通過 |
| P04 | `SSP1` 選択4で既存保険を編集 | POLICY、MOTOR、KSDS の対象項目を更新 | 通過。FORD→FORDX と KSDS 変更を照合し復元 |
| P05 | `SSP2` 初期接続、ENDOWMENT の既存番号/顧客 | `SSMAPP2` と LIFEASSURED | 通過。保険4/顧客8 |
| P06 | `SSP3` 初期接続、HOUSE の既存番号/顧客 | `SSMAPP3` と PROPERTYTYPE | 通過。保険6/顧客4 |
| P07 | `SSP4` 初期接続、COMMERCIAL の既存番号/顧客 | `SSMAPP4` と ADDRESS | 通過。保険9/顧客5 |
| P08 | `SSP2`～`SSP4` の追加・削除、`SSP2`/`SSP3` の更新 | 各詳細表、POLICY、KSDSPOLY を照合 | 通過。`SSP4` の更新分岐は原典に存在しない |
| P09 | 5画面で未登録番号／英字 `ABC` の番号を照会 | 「データなし」、Db2 と FILE は不変 | 通過。英字番号は元 BMS の `NUM` 指定がないため受理され、照会結果は「データなし」 |
| P10 | 5画面それぞれで不正選択9、PF3 | エラー文面と終了 | 通過 |
| B01 | `SSMAPP5` のマップ生成・項目往復 | 6マップ目を正しく描画・入力変換 | 通過。検証用 `SSP5` 入口で TN3270 表示、番号のゼロ詰めと入力エコーを確認 |
| E01 | IBM-037 モードで5画面、SQL、FILE を実行 | 空白、数値、文字変換が一致 | 通過。5画面の照会・未登録・不正選択、顧客と4種の保険の追加／削除、顧客・自動車・生命・住宅の更新、2端末競合、LGCF/LGPF の FILE を照合 |
| X01 | CICS `READ UPDATE` 中に別 Db2 接続で同じ KSDS 行を更新。また2端末で同じ保険を照会し、順に更新 | 行ロックと更新時刻で競合を検出。後続端末の変更を Db2・KSDS に残さない | 通過。行ロックは2プロセス／2接続、画面競合は2本の TN3270 接続で確認 |
| W01 | イベント入力から `LGASTAT1`、HTTP から業務 COBOL | COMMAREA／コンテナー／HTTP／統計への値の受け渡し | 通過。イベントの COMMAREA と2コンテナー、顧客と4種の保険の HTTP 照会・追加、未登録404、追加失敗502を確認。顧客 INSERT 後の KSDS 重複時に Db2 を rollback |

HTTP は元の顧客と4種の保険契約の照会・追加 COBOL を使う。元 `LGTESTP4` には更新選択肢4の業務分岐自体がない。`SSP5` は原典のトランザクションではなく、未使用の6枚目の BMS マップを検査する専用入口である。E01 の対象は TN3270 画面、Db2、FILE の組み合わせであり、IBM-037 版の HTTP API ワーカーは含まない。

## 入力欄と照合点

| 画面 | 送信する BMS 欄 | 照合する原典 |
| --- | --- | --- |
| `SSMAPC1` | `ENT1CNO`、氏名・生年月日・住所・電話・メール、`ENT1OPT` | `CUSTOMER`、`CUSTOMER_SECURE`、225バイト `KSDSCUST` |
| `SSMAPP1` | `ENP1PNO`、`ENP1CNO`、日付、車両・保険料・事故数、`ENP1OPT` | `POLICY`、`MOTOR`、64バイト `KSDSPOLY` |
| `SSMAPP2` | `ENP2PNO`、`ENP2CNO`、生命保険詳細、`ENP2OPT` | `POLICY`、`ENDOWMENT`、`KSDSPOLY` |
| `SSMAPP3` | `ENP3PNO`、`ENP3CNO`、住宅詳細、`ENP3OPT` | `POLICY`、`HOUSE`、`KSDSPOLY` |
| `SSMAPP4` | `ENP4PNO`、`ENP4CNO`、商業保険詳細、`ENP4OPT` | `POLICY`、`COMMERCIAL`、`KSDSPOLY` |

TN3270 試験は CP037 の Read Modified を送る。`JUSTIFY=(RIGHT,ZERO)` の10桁入力へ `1` を送り、照会後の値 `0000000001` を確認する。DB 更新系は試験専用スキーマの前後件数・キーを比較する。画面結果だけで DB 反映済みとは判定しない。

## 再実行手順

PowerShell で Compose の Db2 を起動し、`extras/db2sql/.runtime.env` と `gnucobol-db2-build` イメージを用意してから、リポジトリ直下で次を実行する。スクリプトは31本の通常／IBM-037 ビルド、Rust ホスト、JCL／KSDS の投入、FILE・画面・更新・rollback・定期起動を順に検証する。

```powershell
& extras/cobcics/test-genapp-all.ps1
```

2026-10-02 に同じスクリプトの IBM-037 検証も完走し、`GenApp IBM-037 screen, SQL and FILE suite passed` を確認した。E01 では原典の空白バイト `X'40'` を保持するビルド用コピーを使い、通常版と同じ Rust TN3270 ホスト、Db2 業務表、KSDS フィクスチャで照合した。

個別実行ファイルは `build/genapp/GENAPP`（LGSE/LGST/LGCF/LGPF/SSST）と `build/genapp/lgtestc1` / `lgtestp1`～`lgtestp4`。端末ホストは `build/genapp/cobcicsd`。原資材は変更せず、`build/genapp/normalized` に元ソースの `PROGRAM-ID` の句点、ABSTIME の8バイト領域、ASCII 実行の空白、初回 SEND→入力待ちの継続を補ったコピーを作る。

2026-10-02 に総合スクリプトを再実行し、`GenApp integrated suite passed` を確認した。31/31 変換・リンク、業務8表40行、KSDS各10件、5画面の照会・未登録照会・不正選択・PF3、未使用6枚目の画面表示と入力往復、顧客の追加／更新／rollback、4種の保険追加／削除、生命・住宅・自動車保険の更新、`LGASTAT1` のイベント COMMAREA とコンテナー経路、HTTP 顧客と4種の保険、期限到来の SSST、Db2 可変長ホスト変数の入出力、`READ UPDATE` の別接続に対する行ロックと2端末の古い更新の拒否を確認した。画面操作は実 TCP/TN3270、データ操作は Db2 表と KSDS の前後値で照合する。Rust の28テストと SQL/CICS 前処理の14テストも通過した。`LGUPDB01` の原典にある SELECT 5列と FETCH 6列の不一致、CLOSE 後のエラーコード上書き、および更新失敗後にも VSAM を書き換える動作はビルド用コピーで補正した。CLI のカーソルへ COBOL の宣言名を登録し、`WHERE CURRENT OF` による POLICY の更新時刻変更を有効にした。`LGAPOL01` の原典は追加時に全32,500バイトを下位へ渡すが、可変長データを含まない場合にも末尾を SQL VARCHAR として扱ってしまうため、ビルド用コピーで実データ長を LINK に指定する。生命保険の末尾データを渡す場合の上限は原典の `WS-VARY-CHAR` と同じ3,900バイトである。

イベント試験は端末用 `CCI3` に加えて、16バイトのチャネル名・コンテナー名と長さ付きデータを末尾に付ける `CCI4` 要求を使う。COMMAREA を空にして `DFHEP.DATA.00001`=`01ACUS`、`DFHEP.DATA.00002`=`00` を渡し、`LGASTAT1` が `GENASTRT` TS キューを作ることを確認した。HTTP 試験は `GENAPP-API` ワーカーへ32,500バイトの元 COMMAREA を渡し、元の `LGICUS01` / `LGIPOL01` / `LGACUS01` / `LGAPOL01` を LINK する。顧客と自動車・生命・住宅・商業保険の GET/POST が返す番号・項目を Db2 業務表と KSDS の両方で照合する。生命保険では可変長 `PADDINGDATA` の INSERT も確認した。顧客の Db2 INSERT 後に KSDS キーを重複させ、HTTP 502 と CUSTOMER / CUSTOMER_SECURE の不変を確認した。
