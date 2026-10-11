use imsdb::call_adapter::PcbEncoding;
use imsdb::ffi::{
    imsdb_call, imsdb_checkpoint, imsdb_close, imsdb_commit, imsdb_open, imsdb_pcb_index, ImsSpan,
};
use imsdb::{Database, Dbd};
use std::ffi::CString;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn c_abi_opens_database_and_checks_buffer_lengths() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("imsdb-ffi-{}-{stamp}", std::process::id()));
    let definitions = root.join("DBD");
    fs::create_dir_all(&definitions).unwrap();
    let wal = root.join("test.wal");
    let dbd_file = definitions.join("TEST.asm");
    let psb_file = root.join("TEST.psb");
    fs::write(&dbd_file, " DBD NAME=TEST,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n DBDGEN\n END\n").unwrap();
    fs::write(&psb_file, " PCB TYPE=DB,DBDNAME=TEST,PROCOPT=AP,KEYLEN=2\n SENSEG NAME=ROOT,PARENT=0\n PSBGEN PSBNAME=TESTPSB,LANG=COBOL\n END\n").unwrap();
    let db = Database::open(&wal, vec![Dbd::from_file(&dbd_file).unwrap()]).unwrap();
    let mut tx = db.begin();
    tx.insert(
        "TEST",
        "ROOT",
        None,
        vec![0xc1, 0x40, 1, 2, 3, 4],
        Duration::from_secs(1),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);

    let wal_path = CString::new(wal.to_str().unwrap()).unwrap();
    let dbd_path = CString::new(definitions.to_str().unwrap()).unwrap();
    let psb_path = CString::new(psb_file.to_str().unwrap()).unwrap();
    let mut error = [0i8; 256];
    let handle = unsafe {
        imsdb_open(
            wal_path.as_ptr(),
            dbd_path.as_ptr(),
            psb_path.as_ptr(),
            1,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert!(
        !handle.is_null(),
        "{}",
        String::from_utf8_lossy(unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) }.to_bytes())
    );
    assert_eq!(unsafe { imsdb_pcb_index(handle, b"TEST".as_ptr(), 4) }, 0);
    assert_eq!(
        unsafe { imsdb_pcb_index(handle, b"MISSING".as_ptr(), 7) },
        -1
    );
    let mut ssa = PcbEncoding::Cp1047
        .encode_control(b"ROOT    (ID      EQ")
        .unwrap();
    ssa.extend_from_slice(&[0xc1, 0x40]);
    ssa.extend(PcbEncoding::Cp1047.encode_control(b")").unwrap());
    let spans = [ImsSpan {
        data: ssa.as_mut_ptr(),
        len: ssa.len(),
    }];
    let function = PcbEncoding::Cp1047.encode_control(b"GU  ").unwrap();
    let mut pcb = [0u8; 38];
    let mut io = [0u8; 6];
    let rc = unsafe {
        imsdb_call(
            handle,
            0,
            function.as_ptr(),
            function.len(),
            pcb.as_mut_ptr(),
            pcb.len(),
            io.as_mut_ptr(),
            io.len(),
            spans.as_ptr(),
            spans.len(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(rc, 0);
    assert_eq!(io, [0xc1, 0x40, 1, 2, 3, 4]);
    assert_eq!(&pcb[36..38], &[0xc1, 0x40]);
    let rc = unsafe {
        imsdb_call(
            handle,
            0,
            function.as_ptr(),
            function.len(),
            pcb.as_mut_ptr(),
            pcb.len(),
            pcb.as_mut_ptr(),
            6,
            spans.as_ptr(),
            spans.len(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(rc, -1);
    assert!(unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) }
        .to_str()
        .unwrap()
        .contains("overlap"));
    assert_eq!(
        unsafe { imsdb_commit(handle, error.as_mut_ptr(), error.len()) },
        0
    );
    assert_eq!(
        unsafe {
            imsdb_checkpoint(
                handle,
                b"CHECK001".as_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        },
        0
    );
    unsafe { imsdb_close(handle) };
    let db = Database::open(&wal, vec![Dbd::from_file(&dbd_file).unwrap()]).unwrap();
    assert_eq!(db.latest_checkpoint(), Some(*b"CHECK001"));
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
