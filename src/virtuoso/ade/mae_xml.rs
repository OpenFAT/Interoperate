//! Maestro's setup XML (spec `interop.md` §9.7): the file `axlExportSetup` writes and
//! `axlImportSetup` reads, the view's `maestro.sdb`, and `active.state` (the per-test
//! state) all share one shape, an element holding text and then child elements (one tab
//! per level in the files):
//!
//! ```text
//! <?xml version="1.0"?>
//! <setupdb version="6">name
//!     <active>Active Setup
//!         <vars>
//!             <var>C
//!                 <value>1n</value>
//! ```
//!
//! [`parse`] reads that shape into a [`Node`] tree and [`write`] prints it back byte for
//! byte (tab indentation, text before children, no self-closing tags), so a setup Interop
//! did not change is sent back exactly as Virtuoso wrote it. [`Setup`] is the typed view of
//! a setup database the mappings read (lanes L5 and L6: run modes, optimizer, several
//! tests); [`normalize`] removes what Virtuoso itself adds on an import, so two exports of
//! one setup compare equal.

// Data design: in = one XML file (≤ 2^30 bytes, the exchange's cap); out = a tree of owned
// nodes, children in file order (order is meaning: tests, specs, run options). Depth ≤
// DEPTH_MAX (Maestro nests six deep), so the writer's recursion is bounded. Lifetime: one
// call; the stored text stays the source of truth and the tree is rebuilt when needed.

/// Nesting depth [`parse`] accepts (`active.state` nests about ten deep).
pub const DEPTH_MAX: usize = 64;

/// The XML declaration every Maestro file starts with.
const DECL: &str = "<?xml version=\"1.0\"?>";

/// One element: `<name a="v">text` children `</name>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Tag name.
    pub name: String,
    /// Attributes in file order, values unescaped.
    pub attrs: Vec<(String, String)>,
    /// The text before the first child, unescaped: the element's own value (`T1`, `85`).
    /// Trailing white space before a child is layout, not text, and is not kept.
    pub text: String,
    /// Child elements in file order.
    pub children: Vec<Node>,
}

impl Node {
    /// A childless element `<name>text</name>`.
    pub fn leaf(name: &str, text: &str) -> Node {
        Node {
            name: name.into(),
            text: text.into(),
            ..Node::default()
        }
    }

    /// The value of attribute `k`.
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_str())
    }

    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// See [`Node::child`].
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Node> {
        self.children.iter_mut().find(|c| c.name == name)
    }

    /// The children named `name`, in order.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The text of child `name` (`<value>…</value>` of a var, `<specType>` of a spec).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }

    /// The node at `path`: each step a `(name, text)` pair, text `None` for the first child
    /// of that name (`[("active", None), ("vars", None), ("var", Some("C"))]`).
    pub fn at(&self, path: &[(&str, Option<&str>)]) -> Option<&Node> {
        let mut n = self;
        for (name, text) in path {
            n = n
                .children
                .iter()
                .find(|c| c.name == *name && text.is_none_or(|t| c.text == t))?;
        }
        Some(n)
    }
}

/// Parses Maestro XML: an optional `<?xml …?>` declaration, then one root element. Entities
/// `&lt; &gt; &amp; &quot; &apos;` and `&#…;` are decoded.
///
/// # Errors
///
/// Malformed markup, a mismatched or missing end tag, text after a child element (Maestro
/// never writes mixed content, so it would not print back), nesting deeper than
/// [`DEPTH_MAX`], or anything after the root.
pub fn parse(xml: &str) -> Result<Node, String> {
    let b = xml.as_bytes();
    let mut k = 0;
    let skip_ws = |k: &mut usize| {
        while *k < b.len() && b[*k].is_ascii_whitespace() {
            *k += 1;
        }
    };
    skip_ws(&mut k);
    if xml[k..].starts_with("<?") {
        k += xml[k..].find("?>").ok_or("unterminated <?…?>")? + 2;
    }
    // Open elements; the top is the one being filled.
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        // Text up to the next tag.
        let lt = xml[k..].find('<').map(|x| k + x);
        let raw = &xml[k..lt.unwrap_or(xml.len())];
        match stack.last_mut() {
            Some(top) if !raw.is_empty() => {
                let has_children = !top.children.is_empty();
                if has_children || !top.text.is_empty() {
                    if !raw.trim().is_empty() {
                        return Err(format!("<{}>: text after a child element", top.name));
                    }
                } else {
                    top.text = unescape(raw)?;
                }
            }
            Some(_) => {}
            None if raw.trim().is_empty() => {}
            None => return Err("text outside the root element".into()),
        }
        let Some(lt) = lt else { break };
        k = lt;
        if xml[k..].starts_with("<!--") {
            k += xml[k..].find("-->").ok_or("unterminated comment")? + 3;
            continue;
        }
        let gt = k + xml[k..].find('>').ok_or("unterminated tag")?;
        let tag = &xml[k + 1..gt];
        k = gt + 1;
        if let Some(name) = tag.strip_prefix('/') {
            let mut n = stack
                .pop()
                .ok_or_else(|| format!("</{name}> without its start"))?;
            if n.name != name.trim() {
                return Err(format!("<{}> closed by </{name}>", n.name));
            }
            // The text before the first child ends with the child's indentation.
            if !n.children.is_empty() {
                n.text.truncate(n.text.trim_end().len());
            }
            match stack.last_mut() {
                Some(p) => p.children.push(n),
                None if root.is_none() => root = Some(n),
                None => return Err("a second root element".into()),
            }
            continue;
        }
        if root.is_some() {
            return Err("content after the root element".into());
        }
        let (body, empty) = match tag.strip_suffix('/') {
            Some(t) => (t, true),
            None => (tag, false),
        };
        let n = start_tag(body)?;
        if empty {
            match stack.last_mut() {
                Some(p) => p.children.push(n),
                None => root = Some(n),
            }
        } else {
            if stack.len() >= DEPTH_MAX {
                return Err(format!("nested deeper than {DEPTH_MAX}"));
            }
            stack.push(n);
        }
    }
    if let Some(n) = stack.last() {
        return Err(format!("<{}> is not closed", n.name));
    }
    root.ok_or_else(|| "no root element".into())
}

/// `name a="v" …` of a start tag.
fn start_tag(body: &str) -> Result<Node, String> {
    let body = body.trim();
    let end = body.find(char::is_whitespace).unwrap_or(body.len());
    let mut n = Node {
        name: body[..end].to_owned(),
        ..Node::default()
    };
    if n.name.is_empty() {
        return Err("a tag without a name".into());
    }
    let mut rest = body[end..].trim_start();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("<{}>: attribute without =", n.name))?;
        let key = rest[..eq].trim();
        let v = rest[eq + 1..].trim_start();
        let q = v.chars().next().filter(|c| matches!(c, '"' | '\''));
        let q = q.ok_or_else(|| format!("<{}>: attribute {key} unquoted", n.name))?;
        let close = v[1..]
            .find(q)
            .ok_or_else(|| format!("<{}>: attribute {key} unterminated", n.name))?;
        n.attrs.push((key.to_owned(), unescape(&v[1..1 + close])?));
        rest = v[close + 2..].trim_start();
    }
    Ok(n)
}

fn unescape(s: &str) -> Result<String, String> {
    if !s.contains('&') {
        return Ok(s.to_owned());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find('&') {
        out.push_str(&rest[..a]);
        let semi = rest[a..].find(';').ok_or("an & without ;")? + a;
        let ent = &rest[a + 1..semi];
        let ch = match ent {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = match ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    Some(h) => u32::from_str_radix(h, 16).ok(),
                    None => ent.strip_prefix('#').and_then(|d| d.parse().ok()),
                };
                code.and_then(char::from_u32)
                    .ok_or_else(|| format!("unknown entity &{ent};"))?
            }
        };
        out.push(ch);
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' if attr => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Prints `root` as Maestro does: the declaration line, then each element on its own line
/// indented by one tab per level, its text right after the start tag, children on the
/// following lines and the end tag on its own line (on the same line for a childless one).
pub fn write(root: &Node) -> String {
    let mut out = String::from(DECL);
    out.push('\n');
    write_node(root, 0, &mut out);
    out
}

fn write_node(n: &Node, depth: usize, out: &mut String) {
    assert!(depth <= DEPTH_MAX, "trees come from parse or the mappings");
    for _ in 0..depth {
        out.push('\t');
    }
    out.push('<');
    out.push_str(&n.name);
    for (k, v) in &n.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape(v, true, out);
        out.push('"');
    }
    out.push('>');
    escape(&n.text, false, out);
    if !n.children.is_empty() {
        out.push('\n');
        for c in &n.children {
            write_node(c, depth + 1, out);
        }
        for _ in 0..depth {
            out.push('\t');
        }
    }
    out.push_str("</");
    out.push_str(&n.name);
    out.push_str(">\n");
}

/// Removes from a setup database (`<setupdb>`) what Virtuoso derives on its own, so an
/// export compares equal to the export of the same setup after an `axlImportSetup …
/// 'overwrite` (probe G32, fixtures `mae_import_*_overwrite.sdb`):
/// - the root's text (the export file's base name);
/// - a test's `<var>` with an empty `<value>` (the placeholder of a global the test uses),
///   and a global's `<dependentTest omitted="0">` naming a test without a value of its own,
///   with the `<vars>`/`<dependentTests>` left empty by that;
/// - the run option `startingstateorpoint` at its default `Starting State`;
/// - the order of `<active>`'s sections (stably sorted by name: the order is not meaning).
///
/// Everything else, the order of tests, specs and run options included, is kept.
pub fn normalize(root: &mut Node) {
    root.text.clear();
    let Some(active) = root.child_mut("active") else {
        return;
    };
    // Tests with a value of their own, per var.
    let mut own: Vec<(String, String)> = Vec::new();
    if let Some(tests) = active.child_mut("tests") {
        for t in tests.children.iter_mut().filter(|t| t.name == "test") {
            let test = t.text.clone();
            if let Some(vars) = t.child_mut("vars") {
                vars.children
                    .retain(|v| v.name != "var" || v.get("value").is_none_or(|x| !x.is_empty()));
                for v in &vars.children {
                    own.push((v.text.clone(), test.clone()));
                }
            }
            t.children
                .retain(|c| c.name != "vars" || !c.children.is_empty());
        }
    }
    if let Some(vars) = active.child_mut("vars") {
        for v in vars.children.iter_mut().filter(|v| v.name == "var") {
            let name = v.text.clone();
            if let Some(d) = v.child_mut("dependentTests") {
                d.children.retain(|t| {
                    t.attr("omitted") != Some("0")
                        || own.iter().any(|o| o.0 == name && o.1 == t.text)
                });
            }
            v.children
                .retain(|c| c.name != "dependentTests" || !c.children.is_empty());
        }
    }
    if let Some(ro) = active.child_mut("runoptions") {
        for m in &mut ro.children {
            m.children.retain(|o| {
                !(o.text == "startingstateorpoint" && o.get("value") == Some("Starting State"))
            });
        }
    }
    active.children.sort_by(|a, b| a.name.cmp(&b.name));
}

/// A test of the setup database.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupTest {
    /// Test name.
    pub name: String,
    /// False when `<disabledtests>` lists it.
    pub enabled: bool,
    /// `<tool>` (`ADE`).
    pub tool: String,
    /// `<tooloptions>`: `lib cell view sim path state` and any other, in order.
    pub options: Vec<(String, String)>,
    /// The test's own variable values (an empty value is the placeholder of a global).
    pub vars: Vec<(String, String)>,
}

impl SetupTest {
    /// Tool option `k` (`lib`, `cell`, `view`, `sim`).
    pub fn option(&self, k: &str) -> Option<&str> {
        self.options.iter().find(|o| o.0 == k).map(|o| o.1.as_str())
    }
}

/// A global design variable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupVar {
    /// Variable name.
    pub name: String,
    /// Its value text (a number, an expression, a sweep `a:s:b` or a list `a,b,c`).
    pub value: String,
    /// `<dependentTests>`: (test, omitted). `omitted` true: the test does not see the global.
    pub dependents: Vec<(String, bool)>,
}

/// A device parameter override (`maeSetParameter`), globally or in a corner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupParam {
    /// `lib/cell/view/inst[/inst…]`.
    pub location: String,
    /// The instance property (`v2`).
    pub name: String,
    /// Its value text.
    pub value: String,
}

/// A corner (`_default` is Maestro's Nominal).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupCorner {
    /// Corner name.
    pub name: String,
    /// `enabled="1"`.
    pub enabled: bool,
    /// `(name, value)`, `temperature` included.
    pub vars: Vec<(String, String)>,
    /// The corner's parameter overrides.
    pub params: Vec<SetupParam>,
}

/// A spec (`maeSetSpec`): the bound or goal of one output of one test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupSpec {
    /// `<test>.<output>`.
    pub name: String,
    /// The test.
    pub test: String,
    /// The output.
    pub output: String,
    /// `<specType>`: `gt lt range min max tol info`.
    pub kind: String,
    /// `<min>` (gt, range, min), else empty.
    pub min: String,
    /// `<max>` (lt, range, max), else empty.
    pub max: String,
    /// `<target>` (tol), else empty.
    pub target: String,
    /// `<tol>` (tol): absolute (`0.05`) or relative (`10%`), else empty.
    pub tol: String,
    /// `<specWeight>`, absent when unset.
    pub weight: Option<String>,
}

/// The options of one run mode (`Monte Carlo Sampling`, `Global Optimization`, …).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunMode {
    /// The mode's name.
    pub mode: String,
    /// `(name, value)` in the order Maestro holds them (creation order). A mode with no
    /// option set is absent: defaults are not stored.
    pub options: Vec<(String, String)>,
}

impl RunMode {
    /// The value of option `k`.
    pub fn option(&self, k: &str) -> Option<&str> {
        self.options.iter().find(|o| o.0 == k).map(|o| o.1.as_str())
    }
}

/// The typed view of a setup database's `<active>` setup: what `axlExportSetup` writes for
/// the tags `vars tests parameters corners runoptions specs`, plus `currentmode` (present in
/// `maestro.sdb`, not in an export).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    /// Tests in setup order, disabled ones included.
    pub tests: Vec<SetupTest>,
    /// Global variables in setup order.
    pub vars: Vec<SetupVar>,
    /// Global parameter overrides.
    pub params: Vec<SetupParam>,
    /// Corners in setup order, `_default` (Nominal) included.
    pub corners: Vec<SetupCorner>,
    /// Specs in setup order.
    pub specs: Vec<SetupSpec>,
    /// Run modes with options set, in setup order.
    pub run_modes: Vec<RunMode>,
    /// The current run mode, when the file holds it.
    pub current_mode: Option<String>,
}

impl Setup {
    /// Parses setup XML and reads its `<active>` setup.
    ///
    /// # Errors
    ///
    /// [`parse`]'s errors, or a root that is not `<setupdb>` with an `<active>` child.
    pub fn from_xml(xml: &str) -> Result<Setup, String> {
        Setup::from_root(&parse(xml)?)
    }

    /// Reads the `<active>` setup of a parsed `<setupdb>`.
    ///
    /// # Errors
    ///
    /// The root is not `<setupdb>`, or it has no `<active>`.
    pub fn from_root(root: &Node) -> Result<Setup, String> {
        if root.name != "setupdb" {
            return Err(format!("<{}> is not a setup database", root.name));
        }
        let a = root
            .child("active")
            .ok_or("a setup database without <active>")?;
        let pairs = |n: Option<&Node>, item: &str| -> Vec<(String, String)> {
            n.map_or(Vec::new(), |n| {
                n.all(item)
                    .map(|v| (v.text.clone(), v.get("value").unwrap_or("").to_owned()))
                    .collect()
            })
        };
        let params = |n: Option<&Node>| -> Vec<SetupParam> {
            let mut out = Vec::new();
            for l in n.into_iter().flat_map(|n| n.all("location")) {
                for p in l.all("parameter") {
                    out.push(SetupParam {
                        location: l.text.clone(),
                        name: p.text.clone(),
                        value: p.get("value").unwrap_or("").to_owned(),
                    });
                }
            }
            out
        };
        let disabled: Vec<String> = a.child("disabledtests").map_or(Vec::new(), |d| {
            d.all("test").map(|t| t.text.clone()).collect()
        });
        let tests = a.child("tests").map_or(Vec::new(), |ts| {
            ts.all("test")
                .map(|t| SetupTest {
                    name: t.text.clone(),
                    enabled: !disabled.contains(&t.text),
                    tool: t.get("tool").unwrap_or("").to_owned(),
                    options: pairs(t.child("tooloptions"), "option"),
                    vars: pairs(t.child("vars"), "var"),
                })
                .collect()
        });
        let vars = a.child("vars").map_or(Vec::new(), |vs| {
            vs.all("var")
                .map(|v| SetupVar {
                    name: v.text.clone(),
                    value: v.get("value").unwrap_or("").to_owned(),
                    dependents: v.child("dependentTests").map_or(Vec::new(), |d| {
                        d.all("dependentTest")
                            .map(|t| (t.text.clone(), t.attr("omitted") == Some("1")))
                            .collect()
                    }),
                })
                .collect()
        });
        let corners = a.child("corners").map_or(Vec::new(), |cs| {
            cs.all("corner")
                .map(|c| SetupCorner {
                    name: c.text.clone(),
                    enabled: c.attr("enabled") != Some("0"),
                    vars: pairs(c.child("vars"), "var"),
                    params: params(c.child("parameters")),
                })
                .collect()
        });
        let specs = a.child("specs").map_or(Vec::new(), |ss| {
            ss.all("spec")
                .map(|s| {
                    let g = |k: &str| s.get(k).unwrap_or("").to_owned();
                    SetupSpec {
                        name: s.text.clone(),
                        test: g("testname"),
                        output: g("resname"),
                        kind: g("specType"),
                        min: g("min"),
                        max: g("max"),
                        target: g("target"),
                        tol: g("tol"),
                        weight: s.get("specWeight").map(str::to_owned),
                    }
                })
                .collect()
        });
        let run_modes = a.child("runoptions").map_or(Vec::new(), |ro| {
            ro.all("mode")
                .map(|m| RunMode {
                    mode: m.text.clone(),
                    options: pairs(Some(m), "runoption"),
                })
                .collect()
        });
        Ok(Setup {
            tests,
            vars,
            params: params(a.child("parameters")),
            corners,
            specs,
            run_modes,
            current_mode: a.get("currentmode").map(str::to_owned),
        })
    }

    /// The test named `name`.
    pub fn test(&self, name: &str) -> Option<&SetupTest> {
        self.tests.iter().find(|t| t.name == name)
    }

    /// The global variable named `name`.
    pub fn var(&self, name: &str) -> Option<&SetupVar> {
        self.vars.iter().find(|v| v.name == name)
    }

    /// The corner named `name` (`_default` for Nominal).
    pub fn corner(&self, name: &str) -> Option<&SetupCorner> {
        self.corners.iter().find(|c| c.name == name)
    }

    /// The spec of `output` in `test`.
    pub fn spec(&self, test: &str, output: &str) -> Option<&SetupSpec> {
        self.specs
            .iter()
            .find(|s| s.test == test && s.output == output)
    }

    /// The options of run mode `mode`, `None` when none is set.
    pub fn run_mode(&self, mode: &str) -> Option<&RunMode> {
        self.run_modes.iter().find(|m| m.mode == mode)
    }
}

/// The `<Test Name="…">` elements of an `active.state` file, `(name, element text)`, each
/// text exactly as in the file from its tab-indented start line to its end line: the
/// `maestate` records (§4.2).
///
/// # Errors
///
/// A `<Test>` start without its end line.
pub fn state_tests(state: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    for line in state.split_inclusive('\n') {
        if let Some((_, text)) = &mut cur {
            text.push_str(line);
            if line.trim() == "</Test>" {
                out.push(cur.take().expect("open"));
            }
        } else if let Some(rest) = line.trim_start().strip_prefix("<Test Name=\"") {
            let name = rest.split('"').next().unwrap_or("").to_owned();
            cur = Some((name, line.to_owned()));
        }
    }
    match cur {
        Some((n, _)) => Err(format!("<Test Name=\"{n}\"> is not closed")),
        None => Ok(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDB: &str = "<?xml version=\"1.0\"?>\n<setupdb version=\"6\">x\n\t<active>Active Setup\n\t\t<corners>\n\t\t\t<corner enabled=\"1\">_default</corner>\n\t\t\t<corner enabled=\"0\">cold\n\t\t\t\t<vars>\n\t\t\t\t\t<var>temperature\n\t\t\t\t\t\t<value>-40</value>\n\t\t\t\t\t</var>\n\t\t\t\t</vars>\n\t\t\t</corner>\n\t\t</corners>\n\t\t<specs>\n\t\t\t<spec>T1.v\n\t\t\t\t<testname>T1</testname>\n\t\t\t\t<resname>v</resname>\n\t\t\t\t<specType>tol</specType>\n\t\t\t\t<min></min>\n\t\t\t\t<target>0.1</target>\n\t\t\t\t<tol>10%</tol>\n\t\t\t\t<specWeight>2</specWeight>\n\t\t\t</spec>\n\t\t</specs>\n\t\t<runoptions>\n\t\t\t<mode>Monte Carlo Sampling\n\t\t\t\t<runoption>mcnumpoints\n\t\t\t\t\t<value>7</value>\n\t\t\t\t</runoption>\n\t\t\t</mode>\n\t\t</runoptions>\n\t\t<x a=\"&lt;&quot;\">&lt;Click&gt; &amp; \"q\"</x>\n\t</active>\n</setupdb>\n";

    #[test]
    fn maestro_xml_prints_back_byte_for_byte() {
        let n = parse(SDB).unwrap();
        assert_eq!(write(&n), SDB);
        assert_eq!(
            n.at(&[("active", None), ("x", None)]).unwrap().text,
            "<Click> & \"q\""
        );
        assert_eq!(
            n.at(&[("active", None), ("x", None)]).unwrap().attr("a"),
            Some("<\"")
        );
    }

    #[test]
    fn the_typed_setup_reads_corners_specs_and_run_options() {
        let s = Setup::from_xml(SDB).unwrap();
        let cold = s.corner("cold").unwrap();
        assert!(!cold.enabled);
        assert_eq!(cold.vars, [("temperature".into(), "-40".into())]);
        let sp = s.spec("T1", "v").unwrap();
        assert_eq!(
            (sp.kind.as_str(), sp.target.as_str(), sp.tol.as_str()),
            ("tol", "0.1", "10%")
        );
        assert_eq!(sp.weight.as_deref(), Some("2"));
        assert_eq!(
            s.run_mode("Monte Carlo Sampling")
                .unwrap()
                .option("mcnumpoints"),
            Some("7")
        );
    }

    #[test]
    fn malformed_xml_is_refused() {
        for bad in [
            "<a><b></a>",
            "<a>",
            "<a></a><b></b>",
            "<a><b></b>text</a>",
            "<a x=1></a>",
            "<a>&bogus;</a>",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let deep = "<a>".repeat(DEPTH_MAX + 1) + &"</a>".repeat(DEPTH_MAX + 1);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn state_tests_cut_the_test_elements() {
        let st = "<?xml version=\"1.0\"?>\n<statedb version=\"5\">active\n\t<Test Name=\"T1\">\n\t\t<c>x</c>\n\t</Test>\n\t<Test Name=\"T2\">\n\t</Test>\n</statedb>\n";
        let t = state_tests(st).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(
            t[0],
            (
                "T1".into(),
                "\t<Test Name=\"T1\">\n\t\t<c>x</c>\n\t</Test>\n".into()
            )
        );
        assert!(state_tests("\t<Test Name=\"T\">\n").is_err());
    }
}
