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

fn archive_client(dir: &std::path::Path, report: ReportId, zip: &[u8]) -> Client {
    let client = Client::with_cache_dir(dir.join("cache"))
        .with_archive(dir)
        .offline(true);
    let path = client.archive_path(&report).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, zstd::stream::encode_all(zip, 0).unwrap()).unwrap();
    client
}

#[test]
fn archive_hit_decompresses_report_zip() {
    let dir = tempfile::tempdir().unwrap();
    let report = ReportId::cumulative(Regulator::Cftc, AssetClass::Credits, d(2025, 1, 6));
    let csv = b"synthetic archive CSV";
    let zip = common::zip(&format!("{}.csv", report.file_stem()), csv);
    let client = archive_client(dir.path(), report, &zip);
    use sha2::Digest;
    let digest = format!("{:x}", sha2::Sha256::digest(&zip));
    std::fs::write(
        dir.path().join("MANIFEST.tsv"),
        format!("{}\t{}\t{}\n", report.file_stem(), digest, zip.len()),
    )
    .unwrap();

    assert_eq!(client.csv_bytes(&report).unwrap().unwrap(), csv);
}

#[test]
fn archive_miss_falls_through_to_cache() {
    let dir = tempfile::tempdir().unwrap();
    let report = ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 1, 6));
    let csv = b"cached synthetic CSV";
    let zip = common::zip(&format!("{}.csv", report.file_stem()), csv);
    let client = Client::with_cache_dir(dir.path().join("cache"))
        .with_archive(dir.path().join("archive"))
        .offline(true);
    let path = client.cache_path(&report);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, zip).unwrap();

    assert_eq!(client.csv_bytes(&report).unwrap().unwrap(), csv);
}

#[test]
fn archive_manifest_mismatch_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let report = ReportId::cumulative(Regulator::Cftc, AssetClass::Credits, d(2025, 1, 6));
    let zip = common::zip(&format!("{}.csv", report.file_stem()), b"contents");
    let client = archive_client(dir.path(), report, &zip);
    std::fs::write(
        dir.path().join("MANIFEST.tsv"),
        format!(
            "{}\t{}\t{}\n",
            report.file_stem(),
            "00".repeat(32),
            zip.len()
        ),
    )
    .unwrap();

    assert!(matches!(
        client.zip_bytes(&report),
        Err(Error::Archive { .. })
    ));
}

#[test]
fn offline_archive_serves_reports_and_rejects_uncached_misses() {
    let dir = tempfile::tempdir().unwrap();
    let report = ReportId::cumulative(Regulator::Cftc, AssetClass::Credits, d(2025, 1, 6));
    let zip = common::zip(
        &format!("{}.csv", report.file_stem()),
        b"offline synthetic CSV",
    );
    let client = archive_client(dir.path(), report, &zip);

    assert!(client.zip_bytes(&report).unwrap().is_some());
    let absent = ReportId::cumulative(Regulator::Cftc, AssetClass::Credits, d(2025, 1, 7));
    assert!(matches!(
        client.zip_bytes(&absent),
        Err(Error::NotCached { .. })
    ));
}
