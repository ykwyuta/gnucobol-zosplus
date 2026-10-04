use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Word,
    Number,
    String,
    Symbol,
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    text: String,
    line: usize,
}

fn lex(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut result = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let begin = line;
            i += 2;
            let mut closed = false;
            while i + 1 < chars.len() {
                if chars[i] == '\n' {
                    line += 1;
                }
                if chars[i] == '*' && chars[i + 1] == '/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(format!("line {begin}: unterminated comment"));
            }
        } else if c == '\'' || c == '"' {
            let quote = c;
            let begin = line;
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == quote {
                    if chars.get(i + 1) == Some(&quote) {
                        value.push(quote);
                        i += 2;
                    } else {
                        i += 1;
                        closed = true;
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    value.push(chars[i]);
                    i += 1;
                }
            }
            if !closed {
                return Err(format!("line {begin}: unterminated string"));
            }
            result.push(Token {
                kind: Kind::String,
                text: value,
                line: begin,
            });
        } else if c.is_ascii_alphabetic() || c == '_' {
            let begin = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            result.push(Token {
                kind: Kind::Word,
                text: chars[begin..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_uppercase(),
                line,
            });
        } else if c.is_ascii_digit() {
            let begin = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            result.push(Token {
                kind: Kind::Number,
                text: chars[begin..i].iter().collect(),
                line,
            });
        } else {
            let mut symbol = c.to_string();
            if matches!(c, '|' | '^' | '<' | '>')
                && chars.get(i + 1).is_some_and(|n| *n == '|' || *n == '=')
            {
                symbol.push(chars[i + 1]);
                i += 1;
            }
            result.push(Token {
                kind: Kind::Symbol,
                text: symbol,
                line,
            });
            i += 1;
        }
    }
    result.push(Token {
        kind: Kind::Eof,
        text: String::new(),
        line,
    });
    Ok(result)
}

#[derive(Clone)]
struct Decl {
    name: String,
    init: String,
    owner: String,
    initial: Option<String>,
}

struct Procedure {
    name: String,
    statements: String,
}

struct Cursor {
    sql: String,
    inputs: Vec<String>,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    decls: Vec<Decl>,
    names: HashSet<String>,
    calls: Vec<(String, usize)>,
    procedures: Vec<Procedure>,
    cursors: HashMap<String, Cursor>,
    sql_references: Vec<(String, usize)>,
    sql_used: bool,
    files: HashMap<String, usize>,
    file_handlers: HashMap<(String, String), String>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            decls: Vec::new(),
            names: HashSet::new(),
            calls: Vec::new(),
            procedures: Vec::new(),
            cursors: HashMap::new(),
            sql_references: Vec::new(),
            sql_used: false,
            files: HashMap::new(),
            file_handlers: HashMap::new(),
        }
    }

    fn at(&self, text: &str) -> bool {
        self.tokens[self.pos].kind != Kind::String && self.tokens[self.pos].text == text
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn error(&self, message: &str) -> String {
        format!("line {}: {message}", self.current().line)
    }

    fn take(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if token.kind != Kind::Eof {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, text: &str) -> bool {
        if self.at(text) {
            self.take();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, text: &str) -> Result<(), String> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {text}, found {:?}", self.current().text)))
        }
    }

    fn word(&mut self) -> Result<String, String> {
        if self.current().kind != Kind::Word {
            return Err(self.error("expected identifier"));
        }
        Ok(self.take().text)
    }

    fn integer(&mut self) -> Result<usize, String> {
        if self.current().kind != Kind::Number || self.current().text.contains('.') {
            return Err(self.error("expected unsigned integer"));
        }
        let line = self.current().line;
        self.take()
            .text
            .parse()
            .map_err(|_| format!("line {line}: integer out of range"))
    }

    fn procedure_start(&self) -> bool {
        self.current().kind == Kind::Word
            && self.tokens.get(self.pos + 1).is_some_and(|t| t.text == ":")
            && self
                .tokens
                .get(self.pos + 2)
                .is_some_and(|t| t.text == "PROCEDURE" || t.text == "PROC")
    }

    fn parse(
        mut self,
    ) -> Result<
        (
            String,
            Vec<Decl>,
            Vec<Procedure>,
            bool,
            HashMap<String, usize>,
        ),
        String,
    > {
        if !self.procedure_start() {
            return Err(self.error("expected NAME: PROCEDURE"));
        }
        let main = self.parse_procedure(true)?;
        while self.procedure_start() {
            self.parse_procedure(false)?;
        }
        if self.eat("END") {
            let end_name = self.word()?;
            if end_name != main {
                return Err(self.error(&format!("END {end_name} does not match {main}")));
            }
            self.expect(";")?;
        }
        if self.current().kind != Kind::Eof {
            return Err(self.error("unexpected text after procedure"));
        }
        let procedure_names: HashSet<_> = self.procedures.iter().map(|p| p.name.clone()).collect();
        for (name, line) in &self.calls {
            if !procedure_names.contains(name) {
                return Err(format!("line {line}: undeclared procedure {name}"));
            }
        }
        for (name, line) in &self.sql_references {
            if !self.names.contains(name) {
                return Err(format!("line {line}: undeclared SQL host variable {name}"));
            }
        }
        if self.sql_used && (!self.names.contains("SQLCODE") || !self.names.contains("SQLSTATE")) {
            return Err("EXEC SQL requires SQLCA (EXEC SQL INCLUDE SQLCA)".to_string());
        }
        Ok((main, self.decls, self.procedures, self.sql_used, self.files))
    }

    fn parse_procedure(&mut self, main: bool) -> Result<String, String> {
        let name = self.word()?;
        self.expect(":")?;
        if !self.eat("PROCEDURE") {
            self.expect("PROC")?;
        }
        if self.eat("OPTIONS") {
            self.expect("(")?;
            self.expect("MAIN")?;
            self.expect(")")?;
        } else if main {
            return Err(self.error("entry procedure requires OPTIONS(MAIN)"));
        }
        self.expect(";")?;
        let mut body = String::new();
        while !self.at("END") && !self.procedure_start() {
            if self.current().kind == Kind::Eof {
                return Err(self.error("unterminated procedure"));
            }
            if self.eat("DCL") {
                self.declare(&name)?;
            } else {
                body.push_str(&self.statement(&name)?);
            }
        }
        if !main || !self.procedure_start() {
            self.expect("END")?;
            if self.current().kind == Kind::Word {
                let end_name = self.word()?;
                if end_name != name {
                    return Err(self.error(&format!("END {end_name} does not match {name}")));
                }
            }
            self.expect(";")?;
        }
        self.procedures.push(Procedure {
            name: name.clone(),
            statements: body,
        });
        Ok(name)
    }

    fn declare(&mut self, owner: &str) -> Result<(), String> {
        loop {
            if self.current().kind == Kind::Number {
                self.integer()?;
            }
            let name = self.word()?;
            if self.at("FILE") {
                self.expect("FILE")?;
                self.expect("RECORD")?;
                self.expect("INPUT")?;
                self.expect("ENV")?;
                self.expect("(")?;
                self.expect("F")?;
                self.expect("RECSIZE")?;
                self.expect("(")?;
                let length = self.integer()?;
                self.expect(")")?;
                self.expect(")")?;
                self.expect(";")?;
                if length == 0 || length > 512 || self.files.insert(name.clone(), length).is_some()
                {
                    return Err(self.error("invalid or duplicate FILE declaration"));
                }
                return Ok(());
            }
            let (init, initial) = if self.at(",") || self.at(";") {
                (None, None)
            } else if self.eat("CHAR") || self.eat("CHARACTER") {
                self.expect("(")?;
                let n = self.integer()?;
                self.expect(")")?;
                if n > 512 {
                    return Err(self.error("CHAR length exceeds 512"));
                }
                (Some(format!("pli_storage_char({n})")), self.initializer()?)
            } else if self.eat("BIT") {
                self.expect("(")?;
                let n = self.integer()?;
                self.expect(")")?;
                if n != 1 {
                    return Err(self.error("only BIT(1) is implemented"));
                }
                (
                    Some("pli_storage_num(PLI_BIT, 1, 0)".to_string()),
                    self.initializer()?,
                )
            } else if self.eat("FIXED") {
                let kind = if self.eat("BIN") || self.eat("BINARY") {
                    "PLI_BIN"
                } else if self.eat("DEC") || self.eat("DECIMAL") {
                    "PLI_DEC"
                } else {
                    return Err(self.error("expected BIN or DEC after FIXED"));
                };
                self.expect("(")?;
                let precision = self.integer()?;
                let scale = if self.eat(",") { self.integer()? } else { 0 };
                self.expect(")")?;
                if precision == 0
                    || precision > if kind == "PLI_BIN" { 62 } else { 17 }
                    || scale > precision
                {
                    return Err(self.error("numeric precision or scale is unsupported"));
                }
                (
                    Some(format!("pli_storage_num({kind}, {precision}, {scale})")),
                    self.initializer()?,
                )
            } else {
                return Err(self.error("unsupported declaration attribute"));
            };
            if let Some(init) = init {
                if !self.names.insert(name.clone()) {
                    return Err(self.error(&format!("duplicate declaration {name}")));
                }
                self.decls.push(Decl {
                    name,
                    init,
                    owner: owner.to_string(),
                    initial,
                });
            }
            if self.eat(";") {
                break;
            }
            self.expect(",")?;
        }
        Ok(())
    }

    fn initializer(&mut self) -> Result<Option<String>, String> {
        if self.eat("INIT") || self.eat("INITIAL") {
            self.expect("(")?;
            let expr = self.expr(0)?;
            self.expect(")")?;
            Ok(Some(expr))
        } else {
            Ok(None)
        }
    }

    fn sql_statement(&mut self, owner: &str, line: usize) -> Result<String, String> {
        let mut tokens = Vec::new();
        while !self.at(";") {
            if self.current().kind == Kind::Eof {
                return Err(self.error("unterminated EXEC SQL"));
            }
            tokens.push(self.take());
        }
        self.expect(";")?;
        if tokens.is_empty() {
            return Err(format!("line {line}: empty EXEC SQL"));
        }
        self.sql_used = true;
        match tokens[0].text.as_str() {
            "INCLUDE" if tokens.len() == 2 && tokens[1].text == "SQLCA" => {
                for (name, init) in [
                    ("SQLCODE", "pli_storage_num(PLI_BIN, 31, 0)"),
                    ("SQLSTATE", "pli_storage_char(5)"),
                ] {
                    if !self.names.insert(name.to_string()) {
                        return Err(format!("line {line}: duplicate SQLCA declaration {name}"));
                    }
                    self.decls.push(Decl {
                        name: name.to_string(),
                        init: init.to_string(),
                        owner: owner.to_string(),
                        initial: None,
                    });
                }
                Ok(String::new())
            }
            "DECLARE"
                if tokens.len() >= 5 && tokens[2].text == "CURSOR" && tokens[3].text == "FOR" =>
            {
                let name = tokens[1].text.clone();
                if self.cursors.contains_key(&name) {
                    return Err(format!("line {line}: duplicate cursor {name}"));
                }
                let (sql, inputs) = self.sql_query(&tokens[4..], line)?;
                self.cursors.insert(name, Cursor { sql, inputs });
                Ok(String::new())
            }
            "OPEN" if tokens.len() == 2 => {
                let name = &tokens[1].text;
                let cursor = self
                    .cursors
                    .get(name)
                    .ok_or_else(|| format!("line {line}: undeclared cursor {name}"))?;
                Ok(format!(
                    "pli_db2_open(&v_SQLCODE, &v_SQLSTATE, \"{}\", \"{}\", {}, {});\n",
                    name,
                    c_escape(&cursor.sql),
                    cursor.inputs.len(),
                    sql_params(&cursor.inputs, &[])
                ))
            }
            "CLOSE" if tokens.len() == 2 => {
                let name = &tokens[1].text;
                if !self.cursors.contains_key(name) {
                    return Err(format!("line {line}: undeclared cursor {name}"));
                }
                Ok(format!(
                    "pli_db2_close(&v_SQLCODE, &v_SQLSTATE, \"{}\");\n",
                    name
                ))
            }
            "FETCH" if tokens.len() >= 4 && tokens[2].text == "INTO" => {
                let name = &tokens[1].text;
                if !self.cursors.contains_key(name) {
                    return Err(format!("line {line}: undeclared cursor {name}"));
                }
                let outputs = self.sql_targets(&tokens[3..], line)?;
                Ok(format!(
                    "pli_db2_fetch(&v_SQLCODE, &v_SQLSTATE, \"{}\", {}, {});\n",
                    name,
                    outputs.len(),
                    sql_params(&[], &outputs)
                ))
            }
            "SELECT" => {
                let into = tokens
                    .iter()
                    .position(|token| token.text == "INTO")
                    .ok_or_else(|| format!("line {line}: SELECT requires INTO"))?;
                let from = tokens
                    .iter()
                    .enumerate()
                    .skip(into + 1)
                    .find(|(_, token)| token.text == "FROM")
                    .map(|(i, _)| i)
                    .ok_or_else(|| format!("line {line}: SELECT requires FROM"))?;
                let outputs = self.sql_targets(&tokens[into + 1..from], line)?;
                let query: Vec<_> = tokens[..into]
                    .iter()
                    .chain(tokens[from..].iter())
                    .cloned()
                    .collect();
                let (sql, inputs) = self.sql_query(&query, line)?;
                Ok(format!(
                    "pli_db2_select(&v_SQLCODE, &v_SQLSTATE, \"{}\", {}, {}, {}, {});\n",
                    c_escape(&sql),
                    inputs.len(),
                    sql_params(&inputs, &[]),
                    outputs.len(),
                    sql_params(&[], &outputs)
                ))
            }
            "COMMIT" if tokens.len() == 1 || (tokens.len() == 2 && tokens[1].text == "WORK") => {
                Ok("pli_db2_commit(&v_SQLCODE, &v_SQLSTATE);\n".to_string())
            }
            _ => Err(format!(
                "line {line}: unsupported EXEC SQL statement {}",
                tokens[0].text
            )),
        }
    }

    fn sql_query(
        &mut self,
        tokens: &[Token],
        line: usize,
    ) -> Result<(String, Vec<String>), String> {
        let mut sql = String::new();
        let mut inputs = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let token = &tokens[i];
            if token.text == ":" {
                let var = tokens
                    .get(i + 1)
                    .filter(|t| t.kind == Kind::Word)
                    .ok_or_else(|| format!("line {line}: expected SQL host variable"))?;
                inputs.push(var.text.clone());
                self.sql_references.push((var.text.clone(), var.line));
                sql.push_str("? ");
                i += 2;
            } else {
                if token.kind == Kind::String {
                    sql.push('\'');
                    sql.push_str(&token.text.replace('\'', "''"));
                    sql.push('\'');
                } else {
                    sql.push_str(&token.text);
                }
                sql.push(' ');
                i += 1;
            }
        }
        Ok((sql.trim().to_string(), inputs))
    }

    fn sql_targets(
        &mut self,
        tokens: &[Token],
        line: usize,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let mut result = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text != ":" || i + 1 >= tokens.len() || tokens[i + 1].kind != Kind::Word {
                return Err(format!("line {line}: invalid SQL INTO target"));
            }
            let value = tokens[i + 1].text.clone();
            self.sql_references
                .push((value.clone(), tokens[i + 1].line));
            i += 2;
            let indicator = if i + 1 < tokens.len() && tokens[i].text == ":" {
                let indicator = tokens[i + 1].text.clone();
                self.sql_references
                    .push((indicator.clone(), tokens[i + 1].line));
                i += 2;
                Some(indicator)
            } else {
                None
            };
            result.push((value, indicator));
            if i == tokens.len() {
                break;
            }
            if tokens[i].text != "," {
                return Err(format!("line {line}: expected comma in SQL INTO"));
            }
            i += 1;
        }
        if result.is_empty() {
            return Err(format!("line {line}: empty SQL INTO"));
        }
        Ok(result)
    }

    fn statement(&mut self, owner: &str) -> Result<String, String> {
        let line = self.current().line;
        if self.eat("ON") {
            self.expect("ENDFILE")?;
            self.expect("(")?;
            let file = self.word()?;
            self.expect(")")?;
            if !self.files.contains_key(&file) {
                return Err(format!("line {line}: undeclared FILE {file}"));
            }
            self.expect("BEGIN")?;
            self.expect(";")?;
            let mut body = String::new();
            while !self.at("END") {
                if self.current().kind == Kind::Eof {
                    return Err(self.error("unterminated ON ENDFILE handler"));
                }
                body.push_str(&self.statement(owner)?);
            }
            self.expect("END")?;
            self.expect(";")?;
            if self
                .file_handlers
                .insert((owner.to_string(), file), body)
                .is_some()
            {
                return Err(format!("line {line}: duplicate ON ENDFILE handler"));
            }
            return Ok(String::new());
        }
        if self.eat("OPEN") || self.eat("READ") || self.eat("CLOSE") {
            let operation = self.tokens[self.pos - 1].text.clone();
            self.expect("FILE")?;
            self.expect("(")?;
            let file = self.word()?;
            self.expect(")")?;
            if !self.files.contains_key(&file) {
                return Err(format!("line {line}: undeclared FILE {file}"));
            }
            if operation == "READ" {
                self.expect("INTO")?;
                self.expect("(")?;
                let target = self.word()?;
                self.expect(")")?;
                self.expect(";")?;
                if !self.names.contains(&target) {
                    return Err(format!("line {line}: undeclared READ target {target}"));
                }
                let handler = self
                    .file_handlers
                    .get(&(owner.to_string(), file.clone()))
                    .ok_or_else(|| format!("line {line}: READ FILE requires ON ENDFILE({file})"))?;
                return Ok(format!(
                    "if (!pli_file_read(&f_{file}, &v_{target})) {{ {handler} }}\n"
                ));
            }
            self.expect(";")?;
            if operation == "CLOSE" {
                return Ok(format!("pli_file_close(&f_{file});\n"));
            }
            let handler = self
                .file_handlers
                .get(&(owner.to_string(), file.clone()))
                .ok_or_else(|| format!("line {line}: OPEN FILE requires ON ENDFILE({file})"))?;
            return Ok(format!(
                "if (!pli_file_open(&f_{file}, \"{file}\", {})) {{ {handler} }}\n",
                self.files[&file]
            ));
        }
        if self.eat("EXEC") {
            self.expect("SQL")?;
            return self.sql_statement(owner, line);
        }
        if self.eat("IF") {
            let condition = self.expr(0)?;
            self.expect("THEN")?;
            let yes = self.statement(owner)?;
            let no = if self.eat("ELSE") {
                format!(" else {{ {} }}", self.statement(owner)?)
            } else {
                String::new()
            };
            return Ok(format!("if (pli_truth({condition})) {{ {yes} }}{no}\n"));
        }
        if self.eat("DO") {
            let condition = if self.eat("WHILE") {
                self.expect("(")?;
                let expr = self.expr(0)?;
                self.expect(")")?;
                Some(expr)
            } else {
                None
            };
            self.expect(";")?;
            let mut body = String::new();
            while !self.at("END") {
                if self.current().kind == Kind::Eof {
                    return Err(self.error("unterminated DO"));
                }
                body.push_str(&self.statement(owner)?);
            }
            self.expect("END")?;
            self.expect(";")?;
            return Ok(match condition {
                Some(condition) => format!("while (pli_truth({condition})) {{\n{body}}}\n"),
                None => format!("{{\n{body}}}\n"),
            });
        }
        if self.eat("PUT") {
            if self.eat("PAGE") {
                self.expect(";")?;
                return Ok("pli_put_page();\n".to_string());
            }
            if self.eat("STRING") {
                self.expect("(")?;
                let target = self.word()?;
                self.expect(")")?;
                if !self.names.contains(&target) {
                    return Err(format!(
                        "line {line}: undeclared PUT STRING target {target}"
                    ));
                }
                let result = self.edit_values()?;
                self.expect(";")?;
                return Ok(format!("pli_assign(&v_{target}, {result});\n"));
            }
            self.expect("SKIP")?;
            if self.at("EDIT") {
                let result = self.edit_values()?;
                self.expect(";")?;
                return Ok(format!("pli_put_skip_edit({result});\n"));
            }
            self.expect("LIST")?;
            self.expect("(")?;
            let args = self.arguments()?;
            self.expect(";")?;
            return Ok(format!(
                "pli_put_skip_list({}, (pli_value[]){{{}}});\n",
                args.len(),
                args.join(", ")
            ));
        }
        if self.eat("CALL") {
            let target = self.word()?;
            self.expect(";")?;
            self.calls.push((target.clone(), line));
            return Ok(format!("proc_{target}();\n"));
        }
        if self.eat("RETURN") {
            self.expect(";")?;
            return Ok("return;\n".to_string());
        }
        if self.eat("GO") {
            self.expect("TO")?;
            let target = self.word()?;
            self.expect(";")?;
            return Ok(format!("goto label_{target};\n"));
        }
        if self.current().kind == Kind::Word {
            let name = self.word()?;
            if self.eat(":") {
                let nested = self.statement(owner)?;
                return Ok(format!("label_{name}:; {nested}"));
            }
            if !self.names.contains(&name) {
                return Err(format!("line {line}: undeclared variable {name}"));
            }
            self.expect("=")?;
            let value = self.expr(0)?;
            self.expect(";")?;
            return Ok(format!("pli_assign(&v_{name}, {value});\n"));
        }
        Err(self.error("unsupported statement"))
    }

    fn edit_values(&mut self) -> Result<String, String> {
        self.expect("EDIT")?;
        self.expect("(")?;
        let values = self.arguments()?;
        self.expect("(")?;
        let mut formatted = Vec::new();
        for (index, value) in values.iter().enumerate() {
            let converted = if self.eat("A") {
                value.clone()
            } else if self.eat("F") {
                self.expect("(")?;
                let width = self.integer()?;
                let decimals = if self.eat(",") { self.integer()? } else { 0 };
                self.expect(")")?;
                if width == 0 || width > 512 || decimals > 17 {
                    return Err(self.error("unsupported F edit width or scale"));
                }
                format!("pli_format_fixed({value}, {width}, {decimals})")
            } else {
                return Err(self.error("unsupported EDIT format"));
            };
            formatted.push(converted);
            if index + 1 < values.len() {
                self.expect(",")?;
            }
        }
        self.expect(")")?;
        if formatted.is_empty() {
            return Err(self.error("empty EDIT list"));
        }
        let mut result = formatted.remove(0);
        for value in formatted {
            result = format!("pli_concat({result}, {value})");
        }
        Ok(result)
    }

    fn arguments(&mut self) -> Result<Vec<String>, String> {
        let mut args = Vec::new();
        if !self.eat(")") {
            loop {
                args.push(self.expr(0)?);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        Ok(args)
    }

    fn expr(&mut self, min_binding: u8) -> Result<String, String> {
        let mut left = if self.eat("^") {
            format!("pli_not({})", self.expr(7)?)
        } else if self.eat("-") {
            format!("pli_neg({})", self.expr(7)?)
        } else if self.eat("+") {
            self.expr(7)?
        } else if self.eat("(") {
            let expr = self.expr(0)?;
            self.expect(")")?;
            expr
        } else {
            let token = self.take();
            match token.kind {
                Kind::Number => format!("pli_number(\"{}\")", token.text),
                Kind::String => {
                    if self.eat("B") {
                        match token.text.as_str() {
                            "0" => "pli_bit(0)".to_string(),
                            "1" => "pli_bit(1)".to_string(),
                            _ => {
                                return Err(format!(
                                    "line {}: only BIT(1) literals are implemented",
                                    token.line
                                ))
                            }
                        }
                    } else {
                        format!("pli_literal(\"{}\")", c_escape(&token.text))
                    }
                }
                Kind::Word => {
                    if self.eat("(") {
                        let args = self.arguments()?;
                        builtin(&token.text, &args, token.line)?
                    } else if self.names.contains(&token.text) {
                        format!("v_{}", token.text)
                    } else {
                        return Err(format!(
                            "line {}: undeclared variable {}",
                            token.line, token.text
                        ));
                    }
                }
                _ => return Err(format!("line {}: expected expression", token.line)),
            }
        };
        loop {
            let (binding, function) = match self.current().text.as_str() {
                "|" => (1, "pli_or"),
                "&" => (2, "pli_and"),
                "=" => (3, "pli_eq"),
                "^=" => (3, "pli_ne"),
                "<" => (3, "pli_lt"),
                "<=" => (3, "pli_le"),
                ">" => (3, "pli_gt"),
                ">=" => (3, "pli_ge"),
                "||" => (4, "pli_concat"),
                "+" => (5, "pli_add"),
                "-" => (5, "pli_sub"),
                "*" => (6, "pli_mul"),
                "/" => (6, "pli_div"),
                _ => break,
            };
            if binding < min_binding {
                break;
            }
            self.take();
            let right = self.expr(binding + 1)?;
            left = format!("{function}({left}, {right})");
        }
        Ok(left)
    }
}

fn builtin(name: &str, args: &[String], line: usize) -> Result<String, String> {
    let (function, count) = match name {
        "SUBSTR" => ("pli_substr", 3),
        "TRIM" => ("pli_trim", 1),
        "CENTRE" => ("pli_centre", 2),
        "CHAR" => ("pli_char_builtin", 1),
        "ABS" => ("pli_abs", 1),
        "DATETIME" => ("pli_datetime", 0),
        _ => return Err(format!("line {line}: unsupported builtin {name}")),
    };
    if args.len() != count {
        return Err(format!("line {line}: {name} expects {count} arguments"));
    }
    Ok(format!("{function}({})", args.join(", ")))
}

fn c_escape(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ if c.is_ascii_graphic() || c == ' ' => result.push(c),
            _ => {
                for b in c.to_string().as_bytes() {
                    write!(result, "\\{:03o}", b).unwrap();
                }
            }
        }
    }
    result
}

fn sql_params(inputs: &[String], outputs: &[(String, Option<String>)]) -> String {
    if inputs.is_empty() && outputs.is_empty() {
        return "NULL".to_string();
    }
    let mut items: Vec<String> = inputs
        .iter()
        .map(|name| format!("{{&v_{name}, NULL}}"))
        .collect();
    items.extend(outputs.iter().map(|(name, indicator)| {
        format!(
            "{{&v_{name}, {}}}",
            indicator
                .as_ref()
                .map_or_else(|| "NULL".to_string(), |name| format!("&v_{name}"))
        )
    }));
    format!("(pli_sql_param[]){{{}}}", items.join(", "))
}

fn compile(source: &str) -> Result<String, String> {
    let (main, decls, procedures, sql_used, files) = Parser::new(lex(source)?).parse()?;
    let mut output = String::from("#include \"pli_runtime.h\"\n\n");
    if sql_used {
        output.push_str("#include \"pli_db2.h\"\n\n");
    }
    for decl in &decls {
        writeln!(output, "static pli_value v_{};", decl.name).unwrap();
    }
    let mut file_names: Vec<_> = files.keys().collect();
    file_names.sort();
    for file in file_names {
        writeln!(output, "static pli_file f_{file};").unwrap();
    }
    output.push('\n');
    for proc in &procedures {
        writeln!(output, "static void proc_{}(void);", proc.name).unwrap();
    }
    for proc in &procedures {
        writeln!(output, "\nstatic void proc_{}(void)\n{{", proc.name).unwrap();
        for decl in decls.iter().filter(|d| d.owner == proc.name) {
            writeln!(output, "v_{} = {};", decl.name, decl.init).unwrap();
            if let Some(value) = &decl.initial {
                writeln!(output, "pli_assign(&v_{}, {});", decl.name, value).unwrap();
            }
        }
        output.push_str(&proc.statements);
        output.push_str("}\n");
    }
    writeln!(
        output,
        "\nint main(void)\n{{\nproc_{main}();\nreturn 0;\n}}"
    )
    .unwrap();
    Ok(output)
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-C" => {}
            "-o" => output = Some(PathBuf::from(args.next().ok_or("-o requires a path")?)),
            "-h" | "--help" => {
                println!("usage: pl1c -C INPUT.pli [-o OUTPUT.c]");
                return Ok(());
            }
            _ if arg.starts_with('-') => return Err(format!("unsupported option {arg}")),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err("multiple input files are not implemented".to_string()),
        }
    }
    let input = input.ok_or("input file is required")?;
    let output = output.unwrap_or_else(|| input.with_extension("c"));
    let source = fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let generated = compile(&source).map_err(|e| format!("{}:{e}", input.display()))?;
    fs::write(&output, generated).map_err(|e| format!("{}: {e}", output.display()))?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("pl1c: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_c_for_arithmetic_and_put() {
        let source = include_str!("../tests/hello.pli");
        let c = compile(source).unwrap();
        assert!(c.contains("pli_add(v_COUNTER, pli_number(\"1\"))"));
        assert!(c.contains("pli_put_skip_list(2"));
        assert!(c.contains("proc_HELLO();"));
    }

    #[test]
    fn unsupported_sql_is_diagnostic() {
        let source = "X: PROC OPTIONS(MAIN); EXEC SQL DELETE FROM T; END X;";
        assert!(compile(source)
            .unwrap_err()
            .contains("unsupported EXEC SQL statement DELETE"));
    }

    #[test]
    fn undeclared_name_is_diagnostic() {
        let source = "X: PROC OPTIONS(MAIN); Y = 2; END X;";
        assert!(compile(source)
            .unwrap_err()
            .contains("undeclared variable Y"));
    }

    #[test]
    fn db2_select_binds_host_variable() {
        let c = compile(include_str!("../tests/db2-select.pli")).unwrap();
        assert!(c.contains("IBMREQD = ?"));
        assert!(c.contains("pli_db2_select(&v_SQLCODE"));
        assert!(c.contains("{&v_FLAG, NULL}"));
        assert!(c.contains("{&v_ANSWER, NULL}"));
    }

    #[test]
    fn db2_cursor_emits_open_fetch_close() {
        let c = compile(include_str!("../tests/db2-cursor.pli")).unwrap();
        assert!(c.contains("pli_db2_open(&v_SQLCODE"));
        assert!(c.contains("pli_db2_fetch(&v_SQLCODE"));
        assert!(c.contains("pli_db2_close(&v_SQLCODE"));
    }

    #[test]
    fn db2_null_target_has_indicator() {
        let c = compile(include_str!("../tests/db2-null.pli")).unwrap();
        assert!(c.contains("{&v_VALUE, &v_IND}"));
    }

    #[test]
    fn sql_without_sqlca_is_diagnostic() {
        let source = "X: PROC OPTIONS(MAIN); DCL A FIXED BIN(31); EXEC SQL SELECT 1 INTO :A FROM SYSIBM.SYSDUMMY1; END X;";
        assert!(compile(source).unwrap_err().contains("requires SQLCA"));
    }

    #[test]
    fn compiles_bank_statement_source() {
        let source =
            include_str!("../../../reference/Bank-of-Z-main/src/base/batch/pli/BNKSTMT.pli");
        let c = compile(source).unwrap();
        assert!(c.contains("static void proc_BNKSTMT(void)"));
        assert!(c.contains("pli_file_open(&f_SORTCODE"));
        assert!(c.contains("pli_db2_open(&v_SQLCODE"));
        assert!(c.contains("pli_format_fixed(v_OPENING_BALANCE, 10, 2)"));
    }

    #[test]
    fn compiles_file_edit_example() {
        let c = compile(include_str!("../tests/file-edit.pli")).unwrap();
        assert!(c.contains("pli_file_read(&f_CARD"));
        assert!(c.contains("pli_put_skip_edit(v_OUTPUT_LINE)"));
    }
}
