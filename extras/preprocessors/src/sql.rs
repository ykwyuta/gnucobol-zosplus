//! Fixed-format EXEC SQL translation shared by Db2 and PostgreSQL modes.
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type Host = Vec<String>;
type Cursors = HashMap<String, (String, Vec<Host>)>;

fn regex(source: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("valid SQL preprocessor pattern"))
}

fn host(token: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex(r"^:[A-Za-z][A-Za-z0-9_-]*$", &RE).is_match(token)
}

fn cobol_name(token: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex(r"^[A-Za-z][A-Za-z0-9_-]*$", &RE).is_match(token)
}

fn upper(token: &str) -> String {
    token.to_ascii_uppercase()
}

fn hosts(tokens: &[String]) -> Result<Vec<Host>, String> {
    if tokens.is_empty() {
        return Err("INTO にはホスト変数が必要です".into());
    }
    let mut items = Vec::new();
    let mut current = Vec::new();
    for token in tokens
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(","))
    {
        if token == "," {
            if current.len() == 3 && upper(current[1]) == "INDICATOR" {
                current.remove(1);
            }
            if !matches!(current.len(), 1 | 2) || current.iter().any(|part| !host(part)) {
                return Err("INTO のホスト変数一覧が不正です".into());
            }
            items.push(current.iter().map(|part| part[1..].to_string()).collect());
            current = Vec::new();
        } else {
            current.push(token);
        }
    }
    Ok(items)
}

fn parameters(tokens: &[String]) -> Result<(String, Vec<Host>), String> {
    let mut inputs = Vec::new();
    let mut output = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if host(token) {
            let mut pair = vec![token[1..].to_string()];
            if index + 2 < tokens.len()
                && upper(&tokens[index + 1]) == "INDICATOR"
                && host(&tokens[index + 2])
            {
                pair.push(tokens[index + 2][1..].to_string());
                index += 2;
            } else if index + 1 < tokens.len() && host(&tokens[index + 1]) {
                pair.push(tokens[index + 1][1..].to_string());
                index += 1;
            }
            inputs.push(pair);
            output.push("?".to_string());
        } else {
            output.push(token.clone());
        }
        index += 1;
    }
    let sql = output.join(" ");
    if inputs.len() > 99 || sql.len() > 8192 {
        return Err("SQL またはホスト変数が上限を超えています".into());
    }
    Ok((sql, inputs))
}

fn metadata(inputs: &[Host], outputs: &[Host]) -> String {
    let flags: String = inputs
        .iter()
        .chain(outputs)
        .map(|item| if item.len() == 2 { '1' } else { '0' })
        .collect();
    format!(
        "{:02},{:02}{}",
        inputs.len(),
        outputs.len(),
        if flags.contains('1') {
            flags.as_str()
        } else {
            ""
        }
    )
}

fn call_fields(items: &[Host]) -> Vec<String> {
    items
        .iter()
        .flat_map(|item| item.iter().map(|name| format!("               {name}\n")))
        .collect()
}

fn word_edge(code: &str, index: usize, length: usize) -> bool {
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-';
    (index == 0 || !word(code.as_bytes()[index - 1]))
        && (index + length == code.len() || !word(code.as_bytes()[index + length]))
}

fn scan_end(code: &str, mut block_comment: bool) -> (Option<usize>, bool) {
    let bytes = code.as_bytes();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let current = bytes[index];
        if block_comment {
            if bytes.get(index..index + 2) == Some(b"*/") {
                block_comment = false;
                index += 2;
                continue;
            }
        } else if let Some(delimiter) = quote {
            if current == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
        } else if bytes.get(index..index + 2) == Some(b"--") {
            return (None, block_comment);
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            block_comment = true;
            index += 2;
            continue;
        } else if current == b'\'' || current == b'"' {
            quote = Some(current);
        } else if bytes
            .get(index..index + 8)
            .is_some_and(|part| part.eq_ignore_ascii_case(b"END-EXEC"))
            && word_edge(code, index, 8)
        {
            return (Some(index), block_comment);
        }
        index += 1;
    }
    (None, block_comment)
}

fn tokens(body: &str) -> Result<Vec<String>, String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = regex(
        r#"(?s)\s+|--[^\n]*|/\*.*?\*/|'(?:''|[^'])*'|"(?:""|[^"])*"|:[A-Za-z][A-Za-z0-9_-]*|[A-Za-z_][A-Za-z0-9_-]*|[0-9]+(?:\.[0-9]+)?|<=|>=|<>|!=|[^\s]"#,
        &RE,
    );
    let mut output = Vec::new();
    let mut pos = 0;
    for found in re.find_iter(body) {
        if found.start() != pos {
            return Err("SQL トークンを解析できません".into());
        }
        let token = found.as_str();
        pos = found.end();
        if !token.trim().is_empty() && !token.starts_with("--") && !token.starts_with("/*") {
            output.push(token.to_string());
        }
    }
    if pos != body.len() {
        return Err("SQL トークンを解析できません".into());
    }
    Ok(output)
}

fn cobol_lines(statement: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = "           ".to_string();
    for word in statement.split(' ') {
        let addition = if current.ends_with(' ') {
            word.to_string()
        } else {
            format!(" {word}")
        };
        if current.len() + addition.len() > 72 {
            output.push(format!("{}\n", current.trim_end()));
            current = format!("               {word}");
        } else {
            current.push_str(&addition);
        }
    }
    output.push(format!("{}\n", current.trim_end()));
    output
}

fn literal_lines(sql: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    for ch in sql.chars() {
        let encoded = if ch == '\'' {
            "''".to_string()
        } else {
            ch.to_string()
        };
        if part.len() + encoded.len() > 44 {
            parts.push(std::mem::take(&mut part));
        }
        part.push_str(&encoded);
    }
    if !part.is_empty() {
        parts.push(part);
    }
    let mut result = vec![
        "           MOVE SPACES TO DB2-SQL-TEXT\n".into(),
        "           STRING\n".into(),
    ];
    result.extend(
        parts
            .into_iter()
            .map(|part| format!("               '{part}'\n")),
    );
    result.push("               INTO DB2-SQL-TEXT\n".into());
    result.push("           END-STRING\n".into());
    result
}

fn convert(
    body: &str,
    period: bool,
    cursors: &mut Cursors,
    include_dirs: &[PathBuf],
    stack: &[PathBuf],
) -> Result<Vec<String>, String> {
    let mut tokens = tokens(body)?;
    if tokens.is_empty() {
        return Err("空の EXEC SQL".into());
    }
    let head = upper(&tokens[0]);
    if head == "SET"
        && tokens.len() == 6
        && host(&tokens[1])
        && tokens[2] == "="
        && tokens[3..].iter().map(|t| upper(t)).collect::<Vec<_>>()
            == ["IDENTITY_VAL_LOCAL", "(", ")"]
    {
        return convert(
            &format!(
                "SELECT IDENTITY_VAL_LOCAL() INTO {} FROM SYSIBM.SYSDUMMY1",
                tokens[1]
            ),
            period,
            cursors,
            include_dirs,
            stack,
        );
    }
    if head == "INCLUDE" {
        if tokens.len() != 2 || !cobol_name(&tokens[1]) {
            return Err("INCLUDE のメンバー名が不正です".into());
        }
        if matches!(upper(&tokens[1]).as_str(), "SQLCA" | "SQLDA") {
            return Ok(vec![format!(
                "       COPY '{}.cpy'.\n",
                tokens[1].to_ascii_lowercase()
            )]);
        }
        for directory in include_dirs {
            let candidate = directory.join(format!("{}.cpy", tokens[1]));
            if candidate.is_file() {
                let resolved = candidate
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                if stack.contains(&resolved) {
                    return Err(format!("循環した SQL INCLUDE: {}", tokens[1]));
                }
                let mut nested = stack.to_vec();
                nested.push(resolved);
                let source = fs::read_to_string(&candidate).map_err(|error| error.to_string())?;
                return Ok(vec![transform(
                    &source,
                    include_dirs,
                    &nested,
                    cursors,
                    true,
                )?]);
            }
        }
        return Err(format!("SQL INCLUDE が見つかりません: {}", tokens[1]));
    }
    if head == "DECLARE"
        && tokens
            .iter()
            .skip(2)
            .take(4)
            .any(|token| upper(token) == "TABLE")
    {
        return Ok(vec![
            "      * Db2 DECLARE TABLE: runtime code is not required\n".into(),
        ]);
    }
    if head == "DECLARE" && tokens.len() >= 5 && cobol_name(&tokens[1]) {
        let mut index = 2;
        if tokens
            .get(index..index + 2)
            .is_some_and(|items| upper(&items[0]) == "INSENSITIVE" && upper(&items[1]) == "SCROLL")
        {
            index += 2;
        }
        if tokens.get(index).map(|token| upper(token)) != Some("CURSOR".into()) {
            return Err("未対応の DECLARE CURSOR オプション".into());
        }
        index += 1;
        if tokens
            .get(index..index + 2)
            .is_some_and(|items| upper(&items[0]) == "WITH" && upper(&items[1]) == "HOLD")
        {
            index += 2;
        }
        if tokens.len() <= index + 1 || upper(&tokens[index]) != "FOR" {
            return Err("DECLARE CURSOR には FOR SELECT が必要です".into());
        }
        index += 1;
        let name = upper(&tokens[1]);
        if cursors.contains_key(&name) {
            return Err(format!("カーソル名が重複しています: {name}"));
        }
        if upper(&tokens[index]) != "SELECT" {
            return Err("カーソルには SELECT が必要です".into());
        }
        cursors.insert(name.clone(), parameters(&tokens[index..])?);
        return Ok(vec![format!("      * Db2 cursor {name} declared\n")]);
    }
    if matches!(head.as_str(), "COMMIT" | "ROLLBACK") {
        let parts: Vec<String> = tokens.iter().map(|token| upper(token)).collect();
        if parts != [head.as_str()] && parts != [head.as_str(), "WORK"] {
            return Err(format!("{head} の構文が不正です"));
        }
        return Ok(cobol_lines(&format!(
            "CALL \"DB2SQL_{head}\" USING SQLCODE SQLSTATE{}",
            if period { "." } else { "" }
        )));
    }
    if matches!(head.as_str(), "OPEN" | "CLOSE" | "FETCH") {
        let offset = if head == "FETCH" && tokens.get(1).is_some_and(|token| upper(token) == "FROM")
        {
            2
        } else {
            1
        };
        if tokens.len() <= offset {
            return Err("カーソル名がありません".into());
        }
        let name = upper(&tokens[offset]);
        let (sql, inputs) = cursors
            .get(&name)
            .ok_or_else(|| format!("未定義のカーソル: {name}"))?
            .clone();
        if matches!(head.as_str(), "OPEN" | "CLOSE") && tokens.len() != offset + 1 {
            return Err(format!("{head} の構文が不正です"));
        }
        let mut result = vec![format!("           MOVE '{name}' TO DB2-SQL-CURSOR\n")];
        if head == "OPEN" {
            result.extend(literal_lines(&sql));
            result.push(format!(
                "           MOVE '{}' TO DB2-SQL-META\n",
                metadata(&inputs, &[])
            ));
            result.extend(cobol_lines("CALL \"DB2SQL_OPEN\" USING DB2-SQL-CURSOR DB2-SQL-TEXT DB2-SQL-META SQLCODE SQLSTATE"));
            result.extend(call_fields(&inputs));
        } else if head == "FETCH" {
            if tokens.len() <= offset + 2 || upper(&tokens[offset + 1]) != "INTO" {
                return Err("FETCH ... INTO が必要です".into());
            }
            let outputs = hosts(&tokens[offset + 2..])?;
            result.push(format!(
                "           MOVE '{}' TO DB2-SQL-META\n",
                metadata(&[], &outputs)
            ));
            result.extend(cobol_lines(
                "CALL \"DB2SQL_FETCH\" USING DB2-SQL-CURSOR DB2-SQL-META SQLCODE SQLSTATE",
            ));
            result.extend(call_fields(&outputs));
        } else {
            result.extend(cobol_lines(
                "CALL \"DB2SQL_CLOSE_CURSOR\" USING DB2-SQL-CURSOR SQLCODE SQLSTATE",
            ));
        }
        if period {
            if let Some(last) = result.last_mut() {
                *last = format!("{}.\n", last.trim_end_matches('\n'));
            }
        }
        return Ok(result);
    }
    if !matches!(head.as_str(), "INSERT" | "SELECT" | "UPDATE" | "DELETE") {
        return Err(format!("未対応の SQL 文: {head}"));
    }
    let mut outputs = Vec::new();
    if head == "SELECT" {
        let into = tokens.iter().position(|token| upper(token) == "INTO");
        let from = tokens.iter().position(|token| upper(token) == "FROM");
        let (Some(into), Some(from)) = (into, from) else {
            return Err("SELECT ... INTO ... FROM が必要".into());
        };
        if from <= into + 1 {
            return Err("SELECT ... INTO ... FROM が必要".into());
        }
        outputs = hosts(&tokens[into + 1..from])?;
        tokens.drain(into..from);
    }
    let (sql, inputs) = parameters(&tokens)?;
    if outputs.len() > 99 {
        return Err("出力ホスト変数は 99 個まで".into());
    }
    let mut result = literal_lines(&sql);
    result.push(format!(
        "           MOVE '{}' TO DB2-SQL-META\n",
        metadata(&inputs, &outputs)
    ));
    result.extend(cobol_lines(
        "CALL \"DB2SQL_EXEC\" USING DB2-SQL-TEXT DB2-SQL-META SQLCODE SQLSTATE",
    ));
    result.extend(call_fields(&inputs));
    result.extend(call_fields(&outputs));
    if period {
        if let Some(last) = result.last_mut() {
            *last = format!("{}.\n", last.trim_end_matches('\n'));
        }
    }
    Ok(result)
}

fn code(line: &str) -> &str {
    line.get(7..line.len().min(72)).unwrap_or(line)
}

fn transform(
    source: &str,
    include_dirs: &[PathBuf],
    stack: &[PathBuf],
    cursors: &mut Cursors,
    included: bool,
) -> Result<String, String> {
    static START: OnceLock<Regex> = OnceLock::new();
    static STORAGE: OnceLock<Regex> = OnceLock::new();
    let start = regex(r"(?i)^\s*EXEC\s+SQL\b", &START);
    let storage = regex(r"(?i)\bWORKING-STORAGE\s+SECTION\s*\.", &STORAGE);
    let mut result = String::new();
    let mut in_sql = false;
    let mut in_comment = false;
    let mut body = Vec::new();
    let mut start_line = 0;
    let mut have_storage = included;
    for (number, line) in source.split_inclusive('\n').enumerate() {
        if line
            .as_bytes()
            .get(6)
            .is_some_and(|marker| *marker == b'*' || *marker == b'/')
        {
            if !in_sql {
                result.push_str(line);
            }
            continue;
        }
        let mut text = code(line);
        if !in_sql {
            if storage.is_match(text) {
                have_storage = true;
                result.push_str(line);
                result.push_str("       01 DB2-SQL-TEXT PIC X(8192).\n       01 DB2-SQL-META PIC X(203).\n       01 DB2-SQL-CURSOR PIC X(64).\n");
                continue;
            }
            let Some(found) = start.find(text) else {
                result.push_str(line);
                continue;
            };
            in_sql = true;
            in_comment = false;
            start_line = number + 1;
            text = &text[found.end()..];
        }
        let (end, state) = scan_end(text, in_comment);
        in_comment = state;
        if let Some(end) = end {
            body.push(text[..end].to_string());
            let tail = text[end + 8..].trim();
            if tail != "" && tail != "." {
                return Err(format!(
                    "{}: END-EXEC の後に余分な文字があります",
                    number + 1
                ));
            }
            if !have_storage {
                return Err(format!("{start_line}: WORKING-STORAGE SECTION が必要です"));
            }
            let rendered = convert(&body.join("\n"), tail == ".", cursors, include_dirs, stack)
                .map_err(|error| format!("{start_line}: {error}"))?;
            for line in rendered {
                result.push_str(&line);
            }
            in_sql = false;
            body.clear();
        } else {
            body.push(text.trim_end().to_string());
        }
    }
    if in_sql {
        return Err(format!("{start_line}: END-EXEC がありません"));
    }
    Ok(result)
}

pub fn translate(
    source: &str,
    include_dirs: &[PathBuf],
    source_file: &Path,
) -> Result<String, String> {
    let root = source_file
        .canonicalize()
        .map_err(|error| error.to_string())?;
    transform(source, include_dirs, &[root], &mut HashMap::new(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_parameters_preserve_literals_and_indicators() {
        let tokens = tokens("INSERT INTO T VALUES(:A :AI, ':B', :C)").unwrap();
        let (sql, params) = parameters(&tokens).unwrap();
        assert_eq!(sql, "INSERT INTO T VALUES ( ? , ':B' , ? )");
        assert_eq!(params, [vec!["A", "AI"], vec!["C"]]);
    }
    #[test]
    fn scanner_ignores_end_exec_inside_sql_quotes_and_comments() {
        assert_eq!(
            scan_end("'END-EXEC' /* END-EXEC */ END-EXEC", false).0,
            Some(26)
        );
    }
}
