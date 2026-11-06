//! CDF functions (`cdf*`; cdfuser ch. 8 "CDF SKILL Summary"): base cell CDFs, their
//! params, instance CDFs and the parameter updates an instance sees.

use super::*;

/// The CDF rows of [`SIGNATURES`].
pub(super) const ROWS: &[(&str, &str)] = &[
    ("cdfGetBaseCellCDF", "d"),
    // cdfuser ch. 8: a library's base CDF (none in the mock's libraries).
    ("cdfGetBaseLibCDF", "d"),
    (
        "cdfCreateBaseCellCDF",
        "d ?doneProc ?formInitProc ?fieldWidth ?fieldHeight ?buttonFieldWidth ?promptWidth",
    ),
    ("cdfDeleteCDF", "d"),
    (
        "cdfCreateParam",
        "d ?name ?type ?defValue ?units ?parseAsNumber ?choices ?prompt ?use ?display ?editable ?dontSave ?callback ?storeDefault ?parseAsCEL ?description",
    ),
    ("cdfSaveCDF", "d"),
    ("cdfGetInstCDF", "d"),
    ("cdfFindParamByName", "d t"),
    ("cdfGetCellCDF", "d"),
    ("cdfUpdateInstParam", "d"),
    ("cdfParseFloatString", "t"),
    // skdfref ch. 2: a textDisplay an instance owns shows a CDF value (a master label such
    // as [@w], a property, an attribute).
    ("dbCreateTextDisplay", "d d L g p t t t n [g g g g g T g]"),
];

/// `cdfParseFloatString(s)`: a number with an optional SKILL scale factor (`500n`, `1.5K`,
/// `2M` mega, `5e-07`) as a float. GUESS: text that is not one gives nil.
fn parse_float_string(t: &str) -> V {
    let t = t.trim();
    let exp = |c: char| match c {
        'T' => Some("e12"),
        'G' => Some("e9"),
        'M' => Some("e6"),
        'K' | 'k' => Some("e3"),
        '%' => Some("e-2"),
        'm' => Some("e-3"),
        'u' => Some("e-6"),
        'n' => Some("e-9"),
        'p' => Some("e-12"),
        'f' => Some("e-15"),
        'a' => Some("e-18"),
        _ => None,
    };
    // The mantissa with the factor as an exponent, so 3u is the double nearest 3e-6.
    let text = match t.chars().last().and_then(exp) {
        Some(e) => format!("{}{e}", &t[..t.len() - 1]),
        None => t.to_owned(),
    };
    text.parse::<f64>().map_or(V::Nil, V::Float)
}

impl MockDb {
    // ------------------------------------------------------------------ CDF

    fn cdf_param_type_ok(ty: &str, v: &V) -> bool {
        match ty {
            "int" => matches!(v, V::Int(_)),
            "float" => matches!(v, V::Float(_) | V::Int(_)),
            "boolean" => matches!(v, V::T | V::Nil | V::Str(_)),
            "button" => true,
            _ => matches!(v, V::Str(_)),
        }
    }

    fn inst_cdf(&mut self, inst: u32) -> V {
        let (lib, cell) = (self.a(inst, "libName"), self.a(inst, "cellName"));
        let (Some(lib), Some(cell)) = (s(&lib), s(&cell)) else {
            return V::Nil;
        };
        let Some(base) = self.base_cdf(lib, cell) else {
            return V::Nil;
        };
        let cdf = self.new_obj(
            "cdf",
            vec![
                ("type", V::str("instData")),
                ("id", V::Obj(inst)),
                ("simInfo", self.a(base, "simInfo")),
                ("paramLabelSet", self.a(base, "paramLabelSet")),
            ],
        );
        for p in self.a(base, "parameters").items().to_vec() {
            let Some(p) = Self::obj(&p) else { continue };
            let mut o = self.objs[p as usize].clone();
            o.attrs.insert("sfyInst".into(), V::Obj(inst));
            o.attrs.insert("sfyCdf".into(), V::Obj(cdf));
            o.attrs.remove("value");
            self.objs.push(o);
            let id = (self.objs.len() - 1) as u32;
            self.push(cdf, "parameters", V::Obj(id));
        }
        V::Obj(cdf)
    }

    /// `cdfGetCellCDF(ddCell)`: the cell's effective CDF, here a copy of the base CDF held
    /// by the cell as the one in memory (`cdfUpdateInstParam` reads it), made again when the
    /// base CDF changed shape. Its params have no instance: setting `value` sets the
    /// in-memory default, as on a real cell CDF, and it stays set for every later caller,
    /// which is why callers restore it.
    fn cell_cdf(&mut self, cell: u32) -> V {
        if self.ty(cell) != "ddCell" {
            return V::Nil;
        }
        let Some(base) = Self::obj(&self.a(cell, "sfyBaseCdf")) else {
            return V::Nil;
        };
        if let Some(c) = Self::obj(&self.a(cell, "sfyCellCdf"))
            && self.a(c, "parameters").items().len() == self.a(base, "parameters").items().len()
        {
            return V::Obj(c);
        }
        let cdf = self.new_obj(
            "cdf",
            vec![
                ("type", V::str("cellData")),
                ("id", V::Obj(cell)),
                ("simInfo", self.a(base, "simInfo")),
                ("paramLabelSet", self.a(base, "paramLabelSet")),
            ],
        );
        for p in self.a(base, "parameters").items().to_vec() {
            let Some(p) = Self::obj(&p) else { continue };
            let mut o = self.objs[p as usize].clone();
            o.attrs.insert("sfyCellOf".into(), V::str("cell"));
            self.objs.push(o);
            let id = (self.objs.len() - 1) as u32;
            self.push(cdf, "parameters", V::Obj(id));
        }
        self.put(cell, "sfyCellCdf", V::Obj(cdf));
        V::Obj(cdf)
    }

    /// `cdfUpdateInstParam(inst)`: copies every value of the master cell's in-memory cell
    /// CDF (the latest `cdfGetCellCDF`) onto the instance, through the base parameter so a
    /// value equal to the default leaves no property. Without a cell CDF in memory it
    /// changes nothing.
    fn update_inst_params(&mut self, inst: u32) -> Result<V, String> {
        let (lib, cell) = (self.a(inst, "libName"), self.a(inst, "cellName"));
        let (Some(lib), Some(cell)) = (s(&lib).map(str::to_owned), s(&cell).map(str::to_owned))
        else {
            return Ok(V::Nil);
        };
        let Some(c) = self.dd_find(&lib, Some(&cell), None, None) else {
            return Ok(V::Nil);
        };
        let (Some(cdf), Some(base)) = (
            Self::obj(&self.a(c, "sfyCellCdf")),
            self.base_cdf(&lib, &cell),
        ) else {
            return Ok(V::T);
        };
        for p in self.a(cdf, "parameters").items().to_vec() {
            let Some(p) = Self::obj(&p) else { continue };
            let n = s(&self.a(p, "name")).unwrap_or("").to_owned();
            let Some(bp) = self.cdf_param(base, &n) else {
                continue;
            };
            if s(&self.a(bp, "paramType")) == Some("button") {
                continue;
            }
            // What the callbacks left in the cell CDF: its live value, else its default.
            let v = match self.a(p, "sfyLive") {
                V::Nil => self.a(p, "defValue"),
                live => live,
            };
            self.set_inst_param(bp, inst, v)?;
        }
        Ok(V::T)
    }

    /// `param~>value = v` on an instance CDF: stores the instance property with the CDF's
    /// type.
    /// The value Virtuoso stores for a parseAsNumber param with units: kilo as AEL's `K`
    /// (live ICADVM20.1 run: `r` "10k" of analogLib res came back "10K").
    fn ael_kilo(&self, p: u32, v: V) -> V {
        let numeric = s(&self.a(p, "parseAsNumber")) == Some("yes")
            && s(&self.a(p, "units")).is_some_and(|u| !u.is_empty());
        match &v {
            V::Str(x) if numeric => match x.strip_suffix('k') {
                Some(m) if m.parse::<f64>().is_ok() => V::str(&format!("{m}K")),
                _ => v,
            },
            _ => v,
        }
    }

    /// Live ICADVM20.1 (a foundry FD-SOI PDK, `cdfleak` probes): setting a parameter through an
    /// instance's CDF also sets the in-memory cell CDF's `value` (`sfyLive` here; its
    /// `defValue` and the base CDF stay), and every instance made afterwards stores that
    /// value as a property of its own ([`MockDb::new_inst`]).
    pub(super) fn leak_to_cell(&mut self, inst: u32, p: u32, v: V) {
        let (lib, cell) = (self.a(inst, "libName"), self.a(inst, "cellName"));
        let Some(c) = s(&lib)
            .zip(s(&cell))
            .and_then(|(l, c)| self.dd_find(l, Some(c), None, None))
        else {
            return;
        };
        let name = s(&self.a(p, "name")).unwrap_or("").to_owned();
        if let Some(cdf) = Self::obj(&self.cell_cdf(c))
            && let Some(cp) = self.cdf_param(cdf, &name)
        {
            self.put(cp, "sfyLive", v);
        }
    }

    pub(super) fn set_inst_param(&mut self, p: u32, inst: u32, v: V) -> Result<V, String> {
        let ty = s(&self.a(p, "paramType")).unwrap_or("string").to_owned();
        let name = s(&self.a(p, "name")).unwrap_or("").to_owned();
        if !Self::cdf_param_type_ok(&ty, &v) {
            return Err(format!(
                "*Error* CDF parameter {name} ({ty}): value of the wrong type"
            ));
        }
        let store_default = s(&self.a(p, "storeDefault")) == Some("yes");
        if super::super::equal(&v, &self.a(p, "defValue")) && !store_default {
            // GUESS(G4): a value equal to the default with storeDefault "no" leaves no
            // property (as the form does); the loader never sends one (spec §5.3).
            self.remove_prop(inst, &name);
            return Ok(v);
        }
        let (pty, pv) = match ty.as_str() {
            "int" => ("int", v.clone()),
            "float" => ("float", V::Float(num(&v).unwrap_or(0.0))),
            "boolean" => (
                "boolean",
                if matches!(&v, V::Str(x) if &**x == "nil" || &**x == "FALSE") {
                    V::Nil
                } else if v.truthy() {
                    V::T
                } else {
                    V::Nil
                },
            ),
            // G13: an inherited-connection override is a netSet-typed property (as
            // virtuoso-bridge-lite writes it with dbReplaceProp ... "netSet").
            "netSet" => ("netSet", v.clone()),
            _ => ("string", self.ael_kilo(p, v.clone())),
        };
        self.set_prop(inst, &name, pty, pv);
        Ok(v)
    }

    /// `dbCreateTextDisplay(assoc owner lpp flags xy justify orient font height [drafted
    /// overbar visible nameVisible valueVisible [name]])`: a textDisplay of owner `owner`
    /// (an instance) in its cellview, `assocType` and `text` by what `assoc` is (a label: its
    /// text; a property: its name; else the attribute `name`). ponytail: xy and orient are
    /// kept as given; Virtuoso maps a label's through the instance (live ICADVM20.1), which
    /// the loader undoes by setting both afterwards.
    fn create_text_display(&mut self, a: &[V]) -> Result<V, String> {
        let (Some(assoc), Some(owner)) = (Self::obj(&a[0]), Self::obj(&a[1])) else {
            return Err("*Error* dbCreateTextDisplay: associate and owner must be objects".into());
        };
        let Some(cv) = Self::obj(&self.a(owner, "cellView")) else {
            return Ok(V::Nil);
        };
        if !self.editable(cv) {
            return Ok(V::Nil);
        }
        let (ty, text) = match self.ty(assoc).as_str() {
            "label" => ("labelAssocType", self.a(assoc, "theLabel")),
            "prop" => ("propAssocType", self.a(assoc, "name")),
            _ => match a.get(14) {
                Some(V::Str(n)) => ("attrAssocType", V::Str(n.clone())),
                _ => return Ok(V::Nil),
            },
        };
        let flag = |k: usize, d: bool| a.get(k).map_or(d, V::truthy);
        let b = |x: bool| if x { V::T } else { V::Nil };
        let td = self.new_shape(
            cv,
            "textDisplay",
            &Self::lpp(&a[2]),
            vec![
                ("owner", V::Obj(owner)),
                ("associate", V::Obj(assoc)),
                ("assocType", V::str(ty)),
                ("text", text),
                ("xy", a[4].clone()),
                ("justify", a[5].clone()),
                ("orient", a[6].clone()),
                ("font", a[7].clone()),
                ("height", a[8].clone()),
                ("isDrafted", b(flag(9, false))),
                ("isOverbar", b(flag(10, false))),
                ("isVisible", b(flag(11, true))),
                ("isNameVisible", b(flag(12, true))),
                ("isValueVisible", b(flag(13, true))),
            ],
        );
        self.push(owner, "textDisplays", V::Obj(td));
        Ok(V::Obj(td))
    }

    pub(super) fn cdf_param(&self, cdf: u32, name: &str) -> Option<u32> {
        self.a(cdf, "parameters")
            .items()
            .iter()
            .filter_map(Self::obj)
            .find(|&p| s(&self.a(p, "name")) == Some(name))
    }

    /// The bodies of the CDF rows: `dispatch` hands over every name it has no arm for.
    pub(super) fn dispatch_cdf(
        &mut self,
        name: &str,
        a: &[V],
        kw: &[(String, V)],
    ) -> Result<V, String> {
        let o = |i: usize| -> u32 { Self::obj(&a[i]).unwrap_or(0) };
        let st = |i: usize| -> String { a.get(i).and_then(s).unwrap_or("").to_owned() };
        let key =
            |k: &str| -> Option<V> { kw.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()) };
        Ok(match name {
            "cdfGetBaseCellCDF" => self.a(o(0), "sfyBaseCdf"),
            "cdfGetBaseLibCDF" => self.a(o(0), "sfyBaseCdf"),
            "cdfCreateBaseCellCDF" => {
                let c = o(0);
                if self.ty(c) != "ddCell" || self.a(c, "sfyBaseCdf").truthy() {
                    return Ok(V::Nil);
                }
                let cdf = self.new_obj(
                    "cdf",
                    vec![
                        ("type", V::str("baseCellData")),
                        ("id", V::Obj(c)),
                        ("doneProc", key("doneProc").unwrap_or(V::str(""))),
                        ("formInitProc", key("formInitProc").unwrap_or(V::str(""))),
                    ],
                );
                self.put(c, "sfyBaseCdf", V::Obj(cdf));
                V::Obj(cdf)
            }
            "cdfDeleteCDF" => {
                let cdf = o(0);
                match Self::obj(&self.a(cdf, "id")) {
                    Some(c) if self.ty(c) == "ddCell" => {
                        self.put(c, "sfyBaseCdf", V::Nil);
                        V::T
                    }
                    _ => V::Nil,
                }
            }
            "cdfCreateParam" => {
                let cdf = o(0);
                let (Some(V::Str(pname)), Some(V::Str(ty))) = (key("name"), key("type")) else {
                    return Err(
                        "*Error* cdfCreateParam: ?name and ?type are required strings".into(),
                    );
                };
                if !CDF_TYPES.contains(&&*ty) {
                    return Err(format!("*Error* cdfCreateParam: type {ty}"));
                }
                let def = key("defValue");
                if def.is_none() && !matches!(&*ty, "boolean" | "button") {
                    return Err(format!("*Error* cdfCreateParam: {pname} needs ?defValue"));
                }
                let def = def.unwrap_or(V::Nil);
                if !matches!(&*ty, "button") && def.truthy() && !Self::cdf_param_type_ok(&ty, &def)
                {
                    return Err(format!(
                        "*Error* cdfCreateParam: {pname}: ?defValue does not match type {ty}"
                    ));
                }
                if matches!(&*ty, "radio" | "cyclic") && !matches!(key("choices"), Some(V::List(_)))
                {
                    return Err(format!(
                        "*Error* cdfCreateParam: {pname}: {ty} needs ?choices"
                    ));
                }
                if self.ty(cdf) != "cdf" || self.cdf_param(cdf, &pname).is_some() {
                    return Ok(V::Nil); // an existing name is skipped silently
                }
                // ICADVM20.1 (live run 2026-10-15): "'parseAsNumber' field must be set if
                // 'units' field is set for this type of parameter", and no parameter.
                let units = matches!(key("units"), Some(V::Str(u)) if !u.is_empty());
                let parsed = matches!(key("parseAsNumber"), Some(V::Str(y)) if &*y == "yes");
                if &*ty == "string" && units && !parsed {
                    return Ok(V::Nil);
                }
                // GUESS(G16): every attribute reads back as it was passed ("yes", "t", "nil");
                // an unset one reads nil.
                let mut attrs = vec![
                    ("name", V::Str(pname.clone())),
                    ("paramType", V::Str(ty.clone())),
                    ("defValue", def.clone()),
                    ("sfyCdf", V::Obj(cdf)),
                ];
                for (k, v) in kw {
                    if !matches!(k.as_str(), "name" | "type" | "defValue") {
                        attrs.push((k.as_str(), v.clone()));
                    }
                }
                let p = self.new_obj("cdfParam", attrs);
                self.push(cdf, "parameters", V::Obj(p));
                V::Obj(cdf)
            }
            "cdfSaveCDF" => {
                let cdf = o(0);
                if s(&self.a(cdf, "type")) == Some("baseCellData") {
                    V::T
                } else {
                    V::Nil
                }
            }
            "cdfGetInstCDF" => {
                let i = o(0);
                if self.ty(i) != "inst" {
                    return Ok(V::Nil);
                }
                self.inst_cdf(i)
            }
            "cdfFindParamByName" => self.cdf_param(o(0), &st(1)).map_or(V::Nil, V::Obj),
            "cdfGetCellCDF" => self.cell_cdf(o(0)),
            "cdfUpdateInstParam" => {
                let i = o(0);
                if self.ty(i) != "inst" {
                    return Ok(V::Nil);
                }
                self.update_inst_params(i)?
            }
            "cdfParseFloatString" => parse_float_string(&st(0)),
            "dbCreateTextDisplay" => self.create_text_display(a)?,
            _ => unreachable!("{name} has a signature but no body"),
        })
    }
}
