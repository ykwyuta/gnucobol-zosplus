# Bank of Z CICS 画面項目の試験シナリオ

## 対象と資材

対象は `reference/Bank-of-Z-main/src/base/cics` の COBOL 32 本、BMS 10 本と、
Rust TN3270 ホストを介した 24×80 画面である。IMS の PCB/DLI と IMS 側の履歴
データベースは対象から除く。まず
`python extras/cobcics/audit-bank-syntax.py` が 32 本をオブジェクトまで
コンパイルできることを入口条件とする。

Db2 for z/OS の元定義は
`reference/Bank-of-Z-main/.setup/jcl/cics/Db2-create.j2` の
`BANKZ.ACCOUNT`、`BANKZ.CUSTOMER`、`BANKZ.PROCTRAN`、`BANKZ.CONTROL`。
元の初期データ投入は `BANKDATA.cbl` を
`.setup/jcl/cics/Db2-insert.j2` から
`PARM('1,10000,1,1000000000000000')` で実行する。画面の再現試験には
同じ表定義を Db2 Community Edition の試験専用 `BANKZTEST` スキーマに作成し、固定した顧客・口座・残高を
少数だけ投入する。元の `src/base/ims/LoadData/*.data` は IMS 用の異なる形式で、
CICS の `BANKZ` 表への直接投入には使わない。

テスト顧客は `SORTCODE.cpy` の既定値に合わせたソートコード `987654`、顧客番号 `0000000001`、
口座 `00000001` と `00000002` を使用する。口座残高はそれぞれ
`1000.00`、`500.00`、当座貸越枠は `100.00` とする。
試験前後には両口座の `ACCOUNT_AVAILABLE_BALANCE`、
`ACCOUNT_ACTUAL_BALANCE` と `PROCTRAN` 件数を照合する。
書き込みを伴う試験は独立したデータへ復元してから実行する。

## 画面操作

| ID | 経路 | 入力・操作 | 期待結果 |
| --- | --- | --- | --- |
| M01 | OMEN | 初期接続 | Main Menu と Action 欄、1〜7/A の選択肢が表示される。 |
| M02 | OMEN | Action 空欄で Enter | 入力促進またはエラーを表示し、メニューに留まる。 |
| M03 | OMEN | Action `X` で Enter | `You must enter a valid value (1-7 or A).` を表示する。 |
| M04 | OMEN | PF3/PF12 | 終了文面を表示してセッションを閉じる。 |
| D01 | OMEN→ODCS (1) | 顧客番号 `0000000001` を入力 | 氏名・住所・生年月日・状態・信用スコアが CUSTOMER 表と一致する。 |
| D02 | ODCS | 存在しない顧客番号 | 該当なしのメッセージ。Db2 表は変化しない。 |
| D03 | ODCS | 新規顧客を検索し PF5 | 顧客行を削除し、画面に成功文面を表示する。 |
| D04 | ODCS | PF10 で編集し Enter／取消 | 更新は対象列だけ変化し、取消は変化しない。 |
| A01 | OMEN→ODAC (2) | 口座番号 `00000001` | 種類、顧客番号、残高、日付、貸越枠が ACCOUNT 表と一致する。 |
| A02 | ODAC | 存在しない口座番号 | 該当なし。残高は変化しない。 |
| A03 | ODAC | 新規口座を検索し PF5 | 対象口座が消え、画面に成功文面を表示する。 |
| A04 | ODAC | 口座を表示して PF5 前に PF3 で取消 | 口座行と残高が変化しない。原 COBOL の PF5 は確認なしで削除する。 |
| C01 | OMEN→OCCS (3) | 必須顧客欄を空欄で Enter | 入力検証メッセージが対象欄に表示され、INSERT されない。 |
| C02 | OCCS | 氏名・住所・生年月日等を入力して確定 | CUSTOMER と CONTROL の番号が更新され、成功メッセージが表示される。 |
| C03 | OCCS | 信用スコア子タスク | RUN/FETCH とチャネル返却後の値が画面・CUSTOMER に反映される。 |
| N01 | OMEN→OCAC (4) | 顧客番号などを入力して確定 | ACCOUNT に新口座を INSERT し、画面に新番号を表示する。 |
| U01 | OMEN→OUAC (5) | 口座番号を入力 | 現在の口座項目が表示される。 |
| U02 | OUAC | 貸越枠など更新可能な欄を変更 | ACCOUNT の該当列だけが変化する。 |
| B01 | OMEN→OCRA (6) | 口座、符号、金額を入力 | ACCOUNT 残高と PROCTRAN 履歴が一度だけ更新される。 |
| B02 | OCRA | 不正金額 | エラー表示。残高と履歴は変化しない。 |
| B03 | OCRA | 端末から残高を超える出金 | 元の `DBCRFUN` は端末 (`FACILTYPE=213`) の残高不足判定を適用しないため、負残高への更新を確認する。 |
| T01 | OMEN→OTFN (7) | 振込元・先口座と金額を入力 | 元口座から減額、先口座へ加算し、両方の履歴が記録される。 |
| T02 | OTFN | 同一口座・不正金額 | 検証メッセージ。両口座残高は変化しない。 |
| T03 | OTFN | 残高を超える送金 | 元の `XFRFUN` に残高判定がなく、負残高で両口座の更新・履歴記録が行われることを確認する。 |
| L01 | OMEN→OCCA (A) | 顧客番号 `0000000001` | `ACCOUNT` 繰り返し 10 行に口座一覧を表示し、11 行目以降を越境しない。 |
| L02 | OCCA | 該当口座なし・10 件超 | 空欄と上限処理を確認する。 |
| R01 | 各遷移先 | PF3 | メニューに戻り、Action 欄を再度入力できる。 |
| R02 | 各入力画面 | Clear/PF12 | 画面消去または終了が COBOL の分岐どおりになる。 |

## 入力欄と端末操作の確認点

| 画面 | BMS の入力欄 | 自動試験で送る操作 |
| --- | --- | --- |
| OMEN | `ACTION` | 1〜7/A、空欄、`X`、PF3。 |
| ODCS | `CUSTNO`、PF10 後は氏名・住所 | 顧客 1・未登録 2 の検索、顧客 2 の PF5 削除、PF10 編集・PF3 取消。 |
| ODAC | `ACCNO` | 口座 1・未登録 3 の検索、口座 3 の PF5 削除、PF5 前の PF3 取消。 |
| OCCS | `CUSTTIT`、`CHRISTN`、`CUSTINS`、`CUSTSN`、`CUSTAD1/2`、`CITY`、`POSTCODE`、`COUNTRY`、`DOBDD/MM/YY` | 必須のタイトル空欄と、全入力欄を使った顧客作成。 |
| OCAC | `CUSTNO`、`ACCTYP`、`INTRT`、`OVERDR` | 不正な口座種別、4 欄を同時入力した口座作成。 |
| OUAC | `ACCNO`、`ACTYPE`、`INTRT`、`OVERDR` | 検索後、PF5 で貸越枠を更新。保護 FSET の明細日欄も Read Modified に含める。 |
| OCRA | `ACCNO`、`SIGN`、`AMT` | 正負符号、金額の非数値、入金・出金・残高超過出金。 |
| OTFN | `FACCNO`、`TACCNO`、`AMT` | 同一口座、金額の非数値、通常送金・残高超過送金。 |
| OCCA | `CUSTNO` | 顧客 1 の 2 口座、口座なしの顧客番号、11 口座を用意した場合の画面上限 10 行。 |

各入力欄は BMS の `UNPROT` 属性と COBOL の編集分岐を基準に確認する。
初期フォーカス、保護欄への入力拒否、長さ上限、数値欄の誤入力、
メッセージ欄、複数欄を同時に変更した Read Modified も確認する。
画面操作にはスクリプト化した TN3270 を使用し、必要に応じて
`C:\Program Files\wc3270\wc3270.exe` の `-scriptport` と `Ascii` で表示を照合する。

## 実行記録

### 2026-10-02 実行結果

`python extras/cobcics/audit-bank-syntax.py` で IMS 以外の CICS COBOL 32 本をオブジェクトまでコンパイルした。
`extras/cobcics/test-bank-screens.py` を Rust TN3270 ホストと Docker Compose の Db2 に接続して実行した。
画面テストは CP037 の 3270 Read Modified を送信し、表示文面と Db2 行を照合する。

| ケース | 実行結果・Db2 照合 |
| --- | --- |
| M01、R01、R02 | 1〜7/A の 8 経路すべてで表示、PF3 メニュー復帰、PF12 終了、Clear による消去を確認。 |
| M02〜M04 | 空欄・不正 Action の検証、メニュー PF3 終了を確認。 |
| D01〜D04 | 固定顧客の表示、未登録番号の案内、新規顧客 `0000000002` の PF5 削除、PF10 後の編集と PF3 取消を確認。編集では CUSTOMER の名だけ `Test→Edited`、姓と住所は変わらない。取消時は Db2 行が不変。 |
| A01〜A04 | 固定口座の表示、未登録番号の案内、新規口座 `00000003` の PF5 削除、口座表示後 PF5 前の PF3 取消を確認。取消時は Db2 行と残高が不変。 |
| C01、C02、C03 | 必須欄の検証、新規顧客の作成、信用スコア子タスクの返値を画面・CUSTOMER と照合。最終実行のスコアは 608、審査日は 20261004（乱数と実行日により変わる）。 |
| N01 | 不正な口座種類の検証と `00000003` の作成・採番 `2→3` を確認。 |
| U01、U02 | 口座表示、PF5 で当座貸越枠 `100→200` を確認。 |
| B01、B02 | 入金 `1000→1010`、出金 `1000→990`、不正金額の拒否、履歴各 1 件を確認。 |
| B03 | 端末出金 `2000` では残高 `1000→-1000`、履歴 1 件を確認。 |
| T01、T02 | `10` の送金で `1000→990`、`500→510`、履歴 2 件。同一口座と不正金額の拒否を確認。 |
| T03 | `2000` の送金では残高 `1000→-1000`、`500→2500`、履歴 2 件を確認。 |
| L01、L02 | 2 口座の一覧と、該当口座なしの案内を確認。11 口座では `11 accounts found` を表示し、画面は先頭 10 件のみで `00000011` を表示しない。 |
| BMS 境界 | 保護欄への Read Modified 入力と Action 1 文字欄への 2 文字送信をホストが拒否。`wc3270` では保護欄が変更されず、Action は 1 文字に収まることを確認。 |

作成・更新・削除・送金の後は `BANKZTEST` の固定行、採番カウンター、試験履歴を元へ戻し、
再照合した。`B03` と `T03` は元 COBOL の負残高を許す分岐を別ケースで照合した。
最終照合は顧客の追加行 0・採番 1、口座の追加行 0・採番 2、
固定残高 `1000.00`／`500.00`、試験履歴 0 件だった。

この実行で必要になった元資材の補正は `INQACC.cbl` の未登録時成功フラグを `N` にする変更と、
`CRECUST.cbl` が信用照会子タスクに渡すコンテナーを `CUSTOMER` 本体の長さにする変更である。
Rust 側では保護 FSET の Read Modified 値を BMS へ渡し、COBOL の `DISPLAY` とワーカーの
バイナリ応答を別の出力にした。`CEEDAYS`／`CEELOCT` の互換処理と、暗黙の
`DFHCOMMAREA` を持つリンクプログラムの変換も加えた。

追加ケースは `extras/cobcics/test-bank-boundaries.py` で Db2 と TN3270 の応答を照合した。
`C:\Program Files\wc3270\wc3270.exe` を使う
`extras/cobcics/test-wc3270-boundaries.py` でも PF10 の入力、PF3 取消、
保護欄、Action の最大長を照合した。試験用に追加した 9 口座と変更した顧客名は
終了時に復元し、固定行と採番カウンターを維持した。

### 再実行手順 (PowerShell)

`extras/db2sql/README.md` の手順で `.env` と `.runtime.env` を用意し、Db2 と
`gnucobol-db2-build` イメージを起動・作成する。リポジトリ直下で以下を実行する。

```powershell
$ws = (Resolve-Path .).Path
$driver = '/usr/local/lib64/python3.9/site-packages'
docker run --rm --env-file extras/db2sql/.runtime.env -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build extras/cobcics/setup-bank-test-db.py
& extras/cobcics/build-bank-screens.ps1
docker run --rm --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=BANKZTEST -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build extras/cobcics/test-bank-screens.py
docker run --rm --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=BANKZTEST -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" -v "${ws}:/src" -w /src --entrypoint python3 gnucobol-db2-build extras/cobcics/test-bank-boundaries.py
```

`wc3270` の照合は Docker の公開ポートをローカルホストに限定して実行する。

```powershell
docker run -d --rm --name bankz-wc-boundary -p 127.0.0.1:33327:33327 --env-file extras/db2sql/.runtime.env -e DB2_SCHEMA=BANKZTEST -e "LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:$driver/clidriver/lib:$driver/ibm_db.libs" -v "${ws}:/src" -w /src --entrypoint /src/build/bank-business/cobcicsd gnucobol-db2-build --bms /src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms --transid OMEN --applid BANKAPP --route OMEN=/src/build/bank-syntax/screens/BNKMENU --route ODCS=/src/build/bank-syntax/screens/BNKLINK --map-route ODCS=/src/reference/Bank-of-Z-main/src/base/cics/bms/BNK1DCM.bms --listen 0.0.0.0:33327
python extras/cobcics/test-wc3270-boundaries.py
docker stop bankz-wc-boundary
```
