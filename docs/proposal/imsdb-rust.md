# Rust 製 IMS DB 互換データベース実装提案

## 目的と互換性の境界

Bank of Z と AWS CardDemo の IMS DB 資材を、元の DBD・PSB・COBOL・PL/I・JCL を原則変更せずに実行する。Rust で階層データ、DL/I の位置付けと更新、PCB フィードバック、トランザクション、ロック、索引を実装する。既存の `cobcicsd`、`jclsh`、PL/I コンパイラから同じデータベースを利用する。

**対象は単一ノードでの正しい同時実行と性能**である。複製、クラスタ、リーダー切り替え、フェイルオーバー、オンラインバックアップ、災害対策、無停止アップグレードは実装しない。一方、コミット済みデータの永続化、クラッシュ後の一貫性回復、デッドロック解消はデータベースの正しさに必要なので実装する。IMS の全製品機能、実際の OSAM/VSAM 物理ファイル形式、IBM のランダマイザー内部アルゴリズムとのビット単位の互換は目標にしない。

## 参照資材から確定した範囲

| 資材 | データ定義とアクセス | 呼び出し側 |
| --- | --- | --- |
| [Bank of Z の IMS](../../reference/Bank-of-Z-main/src/base/ims/) | DBD 9 本、PSB 8 本。主に `ACCESS=(HDAM,OSAM)` のルートセグメント、`Cp1047`、固定長・バイナリ・COMP-3 項目。たとえば [CUSTOMER DBD](../../reference/Bank-of-Z-main/src/base/ims/DBD/CUSTOMER.asm)、[IBTRAN PSB](../../reference/Bank-of-Z-main/src/base/ims/PSB/IBTRAN.asm) | DL/I を使う COBOL 11 本と [IBLOGIN.pli](../../reference/Bank-of-Z-main/src/base/ims/pli/IBLOGIN.pli)。`CBLTDLI`／`PLITDLI` の `GU/GN/GHU/GHN/ISRT/REPL`、入出力 PCB を使う。 |
| [CardDemo の IMS](../../reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/) | DBD 4 本、PSB 4 本。[DBPAUTP0](../../reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/DBPAUTP0.dbd) は HIDAM の親 `PAUTSUM0`／子 `PAUTDTL1`、[DBPAUTX0](../../reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/DBPAUTX0.dbd) は二次索引。ほかに GSAM 2 本。 | IMS を使う COBOL 7 本。`EXEC DLI` の `GU/GN/GNP/ISRT/REPL/DLET/CHKP/SCHD/TERM` と `CBLTDLI` の `GN/GNP/ISRT/GU`。[CBPAUP0J.jcl](../../reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/jcl/CBPAUP0J.jcl) 等が `DFSRRC00 PARM='BMP,program,PSB'` で起動する。 |

この件数はファイルと呼び出しの初期棚卸しであり、静的な関数名だけを数えたものではない。たとえば Bank of Z の `NEXT-CALL` のような動的な DL/I 関数も実行時の値として監査する。CardDemo の `SCHD/TERM` と Bank of Z の入出力 PCB は IMS TM との接点であり、DB コアの階層操作とは分けて実装する。

## 構成

```text
COBOL CBLTDLI / EXEC DLI       PL/I PLITDLI       jclsh DFSRRC00
         │                         │                      │
         └──── C/FFI アダプター ────┴──────────────┬───────┘
                                                │ ローカル IPC
                                                ▼
                     imsdbd（Rust、単一ノード・単一の状態管理者）
                       ├─ DBD/PSB カタログと PCB セッション
                       ├─ DL/I 実行器（SSA、位置付け、更新）
                       ├─ ロック管理・デッドロック検出・UOW
                       └─ ページキャッシュ・索引・コミットログ・回復
                                                │
                                     バージョン付きローカル DB ファイル
```

`extras/imsdb/` に Rust ライブラリと `imsdbd`、`imsdbctl` を追加する。別プロセスの COBOL ワーカーとバッチが同じロック・キャッシュ・コミット順序を見るため、データファイルを直接開くのは `imsdbd` だけにする。Linux/WSL では Unix ドメインソケット、Windows では名前付きパイプを使い、長さ付きのバイナリ要求でセグメント本文をコピーする。外部に公開するネットワーク API は設けない。初期のビルドと試験は WSL を基準にし、Windows の IPC は同一プロトコルで後段に追加する。

格納エンジンは Rust で実装する。[redb の書き込みトランザクションは同時に 1 個](https://docs.rs/redb/latest/redb/struct.Database.html)という制約があるため、独立したルートへの並行更新を性能ゲートにする本案の中核には採用しない。ページ管理・索引・ログはモジュール境界を分け、段階 0 のベンチマークでデータ量とボトルネックを先に測る。

### 1. DBD、PSB、データ表現

- DBD／PSB の 80 桁形式、コメント、継続列、`DBD`・`DATASET`・`SEGM`・`FIELD`・`LCHILD`・`PCB`・`SENSEG`・`PSBGEN` を解析して、出典行番号付きのバージョン付きカタログへ変換する。`PROCOPT`、セグメント感度、`KEYLEN`、親子、順序キー、二次索引、GSAM DD を検証する。未知の意味を持つ句を黙って捨てず、対象 DBD と行番号を示して停止する。IBM の [DBD 定義](https://www.ibm.com/docs/en/ims/15.5.0?topic=statements-dbd)と [PSB のプログラムビュー](https://www.ibm.com/docs/en/ims/15.5.0?topic=database-your-programs-view-data)を判定基準にする。
- レコード本文とキーは**バイト列のまま**保存する。CP1047、日本語 EBCDIC、パック十進、COMP-5 を UTF-8 に一律変換しない。DBD の `START/BYTES/TYPE` はキー抽出・長さ検証・比較に用いる。SSA の値比較は定義済みフィールドのバイト範囲で行い、最初は資材で使う等価条件から実装する。[SSA の形式](https://www.ibm.com/docs/en/ims/15.5.0?topic=db-segment-search-arguments-ssas)に従い、非修飾 SSA、修飾 SSA、複数レベル指定、必要なコマンドコードを個別にテストする。
- 物理レコードに不変のセグメント ID、DBD ID、親 ID、兄弟順序、本文を持たせる。一次索引はルートキーと階層パス、子索引は親 ID と順序キー、二次索引は索引キーから対象セグメント ID への参照とする。HIDAM のルート順序と親子走査を保持する。HDAM ではローカルの決定的な走査順序を定義するが、IBM の `DFSHDC40` による物理配置順と同一とは主張しない。
- 既存の [IMS LoadData](../../reference/Bank-of-Z-main/src/base/ims/LoadData/) と CardDemo の EBCDIC データは、専用 `imsdbctl load/export` で明示的なレコード長・コードページ・DBD に従って投入／書き出す。IBM OSAM/VSAM データセットをファイル名だけ変えて直接開かない。投入前に長さ・キー重複・親欠落を全件検査し、失敗時は未コミットのまま戻す。

### 2. DL/I と PCB の互換インターフェース

- Rust の DL/I 実行器は `GU/GN/GNP`、各 Hold 版、`ISRT/REPL/DLET` を実装する。PCB ごとの現在位置、親位置、直前の Hold 対象、SSA 条件を保持する。`REPL/DLET` は有効な Hold と対象 ID が一致する場合のみ許可し、別の取得呼び出しや UOW 終了後の古い Hold を拒否する。[IBM の GN/GHN 規則](https://www.ibm.com/docs/en/ims/15.5.0?topic=management-gnghn-call)を基準とする。
- PCB の DBD 名、セグメントレベル・名前、状態コード、処理オプション、キー・フィードバックを、COBOL/PL/I 側の実際の領域長とバイト配置に従って返す。正常は空白 2 バイト、未発見／終端、重複、SSA エラー、更新権限違反、ロック待ち失敗などは IBM の [DL/I 状態コード表](https://www.ibm.com/docs/en/ims/15.5.0?topic=codes-database-calls-dli-status)と照合し、関数ごとの表とテストで固定する。「失敗なら一律 GE」はしない。
- `CBLTDLI` は GnuCOBOL の CALL 引数個数と領域長を検証する C ABI 薄層にする。SSA 個数を確かめず可変長引数を読み進めない。`PLITDLI` は第 1 引数の個数を解釈する。`EXEC DLI` は既存の CICS/SQL 前処理と同様、PCB 番号・SSA・入出力領域・DIBSTAT を保持する明示的な呼び出しへ変換する。コンパイル成功と業務動作成功を別々に記録する。
- `jclsh` の `DFSRRC00` アダプターは `BMP`／`DLI` モード、プログラム名、PSB 名、DD を解析し、登録済みプログラムだけを起動する。`GSAM` は固定長逐次ファイルの順次取得・挿入と RSA を別の PCB 種別として扱う。`CHKP` は DB UOW の確定点として実装し、`XRST` による保存領域の再開は使用資材を監査して別段階で判定する。[チェックポイントの意味](https://www.ibm.com/docs/en/ims/15.5.0?topic=restart-introducing-checkpoint-calls)を参照する。
- Bank of Z の入出力 PCB の `GU/ISRT` は `cobcicsd` の既存トランザクション入出力へ接続する。CardDemo の `SCHD/TERM` はスケジュール状態を管理するアダプターへ渡す。これらをデータベースセグメントとして保存しない。

### 3. ロック、トランザクション、永続性

- セッションは UOW を持ち、同じ UOW 内の ACCOUNT 更新と HISTORY 挿入を一括コミット／ロールバックできる。`CHKP`、CICS 側の同期点、バッチの正常終了を確定境界とし、異常終了・接続断は未確定変更を破棄する。Db2・MQ・VSAM との横断的な原子コミットは初期範囲に含めず、連携が必要な業務経路と残る非原子性をテストで明示する。
- ロック管理者は DBD/ルート ID を基準に共有・排他ロックを管理し、異なるルートの更新を並行させる。普通の取得は短い共有ロック、Hold 取得と更新は排他ロックを使い、`PROCOPT` とセグメント感度を先に検査する。IBM の [処理オプションとロックの説明](https://www.ibm.com/docs/en/ims/15.5.0?topic=security-preventing-program-from-updating-data-processing-options)に照らして、解放時点を呼び出し種別・ルート移動・UOW 終了ごとに決める。ホットなルートで競合が支配的なら、親の意図ロックと子セグメントロックへ細分化する。
- ロックは一定順序で要求する。共有から排他への昇格、挿入先のキー範囲、二次索引更新も同じ待機グラフに入れる。待機グラフの循環でデッドロックを検出し、被害 UOW をロールバックする。待機上限を設定でき、無限待ちにしない。PCB 状態・診断ログには「権限」「未発見」「タイムアウト／デッドロック」を区別して残す。
- データファイルはバージョン付きページ、スロット付きレコード、B+tree 索引、上限付きページキャッシュを採用する。変更は UOW の私有書き込み集合に置き、**コミットログの確定記録が永続化されるまで未確定ページを書き戻さない**。複数 UOW のログ fsync をまとめる group commit を用い、起動時は確定済みログだけ再適用する。これは単一ノードの一貫性・耐久性のためであり、複製やフェイルオーバーではない。DBD カタログとデータのバージョン不一致は起動時に拒否する。

### 4. 性能を満たすための設計とゲート

- `GU` の主キー・二次索引検索に全表走査を使わない。`GN/GNP` は PCB ごとの索引カーソルを保持し、毎回先頭から探さない。子の連続配置、ページキャッシュ、read-ahead、バルクロード時の索引一括構築を実装する。更新は本文・一次・子・二次索引を 1 UOW で確定する。
- ベンチマークは WSL の Linux ファイルシステム上で CPU・RAM・ストレージ・Rust ビルド設定を固定し、固定 seed の「10 万ルート／100 万子／平均 256 バイト」と参照資材の実データを使う。ホスト側の `/mnt/d` と WSL ext4 は混同しない。単一スレッドと 8 セッションで `GU`、修飾 `GNP`、`GHU→REPL`、別ルート同時更新、同じルート競合を測り、p50/p95/p99、処理件数/秒、ロック待機時間、キャッシュヒット率、ログ fsync、ファイル増加率を記録する。
- 初期受け入れゲートは温キャッシュの `GU` p95 5 ms 以下、同一親の `GNP` 3 万セグメント/秒以上、独立ルート 8 セッションの更新スループットが単一セッションの 3 倍以上とする。測定環境を結果とともに保存する。以後、同じ環境で既承認版から p95 または処理件数が 20% を超えて悪化した変更を性能回帰として止める。ゲートを満たせなければ索引・ロック粒度・ログ書き込みをプロファイルで特定してから次段階へ進む。

## 実装順序と完了条件

| 段階 | 作業 | 完了条件 |
| --- | --- | --- |
| 0. 監査と仕様固定 | 9+4 DBD、8+4 PSB、全 DL/I 呼び出し、SSA、PCB 領域、JCL PARM、ロードデータを機械的に一覧化。 | 対象・未対応句・想定状態コードを出典位置付きで再生成できる。 |
| 1. メタデータと格納 | DBD/PSB パーサー、カタログ、バイト保持、一次・子索引、投入／書き出し。 | Bank の単一ルートと CardDemo の親子・二次索引・GSAM 定義を解析し、取り込み後のレコード数・キー・本文ハッシュが一致する。 |
| 2. DL/I 読み取り | PCB、SSA、`GU/GN/GNP` と Hold 版、状態・キーのフィードバック。 | 親子移動、未発見、終端、複数 PCB、EBCDIC／packed key を自動試験で確認する。 |
| 3. 更新と競合 | `ISRT/REPL/DLET`、UOW、ロック、デッドロック、ログと回復。 | 競合する 2～8 セッションで Lost Update がなく、デッドロック被害側だけが戻り、強制終了後も確定済み UOW と全索引が一致する。 |
| 4. 実行連携 | `CBLTDLI`、`PLITDLI`、`EXEC DLI`、`cobcicsd` 入出力 PCB、`jclsh DFSRRC00`、GSAM。 | Bank の顧客照会・更新・取引、CardDemo の認可照会・更新・削除・ロード／アンロードを元ソース／JCLから実行する。 |
| 5. 性能・文書 | 上記の性能ゲート、利用ガイド、失敗時診断と回帰シナリオ。 | ゲートを満たし、`docs/testcases/` にデータ前後差・状態コード・ロック競合・クラッシュ試験の再実行手順がある。 |

段階 0 で見つかった資材固有の機能は表に追加し、未対応のまま成功扱いしない。特に二次索引経由の検索、GSAM の RSA、`CHKP`、`SCHD/TERM`、入出力 PCB は「DB コア完成」と「参照アプリ完走」を分ける判定項目にする。IBM IMS 実機との比較環境は前提にせず、IBM の公開仕様、最小の状態遷移試験、参照アプリの業務結果で互換性を検証する。
