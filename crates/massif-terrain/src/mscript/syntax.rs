//! MassifScript's text: its tokens and the tree a script parses into.

use super::{Error, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A name; `quoted` when written between backticks (any id).
    Ident(String, bool),
    Str(String),
    Num(f64),
    /// `#rrggbb` written bare: a colour.
    Hex(String),
    /// Punctuation and operators: `(`, `==`, `..`…
    Sym(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

const SYMS: &[&str] = &["..", "==", "!=", "<=", ">=", "&&", "||", "(", ")", "{", "}", "[", "]", ",", ";", ":", ".", "=", "+", "-", "*", "/", "%", "^", "<", ">", "!", "?", "@"];

/// The words the language keeps for itself.
pub const KEYWORDS: &[&str] = &["project", "group", "component", "expose", "output", "const", "let", "if", "else", "for", "in", "import", "true", "false"];

/// The tokens of `src`, ending with [`Tok::Eof`].
pub fn lex(src: &str, file: u32) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    let mut out = Vec::new();
    let span = |start: usize, end: usize, line: u32, col: u32| Span { file, start, end, line, col };
    while i < b.len() {
        let c = b[i];
        // Whitespace and comments.
        if c == b'\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let (l0, c0, s0) = (line, col, i);
            i += 2;
            col += 2;
            loop {
                if i >= b.len() {
                    return Err(Error::new(span(s0, s0 + 2, l0, c0), "this comment is never closed: “*/” is missing"));
                }
                if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    i += 2;
                    col += 2;
                    break;
                }
                if b[i] == b'\n' {
                    line += 1;
                    col = 1;
                } else if (b[i] & 0xC0) != 0x80 {
                    col += 1;
                }
                i += 1;
            }
            continue;
        }
        let (start, l0, c0) = (i, line, col);
        let tok = if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Tok::Ident(src[start..i].to_owned(), false)
        } else if c == b'`' {
            i += 1;
            while i < b.len() && b[i] != b'`' && b[i] != b'\n' {
                i += 1;
            }
            if b.get(i) != Some(&b'`') {
                return Err(Error::new(span(start, i, l0, c0), "this name is never closed: “`” is missing"));
            }
            i += 1;
            Tok::Ident(src[start + 1..i - 1].to_owned(), true)
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
            // A fraction, but not a range's `..`.
            if b.get(i) == Some(&b'.') && b.get(i + 1) != Some(&b'.') {
                i += 1;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                    i += 1;
                }
            }
            if matches!(b.get(i), Some(b'e' | b'E')) && (b.get(i + 1).is_some_and(u8::is_ascii_digit) || (matches!(b.get(i + 1), Some(b'+' | b'-')) && b.get(i + 2).is_some_and(u8::is_ascii_digit))) {
                i += 2;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text = src[start..i].replace('_', "");
            match text.parse::<f64>() {
                Ok(v) => Tok::Num(v),
                Err(_) => return Err(Error::new(span(start, i, l0, c0), format!("“{text}” is not a number"))),
            }
        } else if c == b'"' {
            i += 1;
            let mut s = String::new();
            loop {
                let Some(&d) = b.get(i) else { return Err(Error::new(span(start, i, l0, c0), "this text is never closed: “\"” is missing")) };
                match d {
                    b'"' => {
                        i += 1;
                        break;
                    }
                    b'\n' => return Err(Error::new(span(start, i, l0, c0), "this text is never closed on its line: “\"” is missing")),
                    b'\\' => {
                        let e = b.get(i + 1).copied();
                        s.push(match e {
                            Some(b'n') => '\n',
                            Some(b't') => '\t',
                            Some(b'"') => '"',
                            Some(b'\\') => '\\',
                            _ => return Err(Error::new(span(i, i + 2, line, col + (i - start) as u32), "unknown escape: write \\n, \\t, \\\" or \\\\")),
                        });
                        i += 2;
                    }
                    _ => {
                        let ch = src[i..].chars().next().expect("in the text");
                        s.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            Tok::Str(s)
        } else if c == b'#' {
            i += 1;
            while i < b.len() && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let hex = &src[start..i];
            if !(hex.len() == 7 && hex[1..].chars().all(|c| c.is_ascii_hexdigit())) {
                return Err(Error::new(span(start, i, l0, c0), format!("“{hex}” is not a colour: write #rrggbb")));
            }
            Tok::Hex(hex.to_owned())
        } else if let Some(sym) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            i += sym.len();
            Tok::Sym(sym)
        } else {
            let ch = src[i..].chars().next().expect("a character");
            return Err(Error::new(span(i, i + ch.len_utf8(), l0, c0), format!("“{ch}” has no meaning here")));
        };
        col += src[start..i].chars().count() as u32;
        out.push(Token { tok, span: span(start, i, l0, c0) });
    }
    out.push(Token { tok: Tok::Eof, span: span(b.len(), b.len(), line, col) });
    Ok(out)
}

// ------------------------------------------------------------------ tree --

#[derive(Debug, Clone)]
pub enum Expr {
    Num(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
    List(Vec<Expr>, Span),
    Obj(Vec<(String, Expr)>, Span),
    Name(String, Span),
    /// `a.b`: an output of a node, or a field of an object.
    Field(Box<Expr>, String, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Call(Call),
    Unary(&'static str, Box<Expr>, Span),
    Binary(&'static str, Box<Expr>, Box<Expr>, Span),
    /// `a..b`, from `a` up to `b` left out.
    Range(Box<Expr>, Box<Expr>, Span),
    /// `c ? a : b`
    Cond(Box<Expr>, Box<Expr>, Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Num(_, s) | Expr::Str(_, s) | Expr::Bool(_, s) | Expr::List(_, s) | Expr::Obj(_, s) | Expr::Name(_, s) | Expr::Field(_, _, s) | Expr::Index(_, _, s) | Expr::Unary(_, _, s) | Expr::Binary(_, _, _, s) | Expr::Range(_, _, s) | Expr::Cond(_, _, _, s) => *s,
            Expr::Call(c) => c.span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    /// A name (`FractalNoise`), a type (`noise.fbm`, `"project:x"`).
    pub callee: String,
    pub version: Option<u16>,
    pub args: Vec<Arg>,
    pub span: Span,
    /// Where the callee's name is written.
    pub name_span: Span,
}

#[derive(Debug, Clone)]
pub struct Arg {
    /// Named (`scale: 900`, `mask.moss: m`, `@height: field`) or positional.
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

/// `key: value;` pairs in braces: a node's name, an exposed setting's range.
pub type Attrs = Vec<(String, Expr, Span)>;

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub optional: bool,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Project { name: String, body: Vec<Stmt>, span: Span },
    Group { id: Option<String>, title: Expr, body: Vec<Stmt>, span: Span },
    /// `name = expr;` or `name[i] = expr;`, with attributes for a node.
    Assign { name: String, index: Option<Expr>, value: Expr, attrs: Attrs, span: Span },
    Const { name: String, value: Expr, span: Span },
    Let { name: String, value: Expr, span: Span },
    Output { name: String, value: Option<Expr>, attrs: Attrs, span: Span },
    Component { name: String, params: Vec<Param>, body: Vec<Stmt>, span: Span },
    Expose { name: String, attrs: Attrs, span: Span },
    If { cond: Expr, then: Vec<Stmt>, other: Vec<Stmt>, span: Span },
    For { var: String, iter: Expr, body: Vec<Stmt>, span: Span },
    Import { path: String, span: Span },
}

// ---------------------------------------------------------------- parser --

pub struct Parser<'a> {
    toks: &'a [Token],
    at: usize,
}

type R<T> = Result<T, Error>;

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s, _) => format!("“{s}”"),
        Tok::Str(s) => format!("the text \"{s}\""),
        Tok::Num(n) => format!("the number {n}"),
        Tok::Hex(h) => format!("the colour {h}"),
        Tok::Sym(s) => format!("“{s}”"),
        Tok::Eof => "the end of the script".to_owned(),
    }
}

impl<'a> Parser<'a> {
    pub fn new(toks: &'a [Token]) -> Self {
        Self { toks, at: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.at].tok
    }

    fn peek_at(&self, k: usize) -> &Tok {
        &self.toks[(self.at + k).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.at].span
    }

    fn prev(&self) -> Span {
        self.toks[self.at.saturating_sub(1)].span
    }

    fn bump(&mut self) -> &Token {
        let t = &self.toks[self.at];
        if self.at + 1 < self.toks.len() {
            self.at += 1;
        }
        t
    }

    fn is(&self, sym: &str) -> bool {
        matches!(self.peek(), Tok::Sym(s) if *s == sym)
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s, false) if s == word)
    }

    fn eat(&mut self, sym: &str) -> bool {
        if self.is(sym) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: &str, what: &str) -> R<Span> {
        if self.is(sym) {
            Ok(self.bump().span)
        } else {
            Err(Error::new(self.span(), format!("expected “{sym}” {what}, found {}", describe(self.peek()))))
        }
    }

    /// A name that is not a keyword (or any between backticks).
    fn ident(&mut self, what: &str) -> R<(String, Span)> {
        match self.peek().clone() {
            Tok::Ident(s, quoted) if quoted || !KEYWORDS.contains(&s.as_str()) => {
                let sp = self.bump().span;
                Ok((s, sp))
            }
            t => Err(Error::new(self.span(), format!("expected {what}, found {}", describe(&t)))),
        }
    }

    fn string(&mut self, what: &str) -> R<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(s)
            }
            t => Err(Error::new(self.span(), format!("expected {what} in quotes, found {}", describe(&t)))),
        }
    }

    pub fn script(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        while *self.peek() != Tok::Eof {
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect("{", "to open the block")?;
        let mut out = Vec::new();
        while !self.is("}") {
            if *self.peek() == Tok::Eof {
                return Err(Error::new(self.span(), "this block is never closed: “}” is missing"));
            }
            out.push(self.stmt()?);
        }
        self.bump();
        Ok(out)
    }

    fn attrs(&mut self) -> R<Attrs> {
        let mut out = Vec::new();
        if !self.eat("{") {
            return Ok(out);
        }
        while !self.eat("}") {
            let (key, sp) = self.ident("a name (name, note, label…)")?;
            self.expect(":", "after the name")?;
            let v = self.expr()?;
            out.push((key, v, sp));
            if !self.eat(";") && !self.eat(",") && !self.is("}") {
                return Err(Error::new(self.span(), format!("expected “;” after the value, found {}", describe(self.peek()))));
            }
        }
        Ok(out)
    }

    fn semi(&mut self) -> R<()> {
        if self.eat(";") {
            Ok(())
        } else {
            Err(Error::new(self.prev().after(), format!("expected “;” at the end of the statement, found {}", describe(self.peek()))))
        }
    }

    fn stmt(&mut self) -> R<Stmt> {
        let start = self.span();
        let word = match self.peek() {
            Tok::Ident(s, false) => s.clone(),
            _ => String::new(),
        };
        match word.as_str() {
            "project" => {
                self.bump();
                let name = self.string("the project's name")?;
                let body = self.block()?;
                Ok(Stmt::Project { name, body, span: start.to(self.prev()) })
            }
            "group" => {
                self.bump();
                let id = if matches!(self.peek(), Tok::Ident(..)) { Some(self.ident("the group's id")?.0) } else { None };
                let title = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::Group { id, title, body, span: start.to(self.prev()) })
            }
            "component" => {
                self.bump();
                let (name, _) = self.ident("the component's name")?;
                let mut params = Vec::new();
                if self.eat("(") {
                    while !self.eat(")") {
                        let (pname, psp) = self.ident("a parameter's name")?;
                        self.expect(":", "and its type (height, mask, float, int…)")?;
                        let (ty, _) = self.ident("a type (height, mask, float, int…)")?;
                        let optional = self.eat("?");
                        let default = if self.eat("=") { Some(self.expr()?) } else { None };
                        params.push(Param { name: pname, ty, optional, default, span: psp.to(self.prev()) });
                        if !self.eat(",") && !self.is(")") {
                            return Err(Error::new(self.span(), format!("expected “,” or “)” in the parameters, found {}", describe(self.peek()))));
                        }
                    }
                }
                let body = self.block()?;
                Ok(Stmt::Component { name, params, body, span: start.to(self.prev()) })
            }
            "expose" => {
                self.bump();
                let (name, _) = self.ident("the parameter to expose")?;
                if !self.is("{") {
                    return Err(Error::new(self.span(), "expected “{” with what to show: label, min, max, unit…"));
                }
                let attrs = self.attrs()?;
                self.eat(";");
                Ok(Stmt::Expose { name, attrs, span: start.to(self.prev()) })
            }
            "output" => {
                self.bump();
                let (mut name, _) = self.ident("the output's name (height, water, caves…)")?;
                while self.eat(".") {
                    name.push('.');
                    name.push_str(&self.ident("the rest of the output's name")?.0);
                }
                let value = if self.eat("=") { Some(self.expr()?) } else { None };
                let attrs = self.attrs()?;
                self.semi()?;
                Ok(Stmt::Output { name, value, attrs, span: start.to(self.prev()) })
            }
            "const" | "let" => {
                self.bump();
                let (name, _) = self.ident("a name")?;
                self.expect("=", "and its value")?;
                let value = self.expr()?;
                self.semi()?;
                let span = start.to(self.prev());
                Ok(if word == "const" { Stmt::Const { name, value, span } } else { Stmt::Let { name, value, span } })
            }
            "if" => self.if_stmt(),
            "for" => {
                self.bump();
                let (var, _) = self.ident("the loop's variable")?;
                if !self.is_word("in") {
                    return Err(Error::new(self.span(), "expected “in”: for i in 0..4 { … }"));
                }
                self.bump();
                let iter = self.expr()?;
                let body = self.block()?;
                Ok(Stmt::For { var, iter, body, span: start.to(self.prev()) })
            }
            "import" => {
                self.bump();
                let path = self.string("the file to import")?;
                self.semi()?;
                Ok(Stmt::Import { path, span: start.to(self.prev()) })
            }
            _ => {
                let (name, nsp) = self.ident("a statement (a node, group, output…)")?;
                let index = if self.eat("[") {
                    let e = self.expr()?;
                    self.expect("]", "to close the index")?;
                    Some(e)
                } else {
                    None
                };
                if !self.is("=") {
                    return Err(Error::new(self.span(), format!("expected “=” after “{name}”: name = NodeType(…);, found {}", describe(self.peek()))));
                }
                self.bump();
                let value = self.expr()?;
                let attrs = self.attrs()?;
                self.semi()?;
                Ok(Stmt::Assign { name, index, value, attrs, span: nsp.to(self.prev()) })
            }
        }
    }

    fn if_stmt(&mut self) -> R<Stmt> {
        let start = self.bump().span;
        let cond = self.expr()?;
        let then = self.block()?;
        let other = if self.is_word("else") {
            self.bump();
            if self.is_word("if") { vec![self.if_stmt()?] } else { self.block()? }
        } else {
            Vec::new()
        };
        Ok(Stmt::If { cond, then, other, span: start.to(self.prev()) })
    }

    pub fn expr(&mut self) -> R<Expr> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":", "between the two choices of “?”")?;
            let b = self.expr()?;
            let sp = c.span().to(b.span());
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b), sp));
        }
        if self.eat("..") {
            let b = self.binary(0)?;
            let sp = c.span().to(b.span());
            return Ok(Expr::Range(Box::new(c), Box::new(b), sp));
        }
        Ok(c)
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let (op, prec, right) = match self.peek() {
                Tok::Sym(s @ "||") => (*s, 1, false),
                Tok::Sym(s @ "&&") => (*s, 2, false),
                Tok::Sym(s @ ("==" | "!=")) => (*s, 3, false),
                Tok::Sym(s @ ("<" | ">" | "<=" | ">=")) => (*s, 4, false),
                Tok::Sym(s @ ("+" | "-")) => (*s, 5, false),
                Tok::Sym(s @ ("*" | "/" | "%")) => (*s, 6, false),
                Tok::Sym(s @ "^") => (*s, 8, true),
                _ => break,
            };
            if prec < min {
                break;
            }
            self.bump();
            let rhs = self.binary(if right { prec } else { prec + 1 })?;
            let sp = lhs.span().to(rhs.span());
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs), sp);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        let sp = self.span();
        if self.eat("-") {
            let e = self.binary(7)?;
            let s = sp.to(e.span());
            return Ok(Expr::Unary("-", Box::new(e), s));
        }
        if self.eat("!") {
            let e = self.binary(7)?;
            let s = sp.to(e.span());
            return Ok(Expr::Unary("!", Box::new(e), s));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.is(".") && matches!(self.peek_at(1), Tok::Ident(..)) {
                self.bump();
                let Tok::Ident(f, _) = self.bump().tok.clone() else { unreachable!() };
                // `noise.fbm(…)`: a type written as it is.
                if self.is("(") || self.is("@") {
                    if let Some(path) = dotted(&e) {
                        let name_span = e.span().to(self.prev());
                        e = self.call(format!("{path}.{f}"), name_span)?;
                        continue;
                    }
                }
                let sp = e.span().to(self.prev());
                e = Expr::Field(Box::new(e), f, sp);
            } else if self.eat("[") {
                let i = self.expr()?;
                self.expect("]", "to close the index")?;
                let sp = e.span().to(self.prev());
                e = Expr::Index(Box::new(e), Box::new(i), sp);
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn call(&mut self, callee: String, name_span: Span) -> R<Expr> {
        let version = if self.eat("@") {
            match self.peek().clone() {
                Tok::Num(n) if n >= 1.0 && n.fract() == 0.0 && n < 65536.0 => {
                    self.bump();
                    Some(n as u16)
                }
                t => return Err(Error::new(self.span(), format!("expected a version after “@” (FractalNoise@2), found {}", describe(&t)))),
            }
        } else {
            None
        };
        self.expect("(", "to give the node its inputs and settings")?;
        let mut args = Vec::new();
        while !self.eat(")") {
            let start = self.span();
            let name = self.arg_name();
            let value = self.expr()?;
            args.push(Arg { name, value, span: start.to(self.prev()) });
            if !self.eat(",") && !self.is(")") {
                return Err(Error::new(self.span(), format!("expected “,” or “)” between the arguments, found {}", describe(self.peek()))));
            }
        }
        Ok(Expr::Call(Call { callee, version, args, span: name_span.to(self.prev()), name_span }))
    }

    /// `name:` before an argument (`mask.moss:`, `@height:` too), eaten.
    fn arg_name(&mut self) -> Option<String> {
        let mut k = 0;
        let mut name = String::new();
        if matches!(self.peek_at(0), Tok::Sym("@")) {
            name.push('@');
            k = 1;
        }
        loop {
            match self.peek_at(k) {
                Tok::Ident(s, _) => name.push_str(s),
                _ => return None,
            }
            k += 1;
            match self.peek_at(k) {
                Tok::Sym(":") => break,
                Tok::Sym(".") => {
                    name.push('.');
                    k += 1;
                }
                _ => return None,
            }
        }
        for _ in 0..=k {
            self.bump();
        }
        Some(name)
    }

    fn primary(&mut self) -> R<Expr> {
        let sp = self.span();
        match self.peek().clone() {
            Tok::Num(n) => {
                self.bump();
                Ok(Expr::Num(n, sp))
            }
            Tok::Str(s) => {
                self.bump();
                if self.is("(") {
                    return self.call(s, sp);
                }
                Ok(Expr::Str(s, sp))
            }
            Tok::Hex(h) => {
                self.bump();
                Ok(Expr::Str(h, sp))
            }
            Tok::Ident(s, false) if s == "true" || s == "false" => {
                self.bump();
                Ok(Expr::Bool(s == "true", sp))
            }
            Tok::Ident(s, quoted) if quoted || !KEYWORDS.contains(&s.as_str()) => {
                self.bump();
                if self.is("(") || (self.is("@") && matches!(self.peek_at(1), Tok::Num(_))) {
                    return self.call(s, sp);
                }
                Ok(Expr::Name(s, sp))
            }
            Tok::Sym("(") => {
                self.bump();
                let e = self.expr()?;
                self.expect(")", "to close the parenthesis")?;
                Ok(e)
            }
            Tok::Sym("[") => {
                self.bump();
                let mut items = Vec::new();
                while !self.eat("]") {
                    items.push(self.expr()?);
                    if !self.eat(",") && !self.is("]") {
                        return Err(Error::new(self.span(), format!("expected “,” or “]” in the list, found {}", describe(self.peek()))));
                    }
                }
                Ok(Expr::List(items, sp.to(self.prev())))
            }
            Tok::Sym("{") => {
                self.bump();
                let mut fields = Vec::new();
                while !self.eat("}") {
                    let key = match self.peek().clone() {
                        Tok::Str(s) => {
                            self.bump();
                            s
                        }
                        _ => self.ident("a field's name")?.0,
                    };
                    self.expect(":", "after the field's name")?;
                    fields.push((key, self.expr()?));
                    if !self.eat(",") && !self.is("}") {
                        return Err(Error::new(self.span(), format!("expected “,” or “}}” in the object, found {}", describe(self.peek()))));
                    }
                }
                Ok(Expr::Obj(fields, sp.to(self.prev())))
            }
            t => Err(Error::new(sp, format!("expected a value, found {}", describe(&t)))),
        }
    }
}

/// `a.b.c` written as names, as a type's path.
fn dotted(e: &Expr) -> Option<String> {
    match e {
        Expr::Name(n, _) => Some(n.clone()),
        Expr::Field(inner, f, _) => dotted(inner).map(|p| format!("{p}.{f}")),
        _ => None,
    }
}

/// Parses a whole script.
pub fn parse(src: &str, file: u32) -> Result<Vec<Stmt>, Error> {
    let toks = lex(src, file)?;
    Parser::new(&toks).script()
}
