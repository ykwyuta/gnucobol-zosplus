use imsdb::catalog::{parse_dbd, parse_psb};
use imsdb::csv_load::encode_root_csv;
use imsdb::dli::{ReadOp, Session, Status, WriteOp};
use imsdb::{Database, Dbd, StoreError};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DBD: &str = " DBD NAME=TEST,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n SEGM NAME=CHILD,PARENT=((ROOT,)),BYTES=5\n FIELD NAME=(SEQNO,SEQ,U),START=1,BYTES=2,TYPE=C\n DBDGEN\n END\n";
const PSB: &str = "TESTPCB PCB TYPE=DB,DBDNAME=TEST,PROCOPT=AP,KEYLEN=4\n SENSEG NAME=ROOT,PARENT=0\n SENSEG NAME=CHILD,PARENT=ROOT\n PSBGEN PSBNAME=TESTPSB,LANG=COBOL\n END\n";
const INDEXED_DBD: &str = " DBD NAME=PRIMARY,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n FIELD NAME=ALT,START=3,BYTES=2,TYPE=C\n LCHILD NAME=(PTR,SECOND),POINTER=INDX\n XDFLD NAME=XALT,SRCH=ALT\n DBDGEN\n END\n";
const INDEX_DBD: &str = " DBD NAME=SECOND,ACCESS=(INDEX,VSAM)\n SEGM NAME=PTR,PARENT=0,BYTES=2\n FIELD NAME=(ALTKEY,SEQ,U),START=1,BYTES=2,TYPE=C\n LCHILD NAME=(ROOT,PRIMARY),INDEX=XALT\n DBDGEN\n END\n";
const XDFLD_PRIMARY: &str = " DBD NAME=PRIMARY,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=4\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n LCHILD NAME=(PTR,SECOND),POINTER=INDX\n XDFLD NAME=XALT,SEGMENT=CHILD,SRCH=(NAME,CITY),SUBSEQ=SEQNO\n SEGM NAME=CHILD,PARENT=ROOT,BYTES=8\n FIELD NAME=(SEQNO,SEQ,U),START=1,BYTES=2,TYPE=C\n FIELD NAME=NAME,START=3,BYTES=2,TYPE=C\n FIELD NAME=CITY,START=5,BYTES=2,TYPE=C\n DBDGEN\n END\n";
const XDFLD_INDEX: &str = " DBD NAME=SECOND,ACCESS=(INDEX,VSAM)\n SEGM NAME=PTR,PARENT=0,BYTES=6\n FIELD NAME=(ALTKEY,SEQ,U),START=1,BYTES=6,TYPE=C\n LCHILD NAME=(ROOT,PRIMARY),INDEX=XALT\n DBDGEN\n END\n";
const PROCSEQ_PSB: &str = " PCB TYPE=DB,DBDNAME=PRIMARY,PROCOPT=G,PROCSEQ=SECOND,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0\n PSBGEN PSBNAME=IDXPSB,LANG=COBOL\n END\n";
const DIRECT_INDEX_PSB: &str = " PCB TYPE=DB,DBDNAME=SECOND,PROCOPT=G,KEYLEN=6\n SENSEG NAME=PTR,PARENT=0\n PSBGEN PSBNAME=PTRPSB,LANG=COBOL\n END\n";
const INDICES_PSB: &str = " PCB TYPE=DB,DBDNAME=PRIMARY,PROCOPT=G,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0,INDICES=SECOND\n PSBGEN PSBNAME=PHYPSB,LANG=COBOL\n END\n";

#[test]
fn bank_custaccs_csv_uses_account_customer_number_order() {
    let dbd = parse_dbd(" DBD NAME=CUSTACCS,ENCODING=Cp1047,ACCESS=(HDAM,OSAM)\n SEGM NAME=CUSTACCS,PARENT=0,BYTES=(16)\n FIELD NAME=(CUSTID,SEQ,M),START=1,BYTES=4,TYPE=C,DATATYPE=INT\n FIELD NAME=ACCID,START=5,BYTES=8,TYPE=C,DATATYPE=LONG\n FIELD NAME=ACCNUM,START=13,BYTES=4,TYPE=C,DATATYPE=INT\n DBDGEN\n END\n").unwrap();
    let rows = encode_root_csv(&dbd, "CUSTACCS", "101,1,1\n102,1,2\n").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(&rows[0][..4], &1i32.to_be_bytes());
    assert_eq!(&rows[0][4..12], &101i64.to_be_bytes());
    assert_eq!(&rows[1][..4], &1i32.to_be_bytes());
    assert_eq!(&rows[1][4..12], &102i64.to_be_bytes());
}

#[test]
fn senseg_indices_gu_uses_secondary_key_but_gn_keeps_physical_order() {
    let wal = path();
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(INDEXED_DBD).unwrap(),
            parse_dbd(INDEX_DBD).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    let one = seed
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![1, 0x0c, b'Z', b'Z', 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    let two = seed
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![2, 0x0c, b'A', b'A', 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    seed.commit().unwrap();
    let mut session = Session::new(db.clone(), parse_psb(INDICES_PSB).unwrap()).unwrap();
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQZZ)"])
            .unwrap()
            .record
            .unwrap()
            .id,
        one
    );
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().record.unwrap().id,
        two
    );
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().status,
        Status::End
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQAA)"])
            .unwrap()
            .record
            .unwrap()
            .id,
        two
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQXX)"])
            .unwrap()
            .status,
        Status::NotFound
    );
    let invalid = INDICES_PSB.replace("INDICES=SECOND", "INDICES=MISSING");
    assert!(Session::new(db.clone(), parse_psb(&invalid).unwrap()).is_err());
    drop(session);
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn direct_index_pcb_reads_complete_pointer_keys_only() {
    let wal = path();
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(XDFLD_PRIMARY).unwrap(),
            parse_dbd(XDFLD_INDEX).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    let root = seed
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![1, 0x0c, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    seed.insert(
        "PRIMARY",
        "CHILD",
        Some(root),
        b"01ALNY00".to_vec(),
        Duration::from_secs(1),
    )
    .unwrap();
    seed.insert(
        "PRIMARY",
        "CHILD",
        Some(root),
        b"02ALNY00".to_vec(),
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut session = Session::new(db.clone(), parse_psb(DIRECT_INDEX_PSB).unwrap()).unwrap();
    let first = session.read(0, ReadOp::Gn, &[]).unwrap();
    assert_eq!(first.record.unwrap().data, b"ALNY01");
    assert_eq!(first.key_feedback, b"ALNY01");
    assert_eq!(
        session
            .read(0, ReadOp::Gn, &[])
            .unwrap()
            .record
            .unwrap()
            .data,
        b"ALNY02"
    );
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().status,
        Status::End
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[b"PTR     (ALTKEY  EQALNY02)"])
            .unwrap()
            .record
            .unwrap()
            .data,
        b"ALNY02"
    );
    assert!(session
        .read(0, ReadOp::Gu, &[b"PTR     (ALTKEY  EQALNY)"])
        .is_err());
    assert_eq!(
        session
            .write(0, WriteOp::Isrt, "PTR", Some(b"ALNY03"), &[])
            .unwrap()
            .status,
        Status::NotAllowed
    );
    drop(session);
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn transaction_secondary_overlay_merges_child_changes_and_updated_target() {
    let wal = path();
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(XDFLD_PRIMARY).unwrap(),
            parse_dbd(XDFLD_INDEX).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    let root = seed
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![1, 0x0c, b'A', b'A'],
            Duration::from_secs(1),
        )
        .unwrap();
    let first = seed
        .insert(
            "PRIMARY",
            "CHILD",
            Some(root),
            b"01ALNY00".to_vec(),
            Duration::from_secs(1),
        )
        .unwrap();
    let removed = seed
        .insert(
            "PRIMARY",
            "CHILD",
            Some(root),
            b"03BONY00".to_vec(),
            Duration::from_secs(1),
        )
        .unwrap();
    seed.commit().unwrap();
    let mut tx = db.begin();
    tx.replace(root, vec![1, 0x0c, b'B', b'B'], Duration::from_secs(1))
        .unwrap();
    tx.replace(first, b"01BONY00".to_vec(), Duration::from_secs(1))
        .unwrap();
    tx.delete(removed, Duration::from_secs(1)).unwrap();
    tx.insert(
        "PRIMARY",
        "CHILD",
        Some(root),
        b"02ALNY00".to_vec(),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(tx
        .secondary_entry("SECOND", Some(b"ALNY"), Some((b"ALNY01", first)))
        .unwrap()
        .is_some());
    let (key, source, target) = tx.secondary_entry("SECOND", None, None).unwrap().unwrap();
    assert_eq!(key, b"ALNY02");
    assert_eq!(&target.data[2..4], b"BB");
    assert_eq!(
        tx.secondary_pointer_entry("SECOND", Some(b"ALNY02"), None)
            .unwrap()
            .unwrap()
            .1,
        source
    );
    assert!(tx
        .secondary_pointer_entry("SECOND", Some(b"ALNY01"), None)
        .unwrap()
        .is_none());
    let (next_key, _, _) = tx
        .secondary_entry("SECOND", None, Some((&key, source)))
        .unwrap()
        .unwrap();
    assert_eq!(next_key, b"BONY01");
    assert!(tx
        .secondary_entry("SECOND", None, Some((&next_key, first)))
        .unwrap()
        .is_none());
    tx.rollback();
    assert_eq!(
        db.secondary_entry("SECOND", Some(b"ALNY"), None)
            .unwrap()
            .unwrap()
            .0,
        b"ALNY01"
    );
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn secondary_overlay_rejects_duplicate_pending_unique_key() {
    let wal = path();
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(INDEXED_DBD).unwrap(),
            parse_dbd(INDEX_DBD).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    seed.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![1, 0x0c, b'A', b'A', 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut tx = db.begin();
    tx.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![2, 0x0c, b'A', b'A', 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(
        tx.secondary_entry("SECOND", Some(b"AA"), None),
        Err(StoreError::Duplicate(_))
    ));
    tx.rollback();
    drop(db);
    std::fs::remove_file(wal).unwrap();
}
fn path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("imsdb-test-{}-{nanos}.wal", std::process::id()))
}
fn dbd() -> Dbd {
    parse_dbd(DBD).unwrap()
}

#[test]
fn procseq_gu_and_gn_use_secondary_order_and_see_pending_changes() {
    let wal = path();
    let db = Database::open(
        &wal,
        vec![
            parse_dbd(INDEXED_DBD).unwrap(),
            parse_dbd(INDEX_DBD).unwrap(),
        ],
    )
    .unwrap();
    let mut seed = db.begin();
    seed.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![1, 0x0c, b'Z', b'Z', 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![2, 0x0c, b'A', b'A', 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let psb = parse_psb(PROCSEQ_PSB).unwrap();
    assert_eq!(psb.pcbs[0].procseq.as_deref(), Some("SECOND"));
    let mut session = Session::new(db.clone(), psb).unwrap();
    let first = session.read(0, ReadOp::Gn, &[]).unwrap();
    assert_eq!(first.record.unwrap().data[2..4], *b"AA");
    assert_eq!(first.key_feedback, b"AA");
    let second = session.read(0, ReadOp::Gn, &[]).unwrap();
    assert_eq!(second.record.unwrap().data[2..4], *b"ZZ");
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().status,
        Status::End
    );
    let gu = session
        .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQZZ)"])
        .unwrap();
    assert_eq!(gu.record.unwrap().data[2..4], *b"ZZ");
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().status,
        Status::End
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQXX)"])
            .unwrap()
            .status,
        Status::NotFound
    );
    assert!(session
        .read(0, ReadOp::Gu, &[b"ROOT    (ID      EQ\x01\x0c)"])
        .is_err());

    let mut writer = db.begin();
    writer
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![3, 0x0c, b'B', b'B', 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(
        writer
            .secondary_entry("SECOND", Some(b"BB"), None)
            .unwrap()
            .unwrap()
            .2
            .data[2..4],
        *b"BB"
    );
    writer.rollback();
    assert!(db
        .secondary_entry("SECOND", Some(b"BB"), None)
        .unwrap()
        .is_none());
    let aa = db.secondary_lookup("SECOND", b"AA").unwrap().unwrap();
    let mut writer = db.begin();
    writer
        .replace(
            aa.id,
            vec![2, 0x0c, b'C', b'C', 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    assert!(writer
        .secondary_entry("SECOND", Some(b"AA"), None)
        .unwrap()
        .is_none());
    assert_eq!(
        writer
            .secondary_entry("SECOND", Some(b"CC"), None)
            .unwrap()
            .unwrap()
            .2
            .id,
        aa.id
    );
    writer.delete(aa.id, Duration::from_secs(1)).unwrap();
    assert!(writer
        .secondary_entry("SECOND", Some(b"CC"), None)
        .unwrap()
        .is_none());
    writer.rollback();
    drop(session);
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn xdfld_child_search_and_subsequence_resolve_parent() {
    let wal = path();
    let definitions = || {
        vec![
            parse_dbd(XDFLD_PRIMARY).unwrap(),
            parse_dbd(XDFLD_INDEX).unwrap(),
        ]
    };
    let db = Database::open(&wal, definitions()).unwrap();
    let mut tx = db.begin();
    let root = tx
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![1, 0x0c, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    let child = tx
        .insert(
            "PRIMARY",
            "CHILD",
            Some(root),
            b"01ALNY00".to_vec(),
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.secondary_lookup("SECOND", b"ALNY").unwrap().unwrap().id,
        root
    );
    let psb = PROCSEQ_PSB.replace(
        " SENSEG NAME=ROOT,PARENT=0\n",
        " SENSEG NAME=ROOT,PARENT=0\n SENSEG NAME=CHILD,PARENT=ROOT\n",
    );
    let mut session = Session::new(db.clone(), parse_psb(&psb).unwrap()).unwrap();
    let indexed = session
        .read(0, ReadOp::Gu, &[b"ROOT    (XALT    EQALNY)"])
        .unwrap();
    assert_eq!(indexed.record.unwrap().id, root);
    assert_eq!(indexed.key_feedback, b"ALNY01");
    // Child access after index positioning follows the primary hierarchy.
    assert_eq!(
        session
            .read(0, ReadOp::Gnp, &[b"CHILD   "])
            .unwrap()
            .record
            .unwrap()
            .id,
        child
    );
    drop(session);
    assert!(matches!(
        db.secondary_lookup("SECOND", b"AL"),
        Err(StoreError::Definition(_))
    ));
    let mut tx = db.begin();
    tx.replace(child, b"01BONY00".to_vec(), Duration::from_secs(1))
        .unwrap();
    tx.commit().unwrap();
    assert!(db.secondary_lookup("SECOND", b"ALNY").unwrap().is_none());
    assert_eq!(
        db.secondary_lookup("SECOND", b"BONY").unwrap().unwrap().id,
        root
    );
    let mut tx = db.begin();
    let second = tx
        .insert(
            "PRIMARY",
            "CHILD",
            Some(root),
            b"02BONY00".to_vec(),
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    drop(db);
    let db = Database::open(&wal, definitions()).unwrap();
    assert_eq!(
        db.secondary_lookup("SECOND", b"BONY").unwrap().unwrap().id,
        root
    );
    let mut tx = db.begin();
    tx.delete(child, Duration::from_secs(1)).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.secondary_lookup("SECOND", b"BONY").unwrap().unwrap().id,
        root
    );
    let mut tx = db.begin();
    tx.delete(second, Duration::from_secs(1)).unwrap();
    tx.commit().unwrap();
    assert!(db.secondary_lookup("SECOND", b"BONY").unwrap().is_none());
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn unsupported_xdfld_operand_is_rejected() {
    let source = XDFLD_PRIMARY.replace("SUBSEQ=SEQNO", "NULLVAL=0");
    assert!(parse_dbd(&source)
        .unwrap_err()
        .contains("unsupported XDFLD operand"));
}

#[test]
fn secondary_index_tracks_insert_replace_delete_and_replay() {
    let wal = path();
    let definitions = || {
        vec![
            parse_dbd(INDEXED_DBD).unwrap(),
            parse_dbd(INDEX_DBD).unwrap(),
        ]
    };
    let db = Database::open(&wal, definitions()).unwrap();
    let mut tx = db.begin();
    let id = tx
        .insert(
            "PRIMARY",
            "ROOT",
            None,
            vec![1, 0x0c, b'A', b'A', 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.secondary_lookup("SECOND", b"AA").unwrap().unwrap().id,
        id
    );
    assert!(db.secondary_lookup("SECOND", b"BB").unwrap().is_none());
    let mut tx = db.begin();
    tx.replace(id, vec![1, 0x0c, b'B', b'B', 0, 0], Duration::from_secs(1))
        .unwrap();
    tx.commit().unwrap();
    assert!(db.secondary_lookup("SECOND", b"AA").unwrap().is_none());
    assert_eq!(
        db.secondary_lookup("SECOND", b"BB").unwrap().unwrap().id,
        id
    );
    let mut tx = db.begin();
    tx.insert(
        "PRIMARY",
        "ROOT",
        None,
        vec![2, 0x0c, b'B', b'B', 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(tx.commit(), Err(StoreError::Duplicate(_))));
    assert_eq!(db.count(), 1);
    drop(db);
    let db = Database::open(&wal, definitions()).unwrap();
    assert_eq!(
        db.secondary_lookup("SECOND", b"BB").unwrap().unwrap().id,
        id
    );
    let mut tx = db.begin();
    tx.delete(id, Duration::from_secs(1)).unwrap();
    tx.commit().unwrap();
    assert!(db.secondary_lookup("SECOND", b"BB").unwrap().is_none());
    drop(db);
    let db = Database::open(&wal, definitions()).unwrap();
    assert_eq!(db.count(), 0);
    assert!(db.secondary_lookup("SECOND", b"BB").unwrap().is_none());
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn checkpoint_id_and_changes_survive_reopen_atomically() {
    let wal = path();
    let db = Database::open(&wal, vec![dbd()]).unwrap();
    assert_eq!(db.latest_checkpoint(), None);
    let mut tx = db.begin();
    tx.insert(
        "TEST",
        "ROOT",
        None,
        vec![0x12, 0x3c, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.set_checkpoint(*b"CHECK001");
    tx.commit().unwrap();
    assert_eq!(db.latest_checkpoint(), Some(*b"CHECK001"));
    let mut tx = db.begin();
    tx.set_checkpoint(*b"CHECK002");
    tx.rollback();
    assert_eq!(db.latest_checkpoint(), Some(*b"CHECK001"));
    drop(db);
    let db = Database::open(&wal, vec![dbd()]).unwrap();
    assert_eq!(db.count(), 1);
    assert_eq!(db.latest_checkpoint(), Some(*b"CHECK001"));
    let mut tx = db.begin();
    tx.set_checkpoint(*b"CHECK003");
    tx.commit().unwrap();
    drop(db);
    let db = Database::open(&wal, vec![dbd()]).unwrap();
    assert_eq!(db.count(), 1);
    assert_eq!(db.latest_checkpoint(), Some(*b"CHECK003"));
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn a_wal_has_only_one_open_owner() {
    let wal = path();
    let owner = Database::open(&wal, vec![dbd()]).unwrap();
    let second = Database::open(&wal, vec![dbd()]);
    assert!(matches!(second, Err(StoreError::Conflict(_))));
    drop(owner);
    Database::open(&wal, vec![dbd()]).unwrap();
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn parses_hierarchy_and_psb() {
    let dbd = dbd();
    assert_eq!(dbd.segments[1].parent.as_deref(), Some("ROOT"));
    assert_eq!(dbd.segments[0].key().unwrap().field_type, "P");
    let psb = parse_psb(PSB).unwrap();
    psb.validate(&[dbd]).unwrap();
    assert_eq!(psb.pcbs[0].name.as_deref(), Some("TESTPCB"));
}

#[test]
fn empty_gn_with_unqualified_segment_ssa_returns_end() {
    let wal = path();
    let database = Database::open(&wal, vec![dbd()]).unwrap();
    let mut session = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    let outcome = session.read(0, ReadOp::Gn, &[b"ROOT     "]).unwrap();
    assert_eq!(outcome.status, Status::End);
    let outcome = session.read(0, ReadOp::Gn, &[b"ROOT    "]).unwrap();
    assert_eq!(outcome.status, Status::End);
    drop(session);
    drop(database);
    std::fs::remove_file(wal).unwrap();
}

#[test]
fn transaction_persists_raw_parent_child_bytes_and_rollback_discards_changes() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut tx = database.begin();
    let root = tx
        .insert(
            "TEST",
            "ROOT",
            None,
            vec![0x12, 0x3c, 0, 0xff, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    let child = tx
        .insert(
            "TEST",
            "CHILD",
            Some(root),
            vec![0x40, 0x41, 0x0e, 0x42, 0x0f],
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(database.count(), 2);
    assert_eq!(
        database
            .get("TEST", None, "ROOT", &[0x12, 0x3c])
            .unwrap()
            .id,
        root
    );
    assert_eq!(database.children("TEST", Some(root), "CHILD")[0].id, child);
    let mut tx = database.begin();
    tx.replace(child, vec![0x40, 0x41, 1, 2, 3], Duration::from_secs(1))
        .unwrap();
    tx.rollback();
    drop(database);
    let reopened = Database::open(&path, vec![dbd()]).unwrap();
    assert_eq!(
        reopened.record(child).unwrap().data,
        vec![0x40, 0x41, 0x0e, 0x42, 0x0f]
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn duplicate_and_root_lock_timeout() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut first = database.begin();
    first
        .insert(
            "TEST",
            "ROOT",
            None,
            vec![1, 2, 0, 0, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    let mut second = database.begin();
    assert!(matches!(
        second.insert(
            "TEST",
            "ROOT",
            None,
            vec![1, 2, 1, 1, 1, 1],
            Duration::from_millis(10)
        ),
        Err(StoreError::Timeout)
    ));
    first.commit().unwrap();
    assert!(matches!(
        second.insert(
            "TEST",
            "ROOT",
            None,
            vec![1, 2, 1, 1, 1, 1],
            Duration::from_secs(1)
        ),
        Err(StoreError::Duplicate(_))
    ));
    drop(second);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn nonunique_sequence_field_allows_multiple_occurrences() {
    let path = path();
    let definition = parse_dbd(" DBD NAME=MULTI,ACCESS=(HDAM,OSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=4\n FIELD NAME=(KEY,SEQ,M),START=1,BYTES=2,TYPE=C\n DBDGEN\n END\n").unwrap();
    let database = Database::open(&path, vec![definition]).unwrap();
    let mut tx = database.begin();
    tx.insert(
        "MULTI",
        "ROOT",
        None,
        vec![1, 2, 3, 4],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.insert(
        "MULTI",
        "ROOT",
        None,
        vec![1, 2, 5, 6],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.commit().unwrap();
    let records = database.children("MULTI", None, "ROOT");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].data, vec![1, 2, 3, 4]);
    assert_eq!(records[1].data, vec![1, 2, 5, 6]);
    drop(database);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn incomplete_final_wal_frame_is_discarded() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut tx = database.begin();
    tx.insert(
        "TEST",
        "ROOT",
        None,
        vec![1, 2, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(database);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"IMS1\x10").unwrap();
    drop(file);
    let reopened = Database::open(&path, vec![dbd()]).unwrap();
    assert_eq!(reopened.count(), 1);
    let mut tx = reopened.begin();
    tx.insert(
        "TEST",
        "ROOT",
        None,
        vec![2, 3, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(reopened);
    assert_eq!(Database::open(&path, vec![dbd()]).unwrap().count(), 2);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn dli_gu_gn_gnp_follow_parent_child_position_and_binary_ssa() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut tx = database.begin();
    let first = tx
        .insert(
            "TEST",
            "ROOT",
            None,
            vec![0x12, 0x3c, 0, 0, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    let child1 = tx
        .insert(
            "TEST",
            "CHILD",
            Some(first),
            vec![b'A', b'1', 1, 2, 3],
            Duration::from_secs(1),
        )
        .unwrap();
    let child2 = tx
        .insert(
            "TEST",
            "CHILD",
            Some(first),
            vec![b'A', b'2', 4, 5, 6],
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    let mut tx = database.begin();
    let second = tx
        .insert(
            "TEST",
            "ROOT",
            None,
            vec![0x34, 0x5c, 0, 0, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    tx.commit().unwrap();
    let mut session = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    let mut ssa = b"ROOT    (ID      = ".to_vec();
    ssa.extend_from_slice(&[0x12, 0x3c]);
    ssa.push(b')');
    let outcome = session.read(0, ReadOp::Gu, &[&ssa]).unwrap();
    assert_eq!(outcome.status, Status::Ok);
    assert_eq!(outcome.record.unwrap().id, first);
    assert_eq!(outcome.key_feedback, vec![0x12, 0x3c]);
    let child_ssa = b"CHILD    ";
    assert_eq!(
        session
            .read(0, ReadOp::Gnp, &[child_ssa])
            .unwrap()
            .record
            .unwrap()
            .id,
        child1
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gnp, &[child_ssa])
            .unwrap()
            .record
            .unwrap()
            .id,
        child2
    );
    assert_eq!(
        session.read(0, ReadOp::Gnp, &[child_ssa]).unwrap().status,
        Status::NotFound
    );
    let next_root = session.read(0, ReadOp::Gn, &[]).unwrap();
    assert_eq!(next_root.status, Status::HigherLevel);
    assert_eq!(next_root.record.unwrap().id, second);
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().status,
        Status::End
    );
    assert_eq!(
        session.read(0, ReadOp::Gn, &[]).unwrap().record.unwrap().id,
        first
    );
    assert!(session
        .read(0, ReadOp::Gu, &[b"ROOT    (ID      NE\x12\x3c)"])
        .is_err());
    drop(session);
    drop(database);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hold_replace_requires_permission_and_commits_only_at_syncpoint() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut seed = database.begin();
    seed.insert(
        "TEST",
        "ROOT",
        None,
        vec![1, 2, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut session = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    assert_eq!(
        session
            .write(0, WriteOp::Repl, "ROOT", Some(&[1, 2, 1, 1, 1, 1]), &[])
            .unwrap()
            .status,
        Status::NotHeld
    );
    let mut ssa = b"ROOT    (ID      EQ".to_vec();
    ssa.extend_from_slice(&[1, 2]);
    ssa.push(b')');
    assert_eq!(
        session.read(0, ReadOp::Ghu, &[&ssa]).unwrap().status,
        Status::Ok
    );
    assert_eq!(
        session
            .write(0, WriteOp::Repl, "ROOT", Some(&[1, 3, 1, 1, 1, 1]), &[])
            .unwrap()
            .status,
        Status::NotAllowed
    );
    assert_eq!(
        session
            .write(0, WriteOp::Repl, "ROOT", Some(&[1, 2, 3, 4, 5, 6]), &[])
            .unwrap()
            .status,
        Status::Ok
    );
    assert_eq!(
        session
            .read(0, ReadOp::Gu, &[&ssa])
            .unwrap()
            .record
            .unwrap()
            .data,
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        session
            .write(0, WriteOp::Repl, "ROOT", Some(&[1, 2, 9, 9, 9, 9]), &[])
            .unwrap()
            .status,
        Status::NotHeld
    );
    assert_eq!(
        database.get("TEST", None, "ROOT", &[1, 2]).unwrap().data,
        vec![1, 2, 0, 0, 0, 0]
    );
    session.rollback();
    assert_eq!(
        database.get("TEST", None, "ROOT", &[1, 2]).unwrap().data,
        vec![1, 2, 0, 0, 0, 0]
    );
    session.read(0, ReadOp::Ghu, &[&ssa]).unwrap();
    session
        .write(0, WriteOp::Repl, "ROOT", Some(&[1, 2, 7, 7, 7, 7]), &[])
        .unwrap();
    session.commit().unwrap();
    assert_eq!(
        database.get("TEST", None, "ROOT", &[1, 2]).unwrap().data,
        vec![1, 2, 7, 7, 7, 7]
    );
    drop(session);
    drop(database);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn parent_child_insert_delete_and_duplicate_statuses() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut session = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    let root = session
        .write(
            0,
            WriteOp::Isrt,
            "ROOT",
            Some(&[1, 2, 0, 0, 0, 0]),
            &[b"ROOT     "],
        )
        .unwrap()
        .record
        .unwrap();
    let first = session
        .write(0, WriteOp::Isrt, "CHILD", Some(b"A1abc"), &[b"CHILD    "])
        .unwrap()
        .record
        .unwrap();
    session
        .write(0, WriteOp::Isrt, "CHILD", Some(b"A2def"), &[b"CHILD    "])
        .unwrap();
    assert_eq!(
        session
            .write(0, WriteOp::Isrt, "CHILD", Some(b"A2xyz"), &[b"CHILD    "])
            .unwrap()
            .status,
        Status::Duplicate
    );
    session.commit().unwrap();
    assert_eq!(
        session
            .write(0, WriteOp::Dlet, "ROOT", None, &[])
            .unwrap()
            .status,
        Status::NotHeld
    );
    session.read(0, ReadOp::Ghu, &[b"ROOT     "]).unwrap();
    assert_eq!(
        session
            .read(0, ReadOp::Ghnp, &[b"CHILD    "])
            .unwrap()
            .record
            .unwrap()
            .id,
        first.id
    );
    session.write(0, WriteOp::Dlet, "CHILD", None, &[]).unwrap();
    assert_eq!(
        session
            .read(0, ReadOp::Ghnp, &[b"CHILD    "])
            .unwrap()
            .record
            .unwrap()
            .key,
        b"A2"
    );
    session.read(0, ReadOp::Ghu, &[b"ROOT     "]).unwrap();
    assert_eq!(
        session
            .write(0, WriteOp::Dlet, "ROOT", None, &[])
            .unwrap()
            .status,
        Status::Ok
    );
    session.commit().unwrap();
    assert!(database.record(root.id).is_none());
    assert_eq!(database.count(), 0);
    drop(session);
    drop(database);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn failed_multi_change_commit_undoes_earlier_changes() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut seed = database.begin();
    let parent = seed
        .insert(
            "TEST",
            "ROOT",
            None,
            vec![2, 0, 0, 0, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    seed.insert(
        "TEST",
        "CHILD",
        Some(parent),
        b"A1abc".to_vec(),
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut tx = database.begin();
    tx.insert(
        "TEST",
        "ROOT",
        None,
        vec![1, 0, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.delete(parent, Duration::from_secs(1)).unwrap();
    assert!(matches!(tx.commit(), Err(StoreError::Conflict(_))));
    assert!(database.get("TEST", None, "ROOT", &[1, 0]).is_none());
    assert_eq!(database.count(), 2);
    drop(database);
    let reopened = Database::open(&path, vec![dbd()]).unwrap();
    assert_eq!(reopened.count(), 2);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hold_lock_blocks_same_root_but_allows_another_root() {
    let path = path();
    let database = Database::open(&path, vec![dbd()]).unwrap();
    let mut seed = database.begin();
    seed.insert(
        "TEST",
        "ROOT",
        None,
        vec![1, 0, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.insert(
        "TEST",
        "ROOT",
        None,
        vec![2, 0, 0, 0, 0, 0],
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut first = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    let mut second = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    second.set_lock_timeout(Duration::from_millis(5));
    let mut first_ssa = b"ROOT    (ID      EQ".to_vec();
    first_ssa.extend_from_slice(&[1, 0]);
    first_ssa.push(b')');
    let mut second_ssa = b"ROOT    (ID      EQ".to_vec();
    second_ssa.extend_from_slice(&[2, 0]);
    second_ssa.push(b')');
    assert_eq!(
        first.read(0, ReadOp::Ghu, &[&first_ssa]).unwrap().status,
        Status::Ok
    );
    assert!(second
        .read(0, ReadOp::Ghu, &[&first_ssa])
        .unwrap_err()
        .contains("timeout"));
    assert_eq!(
        second.read(0, ReadOp::Ghu, &[&second_ssa]).unwrap().status,
        Status::Ok
    );
    first.commit().unwrap();
    second.commit().unwrap();
    assert_eq!(
        second.read(0, ReadOp::Ghu, &[&first_ssa]).unwrap().status,
        Status::Ok
    );
    drop(second);
    drop(first);
    drop(database);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn get_only_pcb_ghu_does_not_lock_roots_for_update() {
    let wal = path();
    let database = Database::open(&wal, vec![dbd()]).unwrap();
    let mut seed = database.begin();
    for id in [1, 2] {
        seed.insert(
            "TEST",
            "ROOT",
            None,
            vec![id, 0, 0, 0, 0, 0],
            Duration::from_secs(1),
        )
        .unwrap();
    }
    seed.commit().unwrap();
    let read_only = parse_psb(" PCB TYPE=DB,DBDNAME=TEST,PROCOPT=G,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0\n PSBGEN PSBNAME=READONLY,LANG=COBOL\n END\n").unwrap();
    let mut reader = Session::new(database.clone(), read_only).unwrap();
    let mut updater = Session::new(database.clone(), parse_psb(PSB).unwrap()).unwrap();
    updater.set_lock_timeout(Duration::from_millis(5));
    let ssa = |id| {
        let mut raw = b"ROOT    (ID      EQ".to_vec();
        raw.extend_from_slice(&[id, 0]);
        raw.push(b')');
        raw
    };
    for id in [2, 1] {
        assert_eq!(
            reader.read(0, ReadOp::Ghu, &[&ssa(id)]).unwrap().status,
            Status::Ok
        );
    }
    assert_eq!(
        updater.read(0, ReadOp::Ghu, &[&ssa(1)]).unwrap().status,
        Status::Ok
    );
    drop(reader);
    drop(updater);
    drop(database);
    std::fs::remove_file(wal).unwrap();
}
