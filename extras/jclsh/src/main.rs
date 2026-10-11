use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod datasets;
mod ims;
mod jcl;
mod records;
mod utilities;
mod vsam_sql;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum DdKind {
    Inline(Vec<String>),
    Sysout,
    Dataset(Vec<(String, String)>),
    Dummy,
}

#[derive(Clone, Debug)]
struct Dd {
    name: String,
    line: usize,
    kind: DdKind,
    dcb: Option<String>,
}

#[derive(Clone, Debug)]
struct Step {
    name: String,
    line: usize,
    pgm: String,
    parm: Option<String>,
    dds: Vec<Dd>,
    conditions: Vec<jcl::CondSpec>,
    guards: Vec<jcl::IfGuard>,
}

#[derive(Clone, Debug)]
struct Job {
    name: String,
    attributes: String,
    steps: Vec<Step>,
    job_condition: Option<jcl::CondSpec>,
    joblib: Vec<Dd>,
}

fn identifier(value: &str, line: usize) -> Result<String> {
    if value.is_empty()
        || value.len() > 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$')
    {
        return Err(format!("line {line}: invalid JCL name {value:?}"));
    }
    Ok(value.to_ascii_uppercase())
}

#[derive(Debug)]
struct Profile {
    image: String,
    env_file: PathBuf,
    schema: String,
    system: String,
    plan: String,
    library: String,
    programs: HashMap<String, PathBuf>,
    ims_psbs: HashMap<String, PathBuf>,
    ims_dbd_dir: Option<PathBuf>,
    ims_wal: Option<PathBuf>,
    ims_cbltdli: Option<PathBuf>,
    proclibs: HashMap<String, PathBuf>,
    datasets: HashMap<String, PathBuf>,
    dataset_root: PathBuf,
    site: HashMap<String, String>,
    ld_library_path: String,
    vsam_backend: String,
    vsam_tables: HashMap<String, String>,
    vsam_postgres_container: String,
    vsam_postgres_user: String,
    vsam_postgres_database: String,
    vsam_db2_image: String,
    vsam_db2_helper: PathBuf,
}

fn read_profile(path: &Path) -> Result<Profile> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut values = HashMap::new();
    let mut programs = HashMap::new();
    let mut ims_psbs = HashMap::new();
    let mut proclibs = HashMap::new();
    let mut datasets = HashMap::new();
    let mut site = HashMap::new();
    let mut vsam_tables = HashMap::new();
    for (index, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("{}:{}: expected key=value", path.display(), index + 1))?;
        if let Some(name) = key.strip_prefix("program.") {
            let name = identifier(name, index + 1)?;
            if programs.insert(name, PathBuf::from(value.trim())).is_some() {
                return Err(format!(
                    "{}:{}: duplicate program",
                    path.display(),
                    index + 1
                ));
            }
        } else if let Some(name) = key.strip_prefix("ims_psb.") {
            let name = identifier(name, index + 1)?;
            if ims_psbs.insert(name, PathBuf::from(value.trim())).is_some() {
                return Err(format!(
                    "{}:{}: duplicate IMS PSB",
                    path.display(),
                    index + 1
                ));
            }
        } else if let Some(name) = key.strip_prefix("proclib.") {
            let name = name.to_ascii_uppercase();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".$#@".contains(&b))
            {
                return Err(format!(
                    "{}:{}: invalid proclib name",
                    path.display(),
                    index + 1
                ));
            }
            if proclibs.insert(name, PathBuf::from(value.trim())).is_some() {
                return Err(format!(
                    "{}:{}: duplicate proclib",
                    path.display(),
                    index + 1
                ));
            }
        } else if let Some(name) = key.strip_prefix("dataset.") {
            let name = name.to_ascii_uppercase();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".$#@".contains(&b))
            {
                return Err(format!(
                    "{}:{}: invalid dataset name",
                    path.display(),
                    index + 1
                ));
            }
            if datasets.insert(name, PathBuf::from(value.trim())).is_some() {
                return Err(format!(
                    "{}:{}: duplicate dataset",
                    path.display(),
                    index + 1
                ));
            }
        } else if let Some(name) = key.strip_prefix("site.") {
            let name = name.to_ascii_uppercase();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_".contains(&b))
                || value.contains(['\r', '\n'])
            {
                return Err(format!(
                    "{}:{}: invalid site placeholder",
                    path.display(),
                    index + 1
                ));
            }
            if site.insert(name, value.trim().to_string()).is_some() {
                return Err(format!(
                    "{}:{}: duplicate site placeholder",
                    path.display(),
                    index + 1
                ));
            }
        } else if let Some(name) = key.strip_prefix("vsam_table.") {
            let name = name.to_ascii_uppercase();
            let table = value.trim();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".$#@".contains(&b))
                || !vsam_sql::valid_table(table)
                || vsam_tables.insert(name, table.to_string()).is_some()
            {
                return Err(format!(
                    "{}:{}: invalid VSAM table mapping",
                    path.display(),
                    index + 1
                ));
            }
        } else if matches!(
            key,
            "image"
                | "env_file"
                | "db2_schema"
                | "dsn_system"
                | "dsn_plan"
                | "dsn_library"
                | "ld_library_path"
                | "dataset_root"
                | "vsam_backend"
                | "vsam_postgres_container"
                | "vsam_postgres_user"
                | "vsam_postgres_database"
                | "vsam_db2_image"
                | "vsam_db2_helper"
                | "ims_dbd_dir"
                | "ims_wal"
                | "ims_cbltdli"
        ) {
            if values
                .insert(key.to_string(), value.trim().to_string())
                .is_some()
            {
                return Err(format!(
                    "{}:{}: duplicate key {key}",
                    path.display(),
                    index + 1
                ));
            }
        } else {
            return Err(format!(
                "{}:{}: unknown profile key {key}",
                path.display(),
                index + 1
            ));
        }
    }
    let required = |key: &str| {
        values
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| format!("{}: missing profile key {key}", path.display()))
    };
    let vsam_backend = values
        .get("vsam_backend")
        .map(String::as_str)
        .unwrap_or("local");
    if !matches!(vsam_backend, "local" | "db2" | "postgres") {
        return Err(format!(
            "{}: vsam_backend requires local, db2, or postgres",
            path.display()
        ));
    }
    if vsam_backend == "local" && !vsam_tables.is_empty() {
        return Err(format!(
            "{}: local VSAM cannot use vsam_table mappings",
            path.display()
        ));
    }
    Ok(Profile {
        image: required("image")?,
        env_file: PathBuf::from(required("env_file")?),
        schema: required("db2_schema")?,
        system: required("dsn_system")?,
        plan: required("dsn_plan")?,
        library: required("dsn_library")?,
        programs,
        ims_psbs,
        ims_dbd_dir: values.get("ims_dbd_dir").map(PathBuf::from),
        ims_wal: values.get("ims_wal").map(PathBuf::from),
        ims_cbltdli: values.get("ims_cbltdli").map(PathBuf::from),
        proclibs,
        datasets,
        dataset_root: PathBuf::from(
            values
                .get("dataset_root")
                .map(String::as_str)
                .unwrap_or("build/jclsh/datasets"),
        ),
        site,
        ld_library_path: required("ld_library_path")?,
        vsam_backend: vsam_backend.to_string(),
        vsam_tables,
        vsam_postgres_container: values
            .get("vsam_postgres_container")
            .cloned()
            .unwrap_or_else(|| "gnucobol-pgsql-postgres-1".to_string()),
        vsam_postgres_user: values
            .get("vsam_postgres_user")
            .cloned()
            .unwrap_or_else(|| "gcob".to_string()),
        vsam_postgres_database: values
            .get("vsam_postgres_database")
            .cloned()
            .unwrap_or_else(|| "gcobpg".to_string()),
        vsam_db2_image: values
            .get("vsam_db2_image")
            .cloned()
            .unwrap_or_else(|| "gnucobol-db2-build".to_string()),
        vsam_db2_helper: PathBuf::from(
            values
                .get("vsam_db2_helper")
                .map(String::as_str)
                .unwrap_or("extras/jclsh/vsam_db2.py"),
        ),
    })
}

fn substitute_site(source: &str, site: &HashMap<String, String>) -> Result<String> {
    let mut output = String::new();
    let mut rest = source;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('>') else {
            output.push('<');
            rest = &rest[1..];
            continue;
        };
        let key = &rest[1..end];
        if !key.is_empty()
            && key.len() <= 32
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            let value = site
                .get(&key.to_ascii_uppercase())
                .ok_or_else(|| format!("unresolved site placeholder <{key}>"))?;
            output.push_str(value);
            rest = &rest[end + 1..];
        } else {
            output.push('<');
            rest = &rest[1..];
        }
    }
    output.push_str(rest);
    Ok(output)
}

fn load_catalog(profile: &Profile, workspace: &Path, expand_sites: bool) -> Result<jcl::Catalog> {
    let mut catalog = jcl::Catalog::new();
    for (library, relative) in &profile.proclibs {
        let root = fs::canonicalize(workspace.join(relative))
            .map_err(|error| format!("proclib {}: {error}", relative.display()))?;
        if !root.starts_with(workspace) || !root.is_dir() {
            return Err(format!(
                "proclib {} must be a directory inside workspace",
                relative.display()
            ));
        }
        let mut members = HashMap::new();
        for entry in fs::read_dir(&root).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if !path.is_file() {
                continue;
            }
            let member = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("invalid proclib member {}", path.display()))?
                .to_ascii_uppercase();
            if member.is_empty()
                || member.len() > 8
                || !member
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"$#@".contains(&b))
            {
                continue;
            }
            let resolved = fs::canonicalize(&path).map_err(|error| error.to_string())?;
            if !resolved.starts_with(&root) {
                return Err(format!(
                    "proclib member outside directory: {}",
                    path.display()
                ));
            }
            let source = fs::read_to_string(&resolved)
                .map_err(|error| format!("{}: {error}", resolved.display()))?;
            let source = if expand_sites {
                substitute_site(&source, &profile.site)?
            } else {
                source
            };
            if members.insert(member.clone(), source).is_some() {
                return Err(format!("duplicate proclib member {library}({member})"));
            }
        }
        catalog.insert(library.clone(), members);
    }
    Ok(catalog)
}

fn take_call_argument(text: &str, key: &str) -> Result<(String, String)> {
    let pattern = format!("{key}(");
    let call_start = text
        .find(&pattern)
        .ok_or_else(|| format!("DSN command requires {key}(...)"))?;
    let start = call_start + pattern.len();
    let tail = &text[start..];
    let end = tail
        .find(')')
        .ok_or_else(|| format!("unclosed {key}(...)"))?;
    let value = tail[..end].trim().trim_matches('\'');
    if value.is_empty() {
        return Err(format!("empty {key}(...)"));
    }
    let remainder = format!("{}{}", &text[..call_start], &tail[end + 1..]);
    Ok((value.to_string(), remainder))
}

#[derive(Debug)]
struct DsnRun {
    system: String,
    program: String,
    plan: String,
    library: String,
}

fn parse_systsin(lines: &[String], line: usize) -> Result<DsnRun> {
    let mut commands = Vec::new();
    let mut pending = String::new();
    for raw in lines {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let continuation = text.ends_with('-');
        if !pending.is_empty() {
            pending.push(' ');
        }
        pending.push_str(text.trim_end_matches('-').trim_end());
        if !continuation {
            commands.push(pending.to_ascii_uppercase());
            pending.clear();
        }
    }
    if !pending.is_empty() {
        return Err(format!("line {line}: unfinished SYSTSIN continuation"));
    }
    if commands.len() != 3
        || !commands[0].starts_with("DSN ")
        || !commands[1].starts_with("RUN ")
        || commands[2] != "END"
    {
        return Err(format!(
            "line {line}: only DSN SYSTEM / RUN PROGRAM / END is implemented"
        ));
    }
    let (system, dsn_remainder) = take_call_argument(&commands[0], "SYSTEM")?;
    let (program, run_remainder) = take_call_argument(&commands[1], "PROGRAM")?;
    let (plan, run_remainder) = take_call_argument(&run_remainder, "PLAN")?;
    let (library, run_remainder) = take_call_argument(&run_remainder, "LIB")?;
    if dsn_remainder.trim() != "DSN" || run_remainder.trim() != "RUN" {
        return Err(format!("line {line}: unsupported DSN command operand"));
    }
    Ok(DsnRun {
        system,
        program,
        plan,
        library,
    })
}

fn checked_file(workspace: &Path, relative: &Path, description: &str) -> Result<PathBuf> {
    let resolved = fs::canonicalize(workspace.join(relative))
        .map_err(|error| format!("{description} {}: {error}", relative.display()))?;
    if !resolved.starts_with(workspace) || !resolved.is_file() {
        return Err(format!(
            "{description} must be a file inside workspace: {}",
            relative.display()
        ));
    }
    Ok(resolved)
}

fn checked_directory(workspace: &Path, relative: &Path, description: &str) -> Result<PathBuf> {
    let resolved = fs::canonicalize(workspace.join(relative))
        .map_err(|error| format!("{description} {}: {error}", relative.display()))?;
    if !resolved.starts_with(workspace) || !resolved.is_dir() {
        return Err(format!(
            "{description} must be a directory inside workspace: {}",
            relative.display()
        ));
    }
    Ok(resolved)
}

fn checked_output_path(workspace: &Path, relative: &Path, description: &str) -> Result<PathBuf> {
    let name = relative
        .file_name()
        .ok_or_else(|| format!("{description} needs a filename"))?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parent = fs::canonicalize(workspace.join(parent))
        .map_err(|error| format!("{description} directory {}: {error}", parent.display()))?;
    if !parent.starts_with(workspace) || !parent.is_dir() {
        return Err(format!("{description} must be inside workspace"));
    }
    let path = parent.join(name);
    if path.exists() {
        let resolved = fs::canonicalize(&path)
            .map_err(|error| format!("{description} {}: {error}", path.display()))?;
        if !resolved.starts_with(workspace) || !resolved.is_file() {
            return Err(format!("{description} must be a file inside workspace"));
        }
        return Ok(resolved);
    }
    Ok(path)
}

fn container_path(workspace: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| format!("path outside workspace: {}", path.display()))?;
    Ok(format!(
        "/src/{}",
        relative.to_string_lossy().replace('\\', "/")
    ))
}

fn docker_host_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    path.replace('\\', "/")
}

#[derive(Debug)]
struct PlannedStep {
    step: Step,
    dsn: Option<DsnRun>,
    ims: Option<ims::ImsRun>,
    executable: Option<PathBuf>,
}

fn plan_job(job: &Job, profile: &Profile, workspace: &Path) -> Result<Vec<PlannedStep>> {
    let datasets = datasets::DataSets::new(workspace, profile)?;
    let mut result = Vec::new();
    for step in &job.steps {
        datasets.validate(step)?;
        let dsn = if step.pgm == "IKJEFT01" {
            let systsin = step
                .dds
                .iter()
                .find(|dd| dd.name == "SYSTSIN")
                .ok_or_else(|| format!("line {}: IKJEFT01 requires SYSTSIN", step.line))?;
            let lines = match &systsin.kind {
                DdKind::Inline(lines) => lines,
                _ => return Err(format!("line {}: SYSTSIN must be DD *", systsin.line)),
            };
            let dsn = parse_systsin(lines, systsin.line)?;
            if dsn.system != profile.system
                || dsn.plan != profile.plan
                || dsn.library != profile.library
            {
                return Err(format!(
                    "line {}: DSN SYSTEM/PLAN/LIB does not match profile",
                    systsin.line
                ));
            }
            Some(dsn)
        } else {
            None
        };
        let ims = if step.pgm == "DFSRRC00" {
            Some(ims::parse_parm(step.parm.as_deref(), step.line)?)
        } else {
            None
        };
        let program_name = ims.as_ref().map_or_else(
            || {
                dsn.as_ref()
                    .map_or(step.pgm.as_str(), |dsn| dsn.program.as_str())
            },
            |run| run.program.as_str(),
        );
        let executable = if ims.is_none() && utilities::is_builtin(program_name) {
            None
        } else {
            let program = profile.programs.get(program_name).ok_or_else(|| {
                format!("line {}: unregistered program {}", step.line, program_name)
            })?;
            checked_file(workspace, &profile.env_file, "Db2 env file")?;
            Some(checked_file(workspace, program, "registered program")?)
        };
        if let Some(run) = &ims {
            let psb = profile
                .ims_psbs
                .get(&run.psb)
                .ok_or_else(|| format!("line {}: unregistered IMS PSB {}", step.line, run.psb))?;
            checked_file(workspace, psb, "IMS PSB")?;
            let dbd_dir = profile
                .ims_dbd_dir
                .as_ref()
                .ok_or_else(|| format!("line {}: ims_dbd_dir is required", step.line))?;
            checked_directory(workspace, dbd_dir, "IMS DBD directory")?;
            let wal = profile
                .ims_wal
                .as_ref()
                .ok_or_else(|| format!("line {}: ims_wal is required", step.line))?;
            checked_output_path(workspace, wal, "IMS WAL")?;
            if let Some(shim) = &profile.ims_cbltdli {
                checked_file(workspace, shim, "IMS CBLTDLI shim")?;
            }
        }
        result.push(PlannedStep {
            step: step.clone(),
            dsn,
            ims,
            executable,
        });
    }
    Ok(result)
}

fn print_plan(job: &Job, steps: &[PlannedStep], profile: &Profile, workspace: &Path) -> Result<()> {
    println!("JOB {} {}", job.name, job.attributes);
    if let Some(condition) = &job.job_condition {
        println!("  COND {condition:?}");
    }
    for dd in &job.joblib {
        println!("  JOBLIB {:?} (load library metadata)", dd.kind);
    }
    for planned in steps {
        println!("STEP {} PGM={}", planned.step.name, planned.step.pgm);
        if let Some(parm) = &planned.step.parm {
            println!("  PARM={parm}");
        }
        if let Some(dsn) = &planned.dsn {
            println!(
                "  DSN SYSTEM={} RUN PROGRAM={} PLAN={} LIB={}",
                dsn.system, dsn.program, dsn.plan, dsn.library
            );
        }
        if let Some(ims) = &planned.ims {
            println!("  IMS {} PROGRAM={} PSB={}", ims.mode, ims.program, ims.psb);
        }
        for condition in &planned.step.conditions {
            println!("  COND {condition:?}");
        }
        for guard in &planned.step.guards {
            println!("  IF {} THEN={}", guard.expression, guard.then_branch);
        }
        if let Some(executable) = &planned.executable {
            println!("  adapter {}", container_path(workspace, executable)?);
        } else {
            println!("  Rust utility {}", planned.step.pgm);
        }
        for dd in &planned.step.dds {
            match &dd.kind {
                DdKind::Inline(lines) => {
                    println!("  DD {} inline {} record(s)", dd.name, lines.len())
                }
                DdKind::Sysout => println!("  DD {} SYSOUT=*", dd.name),
                DdKind::Dataset(entries) => {
                    println!(
                        "  DD {} dataset {:?}{}",
                        dd.name,
                        entries,
                        if dd.name == "STEPLIB" {
                            " (load metadata)"
                        } else {
                            ""
                        }
                    );
                    if dd.name != "STEPLIB" {
                        for (dsn, disp) in entries {
                            if dsn.starts_with("&&") || dsn.is_empty() {
                                println!("    {dsn} {disp} -> job temporary dataset");
                            } else {
                                let base =
                                    dsn.split_once('(').map_or(dsn.as_str(), |(base, _)| base);
                                let target = profile
                                    .datasets
                                    .get(base)
                                    .ok_or_else(|| format!("unmapped dataset {base}"))?;
                                println!("    {dsn} {disp} -> {}", target.display());
                            }
                        }
                    }
                }
                DdKind::Dummy => println!("  DD {} DUMMY", dd.name),
            }
        }
    }
    Ok(())
}

fn job_id(name: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{name}-{millis}-{}", std::process::id())
}

fn run_job(
    job: &Job,
    steps: &[PlannedStep],
    profile: &Profile,
    workspace: &Path,
) -> Result<(String, i32)> {
    let id = job_id(&job.name);
    let spool = workspace.join("build/jclsh/spool").join(&id);
    fs::create_dir_all(&spool).map_err(|error| format!("{}: {error}", spool.display()))?;
    let mount = format!("{}:/src", docker_host_path(workspace));
    let mut datasets = datasets::DataSets::new(workspace, profile)?;
    let mut final_rc = 0;
    let mut results = Vec::new();
    let mut previous: Vec<(String, Option<i32>)> = Vec::new();
    let mut abends: HashMap<String, String> = HashMap::new();
    let mut job_bypassed = false;
    for planned in steps {
        let step = &planned.step;
        let step_dir = spool.join(&step.name);
        fs::create_dir_all(&step_dir)
            .map_err(|error| format!("{}: {error}", step_dir.display()))?;
        if !job_bypassed {
            if let Some(condition) = &job.job_condition {
                job_bypassed = jcl::cond_bypasses(condition, &previous, false)?;
            }
        }
        let guard_allows = if job_bypassed {
            false
        } else {
            jcl::guards_allow(&step.guards, &previous, &abends)?
        };
        let step_bypassed = if job_bypassed || !guard_allows {
            false
        } else {
            (step.conditions.is_empty() && !abends.is_empty())
                || step
                    .conditions
                    .iter()
                    .try_fold(false, |bypassed, condition| {
                        if bypassed {
                            Ok(true)
                        } else {
                            jcl::cond_bypasses(condition, &previous, !abends.is_empty())
                        }
                    })?
        };
        if !guard_allows || job_bypassed || step_bypassed {
            println!("JCLSH STEP {} SKIPPED", step.name);
            results.push(format!(
                "{{\"step\":\"{}\",\"status\":\"SKIPPED\",\"rc\":null}}",
                step.name
            ));
            previous.push((step.name.clone(), None));
            continue;
        }
        let abend_file = step_dir.join("abend.txt");
        if profile.vsam_backend != "local" {
            for dd in &step.dds {
                if let DdKind::Dataset(entries) = &dd.kind {
                    for (name, disposition) in entries {
                        if profile.vsam_tables.contains_key(&name.to_ascii_uppercase()) {
                            if entries.len() != 1 {
                                return Err(format!("SQL VSAM DD {name} cannot be concatenated"));
                            }
                            if step.pgm != "IDCAMS"
                                && !(step.pgm == "IEFBR14"
                                    && disposition.to_ascii_uppercase().contains("DELETE"))
                            {
                                return Err(format!(
                                    "SQL VSAM DD {name} is supported by IDCAMS/REPRO only"
                                ));
                            }
                        }
                    }
                }
            }
        }
        let (dataset_env, allocations) = datasets.prepare(step, &step_dir)?;
        let mut dd_paths: HashMap<String, PathBuf> = dataset_env.into_iter().collect();
        for dd in &step.dds {
            if let DdKind::Inline(lines) = &dd.kind {
                if dd.name == "SYSTSIN" {
                    continue;
                }
                let path = step_dir.join(format!("{}.txt", dd.name));
                let records = if lines.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", lines.join("\n"))
                };
                fs::write(&path, records)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                dd_paths.insert(dd.name.clone(), path);
            }
        }
        let (rc, stdout, stderr) = if let Some(executable) = &planned.executable {
            let mut command = Command::new("docker");
            command.args(["run", "--rm", "--env-file"]);
            let env_file = checked_file(workspace, &profile.env_file, "Db2 env file")?;
            command.arg(docker_host_path(&env_file));
            command.args(["-e", &format!("DB2_SCHEMA={}", profile.schema)]);
            if let Some(parm) = &step.parm {
                command.args(["-e", &format!("JCL_PARM={parm}")]);
            }
            if let Some(ims) = &planned.ims {
                let psb = checked_file(workspace, &profile.ims_psbs[&ims.psb], "IMS PSB")?;
                let dbd_dir = checked_directory(
                    workspace,
                    profile
                        .ims_dbd_dir
                        .as_ref()
                        .expect("planned IMS DBD directory"),
                    "IMS DBD directory",
                )?;
                let wal = checked_output_path(
                    workspace,
                    profile.ims_wal.as_ref().expect("planned IMS WAL"),
                    "IMS WAL",
                )?;
                command.args([
                    "-e",
                    &format!("COB_IMS_PSB={}", container_path(workspace, &psb)?),
                ]);
                command.args([
                    "-e",
                    &format!("COB_IMS_DBD_DIR={}", container_path(workspace, &dbd_dir)?),
                ]);
                command.args([
                    "-e",
                    &format!("COB_IMS_WAL={}", container_path(workspace, &wal)?),
                ]);
                command.args(["-e", &format!("COB_IMS_REGION={}", ims.mode)]);
                command.args(["-e", "COB_IMS_AUTOCOMMIT=1"]);
                if let Some(shim) = &profile.ims_cbltdli {
                    let shim = checked_file(workspace, shim, "IMS CBLTDLI shim")?;
                    command.args([
                        "-e",
                        &format!("COB_PRE_LOAD={}", container_path(workspace, &shim)?),
                    ]);
                }
            }
            command.args([
                "-e",
                &format!("LD_LIBRARY_PATH={}", profile.ld_library_path),
            ]);
            command.args([
                "-e",
                &format!("JCL_ABEND_FILE={}", container_path(workspace, &abend_file)?),
            ]);
            for (name, path) in &dd_paths {
                let value = if path == Path::new("/dev/null") {
                    "/dev/null".to_string()
                } else {
                    container_path(workspace, path)?
                };
                command.args(["-e", &format!("DD_{name}={value}")]);
            }
            command.args(["-v", &mount, "-w", "/src", "--entrypoint"]);
            command.arg(container_path(workspace, executable)?);
            command.arg(&profile.image);
            let output = command
                .output()
                .map_err(|error| format!("docker run for {}: {error}", step.name))?;
            let rc = output.status.code().unwrap_or(1);
            if matches!(rc, 125..=127) {
                return Err(format!(
                    "docker failed to start step {}: {}",
                    step.name,
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            (rc, output.stdout, output.stderr)
        } else {
            let output = utilities::run(&step.pgm, &dd_paths, &allocations, &datasets)?;
            (output.rc, output.stdout, output.stderr)
        };
        let abend = if abend_file.exists() {
            let code = fs::read_to_string(&abend_file)
                .map_err(|error| error.to_string())?
                .trim()
                .to_ascii_uppercase();
            let valid = (code.len() == 4
                && code.starts_with('S')
                && code[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                || (code.len() == 5
                    && code.starts_with('U')
                    && code[1..].bytes().all(|b| b.is_ascii_digit()));
            if !valid {
                return Err(format!(
                    "step {} wrote invalid ABEND code {code:?}",
                    step.name
                ));
            }
            Some(code)
        } else if (129..=192).contains(&rc) {
            Some(format!("S{:03X}", rc - 128))
        } else {
            None
        };
        datasets.finish(allocations, abend.is_some())?;
        for dd in &step.dds {
            if dd.kind == DdKind::Sysout {
                let content: &[u8] = match dd.name.as_str() {
                    "SYSTSPRT" => &stderr,
                    _ => &stdout,
                };
                fs::write(step_dir.join(format!("{}.txt", dd.name)), content)
                    .map_err(|error| format!("spool write: {error}"))?;
            }
        }
        io::stdout()
            .write_all(&stdout)
            .map_err(|error| error.to_string())?;
        io::stderr()
            .write_all(&stderr)
            .map_err(|error| error.to_string())?;
        if let Some(code) = abend {
            println!("JCLSH STEP {} ABEND={code}", step.name);
            results.push(format!(
                "{{\"step\":\"{}\",\"status\":\"ABEND\",\"rc\":null,\"abend\":\"{code}\"}}",
                step.name
            ));
            previous.push((step.name.clone(), None));
            abends.insert(step.name.clone(), code);
            final_rc = final_rc.max(16);
        } else {
            println!("JCLSH STEP {} RC={rc}", step.name);
            results.push(format!(
                "{{\"step\":\"{}\",\"status\":\"EXECUTED\",\"rc\":{rc}}}",
                step.name
            ));
            previous.push((step.name.clone(), Some(rc)));
            final_rc = final_rc.max(rc);
        }
    }
    datasets.end_job()?;
    let result = format!(
        "{{\"job\":\"{}\",\"job_id\":\"{}\",\"rc\":{},\"steps\":[{}]}}\n",
        job.name,
        id,
        final_rc,
        results.join(",")
    );
    fs::write(spool.join("result.json"), result).map_err(|error| error.to_string())?;
    println!(
        "JCLSH JOB {} RC={} SPOOL={}",
        job.name,
        final_rc,
        docker_host_path(&spool)
    );
    Ok((id, final_rc))
}

fn spool_directory(workspace: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("invalid job ID {id:?}"));
    }
    Ok(workspace.join("build/jclsh/spool").join(id))
}

fn spool_text(workspace: &Path, id: &str, dd_name: &str) -> Result<String> {
    let root = spool_directory(workspace, id)?;
    let (selected_step, dd_name) = if let Some((step, dd)) = dd_name.rsplit_once('.') {
        if step.split('.').any(|part| part.is_empty())
            || !step
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._$#@".contains(&byte))
        {
            return Err(format!("invalid spool step {step:?}"));
        }
        (Some(step), dd)
    } else {
        (None, dd_name)
    };
    let dd_name = identifier(dd_name, 0)?;
    if let Some(step) = selected_step {
        let path = root.join(step).join(format!("{dd_name}.txt"));
        return fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()));
    }
    let mut steps: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|error| format!("{}: {error}", root.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    steps.sort();
    let step = steps
        .first()
        .ok_or_else(|| format!("job {id} has no spool step"))?;
    let path = step.join(format!("{dd_name}.txt"));
    fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))
}

fn read_source(path: &str) -> Result<String> {
    if path == "-" {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("stdin: {error}"))?;
        Ok(source)
    } else {
        fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))
    }
}

fn execute(
    mode: &str,
    path: &str,
    profile_path: &Path,
    workspace: &Path,
    site_overrides: &HashMap<String, String>,
) -> Result<Option<(String, i32)>> {
    let source = read_source(path)?;
    let mut profile = read_profile(profile_path)?;
    profile.site.extend(site_overrides.clone());
    let catalog = load_catalog(&profile, workspace, mode != "lint")?;
    let source = if mode == "lint" {
        source
    } else {
        substitute_site(&source, &profile.site)?
    };
    let job = jcl::parse_jcl_with_catalog(&source, &catalog)?;
    if mode == "lint" {
        println!("JCLSH LINT {} OK: {} step(s)", job.name, job.steps.len());
        return Ok(None);
    }
    let steps = plan_job(&job, &profile, workspace)?;
    if mode == "plan" {
        print_plan(&job, &steps, &profile, workspace)?;
        return Ok(None);
    }
    if mode == "run" {
        return run_job(&job, &steps, &profile, workspace).map(Some);
    }
    Err(format!("unknown command {mode}"))
}

fn shell(
    profile_path: &Path,
    workspace: &Path,
    mut site_overrides: HashMap<String, String>,
) -> Result<()> {
    let mut jobs: Vec<(String, i32)> = Vec::new();
    let stdin = io::stdin();
    println!("jclsh: submit FILE, plan FILE, jobs, status JOBID, spool JOBID [DD], datasets [PREFIX], set NAME VALUE, exit");
    loop {
        print!("jclsh> ");
        io::stdout().flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        if stdin
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        match parts.as_slice() {
            [] => {}
            ["exit" | "quit"] => break,
            ["help"] => {
                println!("submit FILE | plan FILE | jobs | status JOBID | spool JOBID [DD] | datasets [PREFIX] | set NAME VALUE | exit")
            }
            ["submit", path] => {
                match execute("run", path, profile_path, workspace, &site_overrides) {
                    Ok(Some(result)) => jobs.push(result),
                    Ok(None) => {}
                    Err(error) => eprintln!("jclsh: {error}"),
                }
            }
            ["plan", path] => {
                if let Err(error) = execute("plan", path, profile_path, workspace, &site_overrides)
                {
                    eprintln!("jclsh: {error}");
                }
            }
            ["jobs"] => {
                for (id, rc) in &jobs {
                    println!("{id} RC={rc}");
                }
            }
            ["status", id] => {
                match spool_directory(workspace, id).and_then(|path| {
                    fs::read_to_string(path.join("result.json")).map_err(|error| error.to_string())
                }) {
                    Ok(value) => print!("{value}"),
                    Err(error) => eprintln!("jclsh: {error}"),
                }
            }
            ["spool", id] | ["spool", id, _] => {
                let dd_name = parts.get(2).copied().unwrap_or("SYSPRINT");
                match spool_text(workspace, id, dd_name) {
                    Ok(value) => print!("{value}"),
                    Err(error) => eprintln!("jclsh: {error}"),
                }
            }
            ["datasets"] | ["datasets", _] => {
                let prefix = parts.get(1).copied().unwrap_or("").to_ascii_uppercase();
                match read_profile(profile_path) {
                    Ok(profile) => {
                        let mut names: Vec<_> = profile
                            .datasets
                            .iter()
                            .filter(|(name, _)| name.starts_with(&prefix))
                            .collect();
                        names.sort_by_key(|(name, _)| *name);
                        for (name, path) in names {
                            println!("{name} -> {}", path.display());
                        }
                    }
                    Err(error) => eprintln!("jclsh: {error}"),
                }
            }
            ["set", name, value] => {
                let name = name.to_ascii_uppercase();
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    eprintln!("jclsh: invalid site symbol {name}");
                } else {
                    site_overrides.insert(name.clone(), (*value).to_string());
                    println!("{name} set");
                }
            }
            _ => eprintln!("jclsh: unsupported shell command"),
        }
    }
    Ok(())
}

fn cli() -> Result<()> {
    let workspace = fs::canonicalize(env::current_dir().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut mode: Option<&str> = None;
    let mut input: Option<&str> = None;
    let mut profile = PathBuf::from("extras/jclsh/bank.conf");
    let mut site_overrides = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => {
                index += 1;
                profile = PathBuf::from(args.get(index).ok_or("--profile requires a path")?);
            }
            "--set" => {
                index += 1;
                let setting = args.get(index).ok_or("--set requires NAME=VALUE")?;
                let (name, value) = setting.split_once('=').ok_or("--set requires NAME=VALUE")?;
                let name = name.to_ascii_uppercase();
                if name.is_empty()
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || value.is_empty()
                {
                    return Err("invalid --set NAME=VALUE".to_string());
                }
                site_overrides.insert(name, value.to_string());
            }
            "-h" | "--help" => {
                println!("usage: jclsh [run|plan|lint FILE|-] [--profile FILE] [--set NAME=VALUE]; no arguments starts shell");
                return Ok(());
            }
            "run" | "plan" | "lint" if mode.is_none() => mode = Some(&args[index]),
            other if input.is_none() && mode.is_some() => input = Some(other),
            other => return Err(format!("unexpected argument {other}")),
        }
        index += 1;
    }
    let profile = workspace.join(profile);
    if let Some(mode) = mode {
        let input = input.ok_or("JCL file path is required")?;
        let result = execute(mode, input, &profile, &workspace, &site_overrides)?;
        if let Some((_, rc)) = result {
            if rc != 0 {
                return Err(format!("job RC={rc}"));
            }
        }
        Ok(())
    } else {
        shell(&profile, &workspace, site_overrides)
    }
}

fn main() {
    if let Err(error) = cli() {
        eprintln!("jclsh: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unmodified_bank_job() {
        let job = jcl::parse_jcl(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl"
        ))
        .unwrap();
        assert_eq!(job.name, "BNKSTMT");
        assert_eq!(job.steps.len(), 1);
        assert_eq!(job.steps[0].pgm, "IKJEFT01");
        assert_eq!(job.steps[0].dds.len(), 7);
        assert!(
            matches!(&job.steps[0].dds[0].kind, DdKind::Dataset(entries) if entries.len() == 2)
        );
        let cards = job.steps[0]
            .dds
            .iter()
            .find(|dd| dd.name == "DATECARD")
            .unwrap();
        assert_eq!(cards.kind, DdKind::Inline(vec!["202606".to_string()]));
        let systsin = job.steps[0]
            .dds
            .iter()
            .find(|dd| dd.name == "SYSTSIN")
            .unwrap();
        let DdKind::Inline(lines) = &systsin.kind else {
            panic!("SYSTSIN is not inline")
        };
        let dsn = parse_systsin(lines, systsin.line).unwrap();
        assert_eq!(dsn.program, "BNKSTMT");
        assert_eq!(dsn.plan, "BANKZPLN");
    }

    #[test]
    fn rejects_unmapped_dd_operand() {
        let source =
            "//X JOB 'A'\n//S EXEC PGM=IKJEFT01\n//SYSIN DD DISP=(NEW,CATLG),DSN=A.B,BOGUS=X\n";
        assert!(jcl::parse_jcl(source)
            .unwrap_err()
            .contains("unsupported DD operand"));
    }

    #[test]
    fn rejects_unknown_statement() {
        let source = "//X JOB 'A'\n//S EXEC PGM=IKJEFT01\n//P BOGUS\n";
        assert!(jcl::parse_jcl(source)
            .unwrap_err()
            .contains("unsupported JCL operation"));
    }

    #[test]
    fn rejects_extra_dsn_run_operand() {
        let lines = vec![
            " DSN SYSTEM(DBD1)".to_string(),
            " RUN PROGRAM(BNKSTMT) PLAN(BANKZPLN) LIB('BANKZ.LOAD') UNKNOWN(X)".to_string(),
            " END".to_string(),
        ];
        assert!(parse_systsin(&lines, 1)
            .unwrap_err()
            .contains("unsupported DSN command operand"));
    }

    #[test]
    fn expands_proc_and_override() {
        let source = "//X JOB 'A'\n//P PROC MEM=DEFAULT\n//A EXEC PGM=TEST\n//IN DD DSN=ABC.&MEM,DISP=SHR\n//B EXEC PGM=TEST,COND=(7,LT,A)\n//OUT DD DUMMY\n// PEND\n//S1 EXEC P,MEM=ONE\n//A.IN DD DSN=OTHER,DISP=SHR\n//S2 EXEC P,MEM=TWO\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.steps.len(), 4);
        assert_eq!(job.steps[0].name, "S1.A");
        assert!(
            matches!(&job.steps[0].dds[0].kind, DdKind::Dataset(entries) if entries[0].0 == "OTHER")
        );
        assert!(
            matches!(&job.steps[2].dds[0].kind, DdKind::Dataset(entries) if entries[0].0 == "ABC.TWO")
        );
        assert_eq!(
            job.steps[1].conditions[0].tests[0].step.as_deref(),
            Some("S1.A")
        );
    }

    #[test]
    fn condition_skips_when_comparison_is_true() {
        let previous = vec![("FIRST".to_string(), Some(8)), ("SKIP".to_string(), None)];
        let source = "//X JOB 'A'\n//S EXEC PGM=TEST,COND=(7,LT,FIRST)\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert!(jcl::cond_bypasses(&job.steps[0].conditions[0], &previous, false).unwrap());
        let source = "//X JOB 'A'\n//S EXEC PGM=TEST,COND=(7,GT,FIRST)\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert!(!jcl::cond_bypasses(&job.steps[0].conditions[0], &previous, false).unwrap());
    }

    #[test]
    fn parses_if_and_data_delimiter() {
        let source = "//X JOB 'A'\n//A EXEC PGM=TEST\n//CARD DD DATA,DLM=@@\n//literal record\n@@\n// IF (A.RC = 0) THEN\n//B EXEC PGM=TEST\n// ELSE\n//C EXEC PGM=TEST\n// ENDIF\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.steps.len(), 3);
        assert_eq!(
            job.steps[0].dds[0].kind,
            DdKind::Inline(vec!["//literal record".to_string()])
        );
        let previous = vec![("A".to_string(), Some(0))];
        assert!(jcl::guards_allow(&job.steps[1].guards, &previous, &HashMap::new()).unwrap());
        assert!(!jcl::guards_allow(&job.steps[2].guards, &previous, &HashMap::new()).unwrap());
    }

    #[test]
    fn job_cond_multiple_tests_and_skipped_rc() {
        let source = "//X JOB 'A',COND=((4,LT,A),(12,EQ,B))\n//A EXEC PGM=TEST\n//B EXEC PGM=TEST,COND=ONLY\n//C EXEC PGM=TEST,COND=EVEN\n";
        let job = jcl::parse_jcl(source).unwrap();
        let previous = vec![("A".to_string(), Some(8)), ("B".to_string(), None)];
        assert!(jcl::cond_bypasses(job.job_condition.as_ref().unwrap(), &previous, false).unwrap());
        assert!(jcl::cond_bypasses(&job.steps[1].conditions[0], &previous, false).unwrap());
        assert!(!jcl::cond_bypasses(&job.steps[2].conditions[0], &previous, false).unwrap());
    }

    #[test]
    fn parses_joblib_and_rejects_unknown_proc_override() {
        let source = "//X JOB 'A'\n//JOBLIB DD DSN=ABC.LOAD,DISP=SHR\n//P PROC\n//A EXEC PGM=TEST\n// PEND\n//S EXEC P\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.joblib.len(), 1);
        let broken = source.replace("//S EXEC P\n", "//S EXEC P\n//Z.CARD DD DUMMY\n");
        assert!(jcl::parse_jcl(&broken)
            .unwrap_err()
            .contains("unknown step Z"));
    }

    #[test]
    fn resolves_nested_symbols_and_detects_cycle() {
        let source = "//X JOB 'A'\n// SET SOURCE=VALUE\n//P PROC MEM=DEFAULT\n//A EXEC PGM=TEST,PARM='&MEM'\n// PEND\n//S EXEC P,MEM=&SOURCE\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.steps[0].parm.as_deref(), Some("VALUE"));
        let broken = source.replace("SOURCE=VALUE", "SOURCE=&SOURCE");
        assert!(jcl::parse_jcl(&broken)
            .unwrap_err()
            .contains("cyclic JCL symbol"));
    }

    #[test]
    fn cataloged_proc_and_include_follow_jcllib_order() {
        let mut catalog = jcl::Catalog::new();
        catalog.insert("FIRST.LIB".to_string(), HashMap::from([
            ("CATP".to_string(), "//CATP PROC VALUE=DEFAULT\n//A EXEC PGM=TEST,PARM='&VALUE'\n//CARD DD DUMMY\n// PEND\n".to_string()),
            ("INC1".to_string(), "// SET VALUE=FROM_INCLUDE\n".to_string()),
        ]));
        catalog.insert(
            "SECOND.LIB".to_string(),
            HashMap::from([("CATP".to_string(), "//A EXEC PGM=WRONG\n".to_string())]),
        );
        let source = "//X JOB 'A'\n//LIB JCLLIB ORDER=(FIRST.LIB,SECOND.LIB)\n//INC INCLUDE MEMBER=INC1\n//S EXEC CATP,VALUE=&VALUE\n//A.CARD DD *\n0\n/*\n";
        let job = jcl::parse_jcl_with_catalog(source, &catalog).unwrap();
        assert_eq!(job.steps.len(), 1);
        assert_eq!(job.steps[0].name, "S.A");
        assert_eq!(job.steps[0].parm.as_deref(), Some("FROM_INCLUDE"));
        assert_eq!(
            job.steps[0].dds[0].kind,
            DdKind::Inline(vec!["0".to_string()])
        );
    }

    #[test]
    fn nested_proc_passes_symbols_and_detects_recursion() {
        let source = "//X JOB 'A'\n//P1 PROC VALUE=DEFAULT\n//A EXEC P2,VALUE=&VALUE\n// PEND\n//P2 PROC VALUE=SECOND\n//B EXEC PGM=TEST,PARM='&VALUE'\n// PEND\n//S EXEC P1,VALUE=OUTER\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.steps[0].name, "S.A.B");
        assert_eq!(job.steps[0].parm.as_deref(), Some("OUTER"));
        let broken = source.replace("//A EXEC P2,VALUE=&VALUE", "//A EXEC P1,VALUE=&VALUE");
        assert!(jcl::parse_jcl(&broken).unwrap_err().contains("recursive"));
    }

    #[test]
    fn duplicate_step_names_have_unique_ids_and_ambiguous_cond_is_rejected() {
        let source =
            "//X JOB 'A'\n//A EXEC PGM=TEST\n//A EXEC PGM=TEST\n//B EXEC PGM=TEST,COND=(4,LT,A)\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert_eq!(job.steps[0].name, "A");
        assert_eq!(job.steps[1].name, "A#2");
        let previous = vec![("A".to_string(), Some(0)), ("A#2".to_string(), Some(8))];
        assert!(
            jcl::cond_bypasses(&job.steps[2].conditions[0], &previous, false)
                .unwrap_err()
                .contains("ambiguous")
        );
    }

    #[test]
    fn if_can_test_abend_and_run() {
        let source = "//X JOB 'A'\n//FAIL EXEC PGM=TEST\n// IF (FAIL.ABEND AND NOT FAIL.RC = 0) THEN\n//A EXEC PGM=TEST\n// ENDIF\n";
        let job = jcl::parse_jcl(source).unwrap();
        let previous = vec![("FAIL".to_string(), None)];
        let abends = HashMap::from([("FAIL".to_string(), "U1234".to_string())]);
        assert!(jcl::guards_allow(&job.steps[1].guards, &previous, &abends).unwrap());
        let source = "//X JOB 'A'\n//FAIL EXEC PGM=TEST\n// IF (FAIL.ABENDCC=U1234 AND ¬FAIL.RC = 0) THEN\n//A EXEC PGM=TEST\n// ENDIF\n";
        let job = jcl::parse_jcl(source).unwrap();
        assert!(jcl::guards_allow(&job.steps[1].guards, &previous, &abends).unwrap());
    }

    #[test]
    fn site_placeholders_are_explicit() {
        let source = "//X JOB 'A'\n//S EXEC PGM=TEST\n//INPUT DD DSN=<HLQ>.FILE,DISP=SHR\nA < B\n";
        let site = HashMap::from([("HLQ".to_string(), "LOCAL".to_string())]);
        assert!(substitute_site(source, &site)
            .unwrap()
            .contains("DSN=LOCAL.FILE"));
        assert!(substitute_site(source, &site).unwrap().contains("A < B"));
        assert!(substitute_site(source, &HashMap::new())
            .unwrap_err()
            .contains("<HLQ>"));
    }
}
