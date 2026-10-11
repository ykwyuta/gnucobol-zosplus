use imsdb::call_adapter::{CallAdapter, DliFunction, NumericByteOrder, PcbEncoding};
use imsdb::catalog::parse_psb;
use imsdb::csv_load::encode_root_csv;
use imsdb::dli::{ReadOp, Session, Status, WriteOp};
use imsdb::unload::parse_dfsurgu0;
use imsdb::{Database, Dbd, Psb};

#[test]
#[ignore = "requires CardDemo reference assets"]
fn carddemo_secondary_index_matches_original_unload() {
    let base = "reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq";
    let primary = Dbd::from_file(reference(&format!("{base}/ims/DBPAUTP0.dbd"))).unwrap();
    let index = Dbd::from_file(reference(&format!("{base}/ims/DBPAUTX0.dbd"))).unwrap();
    let input = std::fs::read(reference(&format!(
        "{base}/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat"
    )))
    .unwrap();
    let rows = parse_dfsurgu0(&input, &primary).unwrap();
    let wal = temporary_wal("card-secondary");
    let db = Database::open(&wal, vec![primary.clone(), index.clone()]).unwrap();
    let mut ids = Vec::new();
    let mut tx = db.begin();
    let mut root_keys = Vec::new();
    for row in rows {
        if row.parent_index.is_none() {
            root_keys.push(row.data[..6].to_vec());
        }
        let parent = row.parent_index.map(|index| ids[index]);
        ids.push(
            tx.insert(
                "DBPAUTP0",
                &row.segment,
                parent,
                row.data,
                Duration::from_secs(1),
            )
            .unwrap(),
        );
    }
    tx.commit().unwrap();
    assert_eq!(ids.len(), 224);
    assert_eq!(root_keys.len(), 22);
    for key in &root_keys {
        let record = db.secondary_lookup("DBPAUTX0", key).unwrap().unwrap();
        assert_eq!(record.segment, "PAUTSUM0");
        assert_eq!(&record.data[..6], key);
    }
    let psb = parse_psb(" PCB TYPE=DB,DBDNAME=DBPAUTP0,PROCOPT=G,PROCSEQ=DBPAUTX0,KEYLEN=6\n SENSEG NAME=PAUTSUM0,PARENT=0\n PSBGEN PSBNAME=IDXPAUT,LANG=COBOL\n END\n").unwrap();
    let mut adapter = CallAdapter::new(db.clone(), psb, PcbEncoding::Ascii).unwrap();
    let mut pcb = [0u8; 42];
    let mut io = [0u8; 100];
    for key in &root_keys {
        let mut ssa = b"PAUTSUM0(ACCNTID EQ".to_vec();
        ssa.extend_from_slice(key);
        ssa.push(b')');
        assert_eq!(
            adapter
                .call(0, DliFunction::Gu, &mut pcb, &mut io, &[&ssa])
                .unwrap(),
            Status::Ok
        );
        assert_eq!(&io[..6], key);
        assert_eq!(&pcb[36..42], key);
    }
    let mut observed = Vec::new();
    let mut session = Session::new(db.clone(), parse_psb(" PCB TYPE=DB,DBDNAME=DBPAUTP0,PROCOPT=G,PROCSEQ=DBPAUTX0,KEYLEN=6\n SENSEG NAME=PAUTSUM0,PARENT=0\n PSBGEN PSBNAME=IDXPAUT,LANG=COBOL\n END\n").unwrap()).unwrap();
    loop {
        let outcome = session.read(0, ReadOp::Gn, &[]).unwrap();
        if outcome.status == Status::End {
            break;
        }
        observed.push(outcome.record.unwrap().root_key);
    }
    assert_eq!(observed.len(), 22);
    assert!(observed.windows(2).all(|pair| pair[0] <= pair[1]));
    let direct_psb = parse_psb(" PCB TYPE=DB,DBDNAME=DBPAUTX0,PROCOPT=G,KEYLEN=6\n SENSEG NAME=PAUTINDX,PARENT=0\n PSBGEN PSBNAME=PTRPAUT,LANG=COBOL\n END\n").unwrap();
    let mut direct = CallAdapter::new(db.clone(), direct_psb, PcbEncoding::Ascii).unwrap();
    let mut pointer_pcb = [0u8; 42];
    let mut pointer_io = [0u8; 6];
    for key in &root_keys {
        let mut ssa = b"PAUTINDX(INDXSEQ EQ".to_vec();
        ssa.extend_from_slice(key);
        ssa.push(b')');
        assert_eq!(
            direct
                .call(
                    0,
                    DliFunction::Gu,
                    &mut pointer_pcb,
                    &mut pointer_io,
                    &[&ssa]
                )
                .unwrap(),
            Status::Ok
        );
        assert_eq!(pointer_io.as_slice(), key);
        assert_eq!(&pointer_pcb[36..42], key);
    }
    let mut direct_session = Session::new(db.clone(), parse_psb(" PCB TYPE=DB,DBDNAME=DBPAUTX0,PROCOPT=G,KEYLEN=6\n SENSEG NAME=PAUTINDX,PARENT=0\n PSBGEN PSBNAME=PTRPAUT,LANG=COBOL\n END\n").unwrap()).unwrap();
    let mut pointer_count = 0;
    while direct_session.read(0, ReadOp::Gn, &[]).unwrap().status != Status::End {
        pointer_count += 1;
    }
    assert_eq!(pointer_count, 22);
    drop(direct);
    drop(direct_session);
    drop(adapter);
    drop(session);
    drop(db);
    let db = Database::open(&wal, vec![primary, index]).unwrap();
    for key in &root_keys {
        assert!(db.secondary_lookup("DBPAUTX0", key).unwrap().is_some());
    }
    drop(db);
    std::fs::remove_file(wal).unwrap();
}

#[test]
#[ignore = "requires CardDemo reference assets"]
fn carddemo_gsam_dbd_metadata_matches_fixed_files() {
    let base =
        "reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims";
    for (file, input, output, length) in [
        ("PASFLDBD.DBD", "PASFILIP", "PASFILOP", 100),
        ("PADFLDBD.DBD", "PADFILIP", "PADFILOP", 200),
    ] {
        let dbd = Dbd::from_file(reference(&format!("{base}/{file}"))).unwrap();
        let gsam = dbd.gsam.unwrap();
        assert_eq!(gsam.input_dd, input);
        assert_eq!(gsam.output_dd, output);
        assert_eq!(gsam.record_len, length);
    }
}
use std::path::PathBuf;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn reference(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}
fn temporary_wal(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("imsdb-{name}-{}-{nanos}.wal", std::process::id()))
}
fn qualified_ssa(segment: &str, field: &str, key: &[u8], operator: &[u8; 2]) -> Vec<u8> {
    let mut ssa = format!("{segment:<8}({field:<8}").into_bytes();
    ssa.extend_from_slice(operator);
    ssa.extend_from_slice(key);
    ssa.push(b')');
    ssa
}

#[test]
#[ignore = "requires Bank of Z reference assets"]
fn bank_csv_load_data_encodes_all_nine_dbds() {
    let names = [
        "ACCOUNT", "ACCTYPE", "CUSTACCS", "CUSTOMER", "CUSTTYPE", "HISTORY", "TSTAT", "TSTATTYP",
        "TTYPE",
    ];
    let dbds: Vec<Dbd> = names
        .iter()
        .map(|name| {
            Dbd::from_file(reference(&format!(
                "reference/Bank-of-Z-main/src/base/ims/DBD/{name}.asm"
            )))
            .unwrap()
        })
        .collect();
    let wal = temporary_wal("bank-csv");
    let database = Database::open(&wal, dbds.clone()).unwrap();
    let mut tx = database.begin();
    let mut total = 0;
    for dbd in &dbds {
        let source = std::fs::read_to_string(reference(&format!(
            "reference/Bank-of-Z-main/src/base/ims/LoadData/{}.data",
            dbd.name
        )))
        .unwrap();
        let mut records = encode_root_csv(dbd, &dbd.name, &source).unwrap();
        assert_eq!(records.len(), source.lines().count());
        let definition = dbd.segment(&dbd.name).unwrap();
        records.sort_by(|a, b| {
            definition
                .extract_key(a)
                .unwrap()
                .cmp(definition.extract_key(b).unwrap())
        });
        for record in records {
            tx.insert(&dbd.name, &dbd.name, None, record, Duration::from_secs(1))
                .unwrap();
            total += 1;
        }
    }
    tx.commit().unwrap();
    assert_eq!(database.count(), total);
    let account = database
        .get("ACCOUNT", None, "ACCOUNT", &101i64.to_be_bytes())
        .unwrap();
    assert_eq!(account.data.len(), 25);
    assert_eq!(account.data[8], 0x83); // 'c' in the GnuCOBOL 1047 table.
    assert_eq!(&account.data[9..17], &[0, 0, 0, 0, 0x08, 0x83, 0x00, 0x0c]);
    let customer = database
        .get("CUSTOMER", None, "CUSTOMER", &1i32.to_be_bytes())
        .unwrap();
    assert_eq!(customer.data.len(), 279);
    assert_eq!(&customer.data[4..6], &[0xc1, 0x95]); // 'An' in EBCDIC.
    let psb = Psb::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/PSB/IBTRAN.asm",
    ))
    .unwrap();
    let mut session = Session::new(database.clone(), psb).unwrap();
    let account_ssa = qualified_ssa("ACCOUNT", "ACCID", &101i64.to_be_bytes(), b"= ");
    assert_eq!(
        session.read(0, ReadOp::Gu, &[&account_ssa]).unwrap().status,
        Status::Ok
    );
    drop(session);
    let customer_psb = Psb::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/PSB/IBGCUDAT.asm",
    ))
    .unwrap();
    let mut caller = CallAdapter::new(database.clone(), customer_psb, PcbEncoding::Cp1047).unwrap();
    let mut pcb_mask = vec![0u8; 56];
    let mut io_area = vec![0u8; 279];
    let customer_ssa = qualified_ssa("CUSTOMER", "CUSTID", &1i32.to_be_bytes(), b"EQ");
    assert_eq!(
        caller
            .call(
                0,
                DliFunction::Gu,
                &mut pcb_mask,
                &mut io_area,
                &[&customer_ssa]
            )
            .unwrap(),
        Status::Ok
    );
    assert_eq!(io_area, customer.data);
    assert_eq!(&pcb_mask[36..40], &1i32.to_be_bytes());
    drop(caller);
    let customer_psb = Psb::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/PSB/IBGCUDAT.asm",
    ))
    .unwrap();
    let mut caller = CallAdapter::new(database.clone(), customer_psb, PcbEncoding::Cp1047).unwrap();
    caller.set_numeric_byte_order(NumericByteOrder::Little);
    let mut ssa = PcbEncoding::Cp1047
        .encode_control(b"CUSTOMER(CUSTID  EQ")
        .unwrap();
    ssa.extend_from_slice(&1i32.to_le_bytes());
    ssa.extend(PcbEncoding::Cp1047.encode_control(b")").unwrap());
    let gu = PcbEncoding::Cp1047.encode_control(b"GU  ").unwrap();
    assert_eq!(
        caller
            .call_encoded(0, &gu, &mut pcb_mask, &mut io_area, &[&ssa])
            .unwrap(),
        Status::Ok
    );
    assert_eq!(&io_area[..4], &1i32.to_le_bytes());
    assert_eq!(&pcb_mask[36..40], &1i32.to_le_bytes());
    drop(caller);
    drop(database);
    std::fs::remove_file(wal).unwrap();
}

#[test]
#[ignore = "requires Bank of Z and CardDemo reference assets"]
fn bank_customer_and_carddemo_parent_child_use_original_dbds_and_psbs() {
    let bank_dbd = Dbd::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/DBD/CUSTOMER.asm",
    ))
    .unwrap();
    let bank_psb = Psb::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/PSB/IBSCUDAT.asm",
    ))
    .unwrap();
    let bank_wal = temporary_wal("bank");
    let bank = Database::open(&bank_wal, vec![bank_dbd]).unwrap();
    let mut bank_session = Session::new(bank.clone(), bank_psb).unwrap();
    let bank_key = [0x12, 0x34, 0x56, 0x78];
    let mut customer = vec![0x40; 279];
    customer[..4].copy_from_slice(&bank_key);
    customer[4..8].copy_from_slice(&[0x0e, 0x42, 0x43, 0x0f]); // EBCDIC DBCS shift bytes stay opaque.
    let mut seed = bank.begin();
    seed.insert(
        "CUSTOMER",
        "CUSTOMER",
        None,
        customer.clone(),
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let bank_ssa = qualified_ssa("CUSTOMER", "CUSTID", &bank_key, b"= ");
    let read = bank_session.read(0, ReadOp::Ghu, &[&bank_ssa]).unwrap();
    assert_eq!(read.status, Status::Ok);
    assert_eq!(&read.record.unwrap().data[4..8], &[0x0e, 0x42, 0x43, 0x0f]);
    customer[54] = 0xc1;
    assert_eq!(
        bank_session
            .write(0, WriteOp::Repl, "CUSTOMER", Some(&customer), &[])
            .unwrap()
            .status,
        Status::Ok
    );
    bank_session.commit().unwrap();
    assert_eq!(
        bank.get("CUSTOMER", None, "CUSTOMER", &bank_key)
            .unwrap()
            .data[54],
        0xc1
    );
    drop(bank_session);
    drop(bank);
    std::fs::remove_file(bank_wal).unwrap();

    let card_dbd = Dbd::from_file(reference("reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/DBPAUTP0.dbd")).unwrap();
    let card_psb = Psb::from_file(reference("reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/PSBPAUTB.psb")).unwrap();
    let card_wal = temporary_wal("card");
    let card = Database::open(&card_wal, vec![card_dbd]).unwrap();
    let mut card_session = Session::new(card.clone(), card_psb).unwrap();
    let card_key = [0x12, 0x34, 0x56, 0x78, 0x90, 0x1c];
    let mut root = vec![0x40; 100];
    root[..6].copy_from_slice(&card_key);
    card_session
        .write(0, WriteOp::Isrt, "PAUTSUM0", Some(&root), &[b"PAUTSUM0 "])
        .unwrap();
    let mut child = vec![0x40; 200];
    child[..8].copy_from_slice(b"00000001");
    card_session
        .write(0, WriteOp::Isrt, "PAUTDTL1", Some(&child), &[b"PAUTDTL1 "])
        .unwrap();
    card_session.commit().unwrap();
    let card_ssa = qualified_ssa("PAUTSUM0", "ACCNTID", &card_key, b"EQ");
    assert_eq!(
        card_session
            .read_exec(0, ReadOp::Gu, &[&card_ssa])
            .unwrap()
            .status,
        Status::Ok
    );
    assert_eq!(
        card_session
            .read_exec(0, ReadOp::Gnp, &[b"PAUTDTL1 "])
            .unwrap()
            .record
            .unwrap()
            .data,
        child
    );
    card_session
        .write(0, WriteOp::Dlet, "PAUTDTL1", None, &[])
        .unwrap();
    card_session.read_exec(0, ReadOp::Gu, &[&card_ssa]).unwrap();
    card_session
        .write(0, WriteOp::Dlet, "PAUTSUM0", None, &[])
        .unwrap();
    card_session.commit().unwrap();
    assert_eq!(card.count(), 0);
    drop(card_session);
    drop(card);
    std::fs::remove_file(card_wal).unwrap();
}

#[test]
#[ignore = "requires Bank of Z reference assets"]
fn bank_account_and_history_share_one_unit_of_work() {
    let account_dbd = Dbd::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/DBD/ACCOUNT.asm",
    ))
    .unwrap();
    let history_dbd = Dbd::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/DBD/HISTORY.asm",
    ))
    .unwrap();
    let custaccs_dbd = Dbd::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/DBD/CUSTACCS.asm",
    ))
    .unwrap();
    let psb = Psb::from_file(reference(
        "reference/Bank-of-Z-main/src/base/ims/PSB/IBTRAN.asm",
    ))
    .unwrap();
    let wal = temporary_wal("bank-uow");
    let database = Database::open(&wal, vec![account_dbd, history_dbd, custaccs_dbd]).unwrap();
    let account_key = [1, 2, 3, 4, 5, 6, 7, 8];
    let mut account = vec![0x40; 25];
    account[..8].copy_from_slice(&account_key);
    let mut seed = database.begin();
    seed.insert(
        "ACCOUNT",
        "ACCOUNT",
        None,
        account.clone(),
        Duration::from_secs(1),
    )
    .unwrap();
    seed.commit().unwrap();
    let mut session = Session::new(database.clone(), psb).unwrap();
    let ssa = qualified_ssa("ACCOUNT", "ACCID", &account_key, b"= ");
    assert_eq!(
        session.read(0, ReadOp::Ghu, &[&ssa]).unwrap().status,
        Status::Ok
    );
    account[12] = 0xf1;
    session
        .write(0, WriteOp::Repl, "ACCOUNT", Some(&account), &[])
        .unwrap();
    let mut history = vec![0x40; 56];
    history[..8].copy_from_slice(b"TX000001");
    session
        .write(2, WriteOp::Isrt, "HISTORY", Some(&history), &[b"HISTORY  "])
        .unwrap();
    assert_eq!(
        database
            .get("ACCOUNT", None, "ACCOUNT", &account_key)
            .unwrap()
            .data[12],
        0x40
    );
    assert!(database
        .get("HISTORY", None, "HISTORY", b"TX000001")
        .is_none());
    session.commit().unwrap();
    assert_eq!(
        database
            .get("ACCOUNT", None, "ACCOUNT", &account_key)
            .unwrap()
            .data[12],
        0xf1
    );
    assert_eq!(
        database
            .get("HISTORY", None, "HISTORY", b"TX000001")
            .unwrap()
            .data,
        history
    );
    drop(session);
    drop(database);
    let reopened = Database::open(
        &wal,
        vec![
            Dbd::from_file(reference(
                "reference/Bank-of-Z-main/src/base/ims/DBD/ACCOUNT.asm",
            ))
            .unwrap(),
            Dbd::from_file(reference(
                "reference/Bank-of-Z-main/src/base/ims/DBD/HISTORY.asm",
            ))
            .unwrap(),
            Dbd::from_file(reference(
                "reference/Bank-of-Z-main/src/base/ims/DBD/CUSTACCS.asm",
            ))
            .unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(reopened.count(), 2);
    drop(reopened);
    std::fs::remove_file(wal).unwrap();
}
