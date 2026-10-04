# Rust 製 JCL インタプリタ兼シェルの実装提案

## 目的と互換性の定義

`jclsh`（仮称）を Rust で実装し、z/OS の JCL を **元ファイルを編集せず**、対話シェル、ファイル指定、標準入力のいずれからも投入できるようにする。JCL の解析だけで終わらせず、ジョブステップの条件判定、DD 割り当て、プログラム起動、戻りコード、SYSOUT の取得までを一つの実行系として扱う。

「書き換えなし」は、JCL の `JOB` / `EXEC` / `DD` / プロシージャー / インストリームデータを変更しないことを指す。ホスト上のデータセット、プログラム、Db2 接続先、サンプルに含まれる `<DB2SSID>` などのサイト依存プレースホルダーは**外部プロファイル**で解決する。解決値と変換結果は実行記録に残す。対応していないユーティリティーや z/OS 機能はジョブを明示的に失敗させ、成功したことにはしない。

この提案は JCL インタプリタの計画であり、全 z/OS サブシステムの再実装を意味しない。JCL の記述を受理できることと、指定された `PGM` の業務結果まで再現できることを別々に検証する。

## 現在の資材から見える要件

2026-10-04 時点の作業ツリーでは、`reference/Bank-of-Z-main/src/base/batch/jcl` に 1 本、`reference/cics-genapp/base/cntl` に 29 本の JCL がある。後者には `IKJEFT01`、`IDCAMS`、`DFHCSDUP`、`IEBGENER`、`ASMA90`、`IGYCRCTL`、`HEWL` / `IEWL` のほか、CICS 領域や CICSPlex の運用プログラムが現れる。数値は現時点の資材の棚卸しであり、別の JCL 集合を対象にした対応保証ではない。

| 実物 | 必要な機能 | 実行上の注意 |
| --- | --- | --- |
| [`BNKSTMT.jcl`](../../reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl) | `JOB`、`EXEC PGM=IKJEFT01`、連結 `STEPLIB`、`DD *` の `DATECARD` / `SORTCODE` / `SYSTSIN`、`SYSOUT=*`、`DSN SYSTEM` と `RUN PROGRAM(BNKSTMT)` | 呼び出される [`BNKSTMT.pli`](../../reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli) は **PL/I**。[PL/I → C コンパイラ](pli-to-c-compiler.md)による実行ファイルがなければ帳票は完成しない。 |
| [`cobol.jcl`](../../reference/cics-genapp/base/cntl/cobol.jcl) | インストリーム `PROC` / `PEND`、`EXEC DB2PROC,MEM=...`、プロシージャー内ステップ、`&MEM`、PDS メンバー、`&&LOADSET` / `&&COPYLINK`、`DISP=(NEW,PASS)`、`COND=(7,LT,COBL)` | `IGYCRCTL` → `IEBGENER` → `HEWL` を既存の CICS → SQL → GnuCOBOL ビルド経路へ結ぶ。各 `MEM` の成果物と戻りコードを独立に記録する。 |
| [`asmmap.jcl`](../../reference/cics-genapp/base/cntl/asmmap.jcl) | `IEBGENER`、`ASMA90` の `SYSPARM(MAP)` / `SYSPARM(DSECT)`、一時データセット、`IEWL` | BMS の物理マップとコピー句を既存の Rust `cobcics-mapgen` 相当で生成する。一般の HLASM を実装したことにはしない。 |
| [`db2cre.jcl`](../../reference/cics-genapp/base/cntl/db2cre.jcl)、[`db2bind.jcl`](../../reference/cics-genapp/base/cntl/db2bind.jcl) | 複数 `IKJEFT01` ステップ、`SYSTSIN` の DSN コマンド、`SYSIN` の DDL / DML、`COND`、`<DB2DBID>` 等の置換 | `DSNTIAD` の SQL を Db2 LUW に渡す際、z/OS 固有の `STOGROUP` / `TABLESPACE` などは意味を確認して変換する。`BIND` を無条件の成功扱いにしない。 |
| [`adef121.jcl`](../../reference/cics-genapp/base/cntl/adef121.jcl) | `IDCAMS` の `DELETE` / `DEFINE CLUSTER` / `REPRO`、DD 名による入出力、条件付きステップ | `KSDSCUST` / `KSDSPOLY` のキー・レコード長を、既存 GenApp の Db2 裏付けファイル表と突き合わせる。 |
| `cdef*.jcl`、`samp*.jcl`、`wsa*.jcl` | `DFHCSDUP`、CICS 領域起動、CICSPlex、Web サービス定義 | COBOL バッチジョブとは別の運用資源。まずジョブを正しく解析・分類し、対応する `cobcicsd` 設定へ変換できる部分だけ実行する。CICSPlex や Web サービス全体を動かせない場合は未対応箇所を報告する。 |

## 利用形態

```text
jclsh                         # 対話モード
jclsh run path/to/JOB.jcl --profile local.toml
jclsh lint path/to/JOB.jcl
jclsh plan path/to/JOB.jcl --profile local.toml
jclsh run - --profile local.toml < path/to/JOB.jcl   # Bash など
Get-Content -Raw path/to/JOB.jcl | jclsh run - --profile local.toml  # PowerShell
```

対話モードには `submit <file>`、`plan <file>`、`jobs`、`status <job-id>`、`spool <job-id> [ddname]`、`datasets <pattern>`、`set <symbol> <value>`、`help`、`exit` を設ける。投入後の実行は初期段階では逐次・同期とし、ジョブ ID とスプールは保存する。バックグラウンド実行は状態永続化とジョブ停止・再開の設計を確定してから追加する。JCL の先頭は `//` なので、シェバン行を追加させる方式は採らない。パイプや標準入力でそのまま渡せる形をシェルとしての基本とする。

`lint` は構文と未対応項目の位置を示す。`plan` はプロシージャー展開、シンボル置換、条件、DD の解決先と必要なプログラムアダプターを表示し、副作用を起こさない。`run` は同じ計画に沿って実行し、各ステップの RC / ABEND と SYSOUT を表示・保存する。CLI の終了コードはシェル利用向けに 0／非 0 とし、z/OS のステップ RC 全値および ABEND は機械可読な結果ファイルとスプールで保持する。

実装は `extras/jclsh/` の Rust クレートを想定し、字句解析、展開、実行計画、データセット台帳、スプール、アダプターをモジュールで分ける。BMS と EBCDIC の既存コードは安定した共通ライブラリーに切り出せるか確認し、端末サーバーの内部状態に JCL 実行を依存させない。

## 構成と実装方針

```text
JCL ファイル / stdin / REPL
        │
        ▼
字句解析・継続行・インストリームデータ
        ▼
AST と出典位置 → PROC 展開・&symbol / サンプル変数の解決
        ▼
実行計画 → COND / IF 判定 → DD 割り当て・データセット台帳
        ▼
PGM アダプター → COBOL / Db2 / ユーティリティー / CICS 設定
        ▼
ステップ結果・SYSOUT スプール・監査記録
```

### 1. JCL の解析と展開

- レコードの列規則、`//`・`//*`・`/*`・`//` の終端、列 73–80 の識別欄、継続行、引用符、括弧を扱う。カンマや `//` を含む `DD DATA` の本文を JCL と誤認しない。`DD *`、`DD DATA,DLM=...`、連結 DD を別ノードで保持する。[IBM の JCL 文フィールド定義](https://www.ibm.com/docs/en/zos/3.2.0?topic=statements-jcl-statement-fields)を字句規則の基準とする。
- `JOB`、`EXEC PGM=` / `EXEC PROC=` / `EXEC procname`、`DD`、`PROC` / `PEND`、`SET`、`JCLLIB` / `INCLUDE`、`IF` / `THEN` / `ELSE` / `ENDIF` を構文木へ変換する。プロシージャー内の `stepname.ddname` 上書きと、`&MEM` などのスコープ・優先順位を定義する。プロシージャー検索はインストリーム、プロファイルに登録したライブラリーの順。[IBM のプロシージャー規則](https://www.ibm.com/docs/en/zos/3.2.0?topic=procedures-using-procedure)を照合する。
- JCL の `&NAME` / `&&NAME` を区別する。前者は記号、後者はジョブ内一時データセットであり、単純な文字列置換では扱わない。サンプル固有の `<NAME>` は **IBM JCL 構文ではない**ため、明示的に登録したプレースホルダーのみを字句位置に応じて置換する。`SYSIN` の SQL やユーティリティー制御文中にも現れるので、対象 DD と置換範囲をプロファイルで指定する。値不足や循環参照は実行前エラーにする。[IBM のシンボル規則](https://www.ibm.com/docs/en/zos/3.2.0?topic=symbols-coding-in-jcl)。
- すべての展開結果に元ファイル・行番号と `PROC` 呼び出し履歴を付ける。未知の文・パラメーターは位置と代替方法を報告し、無視して続行しない。

### 2. ジョブの制御とデータセット

- ステップごとに RC、ABEND、未実行理由を保持する。`COND` は**条件が真ならスキップ**という JCL の意味で実装し、`COND=(7,LT,COBL)` の参照先やプロシージャー内ステップを区別する。`IF` 構文、`EVEN` / `ONLY`、異常終了後の処理も別々に検証する。[IBM の戻りコードによるステップ制御](https://www.ibm.com/docs/en/zos/3.2.0?topic=execution-bypassing-executing-steps-based-return-codes)。
- `db2cre.jcl` には `CRTABS` という同名ステップが複数ある。内部では展開順とプロシージャー呼び出しを含む一意な ID を持たせ、同名ステップを参照する条件が曖昧な場合は診断する。z/OS 側でも同名ステップの条件参照結果が不定になり得るため、任意に後のステップへ結び付けない。[IBM の COND ステップ名の説明](https://www.ibm.com/docs/en/zos/3.1.0?topic=cp-subparameter-definition-6)。
- `CLASS`、`MSGCLASS`、`MSGLEVEL`、`NOTIFY` はジョブ記録とスプールの属性として保持し、`REGION` / `MEMLIMIT` / `TIME` はホストで再現できる制限だけに対応付ける。`DYNAMNBR` は `IKJEFT01` アダプターの入力として検証する。`JOBLIB` / `STEPLIB` は登録済みプログラムの探索候補として扱い、z/OS ロードモジュールを直接起動しない。これらの指定を黙って捨てず、`plan` に反映先を表示する。
- DD ごとに `DSN` / `DSNAME`、PDS メンバー、`DISP=(status,normal,abnormal)`、`UNIT`、`SPACE`、`DCB`、`SYSOUT`、`DUMMY`、連結を構造化する。`&&` はジョブごとの一時領域、`PASS` は後続ステップへの引き渡しとして扱い、`NEW` / `OLD` / `SHR` / `MOD` と終了時の `CATLG` / `KEEP` / `DELETE` を台帳の状態遷移として管理する。[IBM の JCL User's Guide](https://www.ibm.com/docs/SSLTBW_3.2.0/pdf/ieab500_v3r2.pdf)。
- 外部プロファイルで、z/OS データセット名を許可されたホストファイル、PDS 相当のメンバーディレクトリー、Db2 裏付けのキー付きデータ、仮想データセットに対応付ける。DD の割り当て結果を子プロセスに渡す ABI は、小さな COBOL 検証プログラムで GnuCOBOL のファイル名解決を確認して固定する。`DDNAME` を単にパス名へ置換するだけで済むとは仮定しない。
- `RECFM=FB/VB`、`LRECL`、`BLKSIZE` はレコード境界と長さの検証に使う。EBCDIC/UTF-8 変換は DD ごとの宣言と COBOL の `-fdata-codeset` に沿って行い、インストリーム SQL・バイナリレコードを一律に変換しない。SYSOUT は DD 名、ジョブ ID、ステップ名を持つスプールに保存する。

### 3. プログラムアダプター

`EXEC PGM=` の名前を信頼済みのアダプター登録表で解決し、任意の文字列をホストのシェルへ直接渡さない。アダプターは `DD`、`PARM`、コードページ、作業ディレクトリー、Db2 接続情報を受け、RC / ABEND・出力 DD・成果データセットを返す。初期実装は別プロセス実行とし、Rust と `libcob` の直接結合は必要性が確認できてから検討する。

| 対象 | 最初に実装する意味 | 限界をどう示すか |
| --- | --- | --- |
| GnuCOBOL の業務プログラム | 登録済み `PGM` をビルド成果物から起動し、DD と `PARM` を渡す。CICS/Db2 が必要なら既存の前処理・ブリッジを使用する。 | 未登録 `PGM` や不足 DD は JCL 実行エラー。 |
| `IKJEFT01` の `DSN` | `SYSTSIN` の `DSN SYSTEM`、`RUN PROGRAM`、`END` を解析する。`BNKSTMT` の `RUN` は登録済みプログラムへ委譲する。[IBM の DSN RUN](https://www.ibm.com/docs/en/db2-for-zos/13.0.0?topic=commands-run-dsn)。 | DSN コマンド全般は実装しない。未対応サブコマンドは停止。 |
| `DSNTIAD` と `BIND` | `SYSIN` の SQL を既存 Compose の Db2 LUW で順次実行し、文ごとの結果と RC を残す。z/OS 固有 DDL は明示的な変換規則で扱う。`BIND` は参照メンバー・登録済み SQL 成果物・実行時設定を検証する。[IBM の DSNTIAD](https://www.ibm.com/docs/en/db2-for-zos/13.0.0?topic=programs-dsntiad-sample-program)。 | LUW で同等にできない SQL や、未存在 DBRM は失敗。Db2 for z/OS のバインド自体を行ったとは記録しない。 |
| `IDCAMS` | `DEFINE CLUSTER` / `DELETE` / `REPRO` を、キーとレコード長を持つローカルカタログへ対応付ける。`adef121.jcl` の KSDS は既存 GenApp Db2 表を使う。[IBM の DEFINE CLUSTER](https://www.ibm.com/docs/en/zos/3.2.0?topic=access-defining-attributes-clusters-cluster-components)、[REPRO](https://www.ibm.com/docs/en/zos/3.2.0?topic=commands-repro)。 | カタログ・VSAM の全属性、テープ等は未対応として診断する。 |
| `IEBGENER` | `SYSUT1` から `SYSUT2` へレコードをコピーし、`DUMMY` と `SYSIN` の扱いを検証する。[IBM のユーティリティー一覧](https://www.ibm.com/docs/en/zos/3.2.0?topic=utilities-data-management)。 | 一般の PDS 変換・編集機能を自動的に含めない。 |
| `IGYCRCTL`、`HEWL` / `IEWL`、BMS 用 `ASMA90` | `cobol.jcl` と `asmmap.jcl` の制御 DD・`PARM` を読み、既存 CICS/SQL 前処理、GnuCOBOL コンパイル、BMS mapgen、リンクへ結ぶ。 | 任意の HLASM や z/OS オブジェクトをビルドするものではない。実行可能な成果物を生成できないステップは失敗。 |
| `DFHCSDUP` と領域運用プログラム | 使用する資源定義を解析し、`cobcicsd` のトランザクション・マップ・ファイル設定へ変換する経路を後段で追加する。 | `DFHSIP`、CICSPlex、Web サービス等は個別のサービス設計が必要。未実装のまま RC=0 にはしない。 |

Bank of Z の `BNKSTMT` は [PL/I → C コンパイラの提案](pli-to-c-compiler.md)に従って元の `.pli` から実行ファイルを生成し、登録済み `PGM` として起動する。**JCL の無変更実行**と **元 PL/I ソースの無変更コンパイル・実行**は別々の達成条件である。初期実装では両方を Bank of Z の 1 ジョブで検証した。

## 実装順序と受け入れ条件

| 段階 | 作業 | 完了条件 |
| --- | --- | --- |
| 0. 対象監査 | 30 本の JCL を解析し、文種、DD、`PGM`、インストリーム制御文、サイト依存値を出典位置つきで一覧化。 | 各ファイルの解析結果と、未対応構文・未登録プログラムの一覧を再生成できる。 |
| 1. シェルと構文 | Rust CLI/REPL、JCL レコード字句解析、AST、`PROC` 展開、記号解決、`lint` / `plan`。 | `BNKSTMT.jcl`、`cobol.jcl`、`asmmap.jcl` を無編集で計画化でき、展開後の DD/ステップが期待値と一致する。 |
| 2. 実行核 | `COND` / `IF`、RC/ABEND、DD 台帳、`DISP`、一時データセット、SYSOUT、ジョブ記録。 | 小型の複数ステップ JCL で正常・失敗・スキップ・`PASS`/削除が検証できる。 |
| 3. 実用アダプター | COBOL、`IKJEFT01`/`DSNTIAD`、`IDCAMS`、`IEBGENER`、ビルド/マップ用アダプター。 | `adef121.jcl` の KSDS 定義・ロード、`db2cre.jcl` の対象業務表・行、`cobol.jcl` の対象モジュール生成を JCL 起点で実行し、成果物とデータを既存の GenApp テストで照合する。 |
| 4. Bank と運用 JCL | `BNKSTMT` の PL/I 実行経路、CSD 定義変換、残る資材の分類と必要なサービス実装。 | `BNKSTMT.jcl` を無編集で実行し、`DATECARD` / `SORTCODE` の値と帳票、Db2 の参照結果を照合する。運用 JCL は実行可否と未対応理由をファイル・ステップ単位で公開する。 |
| 5. 回帰・文書 | Windows/Linux、CP037/日本語 EBCDIC、空 DD、失敗時の後処理、CLI/REPL のテストケースと利用ガイド。 | 元 JCL のチェックサムが実行前後で同じ。副作用・RC・SYSOUT・DB/KSDS の前後差を自動テストで確認できる。 |

各段階で `docs/testcases` に JCL ごとの期待ステップ順、DD 割り当て、RC、スプール、業務データの検査項目を追加する。既存の [`setup-genapp-business-db.py`](../../extras/cobcics/setup-genapp-business-db.py) やビルドスクリプトは比較対象に使い、JCL 経由の結果が同じか検証する。失敗時にスクリプト側の処理へ黙ってフォールバックさせない。まず逐次実行の結果を固定し、並列ジョブ・JES キュー・スケジューラーは別段階で設計する。

## 主要なリスクと判断点

- **JCL と PGM の責務:** パーサーがすべての文を理解しても、PL/I や CICS 運用プログラムの実行器がなければ対応ジョブは完了しない。互換率は「解析可」「計画可」「実行可」「業務結果検証済み」で別々に示す。
- **データ表現:** 固定長レコード、PDS メンバー、Db2 LUW と z/OS Db2 の DDL 差が大きい。変換規則は JCL と別ファイルに置き、入力 JCL、変換後 SQL、結果を保存する。
- **実行境界:** `IDCAMS DELETE` や `DISP=DELETE` が意図しないホストファイルを消さないよう、データセット台帳の許可済みルート内で解決する。実行計画に削除対象を表示し、範囲外はエラーにする。
- **戻りコード:** OS のプロセス終了値だけでは JCL の RC（0～4095）、ABEND、スキップを表せない。アダプター共通の結果形式を定め、条件判定とジョブログの唯一の入力にする。
- **サンプルのサイト依存値:** `<...>` は JCL 標準の記号ではない。プロファイルにない値を勝手に推測せず、未設定の位置を報告する。元 JCL の修正を要求しない。

基準資料: [IBM z/OS 3.2 MVS JCL Reference](https://www.ibm.com/docs/en/zos/3.2.0?topic=mvs-zos-jcl-reference)、[IBM z/OS MVS JCL User's Guide 3.2](https://www.ibm.com/docs/SSLTBW_3.2.0/pdf/ieab500_v3r2.pdf)、[Db2 for z/OS DSN RUN](https://www.ibm.com/docs/en/db2-for-zos/13.0.0?topic=commands-run-dsn)。

## 初期実装の到達点（2026-10-04）

`extras/jclsh/` に Rust 製 CLI/対話シェルを追加した。`BNKSTMT.jcl` の JOB、EXEC、連結 STEPLIB、インストリーム DD、SYSOUT、`IKJEFT01` の `DSN RUN` を解析し、`lint` / `plan` / `run` と標準入力からの投入に対応した。プログラムはプロファイルで登録したものだけを起動し、ジョブ・ステップ RC と DD 別スプールを保存する。

[通し試験](../testcases/jcl-bnkstmt.md)では、原本 JCL と PL/I 原本から作った実行ファイルを用い、Docker の `PL1TEST` Db2 にある 2 口座の帳票を生成した。DATECARD・SORTCODE、取引の期間選択、残高、RC、スプール、JCL 原本のハッシュ不変を確認した。`STEPLIB` はメタデータ、`PLAN` と `LIB` はプロファイルとの照合であり、z/OS のロードモジュール探索・DBRM バインドを行っていない。

2026-10-04 の追加実装で、複数ステップ、インストリーム `PROC` / `PEND`、PROC パラメーターと `SET` 記号、`step.ddname` 上書き、`COND`、`IF` / `ELSE` / `ENDIF`、`PARM`、`JOBLIB`、`DD DATA,DLM=` を解析・実行経路へ追加した。GenApp の `cobol.jcl` は 93 ステップに展開できる。登録済みの直接 `PGM=` も Docker で起動する。非ゼロ RC の後に `COND` と IF を評価し、スキップを `result.json` に明記する。[複数ステップ試験](../testcases/jcl-multistep.md)は Docker で成功し、既存の Bank of Z 試験も再成功した。

追加実装では、`JCLLIB ORDER` によるカタログド PROC／INCLUDE の探索、入れ子 PROC、サイト依存 `<名前>` の外部設定、同名ステップの一意な記録と曖昧参照の検出を加えた。DD はジョブ専用領域で割り当て、永続データセットの変更を `dataset_root` の内側に制限し、正常／異常終了時の `DISP` を処理する。ABEND はアダプターのコードファイルまたは Docker のシグナル終了から明示的に記録し、`COND=EVEN` / `ONLY` と IF の RC／RUN／ABEND／ABENDCC 判定に渡す。[データセット・ABEND 試験](../testcases/jcl-datasets-abend.md)で実際のステップ間 `PASS`、`CATLG`、`DELETE`、`UNCATLG` と異常時 DELETE を確認した。

GenApp 29 本のうち 20 本は現在の資材だけで構文展開できる。残る 9 本はリポジトリにない IBM 提供ユーティリティー PROC `DFHLS2WS` または `EYUJXBTP` を参照するため、登録ライブラリーがなければメンバー未発見と報告する。未登録の z/OS ユーティリティーの実行、およびそれらの外部 PROC 資材の提供は別作業である。構文展開だけで業務処理の実行成功とは扱わない。利用手順と現在の対応範囲は [jclsh README](../../extras/jclsh/README.md)に記載する。
