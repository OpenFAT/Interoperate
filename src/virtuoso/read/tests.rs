use super::super::write::{cdl_siminfo, spectre_siminfo, template_items, template_to_nlp, urad};
use super::{
    BTreeMap, DCell, Notes, ParamIn, PinRow, SimInfo, SimVal, arc_angles, cdl_format,
    net_from_virtuoso, net_of_prop, net_prop, nlp_in, seg_key, si_text, spectre_format,
    unswap_wire_ids,
};
use proptest::prelude::*;

fn pin(name: &str) -> PinRow {
    PinRow {
        name: name.into(),
        dir: "inout".into(),
        ..PinRow::default()
    }
}

fn param(name: &str, string: bool) -> ParamIn {
    ParamIn {
        name: name.into(),
        kind: if string { "string" } else { "number" }.into(),
        default: "1".into(),
        unit: None,
        doc: None,
        choices: Vec::new(),
        computed: None,
    }
}

/// A simInfo's netlisting fields in the order export writes them: `head`, then
/// `termOrder`, `instParameters` and `propMapping` (only for renamed parameters).
fn siminfo(
    sim: &str,
    head: Vec<(String, SimVal)>,
    pins: &[String],
    kc: &[(String, String)],
) -> SimInfo {
    let mut fields = head;
    fields.push(("termOrder".into(), SimVal::List(pins.to_vec())));
    let ks = kc.iter().map(|x| x.0.clone()).collect();
    fields.push(("instParameters".into(), SimVal::List(ks)));
    let pm: Vec<(String, String)> = kc.iter().filter(|x| x.0 != x.1).cloned().collect();
    if !pm.is_empty() {
        fields.push(("propMapping".into(), SimVal::Pairs(pm)));
    }
    SimInfo {
        sim: sim.into(),
        fields,
    }
}

/// Distinct pin names (Virtuoso spelling, buses included), and CDF parameters
/// `(simulator name k, CDF name c, string)`.
fn netlisting() -> impl Strategy<Value = (Vec<String>, Vec<(String, String, bool)>)> {
    (
        prop::collection::btree_set("p[a-z]{0,2}(<[0-9]>)?", 0..4),
        prop::collection::btree_map("q[a-z]{0,2}", (any::<bool>(), any::<bool>()), 0..4),
    )
        .prop_map(|(pins, params)| {
            let ps = params
                .into_iter()
                .map(|(c, (renamed, string))| {
                    let k = if renamed { format!("k{c}") } else { c.clone() };
                    (k, c, string)
                })
                .collect();
            (pins.into_iter().collect(), ps)
        })
}

proptest! {
    /// Import's spectre format of a simInfo reads back, through export's reverse rule, as
    /// the same simInfo (§6.3): master, terminal order, parameters, renames, and the
    /// optional form of string parameters.
    #[test]
    fn spectre_siminfo_round_trips_through_the_format(
        (pins, ps) in netlisting(),
        master in "[A-Za-z][A-Za-z0-9_]{0,5}",
    ) {
        let kc: Vec<(String, String)> = ps.iter().map(|x| (x.0.clone(), x.1.clone())).collect();
        let head = vec![("componentName".to_owned(), SimVal::Str(master.clone()))];
        let si = siminfo("spectre", head, &pins, &kc);
        let names: Vec<String> = pins.iter().map(|p| net_from_virtuoso(p).text).collect();
        let rows: Vec<PinRow> = names.iter().map(|p| pin(p)).collect();
        let params: Vec<ParamIn> = ps.iter().map(|x| param(&x.1, x.2)).collect();
        let t = spectre_format(&DCell::default(), &si, &params, &rows, &mut Notes::default());
        let kinds: Vec<(String, String)> =
            params.iter().map(|p| (p.name.clone(), p.kind.clone())).collect();
        prop_assert_eq!(spectre_siminfo(&t, &names, &kinds), Some(si), "via {}", t);
    }

    /// The same for auCdl's subcircuit call (`X{name} {pins} / comp k={c}…`).
    #[test]
    fn cdl_subckt_siminfo_round_trips_through_the_format(
        (pins, ps) in netlisting(),
        comp in "[A-Za-z][A-Za-z0-9_]{0,5}",
        prefix in "[A-Z]{1,2}",
    ) {
        let kc: Vec<(String, String)> = ps.iter().map(|x| (x.0.clone(), x.1.clone())).collect();
        let head = vec![
            ("netlistProcedure".to_owned(), SimVal::Str("ansCdlSubcktCall".into())),
            ("componentName".to_owned(), SimVal::Str(comp)),
            ("namePrefix".to_owned(), SimVal::Str(prefix)),
        ];
        let si = siminfo("auCdl", head, &pins, &kc);
        let names: Vec<String> = pins.iter().map(|p| net_from_virtuoso(p).text).collect();
        let rows: Vec<PinRow> = names.iter().map(|p| pin(p)).collect();
        let params: Vec<ParamIn> = ps.iter().map(|x| param(&x.1, x.2)).collect();
        let t = cdl_format(&DCell::default(), &si, &params, &rows, &mut Notes::default());
        let kinds: Vec<(String, String)> =
            params.iter().map(|p| (p.name.clone(), p.kind.clone())).collect();
        prop_assert_eq!(cdl_siminfo(&t, &names, &kinds), Some(si), "via {}", t);
    }

    /// A Schemify arc's `(start, sweep)` (milli-degrees, screen clockwise) exported as
    /// Virtuoso's counter-clockwise micro-radian `(start, stop)` reads back exactly, start
    /// normalized into a turn: a micro-radian is finer than half a milli-degree.
    #[test]
    fn arc_angles_round_trip(start in -720_000i32..720_000, sweep in 1i32..360_000) {
        let deg = |m: i32| f64::from(m) / 1000.0;
        // Export (`write::graphic_out`): y flips, so the angles negate and swap ends.
        let a0 = urad(-(deg(start) + deg(sweep)));
        let a1 = urad(-deg(start));
        prop_assert_eq!(arc_angles(a0, a1), (start.rem_euclid(360_000), sweep));
    }

    /// `schemifyNet` reads back as written for any net text without a space.
    #[test]
    fn net_props_round_trip(net in "[A-Za-z0-9_!<>:,*\\[\\]]{1,12}", global: bool) {
        prop_assert_eq!(net_of_prop(&net_prop(&net, global)), (net, global));
    }
}

/// A label template from literals and holes, braces of the literals doubled.
fn template(lits: &[String], holes: &[&str]) -> String {
    let mut t = lits[0].replace('{', "{{").replace('}', "}}");
    for (h, l) in holes.iter().zip(&lits[1..]) {
        t.push_str(&format!("{{{h}}}"));
        t.push_str(&l.replace('{', "{{").replace('}', "}}"));
    }
    t
}

proptest! {
    /// Export's NLP text of a symbol label reads back as the same template (§3.5),
    /// whatever NLP-reserved characters (`\ [ ] : %`) and braces the literals hold.
    #[test]
    fn label_templates_round_trip_through_nlp(
        (holes, lits) in (1usize..4).prop_flat_map(|n| (
            prop::collection::vec(prop::sample::select(vec!["w", "l", "m", "nf"]), n),
            prop::collection::vec("[a-z %:\\[\\]\\\\{}=/]{0,5}", n + 1),
        )),
    ) {
        let t = template(&lits, &holes);
        let items = template_items(&t).expect("balanced");
        let nlp = template_to_nlp(&items, &["w", "l", "m", "nf"]).expect("every hole a param");
        prop_assert_eq!(nlp_in(&nlp, ""), Some(t.clone()), "{} via {}", t, nlp);
    }
}

proptest! {
    /// Respelling a number keeps its value (to rounding of the suffix scale), whatever
    /// its magnitude and sign: both sides of the stash comparison read numbers through
    /// `si_text`, so a respelling that moved the value would hide an edit.
    #[test]
    fn engineering_text_keeps_the_value(m in 1u32..10_000_000, e in -21i32..17, neg: bool) {
        let t = format!("{}{m}e{e}", if neg { "-" } else { "" });
        let x = schemify_expr::parse_num(&t).unwrap();
        let back = si_text(&t);
        let y = schemify_expr::parse_num(&back)
            .unwrap_or_else(|_| panic!("{t} became unreadable {back}"));
        prop_assert!((x - y).abs() <= x.abs() * 1e-14, "{} = {} became {} = {}", t, x, back, y);
    }
}

#[test]
fn an_escaped_percent_is_text() {
    // `\%` is a literal percent (NLP); only the first bare `%` is the value.
    assert_eq!(nlp_in("[@w:50\\%=%]", "").as_deref(), Some("50%={w}"));
    assert_eq!(nlp_in("[@w:a\\%b%c]", "").as_deref(), Some("a%b{w}c"));
    assert_eq!(nlp_in("[@w:%:fallback]", "").as_deref(), Some("{w}"));
    assert_eq!(nlp_in("[@w:no value]", "").as_deref(), Some("no value{w}"));
    assert_eq!(nlp_in("[@refDes]", ""), None);
    assert_eq!(nlp_in("[@w:a:b:c]", ""), None);
    assert_eq!(nlp_in("[@w", ""), None);
}

#[test]
fn swapped_wire_ids_go_back_only_as_a_permutation() {
    let (lo, hi) = (
        seg_key([320, 230], [320, 250]),
        seg_key([320, 250], [320, 270]),
    );
    let stashed: BTreeMap<_, _> = [(lo, "w13".to_owned()), (hi, "w14".to_owned())].into();
    // The ota run: the two pieces came back with each other's ids.
    let got = unswap_wire_ids(&[(1, hi, "w13"), (2, lo, "w14")], &stashed);
    assert_eq!(got, [(1, "w14".to_owned()), (2, "w13".to_owned())].into());
    // As stashed: nothing to do.
    assert!(unswap_wire_ids(&[(1, lo, "w13"), (2, hi, "w14")], &stashed).is_empty());
    // One piece moved in Virtuoso as well: not a permutation, Virtuoso's ids stay.
    let moved = seg_key([0, 0], [0, 10]);
    assert!(unswap_wire_ids(&[(1, hi, "w13"), (2, moved, "w14")], &stashed).is_empty());
}

#[test]
fn numbers_read_as_engineering_text() {
    for (a, b) in [
        ("150.00n", "150n"),
        ("0.15u", "150n"),
        ("900.0m", "900m"),
        ("0.9", "900m"),
        ("1", "1"),
        ("2u", "2u"),
        ("1.8", "1.8"),
        ("-2.5meg", "-2.5meg"),
        ("10k", "10k"),
        ("0", "0"),
        ("W/2", "W/2"),
        ("1e-12", "1p"),
    ] {
        assert_eq!(si_text(a), b, "{a}");
    }
}
