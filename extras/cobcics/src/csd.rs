use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
struct Resource {
    group: String,
    kind: String,
    name: String,
    card: String,
}

#[derive(Default)]
pub struct Catalog {
    resources: Vec<Resource>,
    lists: HashMap<String, HashSet<String>>,
}

fn operand(card: &str, name: &str) -> Option<String> {
    let upper = card.to_ascii_uppercase();
    let start = upper.find(&format!("{name}("))? + name.len() + 1;
    let end = card[start..].find(')')? + start;
    Some(card[start..end].trim().to_ascii_uppercase())
}

impl Catalog {
    pub fn read(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut catalog = Self::default();
        for (line_no, line) in text.lines().enumerate() {
            let fields: Vec<_> = line.splitn(5, '\t').collect();
            match fields.as_slice() {
                ["R", group, kind, name, card] => catalog.resources.push(Resource {
                    group: (*group).to_owned(),
                    kind: (*kind).to_owned(),
                    name: (*name).to_owned(),
                    card: (*card).to_owned(),
                }),
                ["L", list, group] => {
                    catalog
                        .lists
                        .entry((*list).to_owned())
                        .or_default()
                        .insert((*group).to_owned());
                }
                _ => {
                    return Err(format!(
                        "{}:{}: invalid jclsh CSD entry",
                        path.display(),
                        line_no + 1
                    ))
                }
            }
        }
        Ok(catalog)
    }

    fn selected<'a>(&'a self, list: Option<&str>) -> Result<Vec<&'a Resource>, String> {
        let groups = match list {
            Some(name) => Some(
                self.lists
                    .get(&name.to_ascii_uppercase())
                    .ok_or_else(|| format!("CSD list {name} is not defined"))?,
            ),
            None => None,
        };
        Ok(self
            .resources
            .iter()
            .filter(|resource| groups.is_none_or(|groups| groups.contains(&resource.group)))
            .collect())
    }

    pub fn file_datasets(&self, list: Option<&str>) -> Result<HashMap<String, String>, String> {
        let mut files = HashMap::new();
        for resource in self
            .selected(list)?
            .into_iter()
            .filter(|resource| resource.kind == "FILE")
        {
            let dataset = operand(&resource.card, "DSNAME")
                .ok_or_else(|| format!("CSD FILE {} has no DSNAME", resource.name))?;
            match files.insert(resource.name.clone(), dataset.clone()) {
                Some(previous) if previous != dataset => {
                    return Err(format!(
                        "CSD FILE {} has conflicting DSNAMEs",
                        resource.name
                    ))
                }
                _ => {}
            }
        }
        Ok(files)
    }

    pub fn transaction_programs(
        &self,
        list: Option<&str>,
    ) -> Result<HashMap<String, String>, String> {
        let mut transactions = HashMap::new();
        for resource in self.selected(list)? {
            let (transid, program) = match resource.kind.as_str() {
                "TRANSACTION" => (resource.name.clone(), operand(&resource.card, "PROGRAM")),
                "PROGRAM" => (
                    operand(&resource.card, "TRANSID").unwrap_or_default(),
                    Some(resource.name.clone()),
                ),
                _ => continue,
            };
            if transid.is_empty() {
                continue;
            }
            let program =
                program.ok_or_else(|| format!("CSD TRANSACTION {transid} has no PROGRAM"))?;
            match transactions.insert(transid.clone(), program.clone()) {
                Some(previous) if previous != program => {
                    return Err(format!("CSD TRANSID {transid} has conflicting PROGRAMs"))
                }
                _ => {}
            }
        }
        Ok(transactions)
    }
}

pub fn profile_datasets(path: &Path) -> Result<(PathBuf, HashMap<String, PathBuf>), String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut root = None;
    let mut datasets = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "dataset_root" {
            root = Some(PathBuf::from(value.trim()));
        }
        if let Some(name) = key.strip_prefix("dataset.") {
            datasets.insert(name.to_ascii_uppercase(), PathBuf::from(value.trim()));
        }
    }
    Ok((root.ok_or("dataset profile has no dataset_root")?, datasets))
}

pub fn resolved_files(
    catalog: &Catalog,
    list: Option<&str>,
    root: &Path,
    datasets: &HashMap<String, PathBuf>,
) -> Result<HashMap<String, PathBuf>, String> {
    let root = fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let mut files = HashMap::new();
    for (name, dsn) in catalog.file_datasets(list)? {
        let Some(path) = datasets.get(&dsn) else {
            continue;
        };
        let actual =
            fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !actual.is_file() || !actual.starts_with(&root) {
            return Err(format!("CSD FILE {name} is outside dataset_root"));
        }
        files.insert(name, actual);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_file_and_transaction_from_csd() {
        let catalog = Catalog {
            resources: vec![
                Resource {
                    group: "GENA".into(),
                    kind: "FILE".into(),
                    name: "KSDSCUST".into(),
                    card: "DEFINE FILE(KSDSCUST) GROUP(GENA) DSNAME(TEST.GENAPP.KSDSCUST)".into(),
                },
                Resource {
                    group: "GENA".into(),
                    kind: "TRANSACTION".into(),
                    name: "SSC1".into(),
                    card: "DEFINE TRANSACTION(SSC1) GROUP(GENA) PROGRAM(LGACUS01)".into(),
                },
            ],
            lists: HashMap::from([("GENALIST".into(), HashSet::from(["GENA".into()]))]),
        };
        assert_eq!(
            catalog.file_datasets(Some("GENALIST")).unwrap()["KSDSCUST"],
            "TEST.GENAPP.KSDSCUST"
        );
        assert_eq!(
            catalog.transaction_programs(Some("GENALIST")).unwrap()["SSC1"],
            "LGACUS01"
        );
        assert!(catalog.file_datasets(Some("UNKNOWN")).is_err());
    }
}
