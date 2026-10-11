use imsdb::call_adapter::{CallAdapter, DliFunction, PcbEncoding, PcbMask};
use imsdb::catalog::{parse_dbd, parse_psb};
use imsdb::dli::Status;
use imsdb::Database;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DBD: &str = " DBD NAME=TEST,ENCODING=Cp1047,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n DBDGEN\n END\n";
const PSB: &str = " PCB TYPE=DB,DBDNAME=TEST,PROCOPT=AP,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0\n PSBGEN PSBNAME=TESTPSB,LANG=COBOL\n END\n";
const PRIMARY_INDEXED: &str = " DBD NAME=PRIMARY,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n FIELD NAME=ALT,START=3,BYTES=2,TYPE=C\n LCHILD NAME=(PTR,SECOND),POINTER=INDX\n XDFLD NAME=XALT,SRCH=ALT\n DBDGEN\n END\n";
const SECONDARY: &str = " DBD NAME=SECOND,ACCESS=(INDEX,VSAM)\n SEGM NAME=PTR,PARENT=0,BYTES=2\n FIELD NAME=(ALTKEY,SEQ,U),START=1,BYTES=2,TYPE=C\n LCHILD NAME=(ROOT,PRIMARY),INDEX=XALT\n DBDGEN\n END\n";
const INDEX_PSB: &str = " PCB TYPE=DB,DBDNAME=PRIMARY,PROCOPT=G,PROCSEQ=SECOND,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0\n PSBGEN PSBNAME=IDXPSB,LANG=COBOL\n END\n";
const SENSEG_INDEX_PSB: &str = " PCB TYPE=DB,DBDNAME=PRIMARY,PROCOPT=G,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0,INDICES=SECOND\n PSBGEN PSBNAME=PHYPSB,LANG=COBOL\n END\n";

#[test]
fn senseg_indices_decodes_ebcdic_ssa_and_keeps_physical_feedback_key() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let wal =
        std::env::temp_dir().join(format!("imsdb-indices-{}-{stamp}.wal", std::process::id()));
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(PRIMARY_INDEXED).unwrap(),
            parse_dbd(SECONDARY).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    seed.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![1, 0x0c, 0xc1, 0xc2, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut adapter = CallAdapter::new(
        db.clone(),
        parse_psb(SENSEG_INDEX_PSB).unwrap(),
        PcbEncoding::Cp1047,
    )
    .unwrap();
    let mut pcb = vec![0; 38];
    let mut io = vec![0; 6];
    let mut ssa = PcbEncoding::Cp1047
        .encode_control(b"ROOT    (XALT    EQ")
        .unwrap();
    ssa.extend_from_slice(&[0xc1, 0xc2]);
    ssa.extend(PcbEncoding::Cp1047.encode_control(b")").unwrap());
    assert_eq!(
        adapter
            .call_encoded(
                0,
                &PcbEncoding::Cp1047.encode_control(b"GU  ").unwrap(),
                &mut pcb,
                &mut io,
                &[&ssa]
            )
            .unwrap(),
        Status::Ok
    );
    assert_eq!(&io[2..4], &[0xc1, 0xc2]);
    assert_eq!(&pcb[36..38], &[1, 0x0c]);
    drop(adapter);
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn procseq_call_boundary_accepts_ebcdic_ssa_controls_and_raw_search_bytes() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let wal =
        std::env::temp_dir().join(format!("imsdb-procseq-{}-{stamp}.wal", std::process::id()));
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(PRIMARY_INDEXED).unwrap(),
            parse_dbd(SECONDARY).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    seed.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![1, 0x0c, 0xc1, 0xc2, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut adapter = CallAdapter::new(
        db.clone(),
        parse_psb(INDEX_PSB).unwrap(),
        PcbEncoding::Cp1047,
    )
    .unwrap();
    let mut pcb = vec![0; 38];
    let mut io = vec![0; 6];
    let mut ssa = PcbEncoding::Cp1047
        .encode_control(b"ROOT    (XALT    EQ")
        .unwrap();
    ssa.extend_from_slice(&[0xc1, 0xc2]);
    ssa.extend(PcbEncoding::Cp1047.encode_control(b")").unwrap());
    assert_eq!(
        adapter
            .call_encoded(
                0,
                &PcbEncoding::Cp1047.encode_control(b"GU  ").unwrap(),
                &mut pcb,
                &mut io,
                &[&ssa]
            )
            .unwrap(),
        Status::Ok
    );
    assert_eq!(&io[2..4], &[0xc1, 0xc2]);
    assert_eq!(&pcb[36..38], &[0xc1, 0xc2]);
    drop(adapter);
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn call_boundary_writes_pcb_feedback_and_preserves_raw_segment_bytes() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let wal = std::env::temp_dir().join(format!("imsdb-call-{}-{stamp}.wal", std::process::id()));
    let dbd = parse_dbd(DBD).unwrap();
    let psb = parse_psb(PSB).unwrap();
    let database = Database::open(&wal, vec![dbd.clone()]).unwrap();
    let mut seed = database.begin();
    seed.insert(
        "TEST",
        "ROOT",
        None,
        vec![0xc1, 0x40, 0x0e, 0x41, 0x0f, 0xff],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();

    let mut ascii = CallAdapter::new(database.clone(), psb.clone(), PcbEncoding::Ascii).unwrap();
    let mut pcb = vec![0; 38];
    let mut io = vec![0xee; 7];
    let ssa = b"ROOT    (ID      EQ\xc1\x40)";
    assert_eq!(
        ascii
            .call(0, DliFunction::Gu, &mut pcb, &mut io, &[ssa])
            .unwrap(),
        Status::Ok
    );
    assert_eq!(&io[..6], &[0xc1, 0x40, 0x0e, 0x41, 0x0f, 0xff]);
    assert_eq!(io[6], 0xee);
    assert_eq!(&pcb[0..16], b"TEST    01  AP  ");
    assert_eq!(&pcb[20..28], b"ROOT    ");
    assert_eq!(&pcb[28..32], &2u32.to_be_bytes());
    assert_eq!(&pcb[32..36], &1u32.to_be_bytes());
    assert_eq!(&pcb[36..38], &[0xc1, 0x40]);
    assert_eq!(
        ascii
            .call(
                0,
                DliFunction::Gu,
                &mut pcb,
                &mut io,
                &[b"ROOT    (ID      EQ\x09\x09)"]
            )
            .unwrap(),
        Status::NotFound
    );
    assert_eq!(&pcb[10..12], b"GE");
    let mut short_io = [0x55; 5];
    assert!(ascii
        .call(0, DliFunction::Gu, &mut pcb, &mut short_io, &[ssa])
        .is_err());
    assert_eq!(short_io, [0x55; 5]);

    let mut ebcdic = CallAdapter::new(database.clone(), psb, PcbEncoding::Cp1047).unwrap();
    let mut ebcdic_pcb = vec![0; 38];
    let mut record = [0u8; 6];
    let mut ebcdic_ssa = PcbEncoding::Cp1047
        .encode_control(b"ROOT    (ID      EQ")
        .unwrap();
    ebcdic_ssa.extend_from_slice(&[0xc1, 0x40]);
    ebcdic_ssa.extend(PcbEncoding::Cp1047.encode_control(b")").unwrap());
    let ebcdic_ghu = PcbEncoding::Cp1047.encode_control(b"GHU ").unwrap();
    assert_eq!(
        ebcdic
            .call_encoded(0, &ebcdic_ghu, &mut ebcdic_pcb, &mut record, &[&ebcdic_ssa])
            .unwrap(),
        Status::Ok
    );
    assert_eq!(&ebcdic_pcb[0..4], &[0xe3, 0xc5, 0xe2, 0xe3]); // TEST
    assert_eq!(&ebcdic_pcb[8..10], &[0xf0, 0xf1]); // 01
    assert_eq!(&ebcdic_pcb[10..12], &[0x40, 0x40]); // blank status
    record[5] = 0x42;
    let ebcdic_repl = PcbEncoding::Cp1047.encode_control(b"REPL").unwrap();
    assert_eq!(
        ebcdic
            .call_encoded(0, &ebcdic_repl, &mut ebcdic_pcb, &mut record, &[])
            .unwrap(),
        Status::Ok
    );
    ebcdic.commit().unwrap();
    assert_eq!(
        database
            .get("TEST", None, "ROOT", &[0xc1, 0x40])
            .unwrap()
            .data[5],
        0x42
    );
    assert!(PcbMask::new(
        &mut [0u8; 37],
        &dbd,
        &parse_psb(PSB).unwrap().pcbs[0],
        PcbEncoding::Ascii
    )
    .is_err());
    drop(ascii);
    drop(ebcdic);
    drop(database);
    std::fs::remove_file(wal).unwrap();
}
