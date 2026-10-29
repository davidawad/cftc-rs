//! Credit default swap views over credit records: classification, reference-entity keys,
//! index roll schedule, and raw daily aggregates.
//!
//! This module reports what was disseminated. It does not convert upfronts to par spreads or
//! derive hazard rates / default probabilities; that needs a full ISDA standard model and
//! curve inputs, which belong in a pricing library.

use crate::query::Trade;
use crate::record::{Record, UsageScope};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdsKind {
    SingleName,
    Index,
    IndexTranche,
    IndexSwaption,
    Basket,
    TotalReturn,
    Other,
}

impl CdsKind {
    /// Classify from the UPI FISN (`NA/CDS Corp SN Sr`, `NA/CDS Corp Idx`, `NA/CDS Idx Swt`, ...).
    pub fn from_fisn(fisn: &str) -> Self {
        let t: Vec<&str> = fisn.split(['/', ' ']).collect();
        let has = |w: &str| t.contains(&w);
        if has("TRtn") {
            Self::TotalReturn
        } else if has("Swt") || (has("Op") && has("Cr")) {
            Self::IndexSwaption
        } else if has("Tra") {
            Self::IndexTranche
        } else if has("Idx") || has("IdTr") {
            Self::Index
        } else if has("Basket") {
            Self::Basket
        } else if has("SN") {
            Self::SingleName
        } else {
            Self::Other
        }
    }
}

/// How a reference entity / obligation is identified on the tape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityKey {
    /// `RED` (6-char Markit RED entity code), `ISIN`, `CUSIP`, or the source label as given.
    pub scheme: String,
    pub id: String,
}

impl std::fmt::Display for EntityKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.scheme, self.id)
    }
}

/// All identifiers on a single-name record, most specific to the entity first:
/// RED entity code (first 6 chars of a RED pair code), then ISIN, CUSIP, others.
pub fn entity_keys(rec: &Record) -> Vec<EntityKey> {
    let mut keys: Vec<EntityKey> = rec
        .underlier_ids
        .iter()
        .zip(
            rec.underlier_id_sources
                .iter()
                .map(Some)
                .chain(std::iter::repeat(None)),
        )
        .filter_map(|(id, src)| {
            let src = src?.to_ascii_uppercase();
            Some(match src.as_str() {
                "REDID" | "RED" => EntityKey {
                    scheme: "RED".into(),
                    id: id.chars().take(6).collect(),
                },
                _ => EntityKey {
                    scheme: src,
                    id: id.clone(),
                },
            })
        })
        .collect();
    keys.sort_by_key(|k| match k.scheme.as_str() {
        "RED" => 0,
        "ISIN" => 1,
        "CUSIP" => 2,
        _ => 3,
    });
    keys
}

/// Index quote convention as observed on the tape: `Spread-Leg 1` holds a decimal spread for
/// IG-style indices (0.006689 = 66.89 bp) and price/10000 for price-quoted families
/// (CDX.NA.HY 0.0103715 = 103.715). Both scale by 10,000.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "convention", content = "value", rename_all = "snake_case")]
pub enum IndexQuote {
    SpreadBp(f64),
    Price(f64),
}

/// Convention used to interpret an index's disseminated `Spread-Leg 1` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteConvention {
    SpreadBp,
    Price,
    Unknown,
}

/// Likely cause assigned to an off-scale print from common unit-conversion ratios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspectedCause {
    UnitPercent,
    UnitBp,
    Outlier,
    NonPositive,
}

#[derive(Debug, Clone, Copy)]
enum FamilyMatch {
    Exact(&'static str),
    Prefix(&'static str),
}

const INDEX_QUOTE_FAMILIES: &[(FamilyMatch, QuoteConvention)] = &[
    (FamilyMatch::Exact("CDX.NA.IG"), QuoteConvention::SpreadBp),
    (FamilyMatch::Exact("CDX.NA.XO"), QuoteConvention::SpreadBp),
    (
        FamilyMatch::Exact("ITRAXX EUROPE"),
        QuoteConvention::SpreadBp,
    ),
    (
        FamilyMatch::Exact("ITRAXX EUROPE CROSSOVER"),
        QuoteConvention::SpreadBp,
    ),
    (
        FamilyMatch::Exact("ITRAXX EUROPE SENIOR FINANCIALS"),
        QuoteConvention::SpreadBp,
    ),
    (
        FamilyMatch::Exact("ITRAXX EUROPE SUB FINANCIALS"),
        QuoteConvention::SpreadBp,
    ),
    (
        FamilyMatch::Exact("ITRAXX AUSTRALIA"),
        QuoteConvention::SpreadBp,
    ),
    (FamilyMatch::Exact("CDX.NA.HY"), QuoteConvention::Price),
    (FamilyMatch::Exact("CDX.EM"), QuoteConvention::Price),
    // CMBX trades on price; the tape carries no Spread-Leg 1 for it (upfront only).
    (FamilyMatch::Prefix("CMBX."), QuoteConvention::Price),
    // iBoxx rows are zero-coupon total-return swaps with no quoted level.
    (FamilyMatch::Prefix("IBOXX"), QuoteConvention::Unknown),
    (FamilyMatch::Prefix("ABX"), QuoteConvention::Price),
];

/// Multiplier that turns a disseminated `Spread-Leg 1` into quote units (bp or price points),
/// by its spread notation. Observed on the tape: notation `3` (decimal) carries the level
/// divided by 10,000 (CDX.NA.IG 0.006689 = 66.89 bp; CDX.EM 0.009549 = price 95.49), while
/// notation `1` carries the level as is (CDX.EM 95.52). Any other or missing notation is not
/// interpreted.
pub fn notation_scale(notation: Option<&str>) -> Option<f64> {
    match notation.map(str::trim) {
        Some("3") => Some(10_000.0),
        Some("1") => Some(1.0),
        _ => None,
    }
}

pub fn index_quote_convention(family: &str) -> QuoteConvention {
    let family = family.trim();
    INDEX_QUOTE_FAMILIES
        .iter()
        .find_map(|(matcher, convention)| {
            let matches = match matcher {
                FamilyMatch::Exact(expected) => family.eq_ignore_ascii_case(expected),
                FamilyMatch::Prefix(prefix) => family
                    .get(..prefix.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix)),
            };
            matches.then_some(*convention)
        })
        .unwrap_or(QuoteConvention::Unknown)
}

/// One CDS execution, flattened for analysis. All amounts are as disseminated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdsPrint {
    pub usage: UsageScope,
    pub trade_date: NaiveDate,
    /// Date of the cumulative report containing this execution.
    pub report_date: NaiveDate,
    /// Reported more than one day after execution.
    pub late_reported: bool,
    pub executed_at: Option<DateTime<Utc>>,
    pub dissemination_id: String,
    pub corrected: bool,
    pub kind: CdsKind,
    /// Index family (`CDX.NA.IG`, `ITRAXX EUROPE`, ...) for index products.
    pub index_family: Option<String>,
    /// Preferred reference-entity key for single names.
    pub entity: Option<EntityKey>,
    pub all_entity_keys: Vec<EntityKey>,
    /// Free-text hint, usually the reference obligation (`UPI Underlier Name`).
    pub name_hint: Option<String>,
    pub upi: Option<String>,
    pub fisn: Option<String>,
    pub coupon_bp: Option<f64>,
    /// `Spread-Leg 1` exactly as disseminated.
    pub spread_leg1_raw: Option<f64>,
    pub spread_notation: Option<String>,
    pub quote_convention: QuoteConvention,
    pub index_quote: Option<IndexQuote>,
    /// The disseminated level is unusually far from same-day prints in this series.
    pub off_scale: bool,
    pub suspected_cause: Option<SuspectedCause>,
    /// Upfront cash amount (unsigned, includes accrued) and its currency.
    pub upfront_amount: Option<f64>,
    pub upfront_currency: Option<String>,
    /// Upfront in points of notional; only when the notional is not capped.
    pub upfront_points: Option<f64>,
    pub notional: Option<f64>,
    pub notional_capped: bool,
    pub notional_currency: Option<String>,
    pub index_factor: Option<f64>,
    pub effective_date: Option<NaiveDate>,
    pub maturity: Option<NaiveDate>,
    pub tenor_years: Option<f64>,
    /// Nearest standard tenor (0.5, 1, 2, 3, 4, 5, 7, 10, ... years) within half a year.
    pub tenor_bucket: Option<f64>,
    /// Maturity equals the scheduled on-the-run 5Y maturity on the trade date.
    pub on_the_run_5y: bool,
    pub cleared: Option<String>,
    pub platform: Option<String>,
    pub package: bool,
}

const TENORS: &[f64] = &[0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 10.0, 15.0, 20.0, 30.0];

pub fn tenor_bucket(years: f64) -> Option<f64> {
    TENORS
        .iter()
        .copied()
        .min_by(|a, b| (a - years).abs().total_cmp(&(b - years).abs()))
        .filter(|t| (t - years).abs() <= if *t < 1.0 { 0.25 } else { 0.5 })
}

/// Standard semiannual roll dates (20 Mar, 20 Sep) in `[from, to]`. Index series and
/// single-name on-the-run maturities roll on these dates.
pub fn roll_dates(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    (from.year()..=to.year())
        .flat_map(|y| [(y, 3), (y, 9)])
        .filter_map(|(y, m)| NaiveDate::from_ymd_opt(y, m, 20))
        .filter(|d| *d >= from && *d <= to)
        .collect()
}

/// Scheduled maturity of the on-the-run contract of `years` tenor on `date`:
/// on/after 20 Sep -> 20 Dec of year+N; on/after 20 Mar -> 20 Jun of year+N;
/// before 20 Mar -> 20 Dec of year-1+N.
pub fn on_the_run_maturity(date: NaiveDate, years: i32) -> NaiveDate {
    let y = date.year();
    let mar = NaiveDate::from_ymd_opt(y, 3, 20).expect("valid");
    let sep = NaiveDate::from_ymd_opt(y, 9, 20).expect("valid");
    let (yy, m) = if date >= sep {
        (y + years, 12)
    } else if date >= mar {
        (y + years, 6)
    } else {
        (y - 1 + years, 12)
    };
    NaiveDate::from_ymd_opt(yy, m, 20).expect("valid")
}

impl CdsPrint {
    pub fn from_trade(t: &Trade) -> Self {
        let r = &t.record;
        let kind = r
            .upi_fisn
            .as_deref()
            .map(CdsKind::from_fisn)
            .unwrap_or(CdsKind::Other);
        let is_index = matches!(
            kind,
            CdsKind::Index | CdsKind::IndexTranche | CdsKind::IndexSwaption
        );
        let index_family = is_index.then(|| r.upi_underlier_name.clone()).flatten();
        let keys = if kind == CdsKind::SingleName {
            entity_keys(r)
        } else {
            Vec::new()
        };
        let trade_date = r.trade_date();
        let report_date = r.report.date;
        let tenor_years = r
            .expiration_date
            .map(|m| (m - trade_date).num_days() as f64 / 365.25);
        let notional = r.notional_leg1;
        let upfront_amount = r.upfront_amount();
        let quote_convention = index_family
            .as_deref()
            .map(index_quote_convention)
            .unwrap_or(QuoteConvention::Unknown);
        let index_quote = match (
            kind,
            r.spread_leg1
                .and_then(|v| notation_scale(r.spread_notation_leg1.as_deref()).map(|k| v * k)),
        ) {
            (CdsKind::Index, Some(v)) => match quote_convention {
                QuoteConvention::SpreadBp => Some(IndexQuote::SpreadBp(v)),
                QuoteConvention::Price => Some(IndexQuote::Price(v)),
                QuoteConvention::Unknown => None,
            },
            _ => None,
        };
        CdsPrint {
            usage: UsageScope::InternalOnly,
            trade_date,
            report_date,
            late_reported: (report_date - trade_date).num_days() > 1,
            executed_at: r.execution_timestamp,
            dissemination_id: t.original_id.clone(),
            corrected: t.corrected,
            kind,
            index_family,
            entity: keys.first().cloned(),
            all_entity_keys: keys,
            name_hint: r
                .upi_underlier_name
                .clone()
                .filter(|n| !n.eq_ignore_ascii_case("No name obtainable"))
                .or_else(|| r.underlying_asset_name.clone()),
            upi: r.upi.clone(),
            fisn: r.upi_fisn.clone(),
            coupon_bp: r.fixed_rate_leg1.map(|c| c * 10_000.0),
            spread_leg1_raw: r.spread_leg1,
            spread_notation: r.spread_notation_leg1.clone(),
            quote_convention,
            index_quote,
            off_scale: false,
            suspected_cause: None,
            upfront_amount,
            upfront_currency: r.other_payment_currency.clone(),
            upfront_points: match (upfront_amount, notional) {
                (Some(a), Some(n)) if !n.capped && n.amount > 0.0 => Some(a / n.amount * 100.0),
                _ => None,
            },
            notional: notional.map(|n| n.amount),
            notional_capped: notional.is_some_and(|n| n.capped),
            notional_currency: r.notional_currency_leg1.clone(),
            index_factor: r.index_factor,
            effective_date: r.effective_date,
            maturity: r.expiration_date,
            tenor_years,
            tenor_bucket: tenor_years.and_then(tenor_bucket),
            on_the_run_5y: r.expiration_date == Some(on_the_run_maturity(trade_date, 5)),
            cleared: r.cleared.clone(),
            platform: r.platform.clone(),
            package: r.package,
        }
    }

    /// Series key: index family + maturity, or single-name entity + tenor bucket.
    pub fn series_key(&self) -> Option<String> {
        match (&self.index_family, &self.entity) {
            (Some(f), _) => Some(format!(
                "{f}|{}",
                self.maturity.map(|m| m.to_string()).unwrap_or_default()
            )),
            (None, Some(e)) => Some(format!(
                "{e}|{}y",
                self.tenor_bucket
                    .map(|t| t.to_string())
                    .unwrap_or("?".into())
            )),
            _ => None,
        }
    }

    /// The single quoted level used for daily aggregates: index quote, else raw spread in bp.
    pub fn level(&self) -> Option<f64> {
        match self.index_quote {
            Some(IndexQuote::SpreadBp(v) | IndexQuote::Price(v)) => Some(v),
            None => self
                .spread_leg1_raw
                .filter(|_| self.spread_notation.as_deref() == Some("3"))
                .map(|s| s * 10_000.0),
        }
    }
}

/// One row of a daily series. Days without trades still get a row (zero trades) so a stale
/// series reads as stale: compare `as_of` with `last_trade_date`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyRow {
    pub usage: UsageScope,
    pub as_of: NaiveDate,
    pub series: String,
    pub trades: usize,
    /// Number of prints excluded from the level aggregates as off-scale.
    pub off_scale_trades: usize,
    /// Whether this series/day had enough other quoted prints to check for off-scale values.
    pub validated: bool,
    /// At least one execution reported on `as_of` was more than one day late.
    pub late_reported: bool,
    pub capped_trades: usize,
    /// Sum of disseminated notionals; a lower bound when `capped_trades > 0`.
    pub notional_lower_bound: f64,
    pub last_trade_date: Option<NaiveDate>,
    pub days_since_trade: Option<i64>,
    /// Quoted level of the day's trades (bp for spreads, points for price-quoted indices).
    pub level_last: Option<f64>,
    pub level_median: Option<f64>,
    pub level_min: Option<f64>,
    pub level_max: Option<f64>,
    pub coupon_bp: Option<f64>,
    pub upfront_points_median: Option<f64>,
    /// Carried from the last day that traded; `None` before the first trade in the window.
    pub last_level: Option<f64>,
    pub name_hint: Option<String>,
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

fn close_within_25_percent(value: f64, reference: f64) -> bool {
    reference > 0.0 && value.is_finite() && (value - reference).abs() <= reference * 0.25
}

fn is_off_scale(print: &CdsPrint, reference: f64, level: f64) -> bool {
    if !reference.is_finite() || reference <= 0.0 || !level.is_finite() || level <= 0.0 {
        return false;
    }
    match print.quote_convention {
        QuoteConvention::SpreadBp => level >= reference * 10.0 || level <= reference / 10.0,
        QuoteConvention::Price => level < reference / 1.5 || level > reference * 1.5,
        QuoteConvention::Unknown if print.kind == CdsKind::SingleName => {
            level >= reference * 10.0 || level <= reference / 10.0
        }
        QuoteConvention::Unknown => false,
    }
}

fn suspected_cause(level: f64, reference: f64) -> SuspectedCause {
    if [level * 100.0, level / 100.0]
        .into_iter()
        .any(|candidate| close_within_25_percent(candidate, reference))
    {
        SuspectedCause::UnitPercent
    } else if [level * 10_000.0, level / 10_000.0]
        .into_iter()
        .any(|candidate| close_within_25_percent(candidate, reference))
    {
        SuspectedCause::UnitBp
    } else {
        SuspectedCause::Outlier
    }
}

/// Flag levels that are far from the median of at least five other same-day prints in the same
/// series. Values are never changed; prints without enough peers remain unflagged.
pub fn flag_off_scale(prints: &mut [CdsPrint]) {
    let mut groups: BTreeMap<(String, NaiveDate), Vec<usize>> = BTreeMap::new();
    for (index, print) in prints.iter_mut().enumerate() {
        print.off_scale = false;
        print.suspected_cause = None;
        if let Some(level) = print.level() {
            if !level.is_finite() || level <= 0.0 {
                print.off_scale = true;
                print.suspected_cause = Some(SuspectedCause::NonPositive);
            } else if let Some(series) = print.series_key() {
                groups
                    .entry((series, print.trade_date))
                    .or_default()
                    .push(index);
            }
        }
    }

    for indices in groups.values() {
        for &index in indices {
            let peers: Vec<f64> = indices
                .iter()
                .copied()
                .filter(|&peer| peer != index)
                .filter_map(|peer| prints[peer].level())
                .filter(|level| level.is_finite() && *level > 0.0)
                .collect();
            if peers.len() < 5 {
                continue;
            }
            let Some(reference) = median(peers) else {
                continue;
            };
            if let Some(level) = prints[index].level() {
                if is_off_scale(&prints[index], reference, level) {
                    prints[index].off_scale = true;
                    prints[index].suspected_cause = Some(suspected_cause(level, reference));
                }
            }
        }
    }
}

/// Aggregate prints into one row per (series, report day). `days` are the dates that had a
/// report file; pass them so non-trading days are not mistaken for stale ones.
pub fn daily_series(
    prints: &[CdsPrint],
    days: &BTreeSet<NaiveDate>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<DailyRow> {
    let mut flagged_prints = prints.to_vec();
    flag_off_scale(&mut flagged_prints);
    let mut by_key: BTreeMap<String, BTreeMap<NaiveDate, Vec<&CdsPrint>>> = BTreeMap::new();
    for p in &flagged_prints {
        if let Some(k) = p.series_key() {
            by_key
                .entry(k)
                .or_default()
                .entry(p.trade_date)
                .or_default()
                .push(p);
        }
    }
    by_key
        .into_iter()
        .flat_map(|(series, by_day)| {
            let name_hint = by_day.values().flatten().find_map(|p| p.name_hint.clone());
            let mut last_trade: Option<NaiveDate> = None;
            let mut last_level: Option<f64> = None;
            // Pre-window executions provide the opening state, but never create output rows.
            for (date, ps) in by_day.range(..from) {
                let mut sorted: Vec<&CdsPrint> = ps.to_vec();
                sorted.sort_by_key(|p| p.executed_at);
                if let Some(level) = sorted
                    .iter()
                    .filter(|p| !p.off_scale)
                    .filter_map(|p| p.level())
                    .next_back()
                {
                    last_level = Some(level);
                }
                if sorted.iter().any(|p| !p.off_scale) {
                    last_trade = Some(*date);
                }
            }
            let all_days: BTreeSet<NaiveDate> = days
                .range(from..=to)
                .chain(by_day.range(from..=to).map(|(date, _)| date))
                .copied()
                .collect();
            all_days
                .into_iter()
                .map(|d| {
                    let ps = by_day.get(&d).map(Vec::as_slice).unwrap_or(&[]);
                    let late_reported = by_day
                        .values()
                        .flatten()
                        .any(|p| p.report_date == d && p.late_reported);
                    let mut sorted: Vec<&CdsPrint> = ps.to_vec();
                    sorted.sort_by_key(|p| p.executed_at);
                    let levels: Vec<f64> = sorted
                        .iter()
                        .filter(|p| !p.off_scale)
                        .filter_map(|p| p.level())
                        .collect();
                    if sorted.iter().any(|p| !p.off_scale) {
                        last_trade = Some(d);
                    }
                    if let Some(l) = levels.last() {
                        last_level = Some(*l);
                    }
                    let mut coupons: HashMap<i64, usize> = HashMap::new();
                    ps.iter()
                        .filter_map(|p| p.coupon_bp)
                        .for_each(|c| *coupons.entry(c.round() as i64).or_default() += 1);
                    DailyRow {
                        usage: UsageScope::InternalOnly,
                        as_of: d,
                        series: series.clone(),
                        trades: ps.len(),
                        off_scale_trades: ps.iter().filter(|p| p.off_scale).count(),
                        validated: ps.iter().filter(|p| p.level().is_some()).count() >= 6,
                        late_reported,
                        capped_trades: ps.iter().filter(|p| p.notional_capped).count(),
                        notional_lower_bound: ps.iter().filter_map(|p| p.notional).sum(),
                        last_trade_date: last_trade,
                        days_since_trade: last_trade.map(|t| (d - t).num_days()),
                        level_last: levels.last().copied(),
                        level_median: median(levels.clone()),
                        level_min: levels.iter().copied().reduce(f64::min),
                        level_max: levels.iter().copied().reduce(f64::max),
                        coupon_bp: coupons
                            .into_iter()
                            .max_by_key(|(c, n)| (*n, -*c))
                            .map(|(c, _)| c as f64),
                        upfront_points_median: median(
                            ps.iter().filter_map(|p| p.upfront_points).collect(),
                        ),
                        last_level,
                        name_hint: name_hint.clone(),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
