//! Minimal HLSL/C preprocessor for Source-engine FXC shaders.
//!
//! Supports `#define`, `#ifdef`/`#ifndef`, simple `#if`/`#elif` with
//! `defined()`, `#else`/`#endif`, `#include "..."`, and strips `#pragma`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PreprocessOptions {
    pub defines: HashMap<String, String>,
    pub include_dirs: Vec<PathBuf>,
}

impl Default for PreprocessOptions {
    fn default() -> Self {
        Self {
            defines: HashMap::new(),
            include_dirs: Vec::new(),
        }
    }
}

pub fn preprocess(source: &str, options: &PreprocessOptions) -> Result<String, String> {
    let mut ctx = Context {
        defines: options.defines.clone(),
        include_dirs: options.include_dirs.clone(),
        include_stack: Vec::new(),
    };
    ctx.process_source(source, "<input>")
}

struct Context {
    defines: HashMap<String, String>,
    include_dirs: Vec<PathBuf>,
    include_stack: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq)]
enum BranchState {
    /// Currently emitting tokens
    Active,
    /// Skipping because this `#if`/`#ifdef` was false (may become active on `#else`)
    InactiveWaitingElse,
    /// Skipping because a previous branch already won
    InactiveDone,
}

impl Context {
    fn process_source(&mut self, source: &str, from: &str) -> Result<String, String> {
        let mut out = String::with_capacity(source.len());
        let mut branch_stack: Vec<BranchState> = Vec::new();

        for (line_no, raw_line) in source.lines().enumerate() {
            let line = strip_line_comment(raw_line);
            let trimmed = line.trim_start();

            if let Some(directive) = trimmed.strip_prefix('#') {
                let directive = directive.trim_start();
                let (kw, rest) = split_directive(directive);

                match kw {
                    "ifdef" | "ifndef" => {
                        let name = rest.split_whitespace().next().unwrap_or("");
                        let defined = self.defines.contains_key(name);
                        let cond = if kw == "ifdef" { defined } else { !defined };
                        let parent_active =
                            branch_stack.last().copied().unwrap_or(BranchState::Active)
                                == BranchState::Active;
                        branch_stack.push(if !parent_active {
                            BranchState::InactiveDone
                        } else if cond {
                            BranchState::Active
                        } else {
                            BranchState::InactiveWaitingElse
                        });
                        continue;
                    }
                    "if" => {
                        let parent_active =
                            branch_stack.last().copied().unwrap_or(BranchState::Active)
                                == BranchState::Active;
                        let cond = parent_active && self.eval_expr(rest);
                        branch_stack.push(if !parent_active {
                            BranchState::InactiveDone
                        } else if cond {
                            BranchState::Active
                        } else {
                            BranchState::InactiveWaitingElse
                        });
                        continue;
                    }
                    "elif" => {
                        if branch_stack.is_empty() {
                            return Err(format!("{from}:{}: #elif without #if", line_no + 1));
                        }
                        let parent_active = branch_stack
                            .iter()
                            .rev()
                            .nth(1)
                            .copied()
                            .unwrap_or(BranchState::Active)
                            == BranchState::Active;
                        let state = *branch_stack.last().unwrap();
                        let new_state = match state {
                            BranchState::Active => BranchState::InactiveDone,
                            BranchState::InactiveWaitingElse => {
                                if parent_active && self.eval_expr(rest) {
                                    BranchState::Active
                                } else {
                                    BranchState::InactiveWaitingElse
                                }
                            }
                            BranchState::InactiveDone => BranchState::InactiveDone,
                        };
                        *branch_stack.last_mut().unwrap() = new_state;
                        continue;
                    }
                    "else" => {
                        let Some(state) = branch_stack.last_mut() else {
                            return Err(format!("{from}:{}: #else without #if", line_no + 1));
                        };
                        match *state {
                            BranchState::Active => *state = BranchState::InactiveDone,
                            BranchState::InactiveWaitingElse => *state = BranchState::Active,
                            BranchState::InactiveDone => {}
                        }
                        continue;
                    }
                    "endif" => {
                        if branch_stack.pop().is_none() {
                            return Err(format!("{from}:{}: #endif without #if", line_no + 1));
                        }
                        continue;
                    }
                    _ => {
                        // Remaining directives only apply in active branches.
                        if !is_active(&branch_stack) {
                            continue;
                        }
                        match kw {
                            "define" => {
                                let mut parts = rest.splitn(2, char::is_whitespace);
                                let name = parts.next().unwrap_or("").trim();
                                if name.is_empty() {
                                    return Err(format!(
                                        "{from}:{}: #define missing name",
                                        line_no + 1
                                    ));
                                }
                                // Skip function-macro form `NAME(` for now — treat as object macro
                                // only when there is no `(` immediately after the name.
                                if name.contains('(') {
                                    // Unsupported function-like macro — ignore body.
                                    continue;
                                }
                                let value = parts.next().unwrap_or("").trim().to_string();
                                self.defines.insert(name.to_string(), value);
                                continue;
                            }
                            "undef" => {
                                let name = rest.split_whitespace().next().unwrap_or("");
                                self.defines.remove(name);
                                continue;
                            }
                            "include" => {
                                let included = self.handle_include(rest, from, line_no + 1)?;
                                out.push_str(&included);
                                if !out.ends_with('\n') {
                                    out.push('\n');
                                }
                                continue;
                            }
                            "pragma" | "error" | "warning" | "line" => {
                                // Ignored / not required for bytecode emission.
                                continue;
                            }
                            _ => {
                                // Unknown directive — drop it.
                                continue;
                            }
                        }
                    }
                }
            }

            if !is_active(&branch_stack) {
                continue;
            }

            out.push_str(&self.expand_macros(raw_line));
            out.push('\n');
        }

        if !branch_stack.is_empty() {
            return Err(format!("{from}: unterminated #if/#ifdef"));
        }

        Ok(out)
    }

    fn handle_include(&mut self, rest: &str, from: &str, line: usize) -> Result<String, String> {
        let rest = rest.trim();
        let path = if let Some(s) = rest.strip_prefix('"') {
            let end = s
                .find('"')
                .ok_or_else(|| format!("{from}:{line}: malformed #include"))?;
            s[..end].to_string()
        } else if let Some(s) = rest.strip_prefix('<') {
            let end = s
                .find('>')
                .ok_or_else(|| format!("{from}:{line}: malformed #include"))?;
            s[..end].to_string()
        } else {
            return Err(format!("{from}:{line}: malformed #include"));
        };

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(parent) = Path::new(from).parent() {
            candidates.push(parent.join(&path));
        }
        for dir in &self.include_dirs {
            candidates.push(dir.join(&path));
        }
        candidates.push(PathBuf::from(&path));

        let file_path = candidates
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| format!("{from}:{line}: cannot open include \"{path}\""))?;

        if self.include_stack.iter().any(|p| p == &file_path) {
            return Err(format!(
                "{from}:{line}: recursive include of {}",
                file_path.display()
            ));
        }

        let contents = fs::read_to_string(&file_path)
            .map_err(|e| format!("{from}:{line}: reading {}: {e}", file_path.display()))?;

        self.include_stack.push(file_path.clone());
        let result = self.process_source(&contents, &file_path.to_string_lossy());
        self.include_stack.pop();
        result
    }

    fn eval_expr(&self, expr: &str) -> bool {
        // Enough for Source FXC: defined(X), !defined(X), integers, &&, ||, !, comparisons.
        match eval_condition(expr, &self.defines) {
            Ok(v) => v != 0,
            Err(_) => false,
        }
    }

    fn expand_macros(&self, line: &str) -> String {
        // Simple identifier replacement; skip if line is only whitespace.
        if self.defines.is_empty() {
            return line.to_string();
        }

        let mut result = String::with_capacity(line.len());
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if is_ident_start(c) {
                let start = i;
                i += 1;
                while i < chars.len() && is_ident_continue(chars[i]) {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                if let Some(val) = self.defines.get(&ident) {
                    // Don't expand the left-hand side of a `#define` (already handled).
                    result.push_str(val);
                } else {
                    result.push_str(&ident);
                }
            } else {
                result.push(c);
                i += 1;
            }
        }
        result
    }
}

fn is_active(stack: &[BranchState]) -> bool {
    stack.iter().all(|s| *s == BranchState::Active)
}

fn strip_line_comment(line: &str) -> &str {
    // Avoid stripping `http://` etc. — HLSL uses `//`.
    // Keep `//` inside strings roughly by only stripping outside quotes.
    let mut in_str = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        if !in_str && b == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn split_directive(directive: &str) -> (&str, &str) {
    let mut chars = directive.char_indices();
    let mut end = 0;
    for (i, c) in chars.by_ref() {
        if c.is_ascii_alphabetic() || c == '_' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return ("", directive);
    }
    let kw = &directive[..end];
    let rest = directive[end..].trim_start();
    (kw, rest)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn eval_condition(expr: &str, defines: &HashMap<String, String>) -> Result<i64, String> {
    let tokens = tokenize_expr(expr)?;
    let mut parser = ExprParser {
        tokens: &tokens,
        pos: 0,
        defines,
    };
    let value = parser.parse_or()?;
    if parser.pos != tokens.len() {
        return Err("trailing tokens in #if expression".into());
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Number(i64),
    Defined,
    LParen,
    RParen,
    Not,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
}

fn tokenize_expr(expr: &str) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            if ident == "defined" {
                tokens.push(Tok::Defined);
            } else {
                tokens.push(Tok::Ident(ident));
            }
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            tokens.push(Tok::Number(num.parse().unwrap_or(0)));
            continue;
        }
        match c {
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
            }
            '!' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::Ne);
                    i += 2;
                } else {
                    tokens.push(Tok::Not);
                    i += 1;
                }
            }
            '&' if i + 1 < chars.len() && chars[i + 1] == '&' => {
                tokens.push(Tok::And);
                i += 2;
            }
            '|' if i + 1 < chars.len() && chars[i + 1] == '|' => {
                tokens.push(Tok::Or);
                i += 2;
            }
            '=' if i + 1 < chars.len() && chars[i + 1] == '=' => {
                tokens.push(Tok::Eq);
                i += 2;
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::Le);
                    i += 2;
                } else {
                    tokens.push(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Tok::Ge);
                    i += 2;
                } else {
                    tokens.push(Tok::Gt);
                    i += 1;
                }
            }
            '+' => {
                tokens.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Tok::Minus);
                i += 1;
            }
            _ => {
                return Err(format!("unexpected character '{c}' in #if"));
            }
        }
    }
    Ok(tokens)
}

struct ExprParser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    defines: &'a HashMap<String, String>,
}

impl<'a> ExprParser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&'a Tok> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_or(&mut self) -> Result<i64, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::Or)) {
            self.advance();
            let right = self.parse_and()?;
            left = if left != 0 || right != 0 { 1 } else { 0 };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<i64, String> {
        let mut left = self.parse_cmp()?;
        while matches!(self.peek(), Some(Tok::And)) {
            self.advance();
            let right = self.parse_cmp()?;
            left = if left != 0 && right != 0 { 1 } else { 0 };
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> Result<i64, String> {
        let mut left = self.parse_add()?;
        loop {
            match self.peek() {
                Some(Tok::Eq) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left == right { 1 } else { 0 };
                }
                Some(Tok::Ne) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left != right { 1 } else { 0 };
                }
                Some(Tok::Lt) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left < right { 1 } else { 0 };
                }
                Some(Tok::Gt) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left > right { 1 } else { 0 };
                }
                Some(Tok::Le) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left <= right { 1 } else { 0 };
                }
                Some(Tok::Ge) => {
                    self.advance();
                    let right = self.parse_add()?;
                    left = if left >= right { 1 } else { 0 };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<i64, String> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.advance();
                    left += self.parse_unary()?;
                }
                Some(Tok::Minus) => {
                    self.advance();
                    left -= self.parse_unary()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<i64, String> {
        match self.peek() {
            Some(Tok::Not) => {
                self.advance();
                Ok(if self.parse_unary()? == 0 { 1 } else { 0 })
            }
            Some(Tok::Plus) => {
                self.advance();
                self.parse_unary()
            }
            Some(Tok::Minus) => {
                self.advance();
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64, String> {
        match self.advance() {
            Some(Tok::Number(n)) => Ok(*n),
            Some(Tok::Defined) => {
                // defined NAME  or  defined(NAME)
                let name = match self.peek() {
                    Some(Tok::LParen) => {
                        self.advance();
                        let name = match self.advance() {
                            Some(Tok::Ident(name)) => name.clone(),
                            _ => return Err("expected macro name".into()),
                        };
                        if !matches!(self.advance(), Some(Tok::RParen)) {
                            return Err("expected ')' after defined(".into());
                        }
                        name
                    }
                    Some(Tok::Ident(name)) => {
                        let name = name.clone();
                        self.advance();
                        name
                    }
                    _ => return Err("expected macro name after defined".into()),
                };
                Ok(if self.defines.contains_key(&name) {
                    1
                } else {
                    0
                })
            }
            Some(Tok::Ident(name)) => Ok(self.macro_value(name)),
            Some(Tok::LParen) => {
                let v = self.parse_or()?;
                if !matches!(self.advance(), Some(Tok::RParen)) {
                    return Err("expected ')'".into());
                }
                Ok(v)
            }
            other => Err(format!("unexpected token in #if: {other:?}")),
        }
    }

    fn macro_value(&self, name: &str) -> i64 {
        match self.defines.get(name) {
            Some(v) => {
                let v = v.trim();
                if v.is_empty() {
                    // `#define FOO` → defined as 1 in expressions
                    1
                } else {
                    v.parse::<i64>().unwrap_or(0)
                }
            }
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifdef_strips_inactive() {
        let mut opts = PreprocessOptions::default();
        opts.defines.insert("A".into(), "1".into());
        let out = preprocess(
            "#ifdef A\nfloat x;\n#else\nfloat y;\n#endif\nfloat z;\n",
            &opts,
        )
        .unwrap();
        assert!(out.contains("float x;"));
        assert!(!out.contains("float y;"));
        assert!(out.contains("float z;"));
    }

    #[test]
    fn if_defined_expr() {
        let mut opts = PreprocessOptions::default();
        opts.defines
            .insert("SHADER_MODEL_VS_3_0".into(), "1".into());
        opts.defines.insert("MORPHING".into(), "0".into());
        let out = preprocess(
            "#if (defined( SHADER_MODEL_VS_3_0 ) && MORPHING)\nint a;\n#else\nint b;\n#endif\n",
            &opts,
        )
        .unwrap();
        assert!(out.contains("int b;"));
        assert!(!out.contains("int a;"));
    }

    #[test]
    fn struct_field_guard() {
        let opts = PreprocessOptions::default();
        let out = preprocess(
            "struct S {\nfloat a;\n#ifdef SHADER_MODEL_VS_3_0\nfloat b;\n#endif\n};\n",
            &opts,
        )
        .unwrap();
        assert!(out.contains("float a;"));
        assert!(!out.contains("float b;"));
    }
}
