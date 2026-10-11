use imsdb::csv_load::encode_root_csv;
use imsdb::unload::parse_dfsurgu0;
use imsdb::{Database, Dbd, Psb};
use std::path::Path;
use std::time::Duration;

fn load_dbds(path: &str) -> Result<Vec<Dbd>, String> {
    let path = Path::new(path);
    if path.is_file() {
        return Ok(vec![Dbd::from_file(path)?]);
    }
    if !path.is_dir() {
        return Err(format!("{} is not a DBD file or directory", path.display()));
    }
    let mut files = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    files.sort();
    let dbds = files
        .into_iter()
        .filter(|file| {
            file.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("dbd") || ext.eq_ignore_ascii_case("asm")
            })
        })
        .map(Dbd::from_file)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(dbds)
}

fn hex(input: &str) -> Result<Vec<u8>, String> {
    let input = input.trim();
    if !input.len().is_multiple_of(2) {
        return Err("hex string must have even length".into());
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|e| e.to_string())?;
            u8::from_str_radix(pair, 16).map_err(|e| e.to_string())
        })
        .collect()
}
fn display_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, command, files @ ..] if command == "inspect" && !files.is_empty() => {
            let mut dbds = Vec::new();
            let mut psbs = Vec::new();
            for file in files {
                let source = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
                if source.lines().any(|line| line.split_whitespace().any(|word| word.eq_ignore_ascii_case("PSBGEN"))) {
                    psbs.push(Psb::from_file(file)?);
                } else {
                    dbds.push(Dbd::from_file(file)?);
                }
            }
            for psb in &psbs { psb.validate(&dbds)?; }
            for dbd in &dbds {
                println!("DBD {} access={} segments={} logical_children={}", dbd.name, dbd.access, dbd.segments.len(), dbd.logical_children.len());
                for seg in &dbd.segments {
                    println!("  {} parent={} bytes={}..{} key={}", seg.name,
                        seg.parent.as_deref().unwrap_or("0"), seg.min_bytes, seg.max_bytes,
                        seg.key().map(|f| f.name.as_str()).unwrap_or("-"));
                }
            }
            for psb in &psbs { println!("PSB {} pcbs={}", psb.name, psb.pcbs.len()); }
        }
        [_, command, wal, dbd_file, segment, data] if command == "put-root" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let mut tx = db.begin();
            let id = tx.insert(&name, segment, None, hex(data)?, Duration::from_secs(5)).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            println!("inserted {id}");
        }
        [_, command, wal, dbd_file, segment, key] if command == "get-root" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let record = db.get(&name, None, segment, &hex(key)?).ok_or("record not found")?;
            println!("{} {}", record.id, display_hex(&record.data));
        }
        [_, command, wal, dbd_dir, index_dbd, key] if command == "get-secondary" => {
            let db = Database::open(wal, load_dbds(dbd_dir)?).map_err(|e| e.to_string())?;
            let record = db.secondary_lookup(index_dbd, &hex(key)?).map_err(|e| e.to_string())?
                .ok_or("secondary key not found")?;
            println!("{} {}", record.id, display_hex(&record.data));
        }
        [_, command, wal, dbd_file] if command == "count" => {
            let db = Database::open(wal, load_dbds(dbd_file)?).map_err(|e| e.to_string())?;
            println!("{}", db.count());
        }
        [_, command, wal, dbd_file] if command == "latest-checkpoint" => {
            let db = Database::open(wal, load_dbds(dbd_file)?).map_err(|e| e.to_string())?;
            if let Some(id) = db.latest_checkpoint() {
                println!("{}", String::from_utf8_lossy(&id));
            } else {
                println!("none");
            }
        }
        [_, command, wal, dbd_file, segment, input] if command == "import-fixed" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let definition = dbd.segment(segment).ok_or("unknown segment")?;
            if definition.parent.is_some() || definition.min_bytes != definition.max_bytes {
                return Err("import-fixed requires a fixed-length root segment".into());
            }
            let record_size = definition.min_bytes;
            if record_size == 0 {
                return Err("record length must be positive".into());
            }
            let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
            if bytes.len() % record_size != 0 {
                return Err(format!("input length {} is not a multiple of record length {record_size}", bytes.len()));
            }
            let mut records: Vec<Vec<u8>> = bytes.chunks_exact(record_size).map(Vec::from).collect();
            for record in &records {
                definition.validate_data(record)?;
            }
            records.sort_by(|a, b| {
                definition.extract_key(a).expect("validated key")
                    .cmp(definition.extract_key(b).expect("validated key"))
            });
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let mut tx = db.begin();
            for record in records {
                tx.insert(&name, segment, None, record, Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
            }
            let count = bytes.len() / record_size;
            tx.commit().map_err(|e| e.to_string())?;
            println!("imported {count} records");
        }
        [_, command, wal, dbd_file, input] if command == "import-dfsurgu0" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
            let records = parse_dfsurgu0(&bytes, &dbd)?;
            let count = records.len();
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let mut tx = db.begin();
            let mut ids = Vec::with_capacity(count);
            for record in records {
                let parent = record.parent_index.map(|index| ids[index]);
                let id = tx.insert(&name, &record.segment, parent,
                                   record.data, Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
                ids.push(id);
            }
            tx.commit().map_err(|e| e.to_string())?;
            println!("imported {count} DFSURGU0 segments");
        }
        [_, command, dbd_file, input, root_output, child_output]
            if command == "split-dfsurgu0" => {
            if root_output == child_output {
                return Err("root and child output files must differ".into());
            }
            let dbd = Dbd::from_file(dbd_file)?;
            if dbd.segments.len() != 2 || dbd.segments[0].parent.is_some() ||
                dbd.segments[1].parent.as_deref() != Some(&dbd.segments[0].name)
            {
                return Err("split-dfsurgu0 requires one root and one direct child".into());
            }
            let records = parse_dfsurgu0(&std::fs::read(input)
                .map_err(|e| e.to_string())?, &dbd)?;
            let root_def = &dbd.segments[0];
            let mut roots = Vec::new();
            let mut children = Vec::new();
            let mut root_count = 0;
            let mut child_count = 0;
            for record in &records {
                if let Some(parent_index) = record.parent_index {
                    children.extend_from_slice(root_def.extract_key(&records[parent_index].data)?);
                    children.extend_from_slice(&record.data);
                    child_count += 1;
                } else {
                    roots.extend_from_slice(&record.data);
                    root_count += 1;
                }
            }
            std::fs::write(root_output, roots).map_err(|e| e.to_string())?;
            std::fs::write(child_output, children).map_err(|e| e.to_string())?;
            println!("split {root_count} roots and {child_count} children");
        }
        [_, command, wal, dbd_file, segment, input] if command == "import-csv" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let source = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
            let mut records = encode_root_csv(&dbd, segment, &source)?;
            let definition = dbd.segment(segment).ok_or("unknown segment")?;
            records.sort_by(|a, b| {
                definition.extract_key(a).expect("validated key")
                    .cmp(definition.extract_key(b).expect("validated key"))
            });
            let count = records.len();
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let mut tx = db.begin();
            for record in records {
                tx.insert(&name, segment, None, record, Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            println!("imported {count} records");
        }
        [_, command, wal, dbd_dir, data_dir] if command == "import-csv-bundle" => {
            let mut dbds = Vec::new();
            for entry in std::fs::read_dir(dbd_dir).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("asm") || e.eq_ignore_ascii_case("dbd")) {
                    dbds.push(Dbd::from_file(path)?);
                }
            }
            if dbds.is_empty() {
                return Err("DBD directory contains no .asm or .dbd files".into());
            }
            dbds.sort_by(|a, b| a.name.cmp(&b.name));
            let mut records: Vec<(String, String, Vec<u8>, Vec<u8>)> = Vec::new();
            for dbd in &dbds {
                let roots: Vec<_> = dbd.segments.iter().filter(|s| s.parent.is_none()).collect();
                if roots.len() != 1 {
                    return Err(format!("{}: CSV bundle requires exactly one root segment", dbd.name));
                }
                let segment = roots[0];
                let path = std::path::Path::new(data_dir).join(format!("{}.data", dbd.name));
                let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                for record in encode_root_csv(dbd, &segment.name, &source)? {
                    let key = segment.extract_key(&record)?.to_vec();
                    records.push((dbd.name.clone(), segment.name.clone(), key, record));
                }
            }
            records.sort_by(|a, b| (&a.0, &a.2).cmp(&(&b.0, &b.2)));
            let count = records.len();
            let dbd_count = dbds.len();
            let db = Database::open(wal, dbds).map_err(|e| e.to_string())?;
            let mut tx = db.begin();
            for (name, segment, _, record) in records {
                tx.insert(&name, &segment, None, record, Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            println!("imported {count} records across {dbd_count} DBDs");
        }
        [_, command, wal, dbd_file, segment, output] if command == "export-fixed" => {
            let dbd = Dbd::from_file(dbd_file)?;
            let definition = dbd.segment(segment).ok_or("unknown segment")?;
            if definition.parent.is_some() || definition.min_bytes != definition.max_bytes {
                return Err("export-fixed requires a fixed-length root segment".into());
            }
            if definition.min_bytes == 0 {
                return Err("record length must be positive".into());
            }
            let name = dbd.name.clone();
            let db = Database::open(wal, vec![dbd]).map_err(|e| e.to_string())?;
            let records = db.children(&name, None, segment);
            let mut file = std::fs::File::create(output).map_err(|e| e.to_string())?;
            use std::io::Write;
            for record in &records { file.write_all(&record.data).map_err(|e| e.to_string())?; }
            file.sync_all().map_err(|e| e.to_string())?;
            println!("exported {} records", records.len());
        }
        _ => return Err("usage: imsdbctl inspect DBD... [PSB...] | put-root WAL DBD SEGMENT RECORD_HEX | get-root WAL DBD SEGMENT KEY_HEX | get-secondary WAL DBD_DIR INDEX_DBD KEY_HEX | count WAL DBD | latest-checkpoint WAL DBD | import-fixed WAL DBD SEGMENT FILE | import-dfsurgu0 WAL DBD FILE | split-dfsurgu0 DBD FILE ROOT_OUT CHILD_OUT | import-csv WAL DBD SEGMENT FILE | import-csv-bundle WAL DBD_DIR DATA_DIR | export-fixed WAL DBD SEGMENT FILE".into()),
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("imsdbctl: {e}");
        std::process::exit(1);
    }
}
