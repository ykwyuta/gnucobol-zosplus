use std::env;
use std::fs;
use std::path::PathBuf;
use zosplus_preprocessors::sql;

fn run() -> Result<(), String> {
    let mut paths = Vec::new();
    let mut include_dirs = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-I" | "--include-dir" => {
                include_dirs.push(PathBuf::from(args.next().ok_or("-I requires a directory")?))
            }
            _ if arg.starts_with("-I") && arg.len() > 2 => {
                include_dirs.push(PathBuf::from(&arg[2..]))
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.len() != 2 {
        return Err("usage: db2sql-preprocess [-I DIR] SOURCE OUTPUT".into());
    }
    let source = fs::read_to_string(&paths[0])
        .map_err(|error| format!("{}: {error}", paths[0].display()))?;
    include_dirs.insert(
        0,
        paths[0]
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf(),
    );
    let rendered = sql::translate(&source, &include_dirs, &paths[0])
        .map_err(|error| format!("{}: {error}", paths[0].display()))?;
    fs::write(&paths[1], rendered).map_err(|error| format!("{}: {error}", paths[1].display()))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
