//! A read-only reader of SQLite 3 database files, for maestro's `<history>.rdb`: the table
//! B-trees only (no indexes, no WAL, no SQL), std only. Exact by construction: a `REAL`
//! is the file's 8 big-endian bytes, never a printed number (`maeGetOutputValue` rounds to
//! 7 digits, the `sqlite3` shell to 15).
//!
//! Format: <https://www.sqlite.org/fileformat2.html> §1.3 (header), §1.6 (B-tree pages),
//! §1.6.2 (overflow), §2.1 (records). A table's rows come back in rowid order; a column
//! declared `INTEGER PRIMARY KEY` holds the rowid (the record stores NULL there).
//!
//! The reader is total over any bytes: `Err` or rows, never a panic; every page is visited
//! at most once per table, so a cyclic or corrupt file ends.

/// One SQLite value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    /// `NULL`.
    Null,
    /// An integer.
    Int(i64),
    /// A `REAL`, bit for bit.
    Real(f64),
    /// `TEXT` (UTF-8; invalid bytes replaced).
    Text(String),
    /// A `BLOB`.
    Blob(Vec<u8>),
}

impl Val {
    /// The integer, `None` for any other type.
    pub fn int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The text, `""` for any other type.
    pub fn text(&self) -> &str {
        match self {
            Val::Text(s) => s,
            _ => "",
        }
    }
}

/// An opened database: its bytes and page size.
pub struct Db<'a> {
    bytes: &'a [u8],
    page: usize,
    usable: usize,
}

/// Opens `bytes` as a SQLite 3 file.
///
/// # Errors
///
/// Not SQLite 3, a page size it does not allow, or a text encoding other than UTF-8.
pub fn open(bytes: &[u8]) -> Result<Db<'_>, String> {
    if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
        return Err("not a SQLite 3 file".into());
    }
    let page = match u16::from_be_bytes([bytes[16], bytes[17]]) {
        1 => 65536,
        p => usize::from(p),
    };
    if !page.is_power_of_two() || !(512..=65536).contains(&page) {
        return Err(format!("page size {page}"));
    }
    let usable = page - usize::from(bytes[20]);
    if usable < 480 || be(&bytes[56..60]) > 1 {
        return Err("reserved space or a text encoding other than UTF-8".into());
    }
    Ok(Db {
        bytes,
        page,
        usable,
    })
}

impl Db<'_> {
    /// The rows of table `name` in rowid order, one value per declared column (a short
    /// record, from before an `ALTER TABLE ADD COLUMN`, padded with `NULL`).
    ///
    /// # Errors
    ///
    /// No such table, or a page or record that does not parse.
    pub fn table(&self, name: &str) -> Result<Vec<Vec<Val>>, String> {
        // sqlite_master: [rowid, type, name, tbl_name, rootpage, sql].
        let master = self.rows(1)?;
        let def = master
            .iter()
            .find(|r| r.len() >= 6 && r[1].text() == "table" && r[2].text() == name)
            .ok_or_else(|| format!("no table {name}"))?;
        let root = def[4]
            .int()
            .and_then(|p| u32::try_from(p).ok())
            .ok_or_else(|| format!("table {name}: no root page"))?;
        let (ncols, alias) = columns(def[5].text());
        let mut rows = self.rows(root)?;
        for r in &mut rows {
            let rowid = r.remove(0);
            r.resize(ncols.max(r.len()), Val::Null);
            if let Some(k) = alias
                && r[k] == Val::Null
            {
                r[k] = rowid;
            }
        }
        Ok(rows)
    }

    /// The records of the table B-tree rooted at page `root`, each with its rowid first.
    fn rows(&self, root: u32) -> Result<Vec<Vec<Val>>, String> {
        let pages = self.bytes.len() / self.page;
        let mut out = Vec::new();
        let mut todo = vec![root];
        let mut visited = 0usize;
        while let Some(p) = todo.pop() {
            visited += 1;
            if visited > pages {
                return Err("a B-tree visits more pages than the file has".into());
            }
            let base = self.page_at(p)?;
            // Page 1 starts with the 100-byte file header.
            let h = if p == 1 { base + 100 } else { base };
            let b = self.bytes;
            let kind = b[h];
            let n = usize::from(u16::from_be_bytes([b[h + 3], b[h + 4]]));
            let (hdr, interior) = match kind {
                0x0d => (8, false),
                0x05 => (12, true),
                k => return Err(format!("page {p}: kind {k:#x}, not a table page")),
            };
            let cells = h + hdr;
            if cells + 2 * n > base + self.page {
                return Err(format!("page {p}: {n} cells overflow it"));
            }
            let mut children = Vec::new();
            for k in 0..n {
                let off = usize::from(u16::from_be_bytes([b[cells + 2 * k], b[cells + 2 * k + 1]]));
                let at = base + off;
                if off < hdr || at >= base + self.page {
                    return Err(format!("page {p}: cell {k} out of the page"));
                }
                if interior {
                    children.push(be(self.get(at, 4)?));
                } else {
                    out.push(self.leaf_cell(at)?);
                }
            }
            if interior {
                children.push(be(&b[h + 8..h + 12]));
                // Left to right: the stack pops the last pushed first.
                todo.extend(children.into_iter().rev());
            }
        }
        Ok(out)
    }

    /// One table leaf cell at byte `at`: `[rowid, columns…]`.
    fn leaf_cell(&self, at: usize) -> Result<Vec<Val>, String> {
        let (len, k1) = varint(self.bytes, at)?;
        let (rowid, k2) = varint(self.bytes, at + k1)?;
        let len = usize::try_from(len).map_err(|_| "payload size")?;
        let start = at + k1 + k2;
        let u = self.usable;
        let x = u - 35;
        let payload = if len <= x {
            self.get(start, len)?.to_vec()
        } else {
            let m = (u - 12) * 32 / 255 - 23;
            let k = m + (len - m) % (u - 4);
            let local = if k <= x { k } else { m };
            let mut v = self.get(start, local)?.to_vec();
            let mut next = be(self.get(start + local, 4)?);
            let mut hops = 0usize;
            while v.len() < len {
                hops += 1;
                if next == 0 || hops > self.bytes.len() / self.page {
                    return Err("an overflow chain ends early".into());
                }
                let o = self.page_at(next)?;
                next = be(&self.bytes[o..o + 4]);
                let take = (len - v.len()).min(u - 4);
                v.extend_from_slice(self.get(o + 4, take)?);
            }
            v
        };
        let mut row = vec![Val::Int(rowid as i64)];
        row.extend(record(&payload)?);
        Ok(row)
    }

    fn page_at(&self, p: u32) -> Result<usize, String> {
        let p = p as usize;
        if p == 0 || p * self.page > self.bytes.len() {
            return Err(format!("page {p} is outside the file"));
        }
        Ok((p - 1) * self.page)
    }

    fn get(&self, at: usize, n: usize) -> Result<&[u8], String> {
        self.bytes
            .get(at..at.saturating_add(n))
            .ok_or_else(|| "a cell runs past the file".into())
    }
}

fn be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// A SQLite varint at `at`: (value, bytes used).
fn varint(b: &[u8], at: usize) -> Result<(u64, usize), String> {
    let mut v = 0u64;
    for k in 0..9 {
        let c = *b.get(at + k).ok_or("a varint runs past the file")?;
        if k == 8 {
            return Ok(((v << 8) | u64::from(c), 9));
        }
        v = (v << 7) | u64::from(c & 0x7f);
        if c & 0x80 == 0 {
            return Ok((v, k + 1));
        }
    }
    unreachable!("the ninth byte returns")
}

/// The values of one record (§2.1).
fn record(p: &[u8]) -> Result<Vec<Val>, String> {
    let (hlen, mut at) = varint(p, 0)?;
    let hlen = usize::try_from(hlen).map_err(|_| "header size")?;
    if hlen > p.len() {
        return Err("a record header runs past its payload".into());
    }
    let mut body = hlen;
    let mut out = Vec::new();
    while at < hlen {
        let (t, k) = varint(p, at)?;
        at += k;
        let n = match t {
            0 | 8 | 9 => 0,
            1..=4 => t as usize,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err("reserved serial type".into()),
            t => ((t - 12) / 2) as usize,
        };
        let d = p
            .get(body..body + n)
            .ok_or("a record value runs past its payload")?;
        body += n;
        out.push(match t {
            0 => Val::Null,
            8 => Val::Int(0),
            9 => Val::Int(1),
            1..=6 => {
                // Big-endian two's complement of n bytes.
                let mut v = i64::from(d[0] as i8);
                for &c in &d[1..] {
                    v = (v << 8) | i64::from(c);
                }
                Val::Int(v)
            }
            7 => Val::Real(f64::from_be_bytes(d.try_into().expect("8 bytes"))),
            t if t % 2 == 0 => Val::Blob(d.to_vec()),
            _ => Val::Text(String::from_utf8_lossy(d).into_owned()),
        });
    }
    Ok(out)
}

/// The column count of a `CREATE TABLE` statement and the column, if any, that is
/// `INTEGER PRIMARY KEY` (the rowid's alias). Table constraints (`UNIQUE(…)`, `PRIMARY
/// KEY(…)`) are not columns.
fn columns(sql: &str) -> (usize, Option<usize>) {
    let Some(open) = sql.find('(') else {
        return (0, None);
    };
    let body = sql[open + 1..].trim_end().trim_end_matches(')');
    let mut defs = Vec::new();
    let (mut depth, mut from) = (0i32, 0);
    for (k, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                defs.push(&body[from..k]);
                from = k + 1;
            }
            _ => {}
        }
    }
    defs.push(&body[from..]);
    let defs: Vec<String> = defs
        .iter()
        .map(|d| {
            d.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase()
        })
        .filter(|d| {
            ![
                "UNIQUE",
                "PRIMARY KEY",
                "CHECK",
                "FOREIGN KEY",
                "CONSTRAINT",
            ]
            .iter()
            .any(|c| d.starts_with(c))
        })
        .collect();
    let alias = defs.iter().position(|d| d.contains(" INTEGER PRIMARY KEY"));
    (defs.len(), alias)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_and_columns() {
        assert_eq!(varint(&[0x05], 0), Ok((5, 1)));
        assert_eq!(varint(&[0x81, 0x00], 0), Ok((128, 2)));
        assert_eq!(varint(&[0xff; 9], 0), Ok((u64::MAX, 9)));
        assert_eq!(
            columns("CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT, c NUMERIC, UNIQUE(b, c))"),
            (3, Some(0))
        );
        assert_eq!(columns("CREATE TABLE t(x, y)"), (2, None));
    }

    #[test]
    fn a_record_decodes_every_serial_type() {
        // header: 8 bytes; types NULL, i8, f64, 0, 1, text(2), blob(1)
        let mut p = vec![8, 0, 1, 7, 8, 9, 17, 14];
        p.push(0xfe);
        p.extend(1.5f64.to_be_bytes());
        p.extend(b"hi");
        p.push(0xaa);
        assert_eq!(
            record(&p).unwrap(),
            [
                Val::Null,
                Val::Int(-2),
                Val::Real(1.5),
                Val::Int(0),
                Val::Int(1),
                Val::Text("hi".into()),
                Val::Blob(vec![0xaa])
            ]
        );
    }

    #[test]
    fn garbage_is_an_error_never_a_panic() {
        assert!(open(b"nope").is_err());
        let mut b = b"SQLite format 3\0".to_vec();
        b.resize(1024, 0);
        b[16] = 2; // page 512
        let db = open(&b).unwrap();
        assert!(db.table("x").is_err());
    }
}
