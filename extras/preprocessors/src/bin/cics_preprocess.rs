use std::env;
use std::fs;
use std::path::Path;
use zosplus_preprocessors::cics;

fn run() -> Result<(), String> {
    let args: Vec<_> = env::args().skip(1).collect();
    let any_mapset = args.first().map(String::as_str) == Some("--any-mapset");
    let args = if any_mapset { &args[1..] } else { &args[..] };
    if args.len() != 3 {
        return Err("usage: cics-preprocess [--any-mapset] SOURCE COPYBOOK OUTPUT".into());
    }
    let source = fs::read_to_string(&args[0]).map_err(|error| format!("{}: {error}", args[0]))?;
    let copybook = fs::read_to_string(&args[1]).map_err(|error| format!("{}: {error}", args[1]))?;
    let mapset = Path::new(&args[1])
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let mapset = if any_mapset { "" } else { mapset };
    let maps =
        cics::maps_from_copybook(&copybook).map_err(|error| format!("{}: {error}", args[0]))?;
    let rendered =
        cics::translate(&source, &maps, mapset).map_err(|error| format!("{}: {error}", args[0]))?;
    fs::write(&args[2], rendered).map_err(|error| format!("{}: {error}", args[2]))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
