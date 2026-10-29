mod common;

use cftc::{AssetClass, Client, Error, Query, Regulator, ReportId};
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn urls_follow_dtcc_grammar() {
    let r = ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 5, 1));
    assert_eq!(
        r.url(cftc::source::DEFAULT_BASE_URL),
        "https://pddata.dtcc.com/ppd/api/report/cumulative/sec/SEC_CUMULATIVE_CREDITS_2025_05_01.zip"
    );
    let r = ReportId::cumulative(Regulator::Cftc, AssetClass::Rates, d(2026, 10, 16));
    assert_eq!(r.file_stem(), "CFTC_CUMULATIVE_RATES_2026_10_16");
    let slice = ReportId::slice(Regulator::Sec, AssetClass::Credits, d(2025, 5, 1), 2);
    assert_eq!(
        slice.url(cftc::source::DEFAULT_BASE_URL),
        "https://pddata.dtcc.com/ppd/api/report/intraday/sec/SEC_SLICE_CREDITS_2025_05_01_2.zip"
    );
}

#[test]
fn offline_client_reads_cache_and_skips_missing_days() {
    let dir = tempfile::tempdir().unwrap();
    let client = Client::with_cache_dir(dir.path()).offline(true);
    let have = ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 5, 1));
    let row = common::single_name("1", "ZZ1234AB9", "2025-05-01T10:00:00Z", "1,000,000", "10");
    let body = common::csv(&[&common::borrow(&row)]);
    let path = client.cache_path(&have);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        common::zip("SEC_CUMULATIVE_CREDITS_2025_05_01.csv", &body),
    )
    .unwrap();
    // A known-missing day (weekend/holiday) is remembered with a marker file.
    let gone = ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 5, 2));
    std::fs::write(client.cache_path(&gone).with_extension("missing"), b"").unwrap();

    let q = Query::new(
        Regulator::Sec,
        AssetClass::Credits,
        d(2025, 5, 1),
        d(2025, 5, 2),
    );
    let recs = client.records(&q).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].report, have);

    let unknown = ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 5, 3));
    assert!(matches!(client.day(unknown), Err(Error::NotCached { .. })));
}

#[test]
fn offline_slice_probe_reads_cached_sequences_and_stops_at_missing_marker() {
    let dir = tempfile::tempdir().unwrap();
    let client = Client::with_cache_dir(dir.path()).offline(true);
    let date = d(2025, 5, 1);
    for n in 1..=2 {
        let report = ReportId::slice(Regulator::Sec, AssetClass::Credits, date, n);
        let row = common::single_name(
            &format!("slice-{n}"),
            "ZZ1234AB9",
            "2025-05-01T10:00:00Z",
            "1,000,000",
            "10",
        );
        let body = common::csv(&[&common::borrow(&row)]);
        let path = client.cache_path(&report);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            common::zip(&format!("{}.csv", report.file_stem()), &body),
        )
        .unwrap();
    }
    let missing = ReportId::slice(Regulator::Sec, AssetClass::Credits, date, 3);
    std::fs::write(client.cache_path(&missing).with_extension("missing"), b"").unwrap();

    let records = client
        .slices(Regulator::Sec, AssetClass::Credits, date)
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].dissemination_id, "slice-1");
    assert_eq!(records[1].dissemination_id, "slice-2");
}
