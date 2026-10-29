//! Typed, date-range access to the free DTCC Public Price Dissemination (PPD) swap data:
//! CFTC Part 43 (index CDS, rates, FX, ...) and SEC Regulation SBSR (single-name CDS).
//!
//! DTCC serves one zip per regulator, asset class and day. This crate fetches and caches
//! those files, parses them into [`Record`]s, nets lifecycle messages into live trades, and
//! builds CDS views and raw daily aggregates.
//!
//! ```no_run
//! use cftc::{Client, Query, Regulator, AssetClass, net_trades, cds::CdsPrint};
//! use chrono::NaiveDate;
//!
//! let client = Client::new()?;
//! let q = Query::new(Regulator::Sec, AssetClass::Credits,
//!     NaiveDate::from_ymd_opt(2025, 5, 1).unwrap(), NaiveDate::from_ymd_opt(2025, 5, 9).unwrap());
//! let prints: Vec<CdsPrint> = net_trades(client.records(&q)?).iter().map(CdsPrint::from_trade).collect();
//! # Ok::<(), cftc::Error>(())
//! ```
//!
//! Data terms: DTCC permits internal or personal use only. Records carry
//! [`UsageScope::InternalOnly`]; do not redistribute data or derived series.

pub mod cds;
pub mod entity;
pub mod error;
pub mod fetch;
pub mod query;
pub mod record;
pub mod source;

pub use error::{Error, Result};
pub use fetch::Client;
pub use query::{net_trades, Day, Query, Trade};
pub use record::{ActionType, EventType, Notional, Record, UsageScope};
pub use source::{AssetClass, Regulator, ReportId, ReportKind};
