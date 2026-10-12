# IMS TM ローカルキューの回帰シナリオ

実行環境: WSL/Linux、ビルド済み `build/imsdb-linux/debug/libimsdb.so` と `imsdbctl`、本リポジトリの `cobc`／`libcob`。キューは各スクリプトが作る一時ディレクトリを使用する。

| シナリオ | 操作 | 期待結果 |
| --- | --- | --- |
| Rustキュー永続化 | 投入、貸出し、取消、再貸出し、確認、再オープン | 入力の消費と応答の保存が復元後も一致 |
| 並行投入 | 4スレッド×10件 | 40件とID重複なし |
| 受信者クラッシュ | 子プロセスが貸出し後に終了 | OSロック解放後、別の受信者が直ちに再取得 |
| 生バイト | EBCDIC DBCSシフト、NUL、FFを含む本文 | バイト単位で一致 |
| キューファイル破損 | 保存済みメッセージの1バイトを改変 | チェックサム不一致で読出しを拒否 |
| Bank of Z原本 | `IBSCUDAT` に顧客更新メッセージを投入 | 原本COBOLがDBを更新し、送信元 LTERM に応答を1件送る |
| 作業単位取消 | 非ゼロRCのCOBOL出力 | 応答はキューに残らない |
| 代替PCB | `MODIFY=YES` の `CHNG`→`ISRT` | 変更先 LTERM に出力 |
| TP PCB定義 | PSBの `LTERM`／`MODIFY` | 固定宛先を解釈し、宛先も変更権もない定義を拒否 |
| IBM-037 | EBCDICコンパイルした代替PCBプログラム | 宛先はASCIIで管理、本文はEBCDICバイトを保持 |
| PCB順 | 2つのDB PCBを持つPSB | COBOLリンク欄にPSB順のDBD名 |
| 複数セグメント | 代替PCBへ2回 `ISRT`→`PURG`→宛先変更、I/O PCBで `GU`→`GN` | 前者は1メッセージに2セグメント、後者は2セグメントを順に取得し最後は `QD` |
| COBOL受信取消 | `GU`／`GN` 後に非ゼロRCで終了し、別プロセスで再実行 | 入力が残り、再実行で同じ2セグメントを取得して正常終了時に確認済みになる |
| 次の `GU` の同期点 | 2件を順に `GU` し、2件目の後で非ゼロRC終了 | 1件目は確認済み、2件目だけ再配送可能 |

実行コマンド:

```sh
cargo test --manifest-path extras/imsdb/Cargo.toml --offline --test tm
sh extras/imsdb/test-bank-original.sh
sh extras/imsdb/test-tm-alt.sh
```

2026-10-12 に上記のRustテスト8件をWindowsとLinuxで、Bank原本のキュー試験と代替PCB／複数PCB／IBM-037／複数セグメント試験をWSLで実行し、すべて成功した。既存の CardDemo ロード・アンロード・`EXEC DLI` とGSAM試験も実行し、PSB順ランチャー変更による回帰がないことを確認した。
