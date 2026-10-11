use imsdb::{Database, Dbd};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("imsdb-cli-{}-{stamp}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    dir
}

fn command(args: &[&std::path::Path]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_imsdbctl"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn fixed_record_import_export_preserves_ebcdic_and_binary_bytes() {
    let dir = temp_dir();
    let dbd = dir.join("test.dbd");
    let wal = dir.join("test.wal");
    let input = dir.join("input.bin");
    let output = dir.join("output.bin");
    let duplicate = dir.join("duplicate.bin");
    let partial = dir.join("partial.bin");
    fs::write(
        &dbd,
        " DBD NAME=TEST,ACCESS=(HIDAM,VSAM)\n SEGM NAME=ROOT,PARENT=0,BYTES=6\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n DBDGEN\n END\n",
    )
    .unwrap();
    // Two-byte packed keys; the payload contains EBCDIC-like and arbitrary binary bytes.
    let lower = [0x01, 0x2c, 0x42, 0x0e, 0x43, 0x0f];
    let upper = [0x99, 0x9c, 0x5a, 0x00, 0xff, 0x40];
    fs::write(&input, [upper, lower].concat()).unwrap();
    let result = command(&[
        "import-fixed".as_ref(),
        wal.as_path(),
        dbd.as_path(),
        "ROOT".as_ref(),
        input.as_path(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = command(&[
        "export-fixed".as_ref(),
        wal.as_path(),
        dbd.as_path(),
        "ROOT".as_ref(),
        output.as_path(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(&output).unwrap(), [lower, upper].concat());

    fs::write(&duplicate, [lower, lower].concat()).unwrap();
    let result = command(&[
        "import-fixed".as_ref(),
        wal.as_path(),
        dbd.as_path(),
        "ROOT".as_ref(),
        duplicate.as_path(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate"));
    fs::write(&partial, [0x01, 0x2c, 0x42]).unwrap();
    let result = command(&[
        "import-fixed".as_ref(),
        wal.as_path(),
        dbd.as_path(),
        "ROOT".as_ref(),
        partial.as_path(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("multiple"));
    let result = command(&[
        "export-fixed".as_ref(),
        wal.as_path(),
        dbd.as_path(),
        "ROOT".as_ref(),
        output.as_path(),
    ]);
    assert!(result.status.success());
    assert_eq!(fs::read(&output).unwrap(), [lower, upper].concat());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn csv_bundle_validates_every_dataset_before_opening_wal() {
    let dir = temp_dir();
    let definitions = dir.join("DBD");
    let data = dir.join("LoadData");
    fs::create_dir(&definitions).unwrap();
    fs::create_dir(&data).unwrap();
    let wal = dir.join("bundle.wal");
    for name in ["FIRST", "SECOND"] {
        fs::write(
            definitions.join(format!("{name}.asm")),
            format!(" DBD NAME={name},ENCODING=Cp1047,ACCESS=(HDAM,OSAM)\n SEGM NAME={name},PARENT=0,BYTES=5\n FIELD NAME=(ID,SEQ,U),START=1,BYTES=4,DATATYPE=INT\n FIELD NAME=CODE,START=5,BYTES=1,DATATYPE=CHAR\n DBDGEN\n END\n"),
        ).unwrap();
    }
    fs::write(data.join("FIRST.data"), "1,\"A\"\n").unwrap();
    fs::write(data.join("SECOND.data"), "2\n").unwrap();
    let result = command(&[
        "import-csv-bundle".as_ref(),
        wal.as_path(),
        definitions.as_path(),
        data.as_path(),
    ]);
    assert!(!result.status.success());
    assert!(!wal.exists());

    fs::write(data.join("SECOND.data"), "2,\"B\"\n").unwrap();
    let result = command(&[
        "import-csv-bundle".as_ref(),
        wal.as_path(),
        definitions.as_path(),
        data.as_path(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("2 records across 2 DBDs"));
    let dbds = ["FIRST", "SECOND"]
        .map(|name| Dbd::from_file(definitions.join(format!("{name}.asm"))).unwrap());
    let database = Database::open(&wal, dbds.to_vec()).unwrap();
    assert_eq!(database.count(), 2);
    assert_eq!(
        database
            .get("FIRST", None, "FIRST", &1i32.to_be_bytes())
            .unwrap()
            .data,
        vec![0, 0, 0, 1, 0xc1]
    );
    drop(database);
    fs::remove_dir_all(dir).unwrap();
}
