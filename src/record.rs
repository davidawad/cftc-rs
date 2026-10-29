//! Typed rows of a PPD report.
//!
//! The SEC and CFTC credit feeds share one 110-column header (CFTC Parts 43/45 technical
//! specification v3.x / CPMI-IOSCO CDE names). Columns are looked up by header name, so a
//! file with reordered or missing columns still parses; a missing column reads as empty.
//! Columns this module does not type are kept, non-empty only, in [`Record::other`].

use crate::error::{Error, Result};
use crate::source::ReportId;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// DTCC terms permit internal or personal use only; nothing derived from this feed may be
/// redistributed. Every record and aggregate carries this marker so downstream gates can
/// enforce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    #[default]
    InternalOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionType {
    /// New transaction.
    Newt,
    /// Modification of terms (lifecycle event).
    Modi,
    /// Correction of a previously disseminated record.
    Corr,
    /// Termination.
    Term,
    /// Error: the original record should not have been disseminated.
    Eror,
    /// Revival of a record previously cancelled with EROR.
    Revi,
    /// Position-level valuation / collateral or anything unlisted.
    Other(String),
}

impl ActionType {
    fn parse(s: &str) -> Self {
        match s {
            "NEWT" => Self::Newt,
            "MODI" => Self::Modi,
            "CORR" => Self::Corr,
            "TERM" => Self::Term,
            "EROR" => Self::Eror,
            "REVI" => Self::Revi,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// Trade: a price-forming execution.
    Trad,
    /// Novation.
    Nova,
    /// Early termination.
    Etrm,
    /// Clearing.
    Clrg,
    /// Compression.
    Comp,
    /// Exercise.
    Exer,
    Other(String),
}

impl EventType {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "" => return None,
            "TRAD" => Self::Trad,
            "NOVA" => Self::Nova,
            "ETRM" => Self::Etrm,
            "CLRG" => Self::Clrg,
            "COMP" => Self::Comp,
            "EXER" => Self::Exer,
            other => Self::Other(other.to_string()),
        })
    }
}

/// A notional as disseminated. Notionals above the public dissemination cap are shown as
/// the cap with a trailing `+` (e.g. `5,000,000+`), so `amount` is then a lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Notional {
    pub amount: f64,
    pub capped: bool,
}

impl Notional {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let capped = s.ends_with('+');
        let digits: String = s
            .trim_end_matches('+')
            .chars()
            .filter(|c| *c != ',')
            .collect();
        digits
            .parse()
            .ok()
            .map(|amount| Notional { amount, capped })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub report: ReportId,
    pub usage: UsageScope,
    pub dissemination_id: String,
    pub original_dissemination_id: Option<String>,
    pub action: ActionType,
    pub event: Option<EventType>,
    pub event_timestamp: Option<DateTime<Utc>>,
    pub execution_timestamp: Option<DateTime<Utc>>,
    pub effective_date: Option<NaiveDate>,
    pub expiration_date: Option<NaiveDate>,
    pub asset_class: String,
    /// `Y` cleared, `N` uncleared, `I` intent to clear.
    pub cleared: Option<String>,
    /// Execution venue MIC / SEF code; `BILT` = bilateral.
    pub platform: Option<String>,
    pub notional_leg1: Option<Notional>,
    pub notional_currency_leg1: Option<String>,
    /// Decimal fraction, e.g. `0.01` = 100 bp running coupon.
    pub fixed_rate_leg1: Option<f64>,
    /// Decimal value as disseminated; see [`Record::spread_notation_leg1`].
    pub spread_leg1: Option<f64>,
    /// Spread notation code as disseminated (`3` = decimal is the common one).
    pub spread_notation_leg1: Option<String>,
    pub price: Option<f64>,
    pub price_notation: Option<String>,
    /// Upfront / other payment amounts in currency units (`;`-separated in the source).
    pub other_payment_amounts: Vec<f64>,
    /// `UFRO` upfront, `UWIN` unwind, ... (`;`-separated in the source).
    pub other_payment_types: Vec<String>,
    pub other_payment_currency: Option<String>,
    pub index_factor: Option<f64>,
    /// Underlier identifiers (RED codes, ISINs, CUSIPs, UPIs).
    pub underlier_ids: Vec<String>,
    /// Identifier sources aligned with `underlier_ids` where given (`REDID`, `ISIN`, ...).
    pub underlier_id_sources: Vec<String>,
    pub underlying_asset_name: Option<String>,
    pub upi: Option<String>,
    /// ISO 18774 short name of the UPI, e.g. `NA/CDS Corp SN Sr`, `NA/CDS Corp Idx`.
    pub upi_fisn: Option<String>,
    /// e.g. `CDX.NA.IG`, `ITRAXX EUROPE`, or reference-obligation text for single names.
    pub upi_underlier_name: Option<String>,
    pub package: bool,
    pub non_standard_term: bool,
    pub block_trade: bool,
    /// Every other non-empty column, keyed by its header.
    pub other: BTreeMap<String, String>,
}

const TYPED: &[&str] = &[
    "Dissemination Identifier",
    "Original Dissemination Identifier",
    "Action type",
    "Event type",
    "Event timestamp",
    "Execution Timestamp",
    "Effective Date",
    "Expiration Date",
    "Asset Class",
    "Cleared",
    "Platform identifier",
    "Notional amount-Leg 1",
    "Notional currency-Leg 1",
    "Fixed rate-Leg 1",
    "Spread-Leg 1",
    "Spread notation-Leg 1",
    "Price",
    "Price notation",
    "Other payment amount",
    "Other payment type",
    "Other payment currency",
    "Index factor",
    "Underlier ID-Leg 1",
    "Underlier ID source-Leg 1",
    "Underlying Asset Name",
    "Unique Product Identifier",
    "UPI FISN",
    "UPI Underlier Name",
    "Package indicator",
    "Non-standardized term indicator",
    "Block trade election indicator",
];

struct Row<'a> {
    idx: &'a HashMap<&'a str, usize>,
    rec: &'a csv::StringRecord,
}

impl Row<'_> {
    fn get(&self, col: &str) -> &str {
        self.idx
            .get(col)
            .and_then(|&i| self.rec.get(i))
            .map(str::trim)
            .unwrap_or("")
    }
    fn opt(&self, col: &str) -> Option<String> {
        Some(self.get(col))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    fn f64(&self, col: &str) -> Option<f64> {
        let s: String = self.get(col).chars().filter(|c| *c != ',').collect();
        s.parse().ok()
    }
    fn date(&self, col: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.get(col), "%Y-%m-%d").ok()
    }
    fn ts(&self, col: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(self.get(col))
            .ok()
            .map(|t| t.with_timezone(&Utc))
    }
    fn flag(&self, col: &str) -> bool {
        matches!(self.get(col).to_ascii_lowercase().as_str(), "true" | "y")
    }
    fn list(&self, col: &str) -> Vec<String> {
        split_list(self.get(col))
    }
}

/// `['EZ03BNZL0JQ9';'QZDN99PRQFRT']`, `ISIN;REDID` or `GPGFFQAE4` -> items.
pub fn split_list(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(';')
        .map(|p| p.trim().trim_matches('\'').trim())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse a whole report CSV.
pub fn parse_csv(report: ReportId, csv_bytes: &[u8]) -> Result<Vec<Record>> {
    let cerr = |source| Error::Csv {
        context: report.file_stem(),
        source,
    };
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_bytes);
    let headers = rdr.headers().map_err(cerr)?.clone();
    let idx: HashMap<&str, usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim(), i))
        .collect();
    rdr.records()
        .map(|rec| {
            rec.map_err(cerr).map(|rec| {
                from_row(
                    report,
                    &headers,
                    &Row {
                        idx: &idx,
                        rec: &rec,
                    },
                )
            })
        })
        .collect()
}

/// Convert one DTCC ticker trade object to the same typed record used for report CSV rows.
pub fn from_ticker_json(report: ReportId, value: &serde_json::Value) -> Result<Record> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Ticker("tradeList entry is not an object".into()))?;
    let mut columns = BTreeMap::<String, String>::new();
    for (key, value) in object {
        let header = ticker_header(key).unwrap_or_else(|| key.clone());
        columns.insert(header, ticker_cell(value));
    }
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(columns.keys())
        .map_err(|e| Error::Ticker(e.to_string()))?;
    writer
        .write_record(columns.values())
        .map_err(|e| Error::Ticker(e.to_string()))?;
    let bytes = writer
        .into_inner()
        .map_err(|e| Error::Ticker(e.to_string()))?;
    parse_csv(report, &bytes)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Ticker("tradeList entry produced no record".into()))
}

fn ticker_cell(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn ticker_header(key: &str) -> Option<String> {
    let alias = match key {
        "uniqueProductIdentifierShortName" => "UPI FISN",
        "uniqueProductIdentifierUnderlierName" => "UPI Underlier Name",
        "callAmountLeg1" => "Call amount",
        "callCurrencyLeg1" => "Call currency",
        "putAmountLeg1" => "Put amount",
        "putCurrencyLeg1" => "Put currency",
        "settlementLocationLeg1" => "Settlement location",
        "strikePriceCurrencyOrCurrencypair" => "Strike price currency/currency pair",
        _ => return report_headers().get(&normalize_ticker_key(key)).cloned(),
    };
    Some(alias.to_string())
}

/// The DTCC report column names (src/header.csv), keyed by their normalized form.
fn report_headers() -> &'static HashMap<String, String> {
    static HEADERS: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    HEADERS.get_or_init(|| {
        let mut reader = csv::Reader::from_reader(include_str!("header.csv").as_bytes());
        reader
            .headers()
            .map(|h| {
                h.iter()
                    .map(|name| (normalize_ticker_key(name), name.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn normalize_ticker_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn from_row(report: ReportId, headers: &csv::StringRecord, r: &Row) -> Record {
    let other = headers
        .iter()
        .map(str::trim)
        .filter(|h| !TYPED.contains(h))
        .filter_map(|h| r.opt(h).map(|v| (h.to_string(), v)))
        .collect();
    Record {
        report,
        usage: UsageScope::InternalOnly,
        dissemination_id: r.get("Dissemination Identifier").to_string(),
        original_dissemination_id: r.opt("Original Dissemination Identifier"),
        action: ActionType::parse(r.get("Action type")),
        event: EventType::parse(r.get("Event type")),
        event_timestamp: r.ts("Event timestamp"),
        execution_timestamp: r.ts("Execution Timestamp"),
        effective_date: r.date("Effective Date"),
        expiration_date: r.date("Expiration Date"),
        asset_class: r.get("Asset Class").to_string(),
        cleared: r.opt("Cleared"),
        platform: r.opt("Platform identifier"),
        notional_leg1: Notional::parse(r.get("Notional amount-Leg 1")),
        notional_currency_leg1: r.opt("Notional currency-Leg 1"),
        fixed_rate_leg1: r.f64("Fixed rate-Leg 1"),
        spread_leg1: r.f64("Spread-Leg 1"),
        spread_notation_leg1: r.opt("Spread notation-Leg 1"),
        price: r.f64("Price"),
        price_notation: r.opt("Price notation"),
        other_payment_amounts: r
            .list("Other payment amount")
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect(),
        other_payment_types: r.list("Other payment type"),
        other_payment_currency: r.opt("Other payment currency"),
        index_factor: r.f64("Index factor"),
        underlier_ids: r.list("Underlier ID-Leg 1"),
        underlier_id_sources: r.list("Underlier ID source-Leg 1"),
        underlying_asset_name: r.opt("Underlying Asset Name"),
        upi: r.opt("Unique Product Identifier"),
        upi_fisn: r.opt("UPI FISN"),
        upi_underlier_name: r.opt("UPI Underlier Name"),
        package: r.flag("Package indicator"),
        non_standard_term: r.flag("Non-standardized term indicator"),
        block_trade: r.flag("Block trade election indicator"),
        other,
    }
}

impl Record {
    /// A new price-forming execution (`NEWT` + `TRAD`).
    pub fn is_new_trade(&self) -> bool {
        self.action == ActionType::Newt && self.event == Some(EventType::Trad)
    }

    /// Upfront fee in currency units (sum of `UFRO` legs), unsigned as disseminated.
    pub fn upfront_amount(&self) -> Option<f64> {
        let ufro: Vec<f64> = self
            .other_payment_types
            .iter()
            .zip(&self.other_payment_amounts)
            .filter(|(t, _)| t.as_str() == "UFRO")
            .map(|(_, a)| *a)
            .collect();
        (!ufro.is_empty()).then(|| ufro.iter().sum())
    }

    /// The trade's date: execution date, else event date, else the report date.
    pub fn trade_date(&self) -> NaiveDate {
        self.execution_timestamp
            .or(self.event_timestamp)
            .map(|t| t.date_naive())
            .unwrap_or(self.report.date)
    }
}
