use std::env;
use std::fs;

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let source = args
        .next()
        .ok_or("usage: cobcics-mapgen BMS-FILE OUTPUT-COPYBOOK")?;
    let destination = args
        .next()
        .ok_or("usage: cobcics-mapgen BMS-FILE OUTPUT-COPYBOOK")?;
    if args.next().is_some() {
        return Err("usage: cobcics-mapgen BMS-FILE OUTPUT-COPYBOOK".to_owned());
    }
    let bms = fs::read_to_string(&source).map_err(|err| format!("{source}: {err}"))?;
    let maps = cobcics::bms::parse(&bms)?;
    let copybook = cobcics::symbolic::generate(&maps)?;
    fs::write(&destination, copybook).map_err(|err| format!("{destination}: {err}"))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("cobcics-mapgen: {error}");
        std::process::exit(1);
    }
}
