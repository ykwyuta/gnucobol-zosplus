# Rust 製 TN3270 ホストによる CICS 互換機能の提案

## 方針

GnuCOBOL でコンパイルした `EXEC CICS` を、Rust 製の軽量なトランザクション実行環境へ接続する。Rust は wc3270（Windows）または c3270（Linux 等）からの TN3270 接続を受け、BMS マップを 3270 データストリームに変換し、AID キーと入力フィールドを COBOL に返す。COBOL 側には CICS 文の変換器、BMS 由来の symbolic map、EIB と互換コピー句を用意する。既存の Db2 アダプターと同じタスク内で動かし、`SYNCPOINT` と `ABEND` でトランザクションを制御する。

wc3270 と c3270 は TN3270/TN3270E **クライアント**であり、ホストに接続して画面を表示する。したがって、Rust 側は端末のスクリプト API へ画面を書き込む構成ではなく、端末が接続する TN3270 **サーバー**を実装する。両端末のスクリプト API は画面確認とキー入力の自動テストに使う。c3270 と wc3270 の公式マニュアルは、ホストへの Telnet 接続と RFC 1576 / RFC 2355 の実装、および `-scriptport` を説明している（[c3270](https://x3270.bgp.nu/Unix/c3270-man.html)、[wc3270](https://x3270.bgp.nu/Windows/wc3270-man.html)、[スクリプト API](https://x3270.bgp.nu/Unix/x3270-script.html)）。

この機能は、Bank-of-Z を動かすための CICS コマンド、BMS 画面、端末セッション、プログラム制御、Db2 の作業単位から実装する。CICS TS のリージョン管理、セキュリティ、分散トランザクション、全コマンドを初期段階で再現するものとは扱わず、未対応の文とオプションはコンパイル時に名称とソース位置を示して拒否する。

## リポジトリで確認した対象

- `reference/Bank-of-Z-main/src/base/cics/cobol/` に 32 個の `.cbl`、`bms/` に 10 個の `.bms` がある。`BNKMENU.cbl` は `COPY BNK1MAI` と `COPY DFHAID`、`EIBAID` / `EIBCALEN`、`DFHRESP(MAPFAIL)`、`SEND MAP` / `RECEIVE MAP`、`RETURN TRANSID ... COMMAREA` を使う。`BNK1MAI.bms` は 24×80 画面と `ACTION` 入力欄を定義する。
- 他のプログラムには `LINK PROGRAM ... COMMAREA`、`SYNCPOINT` / `SYNCPOINT ROLLBACK`、`ASSIGN`、`ASKTIME`、`FORMATTIME`、`HANDLE`、`ABEND` が多い。さらに `ENQ` / `DEQ` や一部のデータ／資源コマンドがある。全プログラムの採用条件は、各 `EXEC CICS` ブロックとオプションの棚卸し表を作ってから決める。
- [Db2 の提案](db2-embedded-sql.md)にある `--db2-sql` と EBCDIC 対応は既に実装途中である。CICS 変換後のソースを SQL 変換へ渡す統合経路を設け、EBCDIC のバイト長と Db2 接続の所有権をタスク単位で揃える。

## 実行構成

```text
wc3270 / c3270 / s3270
          │ TN3270 (後段で TN3270E)
          ▼
Rust cobcicsd ── セッション・画面・TRANSID 対応表・COMMAREA
          │ ローカル IPC（長さ付きバイナリメッセージ、セッショントークン）
          ▼
タスクごとの worker ── GnuCOBOL モジュール
          │                 │ EXEC CICS 変換で生成した呼び出し
          │                 ▼
          ├────────── Rust C ABI ブリッジ ── cobcicsd
          └────────── 既存 Db2 SQL アダプター ── Db2 Community Edition
```

Rust の `cobcicsd` が端末との TCP 接続を保持する。COBOL タスクはまず別プロセスの worker として直列に実行する。これにより GnuCOBOL ランタイムのスレッド共有状態を避け、画面待ちのあいだは疑似会話型トランザクションを終了できる。タスク間では Rust が COMMAREA、次の TRANSID、セッションの画面状態だけを保持する。`LINK` の呼び出し先は同一 worker の同一 Db2 作業単位で実行する。worker の起動・COBOL モジュール呼び出しには、`libcob` の公開 API を扱う小さな C シムまたは生成された固定 ABI のエントリーラッパーを使い、Rust から GnuCOBOL 内部構造体を直接参照しない。この境界は最初の技術検証で固定する。

COBOL からの呼び出しは、固定長のコマンド ID、引数の長さ、データ領域、`RESP` / `RESP2` 受け取り先を持つ C ABI にする。COBOL の `BY REFERENCE` のポインターは worker 内のブリッジだけが解釈する。IPC にはポインターを流さず、バージョン、セッション ID、タスク ID、コマンド ID、長さ、バイト列、結果コードを送る。接続先は既定でループバックに限定し、worker とサーバー間には起動時のランダムなトークンを使う。

## 端末接続と 3270 データ

1. 最初は RFC 1576 の基本 TN3270 に対応する。Telnet の BINARY、TERMINAL-TYPE、EOR の交渉、IAC エスケープ、EOR ごとのレコード化を実装する。端末は `N:127.0.0.1:<port>` で TN3270E を無効化し、`-model 3279-2`（24×80）で接続する。既定の TN3270E 接続は、RFC 2355 の装置種別、機能交渉と 5 バイトヘッダーを実装してから有効にする（[RFC 1576](https://www.rfc-editor.org/rfc/rfc1576.html)、[RFC 2355](https://www.rfc-editor.org/rfc/rfc2355.html)）。
2. 出力は初期段階で Write / Erase Write、WCC、SBA、SF、IC と基本属性を実装する。入力は Enter、Clear、PF、PA の AID、カーソル位置、SBA で示される変更フィールドを読み取り、保護フィールドへの入力、範囲外アドレス、不正なレコードを検出する。BMS の `ERASE`、`DATAONLY`、`MAPONLY`、`FREEKB`、`ALARM` と、フィールドの `PROT`、`UNPROT`、`IC`、`FSET`、色・強調を段階的に反映する。3270 データストリームの命令は [IBM の資料](https://www.ibm.com/docs/en/cics-ts/6.x?topic=terminals-orders-in-data-stream)に合わせる。
3. Rust 内部では画面を「24×80 のセル、属性、入力可能なフィールド、カーソル」として保持する。`SEND MAP` は画面モデルを更新して 3270 バイト列に符号化し、`RECEIVE MAP` は受信バイト列をそのモデルと突き合わせる。画面テキストのスクレイピングを COBOL 入力の根拠にしない。画面寸法や拡張属性を使うマップは、端末モデルの照合を追加してから受け付ける。
4. 端末に送る文字は指定したホストコードページの EBCDIC で符号化する。既存の COBOL EBCDIC モード、BMS の `LENGTH`（バイト数）、SO/SI を使う DBCS、端末の `-charset` を一組として検証する。最初は Bank-of-Z の英数字画面と CP037 相当を対象にし、日本語の CP930/939/1390/1399 は別の受け入れテストを設ける。端末が切断したときは待機中のタスクをエラーとして終え、未コミットの Db2 作業をロールバックする。

## BMS と COBOL の接続

`.bms` の `DFHMSD` / `DFHMDI` / `DFHMDF` を読み、Rust 用の物理マップ定義と GnuCOBOL 用の symbolic map コピー句を同一の中間表現から生成する。生成コピー句は入力側の `L` / `F` / `I` と出力側の `A` / `O` など、COBOL から参照されるレイアウトを IBM の symbolic map に合わせる。named / unnamed field、`POS`、`LENGTH`、`INITIAL`、属性、`MODE=INOUT`、継続行を最初からテストする。BMS の物理マップと symbolic map の役割は [IBM BMS maps](https://www.ibm.com/docs/en/cics-tx/10.1.0?topic=processor-bms-maps) および [symbolic input map](https://www.ibm.com/docs/en/cics-ts/5.5.0?topic=example-symbolic-input-map) に従う。

`cobc --cics` または専用の前処理段階を設け、固定形式の列規則、コメント、継続行、引用符を認識したうえで `EXEC CICS ... END-EXEC` をブリッジ呼び出しへ変換する。`COPY DFHAID`、EIB、`DFHRESP(...)` を扱う互換コピー句／定数も用意する。元ファイルと行を生成ソースへ保持し、`--save-temps` で変換結果を確認できるようにする。既存の `EXEC SQL` とは変換段階を合成し、CICS → SQL → 通常の COBOL 前処理の順序を統合試験で確定する。SQL や COBOL の引用符内にある `EXEC CICS` を変換しない。

各 CICS コマンドは、サポートするオプションを明示的に解析する。とくに `RESP` / `RESP2`、EIB の `EIBRESP` / `EIBRESP2`、`EIBAID`、`EIBCALEN`、`EIBTRNID` を COBOL の表現に合わせる。`MAPFAIL` は入力が空、SBA がない、Clear / PA などで入力フィールドが送られない場合にも返す。不明なマップ、長さ不一致、切断などの結果も決め、成功に見せかける空実装は置かない（[IBM RECEIVE MAP](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-receive-map)）。

## タスクと Db2 作業単位

端末接続後、Rust は TRANSID と COBOL プログラムの対応表から初回タスクを起動する。`SEND MAP` 後の `RETURN TRANSID(...) COMMAREA(...) LENGTH(...)` では、現在のタスクを正常終了させ、COMMAREA のバイト列と次の TRANSID をセッションに保存する。次の AID で新しい worker を起動し、保存した COMMAREA と `EIBCALEN`、AID 情報を渡す。`RECEIVE MAP` はその AID と変更フィールドを symbolic input map に展開する。これが `BNKMENU` の初回表示から入力処理までの基本経路となる。CICS の `LINK` / `XCTL` / `RETURN` の制御階層と COMMAREA の意味は [IBM の説明](https://www.ibm.com/docs/en/cics-ts/6.x?topic=control-passing-data-other-programs)に合わせる。

Db2 接続は worker のタスク単位で所有する。`EXEC CICS SYNCPOINT` は既存 Db2 アダプターの commit、`SYNCPOINT ROLLBACK` と `ABEND` は rollback に結び付ける。正常な最上位 `RETURN` では未確定の作業を commit し、異常終了・切断・worker の失敗では rollback する。これは CICS のタスク終了時の暗黙 syncpoint と異常終了時の backout に合わせた扱いである（[IBM synchronization points](https://www.ibm.com/docs/en/cics-ts/6.x?topic=work-synchronization-points)）。`EXEC SQL COMMIT WORK` / `ROLLBACK` も同じ接続状態を変更するため、二重 commit や次のタスクへの接続持ち越しが起きないよう SQL アダプターに明示的な作業単位 API を追加する。`ENQ` / `DEQ` は複数セッションにまたがる排他制御として後段で実装する。

## 実装順序と受け入れ条件

1. **通信の縦断検証:** Rust の TN3270 サーバーから c3270 と wc3270 に 24×80 の入力フィールド付き画面を表示し、Enter、PF3、Clear と入力済みフィールドを受け取る。Telnet 分割受信、IAC エスケープ、切断、複数セッションを試す。テスト用の s3270 スクリプト API で同じバイト列と画面を検証する。
2. **最小の COBOL/BMS 統合:** 小さな `.bms` から物理・symbolic map を生成し、`SEND MAP` → `RETURN TRANSID/COMMAREA` → AID → `RECEIVE MAP` を、再起動された COBOL タスクで実行する。`EIBAID`、`EIBCALEN`、`MAPFAIL`、`RESP/RESP2` と画面の入力値を照合する。無効なマップ／未対応オプションは明確なエラーにする。
3. **Bank-of-Z のメニュー:** `BNK1MAI.bms` と `BNKMENU.cbl` を対象に、初回メニュー表示、Action の入力、無効キー、PF3/PF12、次の TRANSID を確認する。必要な `ASSIGN`、`ASKTIME`、`FORMATTIME`、`LINK` などはこの経路に沿って追加する。元ソースの変更箇所を記録する。
4. **Db2 と業務プログラム:** `INQCUST.cbl` 等を既存の Db2 Community Edition と接続し、メニューから照会・更新の経路を通す。`SYNCPOINT`、`ROLLBACK`、異常終了、並行する二つの端末でデータと SQLCA の結果を確認する。SQL/CICS の前処理と EBCDIC モードを併用する。
5. **互換範囲の拡張:** 残りの Bank-of-Z CICS 文を棚卸し順に追加し、TN3270E と Windows／Linux の端末接続を検証する。対応状況をコマンドとオプション単位で公開し、サポートしていない CICS 機能を一覧化する。

受け入れテストでは端末画面だけでなく、COBOL が受けた symbolic map のバイト、EIB、`RESP/RESP2`、COMMAREA、Db2 の commit / rollback 結果を比較する。`wc3270` / `c3270` は対話操作で確認し、自動化には同じ x3270 系のスクリプト機能と、Rust の 3270 プロトコルテストを使う。

## 初期段階の制約

- TN3270E、TLS、端末プリンター、構造化フィールド、高度な BMS 属性と日本語 DBCS は、基本 TN3270 と英数字画面の受け入れ後に追加する。端末とサーバーの文字コード設定が一致しない場合は明示的に診断する。
- CICS の永続キュー、ファイル管理、資源定義、セキュリティ、分散作業単位は別の実装範囲として扱う。Bank-of-Z の該当文は棚卸しで識別し、実装するまで変換を失敗させる。
- worker と SQL アダプターのプロセス境界、GnuCOBOL モジュールの固定 ABI エントリー、`RETURN` 時の正常終了は先に小さな実験で確かめる。ここが成立しない場合は、COBOL worker を常駐させてセッションごとに直列実行する方式へ切り替え、同じ CICS ブリッジと TN3270 サーバーを使う。

## 実装状況（2026-10-01）

`extras/cobcics` に Rust の TN3270 ホストと BMS から COBOL symbolic map
を作るツールを追加した。Bank-of-Z の BMS 10 ファイルについてコピー句を生成し、
GnuCOBOL でまとめてコンパイルした。`BNK1MAI` の画面、Action 入力、PF3、
COBOL worker を通した応答はスクリプト化した TN3270 クライアントで確認した。
後続の検証では Windows に導入された wc3270 を使い、実クライアントで表示を確認した。

追加実装では、環境変数による worker 呼び出しを廃止し、標準入出力上の
バージョン付き長さ付きメッセージと C ABI ブリッジに置き換えた。
`EXEC CICS RECEIVE MAP`、`SEND MAP`、`RETURN` を
限定的に変換し、生成した COBOL を GnuCOBOL でコンパイルして端末との
往復を確認した。未対応の文・オプションはソース行付きで拒否する。

`RETURN TRANSID('OMEN') COMMAREA(...) LENGTH(...)` の最大 256 バイトを
セッションに保存し、次の AID で別の worker プロセスへ `EIBCALEN`、
`EIBTRNID` とともに引き継ぐ経路を追加した。初回表示も worker で実行し、
オプションなしの `RETURN` では画面送信後に接続を終了する。
COBOL のデモ worker とスクリプト化した端末でこの疑似会話型の往復、
PF3 後の切断、および新規セッションの初期化を確認した。
`--transid` と `--route TRANSID=worker` で初回タスクと次の worker を
選択できる。未登録 ID は、指定された場合のみ `--worker` を代替として使う。
`RECEIVE MAP` は入力フィールドがない Enter、Clear、PA を
`MAPFAIL`（36）として `EIBRESP` と指定された `RESP` に返す。
`SEND MAP` と `RETURN` の正常結果も `RESP` / `RESP2` に返し、
`DFHRESP(NORMAL)` と `DFHRESP(MAPFAIL)` を変換する。`RESP` がない
`RECEIVE MAP` の MAPFAIL は worker を異常終了させる。
この判定は基本的な経路に限り、名前付きフィールドのカーソル位置や
BMS の MDT/FSET による例外は未対応。
`COPY DFHAID` に対して Enter、Clear、PA1～PA3、PF1～PF24 の
基本 AID 値を定義した互換コピー句を追加し、デモ worker で利用した。
`SEND CONTROL ERASE FREEKB` を追加し、画面を消去してキーボードを
解除する 3270 Erase/Write レコードを送れるようにした。Clear で空画面を
送信して接続を終了する経路を COBOL worker から確認した。
`SEND MAP` の `ERASE`、`DATAONLY`、`FREEKB`、`DATAONLY ALARM` も追加した。
`DATAONLY` は worker が返したフィールドだけを 3270 Write で更新し、
`ALARM` は WCC の警報ビットを設定する。デモの初回画面、通常の Enter、
MAPFAIL、Clear の各経路で送信コマンドと WCC を検証した。
`ASSIGN APPLID` と `ASSIGN PROGRAM` を追加した。APPLID は起動時の
`--applid`（既定 `COBCICS`）から worker 要求へ渡し、PROGRAM は
前処理器がソースの `PROGRAM-ID` から 8 文字値を生成する。
両方の値を COBOL のデモ worker から画面へ出して確認した。
`LINK PROGRAM(...) COMMAREA(...)` は同一 worker 内の COBOL `CALL` に
変換した。呼び出し先の `DFHCOMMAREA` は参照渡しし、`EIBCALEN` は渡した長さに
設定する。リンク先の `RETURN` は呼び出し元へ戻り、最上位の `RETURN` のみ
端末応答を確定する。小さな二つの COBOL プログラムで、リンク先が変更した
COMMAREA を呼び出し元が読めることを確認した。この時点では元の
`ABNDPROC` の `WRITE FILE` が未対応だった。
`RETURN TRANSID(...) IMMEDIATE` を追加した。worker 応答に即時起動フラグと
画面変更なしの動作を加え、Rust ホストは端末 AID を待たずに次の TRANSID の
worker を起動する。COMMAREA を渡し、連続即時起動は 32 タスクに制限する。
デモでは OMEN から ODCS の別 worker へ即時に切り替え、途中の画面送信なしに
ODCS の画面を表示し、次の AID は OMEN に戻ることを確認した。
`SEND TEXT FROM(...) ERASE FREEKB` も追加した。COBOL データ項目から長さを
補い、画面消去後に保護されたテキスト欄を表示する。Bank-of-Z の終端文面を
デモ worker から送って端末ストリームを確認した。BMS の改行・ページ形成は
この段階では対象外。

`BNKMENU.cbl` に残っていた `ABEND ABCODE('HBNK') NODUMP`、
`ASKTIME ABSTIME(...)`、`FORMATTIME ABSTIME(...) DDMMYYYY(...) TIME(...)
DATESEP` を追加した。`ABEND` は worker を応答なしで終了させ、Rust ホストは
その端末接続を閉じる。時刻はホストのローカル時刻を 1900 年基準のミリ秒数として
8 バイト packed decimal に格納し、`FORMATTIME` で日付・時刻を出す。
デモ worker では現在時刻と 2000 年 2 月 29 日の固定値を照合し、
異常終了のコード `HBNK` と非ゼロ終了を確認した。`EIBTASKN` は現時点で
worker プロセス ID を仮のタスク番号として使用する。`ASKTIME` による
`EIBDATE` / `EIBTIME` の更新、HANDLE ABEND exit、Db2 作業単位の rollback は
この時点では未実装または未検証だった。元の `BNKMENU.cbl` は全 `EXEC CICS` 文の変換が通り、
生成 COBOL は GnuCOBOL のオブジェクトまでコンパイルできた。z/OS 固有の
`PROCESS` / `CBL` 指令は GnuCOBOL から無視の警告が出る。

Bank-of-Z の `cics/cobol` 32 ファイルにある `EXEC CICS` 749 箇所を、
前処理器の文変換器で構文監査した。現在 582 箇所が通り、167 箇所が未対応で、
26 ファイルに残る。これは BMS の各フィールドと COBOL／SQL のコンパイルまで
成功した数ではない。特に `ABEND` の未対応形 46、`SYNCPOINT` 20、
`LINK` の未対応形 17、`SEND MAP` の未対応形 13 が大きい。

BMS は単一マップと基本属性の経路であり、より広い CICS 応答条件、
CICS の資源定義、疑似会話の次タスクへの `DFHCOMMAREA` linkage 受け渡し、
Bank-of-Z 全体の Db2 作業単位は次の実装段階に残る。`cobc --cics-map=<copybook>` を追加し、
`--db2-sql` との併用時に CICS → SQL の順で前処理されることを生成ソースで
確認した。後続のローカル `LINK SYNCONRETURN` 試験で実 Db2 の INSERT、
正常終了時コミット、ABEND 時ロールバックを確認した。
拡張 BMS 属性は
symbolic map に配置しているが、3270 ストリームへの反映は未実装。

### メニュー先への遷移（2026-10-01）

`BNK1DAC`（メニュー選択肢 2、Display Account）と `BNK1TFN`
（選択肢 7、Transfer Funds）を Bank-of-Z の元 COBOL ソースから
前処理・コンパイルした。`BIF DEEDIT` の限定的な変換を追加し、
`LINK SYNCONRETURN` は構文を受け取り、ローカル `LINK` ではリンク自体を
コミット境界とせず、同一タスクで呼び出し先を実行する。リモート DPL における
復帰時コミットは未実装。Db2 アダプターをリンクした worker は正常な最上位
`RETURN` でコミットし、`ABEND` または CICS ブリッジの異常終了でロールバックする。
実 Db2 でリンク先 INSERT の正常終了と、リンクから戻った後の ABEND を比較し、
前者のみ永続化されることを確認した。この時点では Bank-of-Z の呼び出し先
`INQACC`、`DELACC`、`XFRFUN` に別の未対応 CICS 文が残り、画面デモには未リンクだった。

Rust ホストに `--map-route TRANSID=BMS-FILE` を追加し、即時起動で
遷移先の物理マップへ切り替え、PF3 の `OMEN` 復帰時にはメニューの
マップへ戻す。wc3270 の Read Modified が送る 12 ビットアドレスの
別符号を受理し、BMS `INITIAL` 文字列の継続行を単語途中で正しく結合した。
スクリプト化した TN3270 と実 wc3270 の両方で
`OMEN → ODAC → OMEN` と `OMEN → OTFN → OMEN` を確認した。
再現手順は [cobcics README](../../extras/cobcics/README.md) に記載する。

### 口座照会・送金のリンク先までの対応（2026-10-02）

`INQACC`、`DELACC`、`XFRFUN` の `HANDLE ABEND LABEL`、
`ASSIGN ABCODE`、省略可能な `ABCODE` / `NODUMP` / `CANCEL` を持つ
`ABEND`、`SYNCPOINT` / `SYNCPOINT ROLLBACK`、`DELAY FOR SECONDS`
を変換する。`ABNDPROC` の `WRITE FILE('ABNDFILE')` も診断ログ追記として
実装した。`build-bank-business.ps1` は元の COBOL 4 本の CICS 文と
3 本の埋め込み SQL を変換し、親画面 2 本と Db2 アダプターを統合リンクする。
`sqlda.cpy` の無効な `PIC USAGE` 宣言も修正した。

3 本の業務モジュールと `ABNDPROC` はコンパイルできた。合成 worker による
実行試験では、異常終了ハンドラへの移動、`ASSIGN ABCODE`、`ABEND CANCEL`、
診断レコードの追記を確認した。実 Db2 試験では通常終了の commit、
ABEND 時の rollback、明示的な `SYNCPOINT` と `SYNCPOINT ROLLBACK` を確認した。
統合 worker で `OMEN → ODAC → OMEN` と `OMEN → OTFN → OMEN` の
TN3270 画面遷移も確認した。Bank の `ACCOUNT` / `PROCTRAN` 表と
サンプルデータを用いた実業務 SQL の通し試験は別途必要である。

### IMS 以外の Bank of Z 構文監査（2026-10-02）

`src/base/cics/cobol` の 32 本を対象に、BMS 10 本から symbolic map を生成し、
CICS と埋め込み SQL の変換後に全件を GnuCOBOL のオブジェクト生成まで確認した。
`python extras/cobcics/audit-bank-syntax.py` で監査を再実行でき、診断ログは
`build/bank-syntax` に残る。CICS 文 749 箇所に変換時の未対応構文は残らない。
`BANKDATA` は `EXEC CICS` を含まないため SQL 変換のみを適用する。

今回追加した範囲は `GET/PUT CONTAINER`、`RUN TRANSID CHANNEL CHILD`、
`FETCH ANY CHANNEL NOSUSPEND`、`ENQ/DEQ`、`INQUIRE/SET TERMINAL UCTRANST`、
`INQUIRE ASSOCIATION`、`SEND MAP MAPONLY/CURSOR`、`RECEIVE MAP ASIS/TERMINAL`、
`RETURN TRANSID COMMAREA` の長さ省略、BMS `JUSTIFY`、および
`OCCURS 10` の口座一覧フィールドである。繰り返しフィールドには
`ACCO0001` から `ACCO0010` の一意な pipe 名を割り当てる。
不足していた `DFHBMSCA` と `CEEIGZCT` の互換コピー句も追加した。

Rust の単体テストと C ブリッジの実行テストで、繰り返しフィールドの
画面配置・入出力、子タスクへのチャネル複製、結果チャネルの取得、
親チャネルの保持、ENQ/DEQ を確認した。Bank のクレジット照会の
`RUN` は現在同一 worker 内で同期的に子プログラムを `CALL` する実装であり、
CICS の並列タスク実行や独立した Db2 作業単位までは再現しない。
IMS の呼び出し先、LE の `CEEDAYS` / `CEELOCT` 実行サービス、全 32 本の
業務リンク・Db2 データを伴う通し実行は今回の構文監査の対象外である。
