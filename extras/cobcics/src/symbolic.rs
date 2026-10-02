//! COBOL fixed-format symbolic maps generated from the same BMS model as the screen.

use crate::bms::Map;

fn line(output: &mut String, level: &str, name: &str, clause: &str) {
    let record = if clause.is_empty() {
        format!("       {level} {name}.\n")
    } else {
        format!("       {level} {name} {clause}.\n")
    };
    assert!(
        record.trim_end().len() <= 72,
        "symbolic map line exceeds COBOL column 72"
    );
    output.push_str(&record);
}

pub fn generate(maps: &[Map]) -> Result<String, String> {
    if maps.is_empty() {
        return Err("no BMS maps to generate".to_owned());
    }
    let mut output = String::new();
    for map in maps {
        if map.name.len() > 8 {
            return Err(format!(
                "map name is too long for a COBOL symbolic name: {}",
                map.name
            ));
        }
        let input = format!("{}I", map.name);
        let outbound = format!("{}O", map.name);
        line(&mut output, "01", &input, "");
        if map.tioapfx {
            line(&mut output, "02", "FILLER", "PIC X(12)");
        }
        for field in map.fields.iter().filter(|field| field.name.is_some()) {
            let name = field.name.as_deref().unwrap();
            if field.length == 0 || name.len() > 8 {
                return Err(format!("unsupported symbolic field {name} length/name"));
            }
            let level = if field.occurs > 1 {
                line(
                    &mut output,
                    "02",
                    &format!("{name}G"),
                    &format!("OCCURS {}", field.occurs),
                );
                "03"
            } else {
                "02"
            };
            line(&mut output, level, &format!("{name}L"), "PIC S9(4) COMP");
            line(&mut output, level, &format!("{name}F"), "PIC X");
            line(
                &mut output,
                level,
                &format!("{name}A"),
                &format!("REDEFINES {name}F PIC X"),
            );
            line(
                &mut output,
                level,
                &format!("{name}I"),
                &format!("PIC X({})", field.length),
            );
            for _ in &map.extended_attributes {
                line(&mut output, level, "FILLER", "PIC X");
            }
        }
        line(&mut output, "01", &outbound, &format!("REDEFINES {input}"));
        if map.tioapfx {
            line(&mut output, "02", "FILLER", "PIC X(12)");
        }
        for field in map.fields.iter().filter(|field| field.name.is_some()) {
            let name = field.name.as_deref().unwrap();
            let level = if field.occurs > 1 {
                line(
                    &mut output,
                    "02",
                    &format!("{name}G"),
                    &format!("OCCURS {}", field.occurs),
                );
                "03"
            } else {
                "02"
            };
            line(&mut output, level, "FILLER", "PIC X(2)");
            line(&mut output, level, "FILLER", "PIC X");
            let picture = match &field.picout {
                Some(picture)
                    if picture
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || ".,+-/$*()".contains(c)) =>
                {
                    picture.clone()
                }
                Some(_) => return Err(format!("unsupported PICOUT for {name}")),
                None => format!("X({})", field.length),
            };
            line(
                &mut output,
                level,
                &format!("{name}O"),
                &format!("PIC {picture}"),
            );
            for suffix in &map.extended_attributes {
                line(&mut output, level, &format!("{name}{suffix}"), "PIC X");
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_menu_fields_and_layout() {
        let maps = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap();
        let copybook = generate(&maps).unwrap();
        assert!(copybook.contains("01 BNK1MEO REDEFINES BNK1MEI."));
        assert!(copybook.contains("02 ACTIONL PIC S9(4) COMP."));
        assert!(copybook.contains("02 ACTIONA REDEFINES ACTIONF PIC X."));
        assert!(copybook.contains("02 ACTIONO PIC X(1)."));
        assert!(copybook.contains("02 ACTIONC PIC X."));
        assert!(copybook.contains("02 ACTIONH PIC X."));
        assert!(copybook.contains("02 ACTIONU PIC X."));
        assert!(copybook.contains("02 ACTIONM PIC X."));
        assert!(copybook.contains("02 ACTIONP PIC X."));
        assert!(copybook.lines().all(|line| line.len() <= 72));
    }
}
