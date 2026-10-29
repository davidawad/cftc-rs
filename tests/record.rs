mod common;

use cftc::record::{parse_csv, split_list};
use cftc::{
    net_trades, ActionType, AssetClass, EventType, Notional, Regulator, ReportId, UsageScope,
};
use chrono::NaiveDate;

fn rid() -> ReportId {
    ReportId::cumulative(
        Regulator::Sec,
        AssetClass::Credits,
        NaiveDate::from_ymd_opt(2030, 1, 2).unwrap(),
    )
}

#[test]
fn header_has_110_columns() {
    assert_eq!(common::columns().len(), 110);
}

#[test]
fn notional_parses_caps_and_commas() {
    assert_eq!(
        Notional::parse("5,000,000+"),
        Some(Notional {
            amount: 5e6,
            capped: true
        })
    );
    assert_eq!(
        Notional::parse("1,250,000"),
        Some(Notional {
            amount: 1.25e6,
            capped: false
        })
    );
    assert_eq!(Notional::parse(""), None);
}

#[test]
fn split_list_handles_bracketed_and_plain() {
    assert_eq!(split_list("['AAA';'BBB']"), vec!["AAA", "BBB"]);
    assert_eq!(split_list("ISIN;REDID"), vec!["ISIN", "REDID"]);
    assert_eq!(split_list("X1"), vec!["X1"]);
    assert!(split_list("").is_empty());
}

#[test]
fn parses_typed_fields_and_keeps_other_columns() {
    let mut row = common::single_name(
        "1001",
        "ZZ1234AB9",
        "2030-01-02T14:00:00Z",
        "2,000,000",
        "12345.5",
    );
    row.push(("Cleared", "N".into()));
    row.push(("Fixed rate day count convention-leg 1", "A004".into()));
    let body = common::csv(&[&common::borrow(&row)]);
    let recs = parse_csv(rid(), &body).unwrap();
    assert_eq!(recs.len(), 1);
    let r = &recs[0];
    assert_eq!(r.usage, UsageScope::InternalOnly);
    assert_eq!(r.action, ActionType::Newt);
    assert_eq!(r.event, Some(EventType::Trad));
    assert!(r.is_new_trade());
    assert_eq!(
        r.notional_leg1,
        Some(Notional {
            amount: 2e6,
            capped: false
        })
    );
    assert_eq!(r.fixed_rate_leg1, Some(0.01));
    assert_eq!(r.upfront_amount(), Some(12345.5));
    assert_eq!(r.underlier_ids, vec!["ZZ1234AB9"]);
    assert_eq!(r.expiration_date, NaiveDate::from_ymd_opt(2030, 6, 20));
    assert_eq!(
        r.other
            .get("Fixed rate day count convention-leg 1")
            .map(String::as_str),
        Some("A004")
    );
    assert!(
        !r.other.contains_key("Cleared"),
        "typed columns are not duplicated"
    );
}

#[test]
fn missing_columns_read_as_empty() {
    let body = b"Dissemination Identifier,Action type\n7,TERM\n";
    let recs = parse_csv(rid(), body).unwrap();
    assert_eq!(recs[0].action, ActionType::Term);
    assert_eq!(recs[0].notional_leg1, None);
}

#[test]
fn ticker_json_maps_headers_aliases_and_preserves_unknown_keys() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ticker.json")).unwrap();
    let rows = value["tradeList"].as_array().unwrap();
    let report = rid();
    let first = cftc::record::from_ticker_json(report, &rows[0]).unwrap();
    assert_eq!(first.action, ActionType::Newt);
    assert_eq!(first.notional_leg1.unwrap().amount, 2_500_000.0);
    assert_eq!(first.upi_fisn.as_deref(), Some("NA/CDS Corp Idx"));
    assert_eq!(
        cftc::cds::CdsKind::from_fisn(first.upi_fisn.as_deref().unwrap()),
        cftc::cds::CdsKind::Index
    );
    assert_eq!(
        first
            .other
            .get("disseminationTimestamp")
            .map(String::as_str),
        Some("2026-10-28T12:01:00Z")
    );
    assert_eq!(
        first.other.get("callAmountLeg2").map(String::as_str),
        Some("125000")
    );
    let second = cftc::record::from_ticker_json(report, &rows[1]).unwrap();
    assert!(second.notional_leg1.unwrap().capped);
    assert_eq!(second.upi_fisn.as_deref(), Some("NA/CDS Corp SN Sr"));
}

#[test]
fn netting_applies_corrections_and_errors() {
    let a = common::single_name("1", "ZZ1234AB9", "2030-01-02T10:00:00Z", "1,000,000", "100");
    let b = common::single_name("2", "ZZ1234AB9", "2030-01-02T11:00:00Z", "1,000,000", "200");
    let corr = vec![
        ("Dissemination Identifier", "3".to_string()),
        ("Original Dissemination Identifier", "1".into()),
        ("Action type", "CORR".into()),
        ("Fixed rate-Leg 1", "0.05".into()),
    ];
    let corr2 = vec![
        ("Dissemination Identifier", "4".to_string()),
        ("Original Dissemination Identifier", "3".into()),
        ("Action type", "CORR".into()),
        ("Fixed rate-Leg 1", "0.01".into()),
    ];
    let eror = vec![
        ("Dissemination Identifier", "5".to_string()),
        ("Original Dissemination Identifier", "2".into()),
        ("Action type", "EROR".into()),
    ];
    let body = common::csv(&[
        &common::borrow(&a),
        &common::borrow(&b),
        &common::borrow(&corr),
        &common::borrow(&corr2),
        &common::borrow(&eror),
    ]);
    let trades = net_trades(parse_csv(rid(), &body).unwrap());
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].original_id, "1");
    assert!(trades[0].corrected);
    assert_eq!(trades[0].record.dissemination_id, "4");
    assert_eq!(trades[0].record.fixed_rate_leg1, Some(0.01));
}

#[test]
fn packaged_header_list_matches_fixture() {
    assert_eq!(
        include_str!("../src/header.csv"),
        include_str!("fixtures/header.csv")
    );
}
