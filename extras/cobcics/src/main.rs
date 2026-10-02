use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

use cobcics::server::WorkerRouting;

fn usage() -> &'static str {
    "usage: cobcicsd --bms FILE [--map NAME] [--listen 127.0.0.1:3270] [--applid ID] [--worker COBOL-EXE] [--transid ID --route ID=COBOL-EXE --map-route ID=BMS-FILE ...]"
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut bms = None;
    let mut map_name = None;
    let mut listen = "127.0.0.1:3270".to_owned();
    let mut routing = WorkerRouting::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bms" => bms = args.next(),
            "--map" => map_name = args.next(),
            "--listen" => listen = args.next().ok_or_else(|| usage().to_owned())?,
            "--applid" => {
                let name = args.next().ok_or_else(|| usage().to_owned())?;
                routing.applid = cobcics::worker::applid(&name)?;
            }
            "--worker" => {
                routing.default_worker = Some(PathBuf::from(
                    args.next().ok_or_else(|| usage().to_owned())?,
                ));
            }
            "--transid" => {
                let name = args.next().ok_or_else(|| usage().to_owned())?;
                routing.initial_transid = cobcics::worker::transid(&name)?;
            }
            "--route" => {
                let value = args.next().ok_or_else(|| usage().to_owned())?;
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| "--route requires ID=COBOL-EXE".to_owned())?;
                if path.is_empty() {
                    return Err("--route requires a worker path".to_owned());
                }
                let name = cobcics::worker::transid(name)?;
                if routing.routes.insert(name, PathBuf::from(path)).is_some() {
                    return Err(format!("duplicate worker route for TRANSID {name:?}"));
                }
            }
            "--map-route" => {
                let value = args.next().ok_or_else(|| usage().to_owned())?;
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| "--map-route requires ID=BMS-FILE".to_owned())?;
                if path.is_empty() {
                    return Err("--map-route requires a BMS file".to_owned());
                }
                let source = fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
                let mut maps = cobcics::bms::parse(&source)?;
                if maps.len() != 1 {
                    return Err(format!("--map-route BMS file must contain one map: {path}"));
                }
                let name = cobcics::worker::transid(name)?;
                if routing.maps.insert(name, maps.remove(0)).is_some() {
                    return Err(format!("duplicate map route for TRANSID {name:?}"));
                }
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            _ => return Err(usage().to_owned()),
        }
    }
    if !routing.routes.is_empty()
        && routing.default_worker.is_none()
        && !routing.routes.contains_key(&routing.initial_transid)
    {
        return Err("initial TRANSID has no worker route".to_owned());
    }
    let bms = bms.ok_or_else(|| usage().to_owned())?;
    let source = fs::read_to_string(&bms).map_err(|err| format!("{bms}: {err}"))?;
    let maps = cobcics::bms::parse(&source)?;
    let map = match map_name {
        Some(name) => maps
            .into_iter()
            .find(|map| map.name == name)
            .ok_or_else(|| format!("BMS map not found: {name}"))?,
        None if maps.len() == 1 => maps.into_iter().next().unwrap(),
        None => return Err("BMS source has multiple maps; specify --map".to_owned()),
    };
    routing
        .maps
        .entry(routing.initial_transid)
        .or_insert_with(|| map.clone());
    let listener = TcpListener::bind(&listen).map_err(|err| format!("bind {listen}: {err}"))?;
    eprintln!(
        "cobcicsd: {}.{} listening at {}",
        map.mapset,
        map.name,
        listener.local_addr().unwrap()
    );
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let mut map = map.clone();
                let routing = routing.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    if let Err(err) = cobcics::server::serve_connection_with_routing(
                        &mut stream,
                        &mut map,
                        &routing,
                    ) {
                        eprintln!("cobcicsd: terminal session: {err}");
                    }
                });
            }
            Err(err) => eprintln!("cobcicsd: accept: {err}"),
        }
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("cobcicsd: {err}");
        std::process::exit(1);
    }
}
