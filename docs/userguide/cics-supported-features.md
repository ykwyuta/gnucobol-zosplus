# CICS 対応機能と EXEC CICS コマンド一覧

この一覧は、このリポジトリの `bin/cics-preprocess.py` と `extras/cobcics/` の実装を対象とする。リンク先は IBM CICS Transaction Server for z/OS の公式リファレンス（主に 6.x）であり、**IBM CICS の全オプションがこの実装で使えるという意味ではない**。表の「対応範囲」に記した構文だけを受け付ける。`EXEC CICS` は前処理で COBOL とローカルブリッジの呼び出しに変換され、未対応のコマンドやオプションは元ソースの行番号付きエラーになる。

## 対応している機能

| 分野 | この実装でできること | 実装条件・範囲 | IBM 公式資料 |
| --- | --- | --- | --- |
| 3270 端末 | Rust ホスト `cobcicsd` と `wc3270` / `c3270` で画面を送受信する。端末タイプは `IBM-3278-2` / `IBM-3279-2`、基本画面は 24 行 × 80 桁。 | TN3270 の BINARY、TERMINAL-TYPE、EOR を使用する。TN3270E や一般の CICS 領域を提供するものではない。 | [IBM: Terminal control](https://www.ibm.com/docs/en/cics-ts/6.x?topic=programs-terminal-control) |
| BMS | `DFHMSD`、`DFHMDI`、`DFHMDF` の使用部分を読み、物理マップと生成したシンボリックマップの項目を送受信する。変更項目、属性、`OCCURS`、`MAPONLY` / `DATAONLY` を扱う。 | `--bms` で読み込んだマップに限る。マップセットの全機能、ページング、印刷は対象外。 | [IBM: Basic mapping support](https://www.ibm.com/docs/en/cics-ts/6.x?topic=components-basic-mapping-support) |
| トランザクション実行 | `--transid`、`--route`、`--map-route` でトランザクション ID と COBOL ワーカー、BMS マップを結び付ける。`RETURN TRANSID` とローカル `LINK` を扱う。 | ルートは起動時に登録する。分散プログラムリンクや CICS の資源定義・セキュリティー機構は提供しない。 | [IBM: RETURN](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-return)、[IBM: LINK](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-link) |
| データ受け渡し | COMMAREA、チャネルとコンテナー、`EIB`、`RESP` / `RESP2` の一部を扱う。 | コンテナーはローカルのタスク／子タスク内のメモリーに保持する。1 コンテナー最大 2048 バイト、1 チャネル最大 12 コンテナー。 | [IBM: Channels and containers](https://www.ibm.com/docs/en/cics-ts/6.x?topic=channels-containers)、[IBM: EIB](https://www.ibm.com/docs/en/cics-ts/6.x?topic=terminal-eib-feedback-control-operations) |
| 文字コード | IBM-037、IBM-930、IBM-939、IBM-1390、IBM-1399 の画面データを UTF-8 のワーカーパイプと相互変換する。日本語 DBCS は SO/SI を扱う。 | `--charset` と COBOL の `-fdata-codeset` をそろえる。手順は [CICS/BMS ガイド](cics-bms.md#日本語-ebcdic-の画面入出力) を参照。 | [IBM: Basic mapping support](https://www.ibm.com/docs/en/cics-ts/6.x?topic=components-basic-mapping-support) |
| キュー、カウンター、時刻指定起動 | 一時記憶キュー、書き込み専用の一時データキュー、名前付きカウンター、`START AFTER` をローカルファイルで模擬する。 | GenApp 用拡張モジュールのリンクが必要。格納場所は `COBCICS_STORAGE_DIR`、未指定時は一時ディレクトリー。CICS の TS/TD 資源やカップリングファシリティーは使用しない。 | [IBM: Temporary storage](https://www.ibm.com/docs/en/cics-ts/6.x?topic=tuning-cics-temporary-storage-overview)、[IBM: Transient data control](https://www.ibm.com/docs/en/cics-ts/6.x?topic=programs-transient-data-control) |
| ファイル、排他、同期点 | GenApp の `KSDSCUST` / `KSDSPOLY` と、レイアウトを指定したキー付き固定長 FILE の READ / WRITE / REWRITE / DELETE。実体はローカル FB、Db2、PostgreSQL から起動時に一つ選ぶ。`ENQ` / `DEQ` はローカルファイルロック、`SYNCPOINT` は選択した SQL 接続の commit / rollback に接続する。 | IBM の物理 VSAM データセット形式、分散ロック、CICS 全資源にまたがる障害回復は実装しない。SQL モードは対応するランタイムと表を準備する。 | [IBM: File control](https://www.ibm.com/docs/en/cics-ts/6.x?topic=programs-file-control)、[IBM: SYNCPOINT](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-syncpoint) |

ビルドと端末の起動方法は [Bank of Z](cics-bms.md) と [GenApp](cics-genapp.md) を参照。以下の「基本」は CICS ブリッジ、「GenApp」はキュー・ファイル・時刻指定起動の追加モジュールを組み込んだビルドを指す。Db2 の同期点は `COBCICS_DB2SQL` を付けてビルドした場合に働く。

## COBOL の EXEC CICS コマンド

`RESP(x)` / `RESP2(x)` を受け付けるコマンドでは結果を指定領域へ返す。表にない IBM CICS の共通オプションも自動的には使えない。`NOHANDLE` などは対象外。`HANDLE CONDITION` と `HANDLE AID` は下記の限定された形だけを扱う。

### 画面・端末

| コマンド | 対応範囲と動作 | 条件 | IBM 公式リファレンス |
| --- | --- | --- | --- |
| `SEND MAP` | `MAP`、`MAPSET`、`FROM`、`ERASE`、`MAPONLY`、`DATAONLY`、`ALARM`、`FREEKB`、引数なしの `CURSOR`、`RESP` / `RESP2`。読み込んだ BMS マップを送る。`ALARM` は `DATAONLY` と組み合わせた場合だけ。 | 基本 | [SEND MAP](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-send-map) |
| `RECEIVE MAP` | `MAP`、`MAPSET`、`INTO`、`TERMINAL`、`ASIS`、`RESP` / `RESP2`。変更された入力項目をシンボリックマップへ入れ、入力なしなどは `MAPFAIL`（36）を返す。 | 基本 | [RECEIVE MAP](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-receive-map) |
| `SEND CONTROL` | `ERASE FREEKB` を必須とし、画面消去とキーボード解放を行う。`RESP` / `RESP2` も可。 | 基本 | [SEND CONTROL](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-send-control) |
| `SEND TEXT` | `FROM`、任意の `LENGTH`、必須の `ERASE FREEKB`、任意の `WAIT` と `RESP` / `RESP2`。マップを使わず文字列を表示する。最大 1919 バイト。IBM CICS の自動改行・ページングは行わない。 | 基本 | [SEND TEXT](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-send-text) |
| `RECEIVE` | `INTO`、`LENGTH`、`RESP` / `RESP2` の形で端末入力を受ける。このホストでは先頭 4 バイトのトランザクション ID、空白 1 バイト、最初の入力項目を組み立てて返す。3270 入力データストリーム全体をそのまま渡す API ではない。 | 基本 | [RECEIVE (3270 logical)](https://www.ibm.com/docs/en/cics-ts/5.6?topic=summary-receive-3270-logical) |
| `HANDLE AID` | `CLEAR(label)`、`PF3(label)` のみ。記述位置で `EIBAID` を判定し、該当すれば同一 COBOL プログラムのラベルへ分岐する。 | 基本 | [HANDLE AID](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-handle-aid) |
| `HANDLE CONDITION` | `MAPFAIL(label)` のみ。`RECEIVE MAP` の `MAPFAIL` で指定ラベルへ分岐する。 | 基本 | [HANDLE CONDITION](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-handle-condition) |
| `INQUIRE TERMINAL` | `TERMINAL(EIBTRMID)` と `UCTRANST` の照会のみ。`RESP` / `RESP2` も可。初期値は `NOUCTRAN`（451）。 | 基本 | [INQUIRE TERMINAL](https://www.ibm.com/docs/en/cics-ts/6.x?topic=commands-inquire-terminal) |
| `SET TERMINAL` | `TERMINAL(EIBTRMID)` と `UCTRANST` の設定のみ。`RESP` / `RESP2` も可。450 (`UCTRAN`) で入力を大文字化、451 (`NOUCTRAN`) で無変換。452 も受理するが独立した変換動作はない。 | 基本 | [SET TERMINAL](https://www.ibm.com/docs/en/cics-ts/6.x?topic=commands-set-terminal) |

### プログラム・タスク・状態

| コマンド | 対応範囲と動作 | 条件 | IBM 公式リファレンス |
| --- | --- | --- | --- |
| `LINK` | `PROGRAM` と `COMMAREA` を必須とし、`LENGTH`、`SYNCONRETURN`、`RESP` / `RESP2` を受け付ける。リンク済みのローカル COBOL プログラムを呼び、終了後に呼び元へ戻る。COMMAREA は最大 32767 バイト。`SYNCONRETURN` はローカル呼び出しでは動作を変えない。 | 基本 | [LINK](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-link) |
| `RETURN` | 引数なしのタスク終了、または `TRANSID('....')`、任意の `COMMAREA` / `LENGTH`、`IMMEDIATE`、`RESP` / `RESP2`。次の端末入力または即時に登録済みトランザクションへ制御を渡す。`RETURN TRANSID` の COMMAREA は最大 2048 バイト。 | 基本 | [RETURN](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-return) |
| `ASSIGN` | `APPLID`、`PROGRAM`、`ABCODE`、`SYSID`、`STARTCODE`、`INVOKINGPROG` の指定先へ値を返す。`RESP` / `RESP2` も可。`APPLID` はホスト設定、`SYSID` は固定の `L001` などローカル値。 | 基本 | [ASSIGN](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-assign) |
| `INQUIRE ASSOCIATION` | `ASSOCIATION(EIBTASKN)` と `ODAPPLID`、`ODUSERID`、`ODFACILNAME`、`ODNETWORKID`、`ODFACILTYPE` を一緒に指定する形のみ。Bank of Z が参照する発信元情報にローカル固定値を返す。 | 基本 | [INQUIRE ASSOCIATION](https://www.ibm.com/docs/en/cics-ts/6.x?topic=commands-inquire-association) |
| `PUT CONTAINER` | `CONTAINER`、任意の `CHANNEL`、`FROM`、任意の `FLENGTH`、`RESP` / `RESP2`。タスク内の指定チャネルにデータを置く。 | 基本 | [PUT CONTAINER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-put-container-channel) |
| `GET CONTAINER` | `CONTAINER`、任意の `CHANNEL`、`INTO`、任意の `FLENGTH`、`RESP` / `RESP2`。タスク内のデータを取得する。 | 基本 | [GET CONTAINER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-get-container-channel) |
| `RUN TRANSID` | `TRANSID`、`CHANNEL`、`CHILD`、`RESP` / `RESP2`。`OCR1`～`OCR5` を対応するローカル COBOL プログラムとして実行する。**子プログラムは同じワーカー内で同期実行**する。 | Bank of Z の該当プログラムをリンク | [RUN TRANSID](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-run-transid) |
| `FETCH ANY` | `ANY`、`CHANNEL`、`NOSUSPEND`、`COMPSTATUS`、`ABCODE`、`RESP` / `RESP2`。前記のローカル子プログラムの完了結果を取得する。IBM CICS の並列子タスク待ちではない。 | Bank of Z の該当プログラムをリンク | [FETCH ANY](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-fetch-any) |
| `START` | `TRANSID('....') AFTER MINUTES(n)` のみ。ローカルの時刻指定タスクを登録する。最大 8 件、待ち時間は 0～525600 分。`RESP` / `RESP2` も可。 | GenApp | [START](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-start) |
| `DELAY` | `FOR SECONDS(n)` のみ。0～60 秒待機する。`RESP` / `RESP2` も可。 | 基本 | [DELAY](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-delay) |
| `ASKTIME` | `ABSTIME` へローカル時刻に基づく時刻値を返す。`RESP` / `RESP2` も可。 | 基本 | [ASKTIME](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-asktime) |
| `FORMATTIME` | `ABSTIME`、`DDMMYYYY` または `MMDDYYYY` のどちらか、`TIME`、任意の `DATESEP` と `RESP` / `RESP2`。日時を文字列化する。 | 基本 | [FORMATTIME](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-formattime) |
| `BIF DEEDIT` | `FIELD`、任意の `LENGTH`。数字以外を除去して左側をゼロ詰めする。IBM CICS の符号付きゾーン形式の特殊処理は実装しない。 | 基本 | [BIF DEEDIT](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-bif-deedit) |
| `HANDLE ABEND` | `LABEL(label)`、`CANCEL`、`RESET`。同一プログラムの明示的な `ABEND` に対する出口を切り替える。その他の異常終了を捕捉する一般的な CICS 出口ではない。 | 基本 | [HANDLE ABEND](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-handle-abend) |
| `ABEND` | 任意の `ABCODE`、`NODUMP`、`CANCEL`。明示的に異常終了し、設定済み `HANDLE ABEND` へ制御を渡せる。トランザクションダンプは作成しない。 | 基本 | [ABEND](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-abend) |

### キュー・カウンター・ファイル・同期点

| コマンド | 対応範囲と動作 | 条件 | IBM 公式リファレンス |
| --- | --- | --- | --- |
| `READQ TS` | `QUEUE`、`INTO`、`LENGTH`、`ITEM` または `NEXT`、`RESP` / `RESP2`。ローカル一時記憶キューから項目を読む。 | GenApp | [READQ TS](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-readq-ts) |
| `WRITEQ TS` | `QUEUE`、`FROM`、`LENGTH`、任意の `ITEM`、`REWRITE`、`NOSUSPEND`、`RESP` / `RESP2`。項目を追加または上書きする。 | GenApp | [WRITEQ TS](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-writeq-ts) |
| `DELETEQ TS` | `QUEUE`、`RESP` / `RESP2`。ローカル一時記憶キューを削除する。 | GenApp | [DELETEQ TS](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-deleteq-ts) |
| `WRITEQ TD` | `QUEUE`、`FROM`、`LENGTH`、`RESP` / `RESP2`。ローカルの一時データ記録へ追記する。`READQ TD` と `DELETEQ TD` は未対応。 | GenApp | [WRITEQ TD](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-writeq-td) |
| `DEFINE COUNTER` | `COUNTER`、`POOL`、`VALUE`、`RESP` / `RESP2`。ローカルの名前付き整数カウンターを作る。 | GenApp | [DEFINE COUNTER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-define-counter-define-dcounter) |
| `GET COUNTER` | `COUNTER`、`POOL`、`VALUE`、`RESP` / `RESP2`。現在値を取得して 1 増やす。 | GenApp | [GET COUNTER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-get-counter-get-dcounter) |
| `QUERY COUNTER` | `COUNTER`、`POOL`、`VALUE`、`RESP` / `RESP2`。現在値を読み取る。 | GenApp | [QUERY COUNTER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-query-counter-query-dcounter) |
| `DELETE COUNTER` | `COUNTER`、`POOL`、`RESP` / `RESP2`。カウンターを削除する。 | GenApp | [DELETE COUNTER](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-delete-counter-delete-dcounter) |
| `READ` | `FILE('KSDSCUST'|'KSDSPOLY')`、`INTO`、`LENGTH`、`RIDFLD`、`KEYLENGTH`、任意の `GTEQ` / `GENERIC` / `UPDATE`、`RESP` / `RESP2`。Db2 上の該当キー付きレコードを読む。`GENERIC` は `GTEQ` と併用する。 | GenApp + Db2 | [READ](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-read) |
| `WRITE` | 上記 2 ファイルについて `FROM`、`LENGTH`、`RIDFLD`、`KEYLENGTH`、`RESP` / `RESP2` で Db2 に追加する。`FILE('ABNDFILE') FROM(...) RIDFLD(...)` は別扱いで、`COBCICS_ABEND_LOG` のローカル追記ログへ書く。 | GenApp + Db2（`ABNDFILE` は基本） | [WRITE](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-write) |
| `REWRITE` | 上記 2 ファイルについて `FROM`、`LENGTH`、`RESP` / `RESP2` で更新する。 | GenApp + Db2 | [REWRITE](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-rewrite) |
| `DELETE` | 上記 2 ファイルについて `RIDFLD`、`KEYLENGTH`、`RESP` / `RESP2` で削除する。 | GenApp + Db2 | [DELETE](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-delete) |
| `ENQ` | `RESOURCE`、任意の `LENGTH`、`RESP` / `RESP2`。同じローカルホストのロックファイルで資源を排他する。`COBCICS_LOCK_DIR` で保存先を指定できる。 | 基本 | [ENQ](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-enq) |
| `DEQ` | `RESOURCE`、任意の `LENGTH`、`RESP` / `RESP2`。上記のローカル排他を解除する。 | 基本 | [DEQ](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-deq) |
| `SYNCPOINT` | 引数なしで Db2 変更を確定する。GenApp の保留 `START` も反映する。`RESP` / `RESP2` も可。 | Db2 には `COBCICS_DB2SQL`、`START` には GenApp | [SYNCPOINT](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-syncpoint) |
| `SYNCPOINT ROLLBACK` | Db2 変更を取り消し、GenApp の保留 `START` を破棄する。`RESP` / `RESP2` も可。ローカルキューやカウンターなど全資源を一括で戻すものではない。 | Db2 には `COBCICS_DB2SQL`、`START` には GenApp | [SYNCPOINT ROLLBACK](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-syncpoint-rollback) |

### 関連する定数と EIB フィールド

前処理では `DFHRESP(NORMAL|MAPFAIL|NOTFND|INVREQ|NOTFINISHED|QIDERR|ITEMERR|LENGERR|NOSPACE|DUPREC|IOERR)` と `DFHVALUE(UCTRAN|NOUCTRAN|TRANIDONLY|NORMAL|ABEND|SECERROR)` を展開する。`EIBAID`、`EIBCALEN`、`EIBTRNID`、`EIBTRMID`、`EIBTASKN`、`EIBRESP`、`EIBRESP2` の領域も必要に応じて補う。これらは `EXEC CICS` コマンドではない。IBM 側の定義は [EXEC CICS コマンド形式](https://www.ibm.com/docs/en/cics-ts/6.x?topic=reference-exec-cics-command-format-programming-considerations) と [EIB の端末制御フィールド](https://www.ibm.com/docs/en/cics-ts/6.x?topic=terminal-eib-feedback-control-operations) を参照。
