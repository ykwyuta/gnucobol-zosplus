//! IMS batch launcher parameters accepted by DFSRRC00.
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImsRun {
    pub mode: String,
    pub program: String,
    pub psb: String,
}

pub(super) fn parse_parm(parm: Option<&str>, line: usize) -> Result<ImsRun> {
    let parm = parm.ok_or_else(|| format!("line {line}: DFSRRC00 requires PARM"))?;
    let mut fields = parm.split(',').map(str::trim);
    let mode = fields.next().unwrap_or("").to_ascii_uppercase();
    if !matches!(mode.as_str(), "BMP" | "DLI" | "DBB") {
        return Err(format!("line {line}: unsupported DFSRRC00 region {mode:?}"));
    }
    let program = crate::identifier(fields.next().unwrap_or(""), line)?;
    let psb = crate::identifier(fields.next().unwrap_or(""), line)?;
    // Additional positional IMS execution parameters are accepted and passed
    // through in JCL_PARM; the current adapter does not interpret them.
    Ok(ImsRun { mode, program, psb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carddemo_parms_select_registered_program_and_psb() {
        assert_eq!(
            parse_parm(Some("BMP,CBPAUP0C,PSBPAUTB"), 24).unwrap(),
            ImsRun {
                mode: "BMP".into(),
                program: "CBPAUP0C".into(),
                psb: "PSBPAUTB".into()
            }
        );
        assert_eq!(
            parse_parm(Some("DLI,DBUNLDGS,DLIGSAMP,,,,,,,,,,,N"), 38)
                .unwrap()
                .psb,
            "DLIGSAMP"
        );
        assert!(parse_parm(Some("BMP,ONLY"), 1).is_err());
        assert!(parse_parm(Some("MPP,PRG,PSB"), 1).is_err());
    }
}
