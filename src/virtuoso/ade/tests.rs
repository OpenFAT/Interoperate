use super::*;
use proptest::prelude::*;

/// A maestro test whose every scope, spec and name is valid (§9): corners first, vars
/// and models scoped to the test or a corner, specs on declared outputs.
fn tests() -> impl Strategy<Value = Test> {
    let name = "[a-z][a-z0-9]{0,3}";
    (
        prop::collection::btree_set(name, 0..3),
        prop::collection::btree_map(
            "a[a-z]{0,2}",
            (
                any::<bool>(),
                prop::collection::btree_map(name, "[a-z0-9.]{1,3}", 0..3),
            ),
            0..3,
        ),
        prop::collection::btree_map(name, ("[a-z0-9]{1,3}", 0usize..4), 0..4),
        prop::collection::vec(("[a-z]{1,4}\\.scs", "[a-z]{0,2}", 0usize..4), 0..3),
        prop::collection::btree_map(name, (any::<bool>(), "[a-z()]{1,6}", 0u8..3), 0..3),
        prop::collection::btree_map("[a-z]{1,4}\\.va", ".{0,8}", 0..2),
    )
        .prop_map(|(corners, analyses, vars, models, outputs, defns)| {
            let corners: Vec<String> = corners.into_iter().collect();
            // Index 0..len names a corner, past it the test's own scope.
            let scope = |k: usize| corners.get(k).cloned().unwrap_or_default();
            Test {
                name: "t".into(),
                lib: "lib".into(),
                cell: "tb".into(),
                view: "schematic".into(),
                sim: "spectre".into(),
                vars: vars
                    .into_iter()
                    .map(|(n, (v, k))| (n, v, scope(k)))
                    .collect(),
                analyses: analyses
                    .into_iter()
                    .map(|(n, (en, o))| (n, en, o.into_iter().collect()))
                    .collect(),
                corners: corners
                    .iter()
                    .map(|c| (c.clone(), c.len() % 2 == 0))
                    .collect(),
                models: models
                    .into_iter()
                    .map(|(f, sec, k)| (scope(k), f, sec))
                    .collect(),
                specs: outputs
                    .iter()
                    .filter(|(_, o)| o.2 > 0)
                    .map(|(n, o)| {
                        (
                            n.clone(),
                            ["lt", "gt"][usize::from(o.2 - 1)].into(),
                            "1".into(),
                        )
                    })
                    .collect(),
                outputs: outputs
                    .into_iter()
                    .map(|(n, (net, e, _))| (n, if net { "net" } else { "point" }.into(), e))
                    .collect(),
                defns: defns.into_iter().collect(),
                own: vec![("x".into(), "2".into())],
                enable: Some(corners.len().is_multiple_of(2)),
                params: vec![("I0/I1/w".into(), "1u".into(), scope(0))],
                var_enable: vec![("x".into(), corners.len().is_multiple_of(3))],
                out_enable: vec![("o".into(), false)],
                sim_opts: vec![("reltol".into(), "1e-5".into())],
                corner_runs: corners
                    .first()
                    .map(|c| (c.clone(), false))
                    .into_iter()
                    .collect(),
                design: Some(("lib".into(), "other".into())),
                notes: vec![("output".into(), "a note".into())],
            }
        })
}

proptest! {
    /// `parse` reads the records `records` writes back to the same test (compared as
    /// records, the canonical form); dropping a corner a var or model is scoped to, or
    /// the output a spec names, is refused (negative space).
    #[test]
    fn maestro_records_round_trip(t in tests()) {
        let recs = t.records();
        let back = parse(&recs).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back.records(), recs.clone());
        for (k, r) in recs.iter().enumerate() {
            let named = r.str(0).unwrap_or("");
            let used = match r.tag.as_str() {
                // A var's scope is its third field, a model's its first.
                "corner" => recs.iter().any(|x| {
                    (x.tag == "var" && x.str(2).ok() == Some(named))
                        || (x.tag == "model" && x.str(0).ok() == Some(named))
                        || (x.tag == "parameter" && x.str(2).ok() == Some(named))
                }),
                "output" => recs
                    .iter()
                    .any(|x| x.tag == "spec" && x.str(0).ok() == Some(named)),
                _ => false,
            };
            if used {
                let mut cut = recs.clone();
                cut.remove(k);
                prop_assert!(parse(&cut).is_err(), "{} {} dropped", r.tag, named);
            }
        }
    }
}

#[test]
fn maestro_records_refuse_their_negative_space() {
    let m = Record::new(
        "maestro",
        vec![s("t"), s("lib"), s("tb"), s("schematic"), s("spectre")],
    );
    let with = |r: Record| parse(&[m.clone(), r]);
    assert!(parse(&[]).is_err());
    assert!(parse(&[Record::new("var", vec![s("a"), s("1"), s("")])]).is_err());
    assert!(
        with(Record::new(
            "analysis",
            vec![s("tran"), i(2), Value::List(vec![])]
        ))
        .is_err()
    );
    assert!(with(Record::new("output", vec![s("o"), s("wave"), s("v")])).is_err());
    assert!(with(Record::new("spec", vec![s("o"), s("lt"), s("1")])).is_err());
    assert!(with(Record::new("defn", vec![s("a/b.va"), s("")])).is_err());
    assert!(with(Record::new("var", vec![s("a"), s("1"), s("c0")])).is_err());
    assert!(with(m.clone()).is_err(), "a second maestro record");
    let twice = [
        m.clone(),
        Record::new("corner", vec![s("c"), i(1)]),
        Record::new("corner", vec![s("c"), i(0)]),
    ];
    assert!(parse(&twice).is_err());
    let ok = with(Record::new(
        "analysis",
        vec![
            s("tran"),
            i(1),
            pairs([("stop", "1u"), ("anaName", "tran")]),
        ],
    ))
    .unwrap();
    assert_eq!(
        ok.analyses,
        [("tran".to_owned(), true, vec![("stop".into(), "1u".into())])],
        "form fields dropped"
    );
}

/// What ICADVM20.1 answered for exported outputs (live run 2026-10-15: numbers come back
/// reformatted, arguments space-separated) reads back as the calculator form.
#[test]
fn multi_argument_outputs_read_back_from_maestro() {
    for (ade, calc) in [
        (
            r#"cross(VT("/out") 0.6321206 1 "either")"#,
            "cross(v(out), 632.1206m)",
        ),
        (
            r#"cross(VT("/out") 0.6321205588285577 1 "either")"#,
            "cross(v(out), 632.1206m)",
        ),
        (
            r#"cross(VT("/out") 0.5 2 "rising")"#,
            "cross(v(out), 500m, 1, 2)",
        ),
        (
            r#"cross(VT("/out") 0.5 1 "falling")"#,
            "cross(v(out), 500m, -1)",
        ),
        (r#"value(VT("/out") 0.001)"#, "at(v(out), 1m)"),
        (r#"value(VDC("/out") 1m)"#, "at(v(out), 1m)"),
        (r#"OP("/X1/M2" "gm")"#, r#"op("X1/M2", "gm")"#),
        (r#"2*value(VT("/a") 1m - 1u)"#, "2*at(v(a), 1m - 1u)"),
        // settle's own settlingTime form, and delay with every keyword.
        (
            r#"settlingTime(VT("/out") ymin(xval(VT("/out"))) t ymax(xval(VT("/out"))) t 100*0.01/abs(value(VT("/out") ymax(xval(VT("/out"))))-value(VT("/out") ymin(xval(VT("/out"))))))"#,
            "settle(v(out), 10m)",
        ),
        // As ICADVM20.1 prints it back (live a_probe dump).
        (
            r#"settlingTime(VT("/out") ymin(xval(VT("/out"))) t ymax(xval(VT("/out"))) t ((100 * 0.01) / abs((value(VT("/out") ymax(xval(VT("/out")))) - value(VT("/out") ymin(xval(VT("/out"))))))))"#,
            "settle(v(out), 10m)",
        ),
        (
            r#"delay(?wf1 VT("/in") ?value1 0.5 ?edge1 "rising" ?nth1 1 ?wf2 VT("/out") ?value2 0.5 ?edge2 "either" ?nth2 2)"#,
            "cross(v(out), 500m, 0, 2)-cross(v(in), 500m, 1, 1)",
        ),
    ] {
        assert_eq!(out_from_ade(ade).as_deref(), Some(calc), "{ade}");
    }
    assert_eq!(out_from_ade(r#"cross(VT("/out") 0.5 1 "sideways")"#), None);
}

#[test]
fn outputs_translate_both_ways() {
    let vars = vec!["vdd".to_owned()];
    let a = out_to_ade(
        "ymax(v(out)) - avg(v(x1.n)) * 2^2 / vdd + i(V1)",
        &vars,
        "T",
    )
    .unwrap();
    assert_eq!(
        a,
        "ymax(VT(\"/out\")) - average(VT(\"/x1/n\")) * 2**2 / VAR(\"vdd\") + IT(\"/V1/PLUS\")"
    );
    assert_eq!(
        out_from_ade(&a).unwrap(),
        "ymax(v(out)) - avg(v(x1.n)) * 2^2 / vdd + i(V1)"
    );
    assert_eq!(
        out_from_ade(&out_to_ade("v(\"d[3]\")", &[], "F").unwrap()).unwrap(),
        "v(\"d[3]\")"
    );
    assert!(out_to_ade("x > 1", &vars, "T").is_none());
    assert!(out_from_ade("bandwidth(mag(VF(\"/OUT\")) 3 \"low\")").is_none());
    assert!(out_from_ade("IT(\"/M1/D\")").is_none());
    // A `schemify_core` ammeter's terminal `p` reads back as i().
    assert_eq!(out_from_ade("IS(\"/VM/p\")").unwrap(), "i(VM)");
}

/// The Exercise 1/2 output forms, ADE text written from the *ADE Calculator* /
/// *OCEAN Reference* function list (independent of the code's tables).
#[test]
fn calculator_outputs_map_to_ade_functions() {
    let vars: Vec<String> = ["VDD", "W", "VDS"].map(String::from).to_vec();
    let info = |p: &str| -> Option<InstInfo> {
        let (path, terminal) = match p {
            "VMDN" => ("VMDN", "p"),
            "XDUT" | "DUT" => ("DUT", ""),
            "XDUT/M1" | "DUT/M1" => ("DUT/M1", "D"),
            "M7" => ("M7", "D"),
            _ => return None,
        };
        Some(InstInfo {
            path: path.into(),
            terminal: terminal.into(),
        })
    };
    let tr = |e: &str, letter: &str| {
        out_expr(
            e,
            &OutScope {
                vars: &vars,
                letter,
                inst: info,
                test: "",
            },
        )
    };
    for (e, l, want) in [
        (
            "abs(at(i(VMDN),VDD))",
            "S",
            r#"abs(value(IS("/VMDN/p") VAR("VDD")))"#,
        ),
        (
            "1/at(deriv(i(VMDN)),VDS)",
            "S",
            r#"1/value(deriv(IS("/VMDN/p")) VAR("VDS"))"#,
        ),
        (
            "1000*ymin(1/max(deriv(log10(abs(i(VMDN)))),1m))",
            "S",
            r#"1000*ymin(1/((deriv(log10(abs(IS("/VMDN/p"))))+1m+abs(deriv(log10(abs(IS("/VMDN/p"))))-1m))/2))"#,
        ),
        (
            "abs(im(i(VMDN)))/(2*pi*x())",
            "F",
            r#"abs(imag(IF("/VMDN/p")))/(2*3.141592653589793*xval(IF("/VMDN/p")))"#,
        ),
        ("abs(re(i(VMDN)))", "F", r#"abs(real(IF("/VMDN/p")))"#),
        ("db(at(v(out),10))", "F", r#"dB20(value(VF("/out") 10))"#),
        (
            "cross(db(v(out)),0,-1)",
            "F",
            r#"cross(dB20(VF("/out")) 0 1 "falling")"#,
        ),
        (
            "cross(v(XDUT.n1), 0.5, 1, 2)",
            "T",
            r#"cross(VT("/DUT/n1") 0.5 2 "rising")"#,
        ),
        ("phase_margin(-v(out))", "F", r#"phaseMargin(-VF("/out"))"#),
        (
            "settle(v(out),10u)-20n",
            "T",
            r#"settlingTime(VT("/out") ymin(xval(VT("/out"))) t ymax(xval(VT("/out"))) t 100*10u/abs(value(VT("/out") ymax(xval(VT("/out"))))-value(VT("/out") ymin(xval(VT("/out"))))))-20n"#,
        ),
        (
            "min(v(a), v(b), 1)",
            "T",
            r#"((((VT("/a")+VT("/b")-abs(VT("/a")-VT("/b")))/2)+1-abs(((VT("/a")+VT("/b")-abs(VT("/a")-VT("/b")))/2)-1))/2)"#,
        ),
        ("pow(v(a), 2)", "T", r#"VT("/a")**2"#),
        // SKILL reads `(x)` as a call of x (live OCEAN): no group around one operand.
        ("2*(v(a))+(1)", "T", r#"2*VT("/a")+1"#),
        (
            "delay(v(in), 0.5, v(out), 0.5)",
            "T",
            r#"(cross(VT("/out") 0.5 1 "either")-cross(VT("/in") 0.5 1 "either"))"#,
        ),
        // op(): the bench path, or an explicit device inside a subcircuit.
        (r#"op("M7","gm")"#, "F", r#"OP("/M7" "gm")"#),
        (r#"op("/XDUT/M1","id")"#, "F", r#"OP("/DUT/M1" "id")"#),
        (
            r#"op("XDUT/M1/mx","gds")"#,
            "F",
            r#"OP("/DUT/M1/mx" "gds")"#,
        ),
        (r#"op("Q9","vth")"#, "F", r#"OP("/Q9" "vth")"#),
    ] {
        assert_eq!(tr(e, l).as_deref(), Some(want), "{e}");
    }
    // Negative space: no ADE form, never a partial output.
    for e in [
        "x()",
        "cross(v(a), 0.5, 2)",
        "cross(v(a), 0.5, 1, 0)",
        "slope(v(a))",
        "if(v(a)>0, 1, 2)",
        "v(a) > 1",
        r#"op("M1")"#,
        r#"op("M1", "")"#,
        r#"op("M1//x", "gm")"#,
        "at(v(a))",
        "delay(v(a), 0.5, v(b))",
        "unknown_var * 2",
    ] {
        assert_eq!(tr(e, "T"), None, "{e}");
    }
}

#[test]
fn paths_and_words_round_trip() {
    assert_eq!(path_to_ade("${PDK}/m.lib"), "$PDK/m.lib");
    assert_eq!(path_from_ade("$PDK/m.lib"), "${PDK}/m.lib");
    assert_eq!(path_from_ade("a$/b"), "a$/b");
    for v in ["", "a b", "x=\"1\"", "plain", "\\"] {
        let w = spectre_words(&format!("k={}", spectre_word(v))).unwrap();
        assert_eq!(w, vec![format!("k={v}")]);
    }
    assert_eq!(top_level_split("pow(a,b), 2,3"), vec!["pow(a,b)", "2", "3"]);
}

#[test]
fn sweeps_recover_their_shape() {
    let lin = grid(1.0, 3.0, 3, false).unwrap();
    assert_eq!(lin, ["1", "2", "3"]);
    assert_eq!(sweep_shape(&lin).0, "lin");
    let log = grid(1.0, 100.0, 3, true).unwrap();
    assert_eq!(
        sweep_shape(&log),
        ("log", vec!["1".into(), "100".into(), "3".into()])
    );
    let v: Vec<String> = ["1k", "2"].map(String::from).to_vec();
    assert_eq!(sweep_shape(&v).0, "values");
    assert!(grid(0.0, 1.0, 2, true).is_none());
    // Gap: 81 points 0..0.8 were written with binary noise (0.010000000000000002).
    let fine = grid(0.0, 0.8, 81, false).unwrap();
    assert_eq!(&fine[..4], ["0", "0.01", "0.02", "0.03"]);
    assert_eq!(fine[7], "0.07");
    assert!(fine.iter().all(|p| p.len() <= 4), "{fine:?}");
    assert_eq!(
        sweep_shape(&fine),
        ("lin", vec!["0".into(), "0.8".into(), "81".into()])
    );
    assert_eq!(
        grid(1.0, 1000.0, 4, true).unwrap(),
        ["1", "10", "100", "1000"]
    );
}

/// Cross-test and dependent outputs (expr.md §5 `result`): `calcVal("o" "T")` both ways, a
/// bare name of the test's own output as `result(o, <test>)`.
#[test]
fn other_outputs_read_as_result() {
    let own = ["vmax", "gain"];
    for (ade, calc) in [
        (
            r#"(calcVal("vmax" "T1") * 2)"#,
            r#"(result("vmax", "T1") * 2)"#,
        ),
        ("(vmax * 2)", r#"(result("vmax", "T2") * 2)"#),
        (
            r#"calcVal("vmax" "T1") - gain"#,
            r#"result("vmax", "T1") - result("gain", "T2")"#,
        ),
    ] {
        assert_eq!(
            out_from_ade_in(ade, "T2", &own).as_deref(),
            Some(calc),
            "{ade}"
        );
        let scope = OutScope {
            vars: &[],
            letter: "T",
            inst: |_: &str| None,
            test: "T2",
        };
        assert_eq!(out_expr(calc, &scope).as_deref(), Some(ade), "{calc}");
    }
    // Keywords, an unknown bare name, and a bare name outside any test have no form.
    assert!(out_from_ade_in(r#"calcVal("vmax" "T1" ?historyName "h")"#, "T2", &own).is_none());
    assert!(out_from_ade_in("(bw * 2)", "T2", &own).is_none());
    assert!(out_from_ade("(vmax * 2)").is_none());
}
