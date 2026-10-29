//! Synthetic report builder. Uses the real 110-column header (column names only, from the
//! public CFTC technical specification); every value is invented.

use std::collections::HashMap;
use std::io::Write;

pub const HEADER: &str = include_str!("../fixtures/header.csv");

pub fn columns() -> Vec<String> {
    let mut rdr = csv::Reader::from_reader(HEADER.as_bytes());
    rdr.headers().unwrap().iter().map(str::to_string).collect()
}

/// Build a CSV with the full header; each row sets only the given columns.
pub fn csv(rows: &[&[(&str, &str)]]) -> Vec<u8> {
    let cols = columns();
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(&cols).unwrap();
    for row in rows {
        let m: HashMap<&str, &str> = row.iter().copied().collect();
        for k in m.keys() {
            assert!(cols.iter().any(|c| c == k), "unknown column {k}");
        }
        w.write_record(
            cols.iter()
                .map(|c| m.get(c.as_str()).copied().unwrap_or("")),
        )
        .unwrap();
    }
    w.into_inner().unwrap()
}

#[allow(dead_code)]
pub fn zip(name: &str, body: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(body).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

pub fn single_name(
    id: &str,
    red: &str,
    exec: &str,
    notional: &str,
    upfront: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("Dissemination Identifier", id.into()),
        ("Action type", "NEWT".into()),
        ("Event type", "TRAD".into()),
        ("Asset Class", "CR".into()),
        ("Execution Timestamp", exec.into()),
        ("Event timestamp", exec.into()),
        ("Effective Date", exec[..10].into()),
        ("Expiration Date", "2030-06-20".into()),
        ("Notional amount-Leg 1", notional.into()),
        ("Notional currency-Leg 1", "USD".into()),
        ("Fixed rate-Leg 1", "0.01".into()),
        ("Other payment amount", upfront.into()),
        ("Other payment type", "UFRO".into()),
        ("Other payment currency", "USD".into()),
        ("Underlier ID-Leg 1", red.into()),
        ("Underlier ID source-Leg 1", "REDID".into()),
        ("UPI FISN", "NA/CDS Corp SN Sr".into()),
        ("UPI Underlier Name", "USD 4,5 EXAMPLE CORP 25-2031".into()),
        ("Platform identifier", "BILT".into()),
    ]
}

pub fn borrow<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    v.iter().map(|(k, s)| (*k, s.as_str())).collect()
}
