//! Schemify's expression language in scalar mode, and calculator-mode checking: what Interop
//! needs to validate, fold and evaluate the expressions it translates. Written from the public
//! spec (`docs/spec/expr.md` of the Schemify SDK) alone.
//!
//! `compile` lexes and parses in one pass into stack code (post order, jumps for the lazy
//! `?:`, `if`, `&&`, `||`), folding every constant subexpression as it completes: a fold that
//! fails where the expression always runs is the compile error spec §4 asks for. Evaluation
//! runs that code over one value per identifier, in first-occurrence order.
//!
//! ```
//! use interop::expr::{Mode, Program, Scratch, Value};
//! let p = Program::compile("w * 2 + 1u", Mode::Scalar).unwrap();
//! assert_eq!(p.idents().collect::<Vec<_>>(), ["w"]);
//! let v = p.eval(&[Value::Num(1e-6)], &mut Scratch::default()).unwrap();
//! assert_eq!(v, Value::Num(3e-6));
//! assert!(Program::compile("1/0", Mode::Scalar).is_err());
//! assert!(Program::compile("ymax(v(out))", Mode::Calc).is_ok());
//! ```

// ponytail: Interop's own copy because the SDK publishes no expression crate yet; replace it
// with that crate when it does (the names here are the ones Interop calls).

mod eval;
mod text;

pub use schemify_abi::text::push_num as write_num;
pub use text::{parse_num, write_int};

use eval::{Builtin, Op};

/// Error codes of spec §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Code {
    X001,
    X002,
    X003,
    X004,
    X005,
    X006,
    X007,
    X008,
    X009,
    X010,
    X011,
}

/// A compile or evaluation error.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    /// What went wrong.
    pub code: Code,
    /// For people.
    pub message: String,
}

pub(crate) fn err<T>(code: Code, message: impl Into<String>) -> Result<T, Error> {
    Err(Error {
        code,
        message: message.into(),
    })
}

/// Which functions an expression may call (spec §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Params, vars, templates, specs.
    Scalar,
    /// Outputs and traces: also the calculator functions (compile only here).
    Calc,
}

/// A value (spec §1; no columns: Interop evaluates scalar mode only).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `int`.
    Int(i64),
    /// `num`, always finite.
    Num(f64),
    /// `bool`.
    Bool(bool),
    /// `str`.
    Str(String),
    /// `cnum` as (re, im).
    Cnum(f64, f64),
}

/// Evaluation stack, reused across calls.
#[derive(Default)]
pub struct Scratch {
    stack: Vec<Value>,
}

/// A compiled expression (or every hole of a template).
#[derive(Debug)]
pub struct Program {
    code: Vec<Op>,
    consts: Vec<Value>,
    idents: Vec<String>,
    plugins: Vec<String>,
    /// Code range of each root: one for an expression, one per template hole.
    roots: Vec<(u32, u32)>,
}

impl Program {
    /// Compiles `src` (spec §2–§5); constant errors on always-evaluated parts are reported.
    ///
    /// # Errors
    ///
    /// The first lexical, syntax, arity, unknown-function, limit or constant error.
    pub fn compile(src: &str, mode: Mode) -> Result<Program, Error> {
        let mut p = Parser::new(mode);
        p.root(src)?;
        Ok(p.finish())
    }

    /// The identifiers the caller binds, in first-occurrence order.
    pub fn idents(&self) -> impl ExactSizeIterator<Item = &str> {
        self.idents.iter().map(String::as_str)
    }

    /// The plugin functions (`ns.f`) called, each once.
    pub fn plugin_fns(&self) -> impl ExactSizeIterator<Item = &str> {
        self.plugins.iter().map(String::as_str)
    }

    /// Pushes one value per identifier onto `inputs`, from `f`.
    ///
    /// # Errors
    ///
    /// `X004` for the first identifier `f` has no value for.
    pub fn bind(
        &self,
        inputs: &mut Vec<Value>,
        mut f: impl FnMut(&str) -> Option<Value>,
    ) -> Result<(), Error> {
        for id in &self.idents {
            match f(id) {
                Some(v) => inputs.push(v),
                None => return err(Code::X004, format!("unknown identifier `{id}`")),
            }
        }
        Ok(())
    }

    /// Evaluates the (first) root with `inputs`, one per [`Program::idents`].
    ///
    /// # Errors
    ///
    /// The first evaluation error (spec §4, §5); `X006` for a plugin or calculator call.
    pub fn eval(&self, inputs: &[Value], scratch: &mut Scratch) -> Result<Value, Error> {
        if inputs.len() != self.idents.len() {
            return err(Code::X004, "wrong number of inputs");
        }
        let (a, b) = self.roots[0];
        eval::run(
            &self.code,
            &self.consts,
            inputs,
            a as usize,
            b as usize,
            &mut scratch.stack,
        )
    }
}

/// A template (spec §6): text with `{expr}` holes.
#[derive(Debug)]
pub struct Template {
    program: Program,
}

impl Template {
    /// Parses `text`; each hole is a scalar expression, all sharing one identifier list.
    ///
    /// # Errors
    ///
    /// `X009` for an unclosed `{`, an empty hole or a lone `}`; any hole's compile error.
    pub fn parse(text: &str) -> Result<Template, Error> {
        let mut p = Parser::new(Mode::Scalar);
        let b = text.as_bytes();
        let mut k = 0;
        while k < b.len() {
            match b[k] {
                b'{' if b.get(k + 1) == Some(&b'{') => k += 2,
                b'}' if b.get(k + 1) == Some(&b'}') => k += 2,
                b'}' => return err(Code::X009, "lone `}` in template"),
                b'{' => {
                    let start = k + 1;
                    let mut j = start;
                    let mut in_str = false;
                    while j < b.len() && (in_str || b[j] != b'}') {
                        match b[j] {
                            b'\\' if in_str => j += 1,
                            b'"' => in_str = !in_str,
                            _ => {}
                        }
                        j += 1;
                    }
                    if j >= b.len() {
                        return err(Code::X009, "unclosed `{` in template");
                    }
                    if text[start..j].trim().is_empty() {
                        return err(Code::X009, "empty template hole");
                    }
                    p.root(&text[start..j])?;
                    k = j + 1;
                }
                _ => k += 1,
            }
        }
        Ok(Template {
            program: p.finish(),
        })
    }

    /// The holes as one program (its identifiers are the template's).
    pub fn program(&self) -> &Program {
        &self.program
    }
}

// ---------------------------------------------------------------------------------------
// Lexer (spec §2): one token at a time, so errors surface in source order
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(i64),
    Num(f64),
    Str(String),
    /// Identifier, possibly `ns.name`.
    Ident(String),
    /// Operator or punctuation.
    Op(&'static str),
    End,
}

const OPS: [&str; 21] = [
    "==", "!=", "<=", ">=", "&&", "||", "+", "-", "*", "/", "%", "^", "<", ">", "!", "?", ":", "(",
    ")", ",", "=",
];

/// The token at `src[*k..]` after whitespace; advances `*k` past it. Returns its start too.
fn lex_one(src: &str, k: &mut usize) -> Result<(Tok, usize), Error> {
    let b = src.as_bytes();
    while b
        .get(*k)
        .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
    {
        *k += 1;
    }
    let start = *k;
    let Some(&c) = b.get(start) else {
        return Ok((Tok::End, start));
    };
    let tok =
        if c.is_ascii_digit() || (c == b'.' && b.get(start + 1).is_some_and(u8::is_ascii_digit)) {
            let (t, n) = text::lex_number(&src[start..])?;
            *k += n;
            t
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let id = |k: usize| {
                k + b[k..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                    .count()
            };
            let mut e = id(start);
            if b.get(e) == Some(&b'.')
                && b.get(e + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            {
                e = id(e + 1);
            }
            *k = e;
            Tok::Ident(src[start..e].to_owned())
        } else if c == b'"' {
            let (s, n) = text::lex_string(&src[start..])?;
            *k += n;
            Tok::Str(s)
        } else {
            match OPS.iter().find(|o| src[start..].starts_with(**o)) {
                Some(&"=") | None => {
                    return err(Code::X009, format!("unexpected character in `{src}`"));
                }
                Some(op) => {
                    *k += op.len();
                    Tok::Op(op)
                }
            }
        };
    Ok((tok, start))
}

// ---------------------------------------------------------------------------------------
// Parser + code generation (spec §3)
// ---------------------------------------------------------------------------------------

/// Spec §3 limits.
const MAX_NODES: u32 = 65_536;
const MAX_DEPTH: u32 = 256;

/// Syntax errors (lexing, parsing, limits) are reported at once. The others, a call's or
/// name's check and a constant's fold, wait for the whole parse: the one whose node starts
/// first in the source wins, a check before a fold of the same node.
struct Parser {
    mode: Mode,
    src: String,
    /// Lexer cursor and the current token (with its byte position).
    pos: usize,
    tok: (Tok, usize),
    code: Vec<Op>,
    consts: Vec<Value>,
    idents: Vec<String>,
    plugins: Vec<String>,
    roots: Vec<(u32, u32)>,
    depth: u32,
    nodes: u32,
    /// > 0 while inside a lazily evaluated part (an arm, the right of `&&`/`||`).
    lazy: u32,
    /// The first error by (source position of its node, check before fold).
    first: Option<(usize, u8, Error)>,
    stack: Vec<Value>,
    /// The last operand token read (a signal name is one).
    last: Tok,
}

/// Whether the subexpression just emitted (the code suffix) is constant.
type Konst = bool;

impl Parser {
    fn new(mode: Mode) -> Parser {
        Parser {
            mode,
            src: String::new(),
            pos: 0,
            tok: (Tok::End, 0),
            code: Vec::new(),
            consts: Vec::new(),
            idents: Vec::new(),
            plugins: Vec::new(),
            roots: Vec::new(),
            depth: 0,
            nodes: 0,
            lazy: 0,
            first: None,
            stack: Vec::new(),
            last: Tok::End,
        }
    }

    fn finish(self) -> Program {
        Program {
            code: self.code,
            consts: self.consts,
            idents: self.idents,
            plugins: self.plugins,
            roots: self.roots,
        }
    }

    fn root(&mut self, src: &str) -> Result<(), Error> {
        self.src = src.to_owned();
        self.pos = 0;
        self.bump()?;
        let start = self.code.len();
        self.expr()?;
        if self.tok.0 != Tok::End {
            return err(Code::X009, format!("unexpected token in `{src}`"));
        }
        if let Some((.., e)) = self.first.take() {
            return Err(e);
        }
        self.roots.push((start as u32, self.code.len() as u32));
        Ok(())
    }

    /// Moves to the next token.
    fn bump(&mut self) -> Result<(), Error> {
        self.tok = lex_one(&self.src, &mut self.pos)?;
        Ok(())
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.tok.0, Tok::Op(o) if o == op)
    }

    fn eat(&mut self, op: &str) -> Result<bool, Error> {
        let yes = self.is_op(op);
        if yes {
            self.bump()?;
        }
        Ok(yes)
    }

    fn expect(&mut self, op: &str) -> Result<(), Error> {
        if self.eat(op)? {
            Ok(())
        } else {
            err(Code::X009, format!("expected `{op}` in `{}`", self.src))
        }
    }

    /// Records a check (`tier` 0) or fold (1) error of the node starting at byte `at`.
    fn record(&mut self, at: usize, tier: u8, e: Error) {
        if self.first.as_ref().is_none_or(|f| (at, tier) < (f.0, f.1)) {
            self.first = Some((at, tier, e));
        }
    }

    fn check(&mut self, at: usize, code: Code, message: String) {
        self.record(at, 0, Error { code, message });
    }

    fn node(&mut self) -> Result<(), Error> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return err(Code::X002, "expression too large");
        }
        Ok(())
    }

    fn nest<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return err(Code::X002, "expression nested too deeply");
        }
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn lazily<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        self.lazy += 1;
        let r = f(self);
        self.lazy -= 1;
        r
    }

    fn push_const(&mut self, v: Value) {
        self.consts.push(v);
        self.code.push(Op::Const(self.consts.len() as u32 - 1));
    }

    /// Folds the constant suffix starting at code index `start`, the node at byte `at`
    /// (spec §4 static checking). A fold that fails is reported where the node always runs;
    /// either way the node is no longer constant.
    fn fold(&mut self, start: usize, at: usize, konst: Konst) -> Konst {
        let single = self.code.len() == start + 1 && matches!(self.code[start], Op::Const(_));
        if !konst || single {
            return konst;
        }
        let end = self.code.len();
        match eval::run(&self.code, &self.consts, &[], start, end, &mut self.stack) {
            Ok(v) => {
                self.code.truncate(start);
                self.push_const(v);
                true
            }
            Err(e) => {
                if self.lazy == 0 {
                    self.record(at, 1, e);
                }
                false
            }
        }
    }

    fn patch(&mut self, at: usize) {
        let to = self.code.len() as u32;
        match &mut self.code[at] {
            Op::Jf(t) | Op::J(t) | Op::AndJ(t) | Op::OrJ(t) => *t = to,
            _ => unreachable!("patching a jump"),
        }
    }

    fn expr(&mut self) -> Result<Konst, Error> {
        self.nest(Self::ternary)
    }

    fn ternary(&mut self) -> Result<Konst, Error> {
        let (start, at) = (self.code.len(), self.tok.1);
        let c = self.or()?;
        if !self.eat("?")? {
            return Ok(c);
        }
        self.node()?;
        let jf = self.code.len();
        self.code.push(Op::Jf(0));
        let a = self.lazily(Self::expr)?;
        self.expect(":")?;
        let j = self.code.len();
        self.code.push(Op::J(0));
        self.patch(jf);
        let b = self.lazily(Self::expr)?;
        self.patch(j);
        Ok(self.fold(start, at, c && a && b))
    }

    fn or(&mut self) -> Result<Konst, Error> {
        self.logic("||", Self::and, Op::OrJ(0))
    }

    fn and(&mut self) -> Result<Konst, Error> {
        self.logic("&&", Self::cmp, Op::AndJ(0))
    }

    fn logic(
        &mut self,
        op: &str,
        next: fn(&mut Self) -> Result<Konst, Error>,
        jump: Op,
    ) -> Result<Konst, Error> {
        let (start, at) = (self.code.len(), self.tok.1);
        let mut k = next(self)?;
        while self.eat(op)? {
            self.node()?;
            let j = self.code.len();
            self.code.push(jump.clone());
            let r = self.lazily(next)?;
            self.code.push(Op::IsBool);
            self.patch(j);
            k = self.fold(start, at, k && r);
        }
        Ok(k)
    }

    fn cmp(&mut self) -> Result<Konst, Error> {
        const CMP: [&str; 6] = ["==", "!=", "<=", ">=", "<", ">"];
        let (start, at) = (self.code.len(), self.tok.1);
        let a = self.add()?;
        let Some(op) = CMP.iter().find(|o| self.is_op(o)) else {
            return Ok(a);
        };
        self.bump()?;
        self.node()?;
        let b = self.add()?;
        self.code.push(Op::Bin(op));
        if CMP.iter().any(|o| self.is_op(o)) {
            return err(Code::X009, "comparisons do not chain");
        }
        Ok(self.fold(start, at, a && b))
    }

    fn add(&mut self) -> Result<Konst, Error> {
        self.chain(&["+", "-"], Self::mul)
    }

    fn mul(&mut self) -> Result<Konst, Error> {
        self.chain(&["*", "/", "%"], Self::unary)
    }

    fn chain(
        &mut self,
        ops: &[&'static str],
        next: fn(&mut Self) -> Result<Konst, Error>,
    ) -> Result<Konst, Error> {
        let (start, at) = (self.code.len(), self.tok.1);
        let mut k = next(self)?;
        while let Some(&op) = ops.iter().find(|o| self.is_op(o)) {
            self.bump()?;
            self.node()?;
            let r = next(self)?;
            self.code.push(Op::Bin(op));
            k = self.fold(start, at, k && r);
        }
        Ok(k)
    }

    fn unary(&mut self) -> Result<Konst, Error> {
        let (start, at) = (self.code.len(), self.tok.1);
        for (op, code) in [("-", Op::Neg), ("!", Op::Not)] {
            if self.eat(op)? {
                self.node()?;
                let k = self.nest(Self::unary)?;
                self.code.push(code);
                return Ok(self.fold(start, at, k));
            }
        }
        self.pow()
    }

    fn pow(&mut self) -> Result<Konst, Error> {
        let (start, at) = (self.code.len(), self.tok.1);
        let a = self.call()?;
        if !self.eat("^")? {
            return Ok(a);
        }
        self.node()?;
        let b = self.nest(Self::unary)?;
        self.code.push(Op::Bin("^"));
        Ok(self.fold(start, at, a && b))
    }

    fn call(&mut self) -> Result<Konst, Error> {
        let (tok, at) = self.tok.clone();
        match tok {
            Tok::Int(_) | Tok::Num(_) | Tok::Str(_) | Tok::Ident(_) => self.node()?,
            Tok::Op("(") => {}
            _ => return err(Code::X009, format!("expected an operand in `{}`", self.src)),
        }
        self.bump()?;
        if tok != Tok::Op("(") {
            self.last = tok.clone();
        }
        match tok {
            Tok::Int(i) => self.push_const(Value::Int(i)),
            Tok::Num(x) => self.push_const(Value::Num(x)),
            Tok::Str(s) => self.push_const(Value::Str(s)),
            Tok::Op("(") => {
                let k = self.expr()?;
                self.expect(")")?;
                return Ok(k);
            }
            Tok::Ident(name) if self.is_op("(") => {
                let k = self.function(&name, at);
                self.last = Tok::End; // a call is no name
                return k;
            }
            Tok::Ident(name) => return Ok(self.ident(&name, at)),
            _ => unreachable!("operands only"),
        }
        if self.is_op("(") {
            return err(Code::X009, "only identifiers are callable");
        }
        Ok(true)
    }

    fn ident(&mut self, name: &str, at: usize) -> Konst {
        let v = match name {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "pi" => Value::Num(std::f64::consts::PI),
            _ => match name.strip_prefix("phys.") {
                Some(c) => Value::Num(match c {
                    "k" => 1.380649e-23,
                    "q" => 1.602176634e-19,
                    "h" => 6.62607015e-34,
                    "c" => 299_792_458.0,
                    "eps0" => 8.8541878188e-12,
                    "kelvin0" => 273.15,
                    _ => {
                        self.check(at, Code::X004, format!("unknown constant `{name}`"));
                        self.code.push(Op::Plugin); // never runs: the compile fails
                        return false;
                    }
                }),
                None => {
                    let k = match self.idents.iter().position(|i| i == name) {
                        Some(k) => k,
                        None => {
                            self.idents.push(name.to_owned());
                            self.idents.len() - 1
                        }
                    };
                    self.code.push(Op::Load(k as u32));
                    return false;
                }
            },
        };
        self.push_const(v);
        true
    }

    /// `name(` … `)` at byte `at`: a builtin, a calculator function or a plugin function.
    fn function(&mut self, name: &str, at: usize) -> Result<Konst, Error> {
        let start = self.code.len();
        self.bump()?; // `(`
        if matches!(name, "true" | "false" | "pi") {
            return err(Code::X009, format!("`{name}` is not callable"));
        }
        if let Some((ns, _)) = name.split_once('.') {
            let n = self.args(|p, _| p.expr().map(|_| ()))?;
            if ns == "phys" {
                self.check(at, Code::X006, format!("`{name}` is a constant"));
            } else if n > 16 {
                self.check(at, Code::X008, format!("`{name}`: at most 16 arguments"));
            }
            if !self.plugins.iter().any(|p| p == name) {
                self.plugins.push(name.to_owned());
            }
            self.code.push(Op::Plugin);
            return Ok(false);
        }
        if let Some(arity) = eval::calc_arity(name) {
            let signal = matches!(name, "v" | "V" | "i" | "I" | "op" | "result");
            let mut names: Vec<Option<(String, bool)>> = Vec::new();
            let n = self.args(|p, _| {
                let (code, nodes, ids) = (p.code.len(), p.nodes, p.idents.len());
                let saved = p.first.clone();
                p.expr()?;
                // A name: one string literal or bare (dotted) identifier, parentheses allowed,
                // taken verbatim and never bound.
                let name = match &p.last {
                    _ if !signal || p.nodes != nodes + 1 => None,
                    Tok::Str(s) => Some((s.clone(), false)),
                    Tok::Ident(s) if !matches!(s.as_str(), "true" | "false" | "pi") => {
                        Some((s.clone(), true))
                    }
                    _ => None,
                };
                if name.is_some() {
                    p.code.truncate(code);
                    p.idents.truncate(ids);
                    p.first = saved;
                }
                names.push(name);
                Ok(())
            })?;
            if !arity.contains(&n) {
                self.check(
                    at,
                    Code::X008,
                    format!("`{name}`: wrong number of arguments"),
                );
            } else if self.mode == Mode::Scalar {
                self.check(at, Code::X006, format!("`{name}` is calculator-only"));
            } else if signal && !signal_names_ok(name, &names) {
                self.check(at, Code::X003, format!("bad `{name}` argument"));
            }
            self.code.push(Op::Calc);
            return Ok(false);
        }
        let Some(f) = Builtin::find(name) else {
            self.args(|p, _| p.expr().map(|_| ()))?;
            self.check(at, Code::X006, format!("unknown function `{name}`"));
            self.code.push(Op::Plugin);
            return Ok(false);
        };
        let mut k = true;
        let mut jumps: Vec<usize> = Vec::new();
        let n = self.args(|p, i| {
            if f != Builtin::If {
                k &= p.expr()?;
                return Ok(());
            }
            // `if(c, a, b)` = `c ? a : b`: the arms are lazy.
            if i == 1 || i == 2 {
                jumps.push(p.code.len());
                p.code.push(if i == 1 { Op::Jf(0) } else { Op::J(0) });
                if i == 2 {
                    p.patch(jumps[0]);
                }
                k &= p.lazily(Self::expr)?;
            } else {
                k &= p.expr()?;
            }
            Ok(())
        })?;
        if !f.arity_ok(n) {
            self.check(
                at,
                Code::X008,
                format!("`{name}`: wrong number of arguments"),
            );
            // Never runs: the compile fails.
            self.code.push(Op::Plugin);
            return Ok(false);
        }
        if f != Builtin::If {
            self.code.push(Op::Call(f, n as u8));
        } else {
            self.patch(jumps[1]);
        }
        Ok(self.fold(start, at, k))
    }

    /// Comma-separated arguments up to `)`; `each` gets the argument index. Returns the count.
    fn args(
        &mut self,
        mut each: impl FnMut(&mut Self, usize) -> Result<(), Error>,
    ) -> Result<usize, Error> {
        if self.eat(")")? {
            return Ok(0);
        }
        let mut n = 0;
        loop {
            each(self, n)?;
            n += 1;
            if self.eat(")")? {
                return Ok(n);
            }
            self.expect(",")?;
        }
    }
}

/// `v`/`i`/`op`/`result` take names, verbatim (spec §5): (text, whether an identifier).
fn signal_names_ok(f: &str, names: &[Option<(String, bool)>]) -> bool {
    let Some(names) = names
        .iter()
        .cloned()
        .collect::<Option<Vec<(String, bool)>>>()
    else {
        return false;
    };
    // `op`/`result` take undotted identifiers (or strings).
    if matches!(f, "op" | "result") && names.iter().any(|(t, id)| *id && t.contains('.')) {
        return false;
    }
    let names: Vec<&str> = names.iter().map(|(t, _)| t.as_str()).collect();
    match (f, names.as_slice()) {
        ("op", [inst, param]) => {
            !inst.is_empty()
                && !inst.contains(|c: char| c.is_whitespace() || c == ']')
                && !param.is_empty()
                && param
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        }
        ("result", [o, t]) => [o, t]
            .iter()
            .all(|s| !s.is_empty() && !s.contains(['[', ']'])),
        _ => true,
    }
}

#[cfg(test)]
mod tests;
