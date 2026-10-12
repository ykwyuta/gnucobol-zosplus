use std::env;
use std::fs;
use zosplus_preprocessors::imsdli;

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .ok_or("usage: imsdli-preprocess INPUT OUTPUT --dbd DBD")?;
    let output = args
        .next()
        .ok_or("usage: imsdli-preprocess INPUT OUTPUT --dbd DBD")?;
    if args.next().as_deref() != Some("--dbd") {
        return Err("usage: imsdli-preprocess INPUT OUTPUT --dbd DBD".into());
    }
    let dbd = args.next().ok_or("missing DBD name")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let source = fs::read_to_string(&input).map_err(|error| format!("{input}: {error}"))?;
    let rendered = imsdli::translate(&source, &dbd).map_err(|error| format!("{input}: {error}"))?;
    fs::write(&output, rendered).map_err(|error| format!("{output}: {error}"))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
