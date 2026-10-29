//! Where DTCC Public Price Dissemination (PPD) reports live and how they are named.
//!
//! DTCC publishes one cumulative zip per (regulator, asset class, day) at
//! `https://pddata.dtcc.com/ppd/api/report/cumulative/{sec|cftc}/{SEC|CFTC}_CUMULATIVE_{ASSET}_{YYYY_MM_DD}.zip`.
//! The SEC feed carries security-based swaps (single-name CDS) under Reg SBSR; the CFTC
//! feed carries Part 43 swaps (index CDS, rates, FX, ...).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub const DEFAULT_BASE_URL: &str = "https://pddata.dtcc.com/ppd/api/report";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Regulator {
    /// SEC Regulation SBSR: security-based swaps (single-name CDS, narrow-based indices).
    Sec,
    /// CFTC Part 43: swaps (broad-based index CDS, rates, FX, commodities, equity swaps).
    Cftc,
}

impl Regulator {
    pub fn path(self) -> &'static str {
        match self {
            Regulator::Sec => "sec",
            Regulator::Cftc => "cftc",
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            Regulator::Sec => "SEC",
            Regulator::Cftc => "CFTC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetClass {
    Credits,
    Rates,
    Equities,
    Forex,
    Commodities,
}

impl AssetClass {
    pub fn token(self) -> &'static str {
        match self {
            AssetClass::Credits => "CREDITS",
            AssetClass::Rates => "RATES",
            AssetClass::Equities => "EQUITIES",
            AssetClass::Forex => "FOREX",
            AssetClass::Commodities => "COMMODITIES",
        }
    }
}

/// One downloadable report file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReportId {
    pub regulator: Regulator,
    pub asset_class: AssetClass,
    pub date: NaiveDate,
    pub kind: ReportKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ReportKind {
    Cumulative,
    Slice(u32),
}

impl ReportId {
    pub fn cumulative(regulator: Regulator, asset_class: AssetClass, date: NaiveDate) -> Self {
        Self {
            regulator,
            asset_class,
            date,
            kind: ReportKind::Cumulative,
        }
    }

    pub fn slice(regulator: Regulator, asset_class: AssetClass, date: NaiveDate, n: u32) -> Self {
        Self {
            regulator,
            asset_class,
            date,
            kind: ReportKind::Slice(n),
        }
    }

    /// `SEC_CUMULATIVE_CREDITS_2025_05_01`
    pub fn file_stem(&self) -> String {
        if let ReportKind::Slice(n) = self.kind {
            return format!(
                "{}_SLICE_{}_{}_{}",
                self.regulator.prefix(),
                self.asset_class.token(),
                self.date.format("%Y_%m_%d"),
                n
            );
        }
        format!(
            "{}_CUMULATIVE_{}_{}",
            self.regulator.prefix(),
            self.asset_class.token(),
            self.date.format("%Y_%m_%d")
        )
    }

    pub fn url(&self, base_url: &str) -> String {
        format!(
            "{}/{}/{}/{}.zip",
            base_url.trim_end_matches('/'),
            match self.kind {
                ReportKind::Cumulative => "cumulative",
                ReportKind::Slice(_) => "intraday",
            },
            self.regulator.path(),
            self.file_stem()
        )
    }
}

impl fmt::Display for ReportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file_stem())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown {kind}: {value}")]
pub struct ParseEnumError {
    kind: &'static str,
    value: String,
}

impl FromStr for Regulator {
    type Err = ParseEnumError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "sec" => Ok(Regulator::Sec),
            "cftc" => Ok(Regulator::Cftc),
            _ => Err(ParseEnumError {
                kind: "regulator",
                value: s.into(),
            }),
        }
    }
}

impl FromStr for AssetClass {
    type Err = ParseEnumError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "credits" | "credit" | "cr" => Ok(AssetClass::Credits),
            "rates" | "ir" => Ok(AssetClass::Rates),
            "equities" | "equity" | "eq" => Ok(AssetClass::Equities),
            "forex" | "fx" => Ok(AssetClass::Forex),
            "commodities" | "commodity" | "co" => Ok(AssetClass::Commodities),
            _ => Err(ParseEnumError {
                kind: "asset class",
                value: s.into(),
            }),
        }
    }
}
