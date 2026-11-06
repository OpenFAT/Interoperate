//! The stack machine and the scalar builtins (spec §4, §5). Transcendentals come from
//! `libm`, so every platform computes the same bits (D32).

use super::text;
use super::{Code, Error, Value, err};

/// One instruction. Jumps are absolute code indices.
#[derive(Clone, Debug)]
pub(super) enum Op {
    Const(u32),
    Load(u32),
    Neg,
    Not,
    Bin(&'static str),
    Call(Builtin, u8),
    /// A plugin call: no host here, so evaluating it is `X006`.
    Plugin,
    /// A calculator function: compile-checked only.
    Calc,
    /// Pops a bool; jumps when false.
    Jf(u32),
    J(u32),
    /// `&&`/`||`: the left operand must be bool; jumps keeping it when it decides.
    AndJ(u32),
    OrJ(u32),
    /// The right operand of `&&`/`||` must be bool.
    IsBool,
}

/// Calculator-only functions and their argument counts (spec §5).
pub(super) fn calc_arity(name: &str) -> Option<std::ops::RangeInclusive<usize>> {
    Some(match name {
        "x" => 0..=0,
        "v" | "V" | "i" | "I" | "deriv" | "integ" | "avg" | "rms" | "ymax" | "ymin" | "pp"
        | "fft" | "gain_margin" | "phase_margin" => 1..=1,
        "op" | "result" | "at" | "settle" => 2..=2,
        "cross" => 2..=4,
        "delay" => 4..=4,
        _ => return None,
    })
}

macro_rules! builtins {
    ($($v:ident $name:literal $lo:literal $hi:expr),* $(,)?) => {
        /// A scalar builtin (spec §5).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(super) enum Builtin { $($v),* }

        impl Builtin {
            pub(super) fn find(name: &str) -> Option<Builtin> {
                match name { $($name => Some(Builtin::$v),)* _ => None }
            }

            pub(super) fn arity_ok(self, n: usize) -> bool {
                // `steps`: x, (b, y) pairs, the last breakpoint.
                let shape = self != Builtin::Steps || n % 2 == 0;
                match self {
                    $(Builtin::$v => shape && ($lo..=$hi).contains(&n),)*
                }
            }
        }
    };
}

builtins! {
    Abs "abs" 1 1, Sqrt "sqrt" 1 1, Exp "exp" 1 1, Ln "ln" 1 1, Log10 "log10" 1 1,
    Sin "sin" 1 1, Cos "cos" 1 1, Tan "tan" 1 1, Asin "asin" 1 1, Acos "acos" 1 1,
    Atan "atan" 1 1, Atan2 "atan2" 2 2, Sinh "sinh" 1 1, Cosh "cosh" 1 1, Tanh "tanh" 1 1,
    Floor "floor" 1 1, Ceil "ceil" 1 1, Round "round" 1 1, Int "int" 1 1,
    Min "min" 2 usize::MAX, Max "max" 2 usize::MAX, Clamp "clamp" 3 3, Sign "sign" 1 1,
    Pow "pow" 2 2, If "if" 3 3, Snap "snap" 2 2, Steps "steps" 4 usize::MAX,
    Eng "eng" 1 2, Str "str" 1 1, Num "num" 1 1, Len "len" 1 1,
    SpectreSource "spectre_source" 1 1,
    Re "re" 1 1, Im "im" 1 1, Mag "mag" 1 1, Ph "ph" 1 1, Db "db" 1 1, Conj "conj" 1 1,
    Cplx "cplx" 2 2,
}

/// Runs `code[start..end]`, leaving one value.
pub(super) fn run(
    code: &[Op],
    consts: &[Value],
    inputs: &[Value],
    start: usize,
    end: usize,
    stack: &mut Vec<Value>,
) -> Result<Value, Error> {
    stack.clear();
    let mut pc = start;
    while pc < end {
        let op = &code[pc];
        pc += 1;
        match op {
            Op::Const(k) => stack.push(consts[*k as usize].clone()),
            Op::Load(k) => stack.push(inputs[*k as usize].clone()),
            Op::Neg => {
                let a = pop(stack);
                stack.push(neg(a)?);
            }
            Op::Not => match pop(stack) {
                Value::Bool(b) => stack.push(Value::Bool(!b)),
                _ => return err(Code::X003, "`!` needs a bool"),
            },
            Op::Bin(o) => {
                let b = pop(stack);
                let a = pop(stack);
                stack.push(binary(o, a, b)?);
            }
            Op::Call(f, n) => {
                let args = stack.split_off(stack.len() - usize::from(*n));
                stack.push(call(*f, args)?);
            }
            Op::Plugin => return err(Code::X006, "plugin functions need the host"),
            Op::Calc => return err(Code::X006, "calculator functions need results"),
            Op::Jf(t) => {
                if !want_bool(&pop(stack))? {
                    pc = *t as usize;
                }
            }
            Op::J(t) => pc = *t as usize,
            Op::AndJ(t) | Op::OrJ(t) => {
                let decides =
                    want_bool(stack.last().expect("left operand"))? == matches!(op, Op::OrJ(_));
                if decides {
                    pc = *t as usize;
                } else {
                    stack.pop();
                }
            }
            Op::IsBool => {
                want_bool(stack.last().expect("right operand"))?;
            }
        }
    }
    debug_assert_eq!(stack.len(), 1, "one value per root");
    Ok(pop(stack))
}

fn pop(stack: &mut Vec<Value>) -> Value {
    stack.pop().expect("compiled code balances its stack")
}

fn want_bool(v: &Value) -> Result<bool, Error> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => err(Code::X003, "a condition must be a bool"),
    }
}

/// A finite `num`, else `X010`.
fn num(x: f64) -> Result<Value, Error> {
    if x.is_finite() {
        Ok(Value::Num(x))
    } else {
        err(Code::X010, "NaN or infinity")
    }
}

fn cnum(re: f64, im: f64) -> Result<Value, Error> {
    if re.is_finite() && im.is_finite() {
        Ok(Value::Cnum(re, im))
    } else {
        err(Code::X010, "NaN or infinity")
    }
}

/// `int`/`num` as f64.
fn real(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Num(x) => Some(*x),
        _ => None,
    }
}

fn want_real(v: &Value) -> Result<f64, Error> {
    real(v).map_or_else(|| err(Code::X003, "expected a number"), Ok)
}

/// `int`/`num`/`cnum` as (re, im).
fn complex(v: &Value) -> Option<(f64, f64)> {
    match v {
        Value::Cnum(r, i) => Some((*r, *i)),
        _ => real(v).map(|x| (x, 0.0)),
    }
}

fn neg(a: Value) -> Result<Value, Error> {
    match a {
        Value::Int(i) => Ok(i.checked_neg().map_or(Value::Num(-(i as f64)), Value::Int)),
        Value::Num(x) => Ok(Value::Num(-x)),
        Value::Cnum(r, i) => Ok(Value::Cnum(-r, -i)),
        _ => err(Code::X003, "unary `-` needs a number"),
    }
}

fn binary(op: &str, a: Value, b: Value) -> Result<Value, Error> {
    use Value::{Bool, Cnum, Int, Str};
    if matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") {
        let ord = match (&a, &b) {
            (Int(x), Int(y)) => Some(x.cmp(y)),
            (Str(x), Str(y)) => Some(x.as_bytes().cmp(y.as_bytes())),
            (Bool(_), Bool(_)) | (Cnum(..), Cnum(..)) if matches!(op, "==" | "!=") => {
                return Ok(Bool((a == b) == (op == "==")));
            }
            _ => match (real(&a), real(&b)) {
                (Some(x), Some(y)) => x.partial_cmp(&y),
                _ => return err(Code::X003, format!("`{op}` on these types")),
            },
        };
        let Some(o) = ord else {
            return err(Code::X010, "NaN compared");
        };
        return Ok(Bool(match op {
            "==" => o.is_eq(),
            "!=" => o.is_ne(),
            "<" => o.is_lt(),
            "<=" => o.is_le(),
            ">" => o.is_gt(),
            _ => o.is_ge(),
        }));
    }
    match (&a, &b) {
        (Str(x), Str(y)) if op == "+" => {
            if x.len() + y.len() > text::MAX_STR {
                return err(Code::X002, "string over 1 MiB");
            }
            Ok(Str(format!("{x}{y}")))
        }
        (Int(x), Int(y)) if matches!(op, "+" | "-" | "*" | "%") => {
            if op == "%" && *y == 0 {
                return err(Code::X010, "`% 0`");
            }
            let r = match op {
                "+" => x.checked_add(*y),
                "-" => x.checked_sub(*y),
                "*" => x.checked_mul(*y),
                _ => x.checked_rem(*y),
            };
            match r {
                Some(r) => Ok(Int(r)),
                None => real_op(op, *x as f64, *y as f64),
            }
        }
        (Cnum(..), _) | (_, Cnum(..)) => {
            let (Some(x), Some(y)) = (complex(&a), complex(&b)) else {
                return err(Code::X003, format!("`{op}` on these types"));
            };
            complex_op(op, x, y)
        }
        _ => match (real(&a), real(&b)) {
            (Some(x), Some(y)) => real_op(op, x, y),
            _ => err(Code::X003, format!("`{op}` on these types")),
        },
    }
}

fn real_op(op: &str, x: f64, y: f64) -> Result<Value, Error> {
    match op {
        "+" => num(x + y),
        "-" => num(x - y),
        "*" => num(x * y),
        "/" => num(x / y),
        "%" if y == 0.0 => err(Code::X010, "`% 0`"),
        "%" => num(x % y),
        _ => num(libm::pow(x, y)),
    }
}

fn complex_op(op: &str, (a, b): (f64, f64), (c, d): (f64, f64)) -> Result<Value, Error> {
    match op {
        "+" => cnum(a + c, b + d),
        "-" => cnum(a - c, b - d),
        "*" => cnum(a * c - b * d, a * d + b * c),
        "/" => {
            let n = c * c + d * d;
            cnum((a * c + b * d) / n, (b * c - a * d) / n)
        }
        "^" => {
            // z^w = exp(w ln z); 0^w is 0.
            if a == 0.0 && b == 0.0 {
                return cnum(0.0, 0.0);
            }
            let (lr, li) = (libm::log(libm::hypot(a, b)), libm::atan2(b, a));
            let (er, ei) = (c * lr - d * li, c * li + d * lr);
            let m = libm::exp(er);
            cnum(m * libm::cos(ei), m * libm::sin(ei))
        }
        _ => err(Code::X003, "`%` on a cnum"),
    }
}

fn want_str(v: &Value) -> Result<&str, Error> {
    match v {
        Value::Str(s) => Ok(s),
        _ => err(Code::X003, "expected a string"),
    }
}

fn call(f: Builtin, args: Vec<Value>) -> Result<Value, Error> {
    use Builtin as B;
    let all_int = args.iter().all(|a| matches!(a, Value::Int(_)));
    let a = &args[0];
    let unary = |g: fn(f64) -> f64| -> Result<Value, Error> {
        match a {
            Value::Cnum(..) => err(Code::X003, format!("{f:?} of a cnum")),
            _ => num(g(want_real(a)?)),
        }
    };
    match f {
        B::Abs => match a {
            Value::Int(i) => Ok(i
                .checked_abs()
                .map_or(Value::Num((*i as f64).abs()), Value::Int)),
            Value::Cnum(r, i) => num(libm::hypot(*r, *i)),
            _ => num(want_real(a)?.abs()),
        },
        B::Sqrt => match a {
            Value::Cnum(r, i) => {
                // Principal root.
                let m = libm::hypot(*r, *i);
                let re = libm::sqrt((m + r) / 2.0);
                let im = libm::sqrt((m - r) / 2.0).copysign(*i);
                cnum(re, im)
            }
            _ => num(libm::sqrt(want_real(a)?)),
        },
        B::Exp => match a {
            Value::Cnum(r, i) => {
                let m = libm::exp(*r);
                cnum(m * libm::cos(*i), m * libm::sin(*i))
            }
            _ => num(libm::exp(want_real(a)?)),
        },
        B::Ln => match a {
            Value::Cnum(r, i) => cnum(libm::log(libm::hypot(*r, *i)), libm::atan2(*i, *r)),
            _ => num(libm::log(want_real(a)?)),
        },
        // ponytail: the other elementary functions take no cnum here (Interop folds none).
        B::Log10 => unary(libm::log10),
        B::Sin => unary(libm::sin),
        B::Cos => unary(libm::cos),
        B::Tan => unary(libm::tan),
        B::Asin => unary(libm::asin),
        B::Acos => unary(libm::acos),
        B::Atan => unary(libm::atan),
        B::Sinh => unary(libm::sinh),
        B::Cosh => unary(libm::cosh),
        B::Tanh => unary(libm::tanh),
        B::Atan2 => num(libm::atan2(want_real(a)?, want_real(&args[1])?)),
        B::Floor | B::Ceil | B::Round if matches!(a, Value::Int(_)) => Ok(a.clone()),
        B::Floor => unary(libm::floor),
        B::Ceil => unary(libm::ceil),
        B::Round => unary(libm::round),
        B::Int => match a {
            Value::Int(_) => Ok(a.clone()),
            _ => {
                let t = libm::trunc(want_real(a)?);
                if (i64::MIN as f64..-(i64::MIN as f64)).contains(&t) {
                    Ok(Value::Int(t as i64))
                } else {
                    err(Code::X010, "int() out of range")
                }
            }
        },
        B::Sign => match a {
            Value::Int(i) => Ok(Value::Int(i.signum())),
            _ => {
                let x = want_real(a)?;
                num(if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                })
            }
        },
        B::Min | B::Max => {
            let max = f == B::Max;
            if all_int {
                let it = args.iter().map(|v| match v {
                    Value::Int(i) => *i,
                    _ => unreachable!("all int"),
                });
                return Ok(Value::Int(
                    if max { it.max() } else { it.min() }.expect("≥ 2"),
                ));
            }
            let mut best = want_real(a)?;
            for v in &args[1..] {
                let x = want_real(v)?;
                if (max && x > best) || (!max && x < best) {
                    best = x;
                }
            }
            num(best)
        }
        B::Clamp => {
            let mx = call(B::Max, vec![args[0].clone(), args[1].clone()])?;
            call(B::Min, vec![mx, args[2].clone()])
        }
        B::Pow => binary("^", args[0].clone(), args[1].clone()),
        B::If => unreachable!("`if` compiles to jumps"),
        B::Snap => {
            let (x, g) = (want_real(a)?, want_real(&args[1])?);
            if g == 0.0 {
                return err(Code::X010, "snap grid 0");
            }
            num(libm::round(x / g) * g)
        }
        B::Steps => steps(&args),
        B::Eng => {
            let digits = match args.get(1) {
                None => 3,
                Some(Value::Int(d)) if (1..=17).contains(d) => *d as usize,
                Some(_) => return err(Code::X003, "eng digits: an int in 1..=17"),
            };
            Ok(Value::Str(text::eng(want_real(a)?, digits)))
        }
        B::Str => match a {
            Value::Int(i) => Ok(Value::Str(i.to_string())),
            Value::Num(x) => {
                let mut s = String::new();
                super::write_num(&mut s, *x);
                Ok(Value::Str(s))
            }
            Value::Bool(b) => Ok(Value::Str(b.to_string())),
            Value::Str(_) => Ok(a.clone()),
            Value::Cnum(..) => err(Code::X003, "str() of a cnum"),
        },
        B::Num => match a {
            Value::Str(s) => text::parse_num(s).map(Value::Num),
            _ => num(want_real(a)?),
        },
        B::Len => Ok(Value::Int(want_str(a)?.chars().count() as i64)),
        B::SpectreSource => text::spectre_source(want_str(a)?).map(Value::Str),
        B::Re | B::Im | B::Mag | B::Ph | B::Db | B::Conj => {
            let Some((r, i)) = complex(a) else {
                return err(Code::X003, "expected a number");
            };
            match f {
                B::Re => num(r),
                B::Im => num(i),
                B::Mag => num(libm::hypot(r, i)),
                B::Ph if r == 0.0 && i == 0.0 => num(0.0),
                B::Ph => num(libm::atan2(i, r).to_degrees()),
                B::Db => num(20.0 * libm::log10(libm::hypot(r, i))),
                // `conj` of a real is that real.
                _ if !matches!(a, Value::Cnum(..)) => Ok(a.clone()),
                _ => cnum(r, -i),
            }
        }
        B::Cplx => cnum(want_real(a)?, want_real(&args[1])?),
    }
}

/// `steps(x, b0, y0, …, bn)` (spec §5).
fn steps(args: &[Value]) -> Result<Value, Error> {
    let near = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs());
    let x = want_real(&args[0])?;
    let n = (args.len() - 2) / 2;
    let b = |k: usize| want_real(&args[1 + 2 * k]);
    let last = want_real(&args[args.len() - 1])?;
    for k in 0..n {
        let next = if k + 1 < n { b(k + 1)? } else { last };
        if next <= b(k)? || near(next, b(k)?) {
            return err(Code::X003, "steps breakpoints must increase");
        }
    }
    if (x < b(0)? && !near(x, b(0)?)) || (x > last && !near(x, last)) {
        return err(Code::X010, "steps: outside the measured range");
    }
    for k in 0..n {
        want_real(&args[2 + 2 * k])?;
    }
    let mut pick = 0;
    for k in 0..n {
        if x >= b(k)? || near(x, b(k)?) {
            pick = k;
        }
    }
    Ok(args[2 + 2 * pick].clone())
}
