mod common;

use cftc::cds::{
    daily_series, entity_keys, flag_off_scale, index_quote_convention, on_the_run_maturity,
    roll_dates, tenor_bucket, CdsKind, CdsPrint, EntityKey, IndexQuote, QuoteConvention,
    SuspectedCause,
};
use cftc::entity::{EntityMap, Issuer};
use cftc::record::parse_csv;
use cftc::{net_trades, AssetClass, Regulator, ReportId};
use chrono::NaiveDate;
use std::collections::BTreeSet;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn prints(reg: Regulator, date: NaiveDate, rows: &[Vec<(&'static str, String)>]) -> Vec<CdsPrint> {
    let refs: Vec<Vec<(&str, &str)>> = rows.iter().map(|r| common::borrow(r)).collect();
    let refs2: Vec<&[(&str, &str)]> = refs.iter().map(Vec::as_slice).collect();
    let body = common::csv(&refs2);
    let recs = parse_csv(ReportId::cumulative(reg, AssetClass::Credits, date), &body).unwrap();
    net_trades(recs).iter().map(CdsPrint::from_trade).collect()
}

#[test]
fn fisn_classification() {
    assert_eq!(CdsKind::from_fisn("NA/CDS Corp SN Sr"), CdsKind::SingleName);
    assert_eq!(CdsKind::from_fisn("NA/CDS Sov SN Sr"), CdsKind::SingleName);
    assert_eq!(CdsKind::from_fisn("NA/CDS Corp Idx"), CdsKind::Index);
    assert_eq!(CdsKind::from_fisn("NA/CDS Idx Swt"), CdsKind::IndexSwaption);
    assert_eq!(
        CdsKind::from_fisn("NA/CDS Corp Idx Tra"),
        CdsKind::IndexTranche
    );
    assert_eq!(CdsKind::from_fisn("NA/CDS Corp Basket"), CdsKind::Basket);
    assert_eq!(
        CdsKind::from_fisn("NA/Cr Sw SN TRtn Corp Cs"),
        CdsKind::TotalReturn
    );
}

#[test]
fn roll_schedule() {
    assert_eq!(on_the_run_maturity(d(2025, 5, 1), 5), d(2030, 6, 20));
    assert_eq!(on_the_run_maturity(d(2025, 9, 20), 5), d(2030, 12, 20));
    assert_eq!(on_the_run_maturity(d(2025, 2, 1), 5), d(2029, 12, 20));
    assert_eq!(
        roll_dates(d(2025, 1, 1), d(2025, 12, 31)),
        vec![d(2025, 3, 20), d(2025, 9, 20)]
    );
}

#[test]
fn tenor_buckets() {
    assert_eq!(tenor_bucket(5.13), Some(5.0));
    assert_eq!(tenor_bucket(4.6), Some(5.0));
    assert_eq!(tenor_bucket(6.0), None);
    assert_eq!(tenor_bucket(9.8), Some(10.0));
}

#[test]
fn single_name_print_fields() {
    let date = d(2025, 5, 1);
    let ps = prints(
        Regulator::Sec,
        date,
        &[
            common::single_name(
                "1",
                "ZZ1234AB9",
                "2025-05-01T10:00:00Z",
                "2,000,000",
                "40000",
            ),
            common::single_name(
                "2",
                "ZZ1234AB9",
                "2025-05-01T11:00:00Z",
                "5,000,000+",
                "90000",
            ),
        ],
    );
    assert_eq!(ps.len(), 2);
    let p = &ps[0];
    assert_eq!(p.kind, CdsKind::SingleName);
    assert_eq!(
        p.entity,
        Some(EntityKey {
            scheme: "RED".into(),
            id: "ZZ1234".into()
        })
    );
    assert_eq!(p.coupon_bp, Some(100.0));
    assert_eq!(p.upfront_points, Some(2.0));
    assert_eq!(p.tenor_bucket, Some(5.0));
    assert!(p.on_the_run_5y);
    assert_eq!(ps[1].upfront_points, None, "capped notional -> no points");
    assert!(ps[1].notional_capped);
}

#[test]
fn entity_keys_align_sources() {
    let mut row = common::single_name("1", "", "2025-05-01T10:00:00Z", "1,000,000", "1");
    row.retain(|(k, _)| !k.starts_with("Underlier ID"));
    row.push(("Underlier ID-Leg 1", "['XS0000000001';'ZZ9999AA1']".into()));
    row.push(("Underlier ID source-Leg 1", "ISIN;REDID".into()));
    let ps = prints(Regulator::Sec, d(2025, 5, 1), &[row]);
    let keys = &ps[0].all_entity_keys;
    assert_eq!(
        keys[0],
        EntityKey {
            scheme: "RED".into(),
            id: "ZZ9999".into()
        }
    );
    assert_eq!(
        keys[1],
        EntityKey {
            scheme: "ISIN".into(),
            id: "XS0000000001".into()
        }
    );
    let _ = entity_keys;
}

fn index_row(id: &str, family: &str, spread: &str, exec: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Dissemination Identifier", id.into()),
        ("Action type", "NEWT".into()),
        ("Event type", "TRAD".into()),
        ("Execution Timestamp", exec.into()),
        ("Expiration Date", "2030-06-20".into()),
        ("Notional amount-Leg 1", "10,000,000".into()),
        ("Fixed rate-Leg 1", "0.01".into()),
        ("Spread-Leg 1", spread.into()),
        ("Spread notation-Leg 1", "3".into()),
        ("UPI FISN", "NA/CDS Corp Idx".into()),
        ("UPI Underlier Name", family.into()),
        ("Index factor", "1".into()),
    ]
}

fn same_day_index_prints(family: &str, levels: &[&str]) -> Vec<CdsPrint> {
    let rows: Vec<_> = levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            index_row(
                &format!("scale-{i}"),
                family,
                level,
                &format!("2025-05-01T{:02}:00:00Z", 10 + i),
            )
        })
        .collect();
    let refs: Vec<Vec<(&str, &str)>> = rows.iter().map(|r| common::borrow(r)).collect();
    let refs2: Vec<&[(&str, &str)]> = refs.iter().map(Vec::as_slice).collect();
    let body = common::csv(&refs2);
    let recs = parse_csv(
        ReportId::cumulative(Regulator::Cftc, AssetClass::Credits, d(2025, 5, 1)),
        &body,
    )
    .unwrap();
    net_trades(recs).iter().map(CdsPrint::from_trade).collect()
}

#[test]
fn off_scale_flags_100x_spread_and_identifies_percent_unit() {
    let mut ps = same_day_index_prints(
        "CDX.NA.IG",
        &["0.0054", "0.0055", "0.0056", "0.0055", "0.0055", "0.6746"],
    );
    flag_off_scale(&mut ps);
    assert_eq!(ps[5].level(), Some(6746.0));
    assert!(ps[5].off_scale);
    assert_eq!(ps[5].suspected_cause, Some(SuspectedCause::UnitPercent));
    assert!(ps[..5].iter().all(|p| !p.off_scale));
}

#[test]
fn off_scale_uses_notation_three_levels_for_single_names() {
    let rows: Vec<_> = ["0.0054", "0.0055", "0.0056", "0.0055", "0.0055", "0.6746"]
        .into_iter()
        .enumerate()
        .map(|(i, level)| {
            let mut row = common::single_name(
                &format!("single-{i}"),
                "ZZ1234AB9",
                &format!("2025-05-01T{:02}:00:00Z", 10 + i),
                "1,000,000",
                "1",
            );
            row.push(("Spread-Leg 1", level.into()));
            row.push(("Spread notation-Leg 1", "3".into()));
            row
        })
        .collect();
    let refs: Vec<Vec<(&str, &str)>> = rows.iter().map(|r| common::borrow(r)).collect();
    let refs2: Vec<&[(&str, &str)]> = refs.iter().map(Vec::as_slice).collect();
    let body = common::csv(&refs2);
    let recs = parse_csv(
        ReportId::cumulative(Regulator::Sec, AssetClass::Credits, d(2025, 5, 1)),
        &body,
    )
    .unwrap();
    let mut ps: Vec<_> = net_trades(recs).iter().map(CdsPrint::from_trade).collect();
    flag_off_scale(&mut ps);
    assert_eq!(ps[5].kind, CdsKind::SingleName);
    assert_eq!(ps[5].level(), Some(6746.0));
    assert!(ps[5].off_scale);
    assert_eq!(ps[5].suspected_cause, Some(SuspectedCause::UnitPercent));
}

#[test]
fn off_scale_identifies_10000x_basis_point_unit() {
    let mut ps = same_day_index_prints(
        "CDX.NA.IG",
        &["0.0054", "0.0055", "0.0056", "0.0055", "0.0055", "55.0"],
    );
    flag_off_scale(&mut ps);
    assert!(ps[5].off_scale);
    assert_eq!(ps[5].suspected_cause, Some(SuspectedCause::UnitBp));
}

#[test]
fn off_scale_flags_price_family_unit_error_but_not_three_point_move() {
    let mut bad = same_day_index_prints(
        "CDX.NA.HY",
        &["0.0100", "0.0101", "0.0099", "0.0100", "0.0100", "0.0001"],
    );
    flag_off_scale(&mut bad);
    assert!(bad[5].off_scale);
    assert_eq!(bad[5].suspected_cause, Some(SuspectedCause::UnitPercent));

    let mut normal = same_day_index_prints(
        "CDX.NA.HY",
        &["0.0100", "0.0101", "0.0099", "0.0100", "0.0100", "0.0103"],
    );
    flag_off_scale(&mut normal);
    assert!(
        !normal[5].off_scale,
        "103 points is a 3-point move from 100"
    );
}

#[test]
fn off_scale_requires_five_other_prints_and_daily_excludes_flagged_level() {
    let mut too_few = same_day_index_prints(
        "CDX.NA.IG",
        &["0.0055", "0.0055", "0.0055", "0.0055", "0.6746"],
    );
    flag_off_scale(&mut too_few);
    assert!(too_few.iter().all(|p| !p.off_scale));
    let days = BTreeSet::from([d(2025, 5, 1)]);
    let rows = daily_series(&too_few, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert!(!rows[0].validated);
    assert_eq!(rows[0].off_scale_trades, 0);

    let prints = same_day_index_prints(
        "CDX.NA.IG",
        &["0.0054", "0.0055", "0.0056", "0.0055", "0.0055", "0.6746"],
    );
    let rows = daily_series(&prints, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert!(rows[0].validated);
    assert_eq!(rows[0].trades, 6);
    assert_eq!(rows[0].off_scale_trades, 1);
    assert_eq!(rows[0].level_last, Some(55.0));
    assert_eq!(rows[0].last_level, Some(55.0));
    assert_eq!(rows[0].last_trade_date, Some(d(2025, 5, 1)));
}

#[test]
fn non_positive_and_non_finite_levels_are_flagged_and_excluded_from_daily_state() {
    let days = BTreeSet::from([d(2025, 5, 1)]);

    let mut zero_with_peers = same_day_index_prints(
        "CDX.NA.HY",
        &["0.0100", "0.0101", "0.0099", "0.0100", "0.0100", "0"],
    );
    flag_off_scale(&mut zero_with_peers);
    assert!(zero_with_peers[5].off_scale);
    assert_eq!(
        zero_with_peers[5].suspected_cause,
        Some(SuspectedCause::NonPositive)
    );
    let rows = daily_series(&zero_with_peers, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert_eq!(rows[0].off_scale_trades, 1);
    assert!((rows[0].level_min.unwrap() - 99.0).abs() < 1e-9);
    assert!((rows[0].level_max.unwrap() - 101.0).abs() < 1e-9);
    assert!((rows[0].level_last.unwrap() - 100.0).abs() < 1e-9);
    assert!((rows[0].last_level.unwrap() - 100.0).abs() < 1e-9);
    assert_eq!(rows[0].last_trade_date, Some(d(2025, 5, 1)));

    let mut thin_zero = same_day_index_prints("CDX.NA.HY", &["0"]);
    flag_off_scale(&mut thin_zero);
    assert!(thin_zero[0].off_scale);
    assert_eq!(
        thin_zero[0].suspected_cause,
        Some(SuspectedCause::NonPositive)
    );
    let rows = daily_series(&thin_zero, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert_eq!(rows[0].off_scale_trades, 1);
    assert_eq!(rows[0].level_min, None);
    assert_eq!(rows[0].level_median, None);
    assert_eq!(rows[0].level_last, None);
    assert_eq!(rows[0].last_level, None);
    assert_eq!(rows[0].last_trade_date, None);

    let mut negative_spread = same_day_index_prints("CDX.NA.IG", &["-0.0055"]);
    flag_off_scale(&mut negative_spread);
    assert!(negative_spread[0].off_scale);
    assert_eq!(
        negative_spread[0].suspected_cause,
        Some(SuspectedCause::NonPositive)
    );
    let rows = daily_series(&negative_spread, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert_eq!(rows[0].off_scale_trades, 1);
    assert_eq!(rows[0].level_min, None);
    assert_eq!(rows[0].last_level, None);
    assert_eq!(rows[0].last_trade_date, None);

    let mut nan_spread = same_day_index_prints("CDX.NA.IG", &["NaN"]);
    assert!(nan_spread[0].spread_leg1_raw.unwrap().is_nan());
    assert!(nan_spread[0].level().unwrap().is_nan());
    flag_off_scale(&mut nan_spread);
    assert!(nan_spread[0].off_scale);
    assert_eq!(
        nan_spread[0].suspected_cause,
        Some(SuspectedCause::NonPositive)
    );
    let rows = daily_series(&nan_spread, &days, d(2025, 5, 1), d(2025, 5, 1));
    assert_eq!(rows[0].off_scale_trades, 1);
    assert_eq!(rows[0].level_min, None);
    assert_eq!(rows[0].last_level, None);
    assert_eq!(rows[0].last_trade_date, None);
}

#[test]
fn index_quote_conventions() {
    let ps = prints(
        Regulator::Cftc,
        d(2025, 5, 1),
        &[
            index_row("1", "CDX.NA.IG", "0.0055", "2025-05-01T10:00:00Z"),
            index_row("2", "CDX.NA.HY", "0.0104", "2025-05-01T10:00:00Z"),
        ],
    );
    match ps[0].index_quote {
        Some(IndexQuote::SpreadBp(v)) => assert!((v - 55.0).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
    match ps[1].index_quote {
        Some(IndexQuote::Price(v)) => assert!((v - 104.0).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
}

#[test]
fn index_quote_family_table_is_explicit_and_unknown_is_unquoted() {
    let spread_families = [
        "CDX.NA.IG",
        "CDX.NA.XO",
        "ITRAXX EUROPE",
        "ITRAXX EUROPE CROSSOVER",
        "ITRAXX EUROPE SENIOR FINANCIALS",
        "ITRAXX EUROPE SUB FINANCIALS",
        "ITRAXX AUSTRALIA",
    ];
    let price_families = ["CDX.NA.HY", "CDX.EM", "ABX.HE.HE", "CMBX.NA.15"];
    for family in spread_families {
        assert_eq!(
            index_quote_convention(family),
            QuoteConvention::SpreadBp,
            "{family}"
        );
    }
    for family in price_families {
        assert_eq!(
            index_quote_convention(family),
            QuoteConvention::Price,
            "{family}"
        );
    }
    assert_eq!(
        index_quote_convention("UNSEEN INDEX FAMILY"),
        QuoteConvention::Unknown
    );
    assert_eq!(
        index_quote_convention("IBOXX EUROPE"),
        QuoteConvention::Unknown
    );
    let unknown = prints(
        Regulator::Cftc,
        d(2025, 5, 1),
        &[index_row(
            "unknown",
            "UNSEEN INDEX FAMILY",
            "0.0055",
            "2025-05-01T10:00:00Z",
        )],
    );
    assert_eq!(unknown[0].quote_convention, QuoteConvention::Unknown);
    assert_eq!(unknown[0].index_quote, None);
}

#[test]
fn daily_series_marks_staleness() {
    let mut ps = prints(
        Regulator::Cftc,
        d(2025, 5, 1),
        &[index_row(
            "1",
            "CDX.NA.IG",
            "0.0055",
            "2025-05-01T10:00:00Z",
        )],
    );
    ps.extend(prints(
        Regulator::Cftc,
        d(2025, 5, 5),
        &[
            index_row("2", "CDX.NA.IG", "0.0060", "2025-05-05T10:00:00Z"),
            index_row("3", "CDX.NA.IG", "0.0058", "2025-05-05T12:00:00Z"),
        ],
    ));
    let days: BTreeSet<NaiveDate> = [d(2025, 5, 1), d(2025, 5, 2), d(2025, 5, 5)].into();
    let rows = daily_series(&ps, &days, d(2025, 5, 1), d(2025, 5, 5));
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].as_of, d(2025, 5, 2));
    assert_eq!(rows[1].trades, 0);
    assert_eq!(rows[1].days_since_trade, Some(1));
    assert_eq!(rows[1].level_last, None);
    assert!((rows[1].last_level.unwrap() - 55.0).abs() < 1e-9);
    assert_eq!(rows[2].trades, 2);
    assert!((rows[2].level_last.unwrap() - 58.0).abs() < 1e-9);
    assert!((rows[2].level_median.unwrap() - 59.0).abs() < 1e-9);
    assert_eq!(rows[2].coupon_bp, Some(100.0));
}

#[test]
fn daily_series_clips_window_and_seeds_from_late_prints() {
    let late = prints(
        Regulator::Cftc,
        d(2025, 5, 5),
        &[index_row(
            "late",
            "CDX.NA.IG",
            "0.0061",
            "2025-05-01T10:00:00Z",
        )],
    );
    assert!(late[0].late_reported);
    assert_eq!(late[0].report_date, d(2025, 5, 5));

    let days: BTreeSet<NaiveDate> = [d(2025, 5, 1), d(2025, 5, 5), d(2025, 5, 6)].into();
    let rows = daily_series(&late, &days, d(2025, 5, 5), d(2025, 5, 6));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].as_of, d(2025, 5, 5));
    assert_eq!(rows[0].trades, 0);
    assert_eq!(rows[0].last_trade_date, Some(d(2025, 5, 1)));
    assert!((rows[0].last_level.unwrap() - 61.0).abs() < 1e-9);
    assert!(rows[0].late_reported);
    assert_eq!(rows[1].as_of, d(2025, 5, 6));
    assert!(!rows[1].late_reported);
}

#[test]
fn entity_map_resolves_tickers() {
    let m = EntityMap::from_issuers([Issuer {
        scheme: "RED".into(),
        id: "zz1234".into(),
        ticker: Some("EXMP".into()),
        lei: None,
        name: Some("Example Corp".into()),
    }]);
    let k = EntityKey {
        scheme: "RED".into(),
        id: "ZZ1234".into(),
    };
    assert_eq!(m.get(&k).and_then(|i| i.ticker.as_deref()), Some("EXMP"));
    assert_eq!(m.keys_for_ticker("exmp"), vec![k]);
}

#[test]
fn index_quote_scales_by_notation() {
    let mut decimal = index_row("1", "CDX.EM", "0.009549", "2025-05-01T10:00:00Z");
    let mut direct = index_row("2", "CDX.EM", "95.52", "2025-05-01T11:00:00Z");
    let mut unknown = index_row("3", "CDX.EM", "95.52", "2025-05-01T12:00:00Z");
    for (row, notation) in [(&mut decimal, "3"), (&mut direct, "1"), (&mut unknown, "4")] {
        row.retain(|(k, _)| *k != "Spread notation-Leg 1");
        row.push(("Spread notation-Leg 1", notation.into()));
    }
    let ps = prints(Regulator::Cftc, d(2025, 5, 1), &[decimal, direct, unknown]);
    let prices: Vec<Option<f64>> = ps
        .iter()
        .map(|p| match p.index_quote {
            Some(IndexQuote::Price(v)) => Some(v),
            _ => None,
        })
        .collect();
    assert!((prices[0].unwrap() - 95.49).abs() < 1e-9);
    assert!((prices[1].unwrap() - 95.52).abs() < 1e-9);
    assert_eq!(prices[2], None, "unknown notation is never guessed");
}

#[test]
fn cmbx_is_price_and_iboxx_is_unknown() {
    use cftc::cds::{index_quote_convention, QuoteConvention};
    assert_eq!(
        index_quote_convention("CMBX.NA.BBB-"),
        QuoteConvention::Price
    );
    assert_eq!(
        index_quote_convention("IBOXX USD LIQUID HIGH YIELD INDEX"),
        QuoteConvention::Unknown
    );
}
