//! The spec's own examples (`docs/spec/expr.md`).

use super::*;

fn ev(src: &str) -> Result<Value, Code> {
    let p = Program::compile(src, Mode::Scalar).map_err(|e| e.code)?;
    p.eval(&[], &mut Scratch::default()).map_err(|e| e.code)
}

#[test]
fn spec_examples() {
    assert_eq!(ev("-2^2"), Ok(Value::Num(-4.0)));
    assert_eq!(ev("2^3^2"), Ok(Value::Num(512.0)));
    assert_eq!(ev("2^-1"), Ok(Value::Num(0.5)));
    assert_eq!(ev("1 < 2 < 3"), Err(Code::X009));
    assert_eq!(ev("false && 1/0 > 0"), Ok(Value::Bool(false)));
    assert_eq!(ev("1/0"), Err(Code::X010));
    assert_eq!(ev("7 % -2"), Ok(Value::Int(1)));
    assert_eq!(ev("1 % 0"), Err(Code::X010));
    assert_eq!(ev("2.5p"), Ok(Value::Num(2.5e-12)));
    assert_eq!(ev("1mil"), Ok(Value::Num(25.4e-6)));
    assert_eq!(ev("1."), Err(Code::X001));
    assert_eq!(ev("1ms"), Err(Code::X001));
    assert_eq!(ev("10Meg"), Ok(Value::Num(10e6)));
    assert_eq!(ev("eng(4700)"), Ok(Value::Str("4.7k".into())));
    assert_eq!(ev("eng(2.2e6)"), Ok(Value::Str("2.2meg".into())));
    assert_eq!(ev("eng(1e-20)"), Ok(Value::Str("1e-20".into())));
    assert_eq!(ev("eng(1e18)"), Ok(Value::Str("1e18".into())));
    assert_eq!(ev("eng(1e-18)"), Ok(Value::Str("0.001f".into())));
    assert_eq!(ev("eng(999999e12, 6)"), Ok(Value::Str("999999t".into())));
    assert_eq!(
        ev("eng(1.7976931348623157e308, 4)"),
        Ok(Value::Str("1.797e308".into()))
    );
    assert_eq!(ev("sqrt(-4)"), Err(Code::X010));
    assert_eq!(ev("round(-2.5)"), Ok(Value::Num(-3.0)));
    assert_eq!(ev("max(1, 2, 3)"), Ok(Value::Int(3)));
    assert_eq!(ev("max(1)"), Err(Code::X008));
    assert_eq!(ev("steps(1.5, 1, 10, 2, 20, 3)"), Ok(Value::Int(10)));
    assert_eq!(ev("steps(3, 1, 10, 2, 20, 3)"), Ok(Value::Int(20)));
    assert_eq!(ev("steps(4, 1, 10, 2, 20, 3)"), Err(Code::X010));
    assert_eq!(ev("str(0.0025)"), Ok(Value::Str("0.0025".into())));
    assert_eq!(ev("num(\"2.5k\")"), Ok(Value::Num(2500.0)));
    assert_eq!(ev("nope(1)"), Err(Code::X006));
    assert_eq!(ev("v(out)"), Err(Code::X006));
    assert_eq!(ev("phys.q"), Ok(Value::Num(1.602176634e-19)));
    assert_eq!(ev("phys.nope"), Err(Code::X004));
    assert_eq!(ev("\"a\" + \"b\""), Ok(Value::Str("ab".into())));
    assert_eq!(ev("\"a\" + 1"), Err(Code::X003));
    assert_eq!(ev("true ? 1 : 1/0"), Ok(Value::Int(1)));
    assert_eq!(ev("if(false, 1/0, 2)"), Ok(Value::Int(2)));
    assert_eq!(ev("if(true, 1)"), Err(Code::X008));
    assert_eq!(
        ev("spectre_source(\"dc 1 ac 1 pulse(0 1 1n)\")"),
        Ok(Value::Str(
            "dc=1 mag=1 type=pulse val0=0 val1=1 delay=1e-9".into()
        ))
    );
    assert_eq!(ev(&"(".repeat(300)), Err(Code::X002));
}

#[test]
fn identifiers_bind_in_first_occurrence_order() {
    let p = Program::compile("b + a * b + ns.f(c) + phys.k + pi", Mode::Scalar).unwrap();
    assert_eq!(p.idents().collect::<Vec<_>>(), ["b", "a", "c"]);
    assert_eq!(p.plugin_fns().collect::<Vec<_>>(), ["ns.f"]);
    let p = Program::compile("a * (b / a) ^ t", Mode::Scalar).unwrap();
    let v = p.eval(
        &[Value::Num(1.0), Value::Num(100.0), Value::Num(0.5)],
        &mut Scratch::default(),
    );
    assert_eq!(v, Ok(Value::Num(10.0)));
    let t = Template::parse("{name} {pins} r={r*2} {{x}}").unwrap();
    assert_eq!(
        t.program().idents().collect::<Vec<_>>(),
        ["name", "pins", "r"]
    );
    assert!(Template::parse("{a").is_err());
    assert!(Template::parse("a}").is_err());
    assert!(Template::parse("{}").is_err());
}

#[test]
fn calculator_mode_takes_signal_names_verbatim() {
    let p = Program::compile(
        "ymax(v(out)) - op(M1, gm) + result(\"o\", T) + k",
        Mode::Calc,
    )
    .unwrap();
    assert_eq!(p.idents().collect::<Vec<_>>(), ["k"]);
    assert!(Program::compile("v(a + b)", Mode::Calc).is_err());
    assert!(Program::compile("cross(v(a), 0.5, 1, 1)", Mode::Calc).is_ok());
    assert!(Program::compile("a+".repeat(20_000).trim_end_matches('+'), Mode::Scalar).is_ok());
}

#[test]
fn parse_num_is_one_literal() {
    assert_eq!(parse_num("-1.5u").ok(), Some(-1.5e-6));
    assert_eq!(parse_num("+2"), Ok(2.0));
    assert!(parse_num(" 2").is_err());
    assert!(parse_num("2 ").is_err());
    assert!(parse_num("1e").is_err());
}
