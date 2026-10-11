use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

use cobcics::server::WorkerRouting;

fn usage() -> &'static str {
    "usage: cobcicsd --bms FILE [--map NAME] [--csd FILE --csd-list LIST --dataset-profile FILE --program NAME=COBOL-EXE ...] [--check-csd] [--tn3270e] [--nvt] [--printer-queue NAME --printer-input host|utf8] [--indfile-root DIR] [--tls-cert PEM --tls-key PEM [--starttls]] [--charset IBM-037|IBM-930|IBM-939|IBM-1390|IBM-1399] [--listen 127.0.0.1:3270] [--applid ID] [--worker COBOL-EXE] [--transid ID --route ID=COBOL-EXE --map-route ID=BMS-FILE#MAP ...]"
}

fn starttls(stream: &mut std::net::TcpStream) -> Result<(), String> {
    // Require an immediate upgrade. No application data is sent in cleartext,
    // and a refusal closes the connection rather than downgrading it.
    stream
        .write_all(&[0xff, 0xfd, 46])
        .map_err(|err| format!("STARTTLS offer: {err}"))?;
    let mut response = [0_u8; 9];
    stream
        .read_exact(&mut response)
        .map_err(|err| format!("STARTTLS negotiation: {err}"))?;
    if response != [0xff, 0xfb, 46, 0xff, 0xfa, 46, 1, 0xff, 0xf0] {
        return Err("client did not accept Telnet STARTTLS".to_owned());
    }
    stream
        .write_all(&[0xff, 0xfa, 46, 1, 0xff, 0xf0])
        .map_err(|err| format!("STARTTLS FOLLOWS: {err}"))?;
    Ok(())
}

fn tls_config(cert: &str, key: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    let certificates: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert)
        .map_err(|err| format!("TLS certificate {cert}: {err}"))?
        .collect::<Result<_, _>>()
        .map_err(|err| format!("TLS certificate {cert}: {err}"))?;
    if certificates.is_empty() {
        return Err(format!("TLS certificate {cert}: no certificates"));
    }
    let private_key =
        PrivateKeyDer::from_pem_file(key).map_err(|err| format!("TLS private key {key}: {err}"))?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|err| format!("TLS configuration: {err}"))?;
    Ok(Arc::new(config))
}

fn select_map(source: &str, name: Option<&str>) -> Result<cobcics::bms::Map, String> {
    let maps = cobcics::bms::parse(source)?;
    match name {
        Some(name) => maps
            .into_iter()
            .find(|map| map.name == name)
            .ok_or_else(|| format!("BMS map not found: {name}")),
        None if maps.len() == 1 => Ok(maps.into_iter().next().unwrap()),
        None => Err("BMS source has multiple maps; specify a map name".to_owned()),
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut bms = None;
    let mut map_name = None;
    let mut listen = "127.0.0.1:3270".to_owned();
    let mut tls_cert = None;
    let mut tls_key = None;
    let mut use_starttls = false;
    let mut printer_input_seen = false;
    let mut csd_path = None;
    let mut csd_list = None;
    let mut dataset_profile = None;
    let mut programs = HashMap::new();
    let mut check_csd = false;
    let mut csd_files = 0;
    let mut routing = WorkerRouting::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bms" => bms = args.next(),
            "--csd" => csd_path = args.next(),
            "--csd-list" => csd_list = args.next(),
            "--dataset-profile" => dataset_profile = args.next(),
            "--check-csd" => check_csd = true,
            "--program" => {
                let value = args.next().ok_or_else(|| usage().to_owned())?;
                let (name, path) = value
                    .split_once('=')
                    .ok_or("--program requires NAME=COBOL-EXE")?;
                if name.is_empty() || path.is_empty() {
                    return Err("--program requires NAME=COBOL-EXE".to_owned());
                }
                programs.insert(name.to_ascii_uppercase(), PathBuf::from(path));
            }
            "--map" => map_name = args.next(),
            "--tn3270e" => routing.tn3270e = true,
            "--nvt" => routing.nvt = true,
            "--indfile-root" => {
                routing.indfile_root = Some(PathBuf::from(
                    args.next().ok_or_else(|| usage().to_owned())?,
                ))
            }
            "--printer-queue" => {
                let name = args.next().ok_or_else(|| usage().to_owned())?;
                if name.is_empty()
                    || name.len() > 8
                    || !name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return Err(
                        "--printer-queue requires an alphanumeric name of at most 8 bytes"
                            .to_owned(),
                    );
                }
                let mut queue = [b' '; 8];
                queue[..name.len()].copy_from_slice(name.to_ascii_uppercase().as_bytes());
                routing.printer_queue = Some(queue);
                routing.tn3270e = true;
            }
            "--printer-input" => {
                printer_input_seen = true;
                routing.printer_utf8 = match args.next().as_deref() {
                    Some("host") => false,
                    Some("utf8") => true,
                    _ => return Err("--printer-input requires host or utf8".to_owned()),
                };
            }
            "--tls-cert" => tls_cert = args.next(),
            "--tls-key" => tls_key = args.next(),
            "--starttls" => use_starttls = true,
            "--listen" => listen = args.next().ok_or_else(|| usage().to_owned())?,
            "--charset" => {
                let name = args.next().ok_or_else(|| usage().to_owned())?;
                routing.codec = cobcics::codec::HostCodec::parse(&name)?;
            }
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
                    .ok_or_else(|| "--map-route requires ID=BMS-FILE#MAP".to_owned())?;
                if path.is_empty() {
                    return Err("--map-route requires a BMS file".to_owned());
                }
                let (path, map_name) = match path.split_once('#') {
                    Some((path, name)) if !path.is_empty() && !name.is_empty() => {
                        (path, Some(name))
                    }
                    Some(_) => return Err("--map-route requires BMS-FILE#MAP".to_owned()),
                    None => (path, None),
                };
                let source = fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
                let map = select_map(&source, map_name)?;
                let name = cobcics::worker::transid(name)?;
                if routing.maps.insert(name, map).is_some() {
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
    if csd_list.is_some() && csd_path.is_none() || dataset_profile.is_some() && csd_path.is_none() {
        return Err("--csd-list and --dataset-profile require --csd".to_owned());
    }
    if let Some(path) = csd_path {
        let catalog = cobcics::csd::Catalog::read(PathBuf::from(path).as_path())?;
        for (transid, program) in catalog.transaction_programs(csd_list.as_deref())? {
            if let Some(worker) = programs.get(&program) {
                let transid = cobcics::worker::transid(&transid)?;
                routing
                    .routes
                    .entry(transid)
                    .or_insert_with(|| worker.clone());
            }
        }
        if let Some(profile) = dataset_profile {
            let (root, datasets) =
                cobcics::csd::profile_datasets(PathBuf::from(profile).as_path())?;
            for (name, path) in
                cobcics::csd::resolved_files(&catalog, csd_list.as_deref(), &root, &datasets)?
            {
                if name.is_empty()
                    || name.len() > 8
                    || !name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return Err(format!("invalid CSD FILE name {name}"));
                }
                std::env::set_var(format!("COBCICS_VSAM_{name}"), path);
                csd_files += 1;
            }
        }
    }
    if !routing.routes.is_empty()
        && routing.default_worker.is_none()
        && !routing.routes.contains_key(&routing.initial_transid)
    {
        return Err("initial TRANSID has no worker route".to_owned());
    }
    let bms = bms.ok_or_else(|| usage().to_owned())?;
    let tls = match (tls_cert.as_deref(), tls_key.as_deref()) {
        (Some(cert), Some(key)) => Some(tls_config(cert, key)?),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key must be provided together".to_owned()),
    };
    if use_starttls && tls.is_none() {
        return Err("--starttls requires --tls-cert and --tls-key".to_owned());
    }
    if routing.nvt && routing.printer_queue.is_some() {
        return Err("--nvt and --printer-queue require separate listeners".to_owned());
    }
    if printer_input_seen && routing.printer_queue.is_none() {
        return Err("--printer-input requires --printer-queue".to_owned());
    }
    let source = fs::read_to_string(&bms).map_err(|err| format!("{bms}: {err}"))?;
    let map = select_map(&source, map_name.as_deref())?;
    if check_csd {
        println!("CSD routes={} files={}", routing.routes.len(), csd_files);
        return Ok(());
    }
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
    cobcics::schedule::start(routing.clone(), map.clone());
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let mut map = map.clone();
                let routing = routing.clone();
                let tls = tls.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let outcome = if let Some(config) = tls {
                        let upgrade = if use_starttls {
                            starttls(&mut stream)
                        } else {
                            Ok(())
                        };
                        match upgrade.and_then(|()| {
                            rustls::ServerConnection::new(config)
                                .map_err(|err| format!("TLS connection: {err}"))
                        }) {
                            Ok(connection) => {
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
                                let mut secure = rustls::StreamOwned::new(connection, stream);
                                cobcics::server::serve_connection_with_io(
                                    &mut secure,
                                    &mut map,
                                    &routing,
                                )
                            }
                            Err(err) => Err(err),
                        }
                    } else {
                        cobcics::server::serve_connection_with_routing(
                            &mut stream,
                            &mut map,
                            &routing,
                        )
                    };
                    if let Err(err) = outcome {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_one_map_from_a_mapset() {
        let source = "SSMAP DFHMSD TYPE=MAP,MODE=INOUT\n\
SSMAPC1 DFHMDI SIZE=(24,80)\n\
CUST DFHMDF POS=(1,1),LENGTH=4\n\
SSMAPP1 DFHMDI SIZE=(24,80)\n\
POLICY DFHMDF POS=(1,1),LENGTH=4\n";
        assert_eq!(select_map(source, Some("SSMAPP1")).unwrap().name, "SSMAPP1");
        assert!(select_map(source, None).is_err());
        assert!(select_map(source, Some("MISSING")).is_err());
    }
}
