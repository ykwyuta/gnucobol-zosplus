use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{DdKind, Profile, Result, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Delete,
    Pass,
    Keep,
    Catalog,
    Uncatalog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    New,
    Old,
    Shared,
    Modify,
}

#[derive(Clone, Copy, Debug)]
struct Disp {
    status: Status,
    normal: Action,
    abnormal: Action,
}

#[derive(Debug)]
pub(super) struct Allocation {
    name: String,
    path: PathBuf,
    target: Option<PathBuf>,
    temporary: bool,
    disp: Disp,
}

pub(super) struct DataSets<'a> {
    workspace: &'a Path,
    root: PathBuf,
    profile: &'a Profile,
    passed: HashMap<String, PathBuf>,
}

fn disposition(value: &str) -> Result<Disp> {
    let value = value.trim().trim_start_matches('(').trim_end_matches(')');
    let parts: Vec<_> = value.split(',').map(str::trim).collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(format!("invalid DISP={value}"));
    }
    let status = match parts[0] {
        "NEW" => Status::New,
        "OLD" => Status::Old,
        "SHR" => Status::Shared,
        "MOD" => Status::Modify,
        other => return Err(format!("unsupported DISP status {other}")),
    };
    let default = if status == Status::New {
        Action::Delete
    } else {
        Action::Keep
    };
    let action = |text: &str| -> Result<Action> {
        match text {
            "DELETE" => Ok(Action::Delete),
            "PASS" => Ok(Action::Pass),
            "KEEP" => Ok(Action::Keep),
            "CATLG" => Ok(Action::Catalog),
            "UNCATLG" => Ok(Action::Uncatalog),
            other => Err(format!("unsupported DISP action {other}")),
        }
    };
    let normal = if parts.get(1).is_some_and(|part| !part.is_empty()) {
        action(parts[1])?
    } else {
        default
    };
    let abnormal = if parts.get(2).is_some_and(|part| !part.is_empty()) {
        action(parts[2])?
    } else {
        default
    };
    Ok(Disp {
        status,
        normal,
        abnormal,
    })
}

fn dataset_name(name: &str) -> Result<String> {
    if name.is_empty() {
        return Err("missing DSN".to_string());
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b".$#@_&()".contains(&b))
    {
        return Err(format!("invalid DSN {name:?}"));
    }
    Ok(name.to_ascii_uppercase())
}

impl<'a> DataSets<'a> {
    pub(super) fn new(workspace: &'a Path, profile: &'a Profile) -> Result<Self> {
        let root = workspace.join(&profile.dataset_root);
        fs::create_dir_all(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        if !root.starts_with(workspace) {
            return Err("dataset_root must be inside workspace".to_string());
        }
        Ok(Self {
            workspace,
            root,
            profile,
            passed: HashMap::new(),
        })
    }

    fn target(&self, dsn: &str, writable: bool) -> Result<Option<PathBuf>> {
        if dsn.starts_with("&&") {
            return Ok(None);
        }
        let (base, member) = if let Some((base, member)) = dsn.split_once('(') {
            (
                base,
                Some(
                    member
                        .strip_suffix(')')
                        .ok_or_else(|| format!("invalid PDS member {dsn}"))?,
                ),
            )
        } else {
            (dsn, None)
        };
        let relative = self
            .profile
            .datasets
            .get(base)
            .ok_or_else(|| format!("dataset {base} is not mapped in profile"))?;
        let mut target = self.workspace.join(relative);
        if let Some(member) = member {
            if member.is_empty()
                || !member
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"$#@_".contains(&b))
            {
                return Err(format!("invalid PDS member {dsn}"));
            }
            target.push(member);
        }
        let parent = target
            .parent()
            .ok_or_else(|| format!("invalid dataset path {}", target.display()))?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| format!("dataset parent {}: {error}", parent.display()))?;
        if !parent.starts_with(self.workspace) || (writable && !parent.starts_with(&self.root)) {
            return Err(format!(
                "dataset path outside permitted root: {}",
                target.display()
            ));
        }
        if target.exists() {
            let actual = fs::canonicalize(&target).map_err(|error| error.to_string())?;
            if !actual.is_file()
                || !actual.starts_with(self.workspace)
                || (writable && !actual.starts_with(&self.root))
            {
                return Err(format!(
                    "dataset path outside permitted root: {}",
                    target.display()
                ));
            }
            target = actual;
        }
        Ok(Some(target))
    }

    pub(super) fn validate(&self, step: &Step) -> Result<()> {
        for dd in &step.dds {
            if let DdKind::Dataset(entries) = &dd.kind {
                if dd.name == "STEPLIB" {
                    continue;
                }
                for (index, (name, value)) in entries.iter().enumerate() {
                    let dsn = if name.is_empty() {
                        format!("&&{}.{}.{}", step.name, dd.name, index)
                    } else {
                        dataset_name(name)?
                    };
                    let disp = disposition(value)?;
                    if entries.len() > 1 && disp.status != Status::Shared {
                        return Err(format!(
                            "line {}: concatenated DD {} must use DISP=SHR",
                            dd.line, dd.name
                        ));
                    }
                    self.target(
                        &dsn,
                        disp.status != Status::Shared
                            || disp.normal == Action::Delete
                            || disp.abnormal == Action::Delete,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn allocate(
        &self,
        step: &Step,
        dd_name: &str,
        index: usize,
        raw_name: &str,
        raw_disp: &str,
        step_dir: &Path,
    ) -> Result<Allocation> {
        let name = if raw_name.is_empty() {
            format!("&&{}.{}.{}", step.name, dd_name, index)
        } else {
            dataset_name(raw_name)?
        };
        let disp = disposition(raw_disp)?;
        let temporary = name.starts_with("&&");
        let target = self.target(
            &name,
            disp.status != Status::Shared
                || disp.normal == Action::Delete
                || disp.abnormal == Action::Delete,
        )?;
        let prior = self.passed.get(&name).or(target.as_ref());
        match disp.status {
            Status::New if prior.is_some_and(|path| path.exists()) => {
                return Err(format!("dataset {name} already exists for DISP=NEW"))
            }
            Status::Old | Status::Shared if prior.is_none_or(|path| !path.exists()) => {
                return Err(format!(
                    "dataset {name} does not exist for DISP={:?}",
                    disp.status
                ))
            }
            _ => {}
        }
        let path = step_dir.join(format!("dataset-{dd_name}-{index}.dat"));
        if let Some(prior) = prior.filter(|path| path.exists()) {
            fs::copy(prior, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        } else {
            fs::write(&path, []).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(Allocation {
            name,
            path,
            target,
            temporary,
            disp,
        })
    }

    pub(super) fn prepare(
        &self,
        step: &Step,
        step_dir: &Path,
    ) -> Result<(Vec<(String, PathBuf)>, Vec<Allocation>)> {
        let mut env = Vec::new();
        let mut allocations = Vec::new();
        for dd in &step.dds {
            if dd.name == "STEPLIB" {
                continue;
            }
            match &dd.kind {
                DdKind::Dummy => env.push((dd.name.clone(), PathBuf::from("/dev/null"))),
                DdKind::Dataset(entries) => {
                    let mut parts = Vec::new();
                    for (index, (name, disp)) in entries.iter().enumerate() {
                        let alloc = self.allocate(step, &dd.name, index, name, disp, step_dir)?;
                        parts.push(alloc.path.clone());
                        allocations.push(alloc);
                    }
                    let path = if parts.len() == 1 {
                        parts.remove(0)
                    } else {
                        let concat = step_dir.join(format!("concat-{}.dat", dd.name));
                        let mut bytes = Vec::new();
                        for part in parts {
                            bytes.extend(fs::read(part).map_err(|error| error.to_string())?);
                        }
                        fs::write(&concat, bytes).map_err(|error| error.to_string())?;
                        concat
                    };
                    env.push((dd.name.clone(), path));
                }
                _ => {}
            }
        }
        Ok((env, allocations))
    }

    pub(super) fn finish(&mut self, allocations: Vec<Allocation>, abended: bool) -> Result<()> {
        for alloc in allocations {
            let action = if abended {
                alloc.disp.abnormal
            } else {
                alloc.disp.normal
            };
            let action = if alloc.temporary && matches!(action, Action::Keep | Action::Catalog) {
                Action::Pass
            } else {
                action
            };
            match action {
                Action::Pass => {
                    if let Some(old) = self.passed.insert(alloc.name, alloc.path.clone()) {
                        if old != alloc.path && old.exists() {
                            fs::remove_file(old).map_err(|error| error.to_string())?;
                        }
                    }
                }
                Action::Keep | Action::Catalog => {
                    if let Some(target) = &alloc.target {
                        fs::copy(&alloc.path, target)
                            .map_err(|error| format!("{}: {error}", target.display()))?;
                    }
                    if let Some(old) = self.passed.remove(&alloc.name) {
                        if old != alloc.path && old.exists() {
                            fs::remove_file(old).map_err(|error| error.to_string())?;
                        }
                    }
                    fs::remove_file(&alloc.path).map_err(|error| error.to_string())?;
                }
                Action::Delete => {
                    if let Some(old) = self.passed.remove(&alloc.name) {
                        if old != alloc.path && old.exists() {
                            fs::remove_file(old).map_err(|error| error.to_string())?;
                        }
                    }
                    if let Some(target) = &alloc.target {
                        if target.exists() {
                            fs::remove_file(target)
                                .map_err(|error| format!("{}: {error}", target.display()))?;
                        }
                    }
                    fs::remove_file(&alloc.path).map_err(|error| error.to_string())?;
                }
                Action::Uncatalog => {
                    self.passed.remove(&alloc.name);
                    if alloc.temporary {
                        fs::remove_file(&alloc.path).map_err(|error| error.to_string())?;
                    } else if let Some(target) = &alloc.target {
                        let archive = self.root.join(".uncataloged");
                        fs::create_dir_all(&archive).map_err(|error| error.to_string())?;
                        let archive_path = archive.join(alloc.name.replace(['(', ')', '.'], "_"));
                        fs::copy(&alloc.path, &archive_path).map_err(|error| error.to_string())?;
                        if target.exists() {
                            fs::remove_file(target).map_err(|error| error.to_string())?;
                        }
                        fs::remove_file(&alloc.path).map_err(|error| error.to_string())?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn end_job(&mut self) -> Result<()> {
        for (_, path) in self.passed.drain() {
            if path.exists() {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_normal_and_abnormal_dispositions() {
        let new = disposition("(NEW,PASS,DELETE)").unwrap();
        assert_eq!(new.status, Status::New);
        assert_eq!(new.normal, Action::Pass);
        assert_eq!(new.abnormal, Action::Delete);
        let old = disposition("(OLD,UNCATLG,KEEP)").unwrap();
        assert_eq!(old.normal, Action::Uncatalog);
        assert_eq!(old.abnormal, Action::Keep);
        assert_eq!(disposition("SHR").unwrap().normal, Action::Keep);
        assert!(disposition("(NEW,BOGUS)").is_err());
    }
}
