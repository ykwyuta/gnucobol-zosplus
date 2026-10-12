use imsdb::tm::Queue;
use std::sync::Arc;

fn name(value: &str) -> [u8; 8] {
    let mut result = [b' '; 8];
    result[..value.len()].copy_from_slice(value.as_bytes());
    result
}

fn message(body: &[u8]) -> Vec<u8> {
    let mut data = Vec::from(((body.len() + 4) as u16).to_be_bytes());
    data.extend_from_slice(&[0, 0]);
    data.extend_from_slice(body);
    data
}

#[test]
fn tm_queue_commit_rollback_and_persistence() {
    let path = std::env::temp_dir().join(format!(
        "imsdb-tm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let queue = Queue::open(&path).unwrap();
    let input = message(b"HELLO");
    let id = queue
        .enqueue(name("IBSCUDAT"), name("TERM0001"), &input)
        .unwrap();
    let read = queue.get(name("IBSCUDAT"), 12).unwrap().unwrap();
    assert_eq!(read.id, id);
    assert_eq!(read.source, name("TERM0001"));
    assert_eq!(read.data, input);
    assert!(queue.get(name("IBSCUDAT"), 34).unwrap().is_none());
    queue.rollback(12, &[id]).unwrap();
    assert_eq!(queue.get(name("IBSCUDAT"), 34).unwrap().unwrap().id, id);
    let response = message(b"DONE");
    queue
        .complete(
            34,
            &[id],
            &[(name("TERM0001"), name("IBSCUDAT"), response.clone())],
        )
        .unwrap();
    assert!(queue.get(name("IBSCUDAT"), 12).unwrap().is_none());
    drop(queue);
    let reopened = Queue::open(&path).unwrap();
    assert_eq!(
        reopened.pending(name("TERM0001")).unwrap()[0].data,
        response
    );
    assert!(reopened.complete(34, &[id], &[]).is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tm_queue_concurrent_producers_do_not_lose_messages() {
    let path = std::env::temp_dir().join(format!(
        "imsdb-tm-par-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let queue = Arc::new(Queue::open(&path).unwrap());
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let queue = queue.clone();
            std::thread::spawn(move || {
                for _ in 0..10 {
                    queue
                        .enqueue(name("TRAN"), name("TERM"), &message(b"X"))
                        .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let entries = queue.pending(name("TRAN")).unwrap();
    assert_eq!(entries.len(), 40);
    let unique: std::collections::HashSet<_> = entries.iter().map(|m| m.id).collect();
    assert_eq!(unique.len(), 40);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tm_ffi_accepts_empty_commit_lists() {
    let path = std::env::temp_dir().join(format!("imsdb-tm-ffi-{}", std::process::id()));
    let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    let mut error = [0_i8; 128];
    let result = unsafe {
        imsdb::tm::imstm_complete(
            c_path.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(result, 0);
    let result = unsafe {
        imsdb::tm::imstm_rollback(
            c_path.as_ptr(),
            std::ptr::null(),
            0,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    assert_eq!(result, 0);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tm_preserves_ebcdic_dbcs_and_binary_bytes() {
    let path = std::env::temp_dir().join(format!("imsdb-tm-raw-{}", std::process::id()));
    let queue = Queue::open(&path).unwrap();
    let raw = [0, 11, 0, 0, 0x0e, 0x42, 0x43, 0x0f, 0, 0xff, 0x40];
    queue
        .enqueue(name("JPNTRAN"), name("JPNTERM"), &raw)
        .unwrap();
    assert_eq!(queue.get(name("JPNTRAN"), 99).unwrap().unwrap().data, raw);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tm_crash_helper() {
    let Ok(path) = std::env::var("IMS_TM_CRASH_PATH") else {
        return;
    };
    let queue = Queue::open(path).unwrap();
    assert!(queue.get(name("CRASH"), 100).unwrap().is_some());
    std::process::exit(0);
}

#[test]
fn tm_reclaims_input_after_process_crash() {
    let path = std::env::temp_dir().join(format!("imsdb-tm-crash-{}", std::process::id()));
    let queue = Queue::open(&path).unwrap();
    let id = queue
        .enqueue(name("CRASH"), name("TERM"), &message(b"X"))
        .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tm_crash_helper")
        .arg("--nocapture")
        .env("IMS_TM_CRASH_PATH", &path)
        .status()
        .unwrap();
    assert!(child.success());
    assert_eq!(queue.get(name("CRASH"), 200).unwrap().unwrap().id, id);
    queue.complete(200, &[id], &[]).unwrap();
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tm_rejects_corrupt_queue_snapshot() {
    let path = std::env::temp_dir().join(format!("imsdb-tm-corrupt-{}", std::process::id()));
    let queue = Queue::open(&path).unwrap();
    queue
        .enqueue(name("CORRUPT"), name("TERM"), &message(b"SAFE"))
        .unwrap();
    let file = path.join("queue.dat");
    let mut bytes = std::fs::read(&file).unwrap();
    *bytes.last_mut().unwrap() ^= 0x01;
    std::fs::write(&file, bytes).unwrap();
    assert!(queue
        .pending(name("CORRUPT"))
        .unwrap_err()
        .contains("checksum"));
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn tp_pcb_metadata_requires_destination_or_modifiability() {
    let source = "         PCB TYPE=TP,LTERM=TERMALT,MODIFY=NO\n         PSBGEN PSBNAME=TMTEST,LANG=COBOL\n         END\n";
    let psb = imsdb::catalog::parse_psb(source).unwrap();
    assert_eq!(psb.pcbs[0].lterm.as_deref(), Some("TERMALT"));
    assert!(!psb.pcbs[0].modifiable);
    psb.validate(&[]).unwrap();
    let invalid =
        "         PCB TYPE=TP,MODIFY=NO\n         PSBGEN PSBNAME=TMTEST,LANG=COBOL\n         END\n";
    assert!(imsdb::catalog::parse_psb(invalid)
        .unwrap()
        .validate(&[])
        .is_err());
    let invalid_modifier = source.replace("MODIFY=NO", "MODIFY=MAYBE");
    assert!(imsdb::catalog::parse_psb(&invalid_modifier).is_err());
}
