//! Date-range queries over the daily report files.

use crate::error::Result;
use crate::fetch::Client;
use crate::record::{parse_csv, ActionType, Record};
use crate::source::{AssetClass, Regulator, ReportId};
use chrono::NaiveDate;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
    pub regulator: Regulator,
    pub asset_class: AssetClass,
    pub from: NaiveDate,
    /// Inclusive.
    pub to: NaiveDate,
}

impl Query {
    pub fn new(
        regulator: Regulator,
        asset_class: AssetClass,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Self {
        Self {
            regulator,
            asset_class,
            from,
            to,
        }
    }

    pub fn reports(&self) -> impl Iterator<Item = ReportId> + '_ {
        self.from
            .iter_days()
            .take_while(move |d| *d <= self.to)
            .map(move |d| ReportId::cumulative(self.regulator, self.asset_class, d))
    }
}

/// The parsed contents of one day's cumulative report.
#[derive(Debug, Clone)]
pub struct Day {
    pub report: ReportId,
    pub records: Vec<Record>,
}

impl Client {
    /// One day's records; `None` when DTCC has no file for that day.
    pub fn day(&self, report: ReportId) -> Result<Option<Day>> {
        self.csv_bytes(&report)?
            .map(|csv| parse_csv(report, &csv).map(|records| Day { report, records }))
            .transpose()
    }

    /// Lazily walk every day in the query; missing days are skipped.
    pub fn days<'a>(&'a self, q: &'a Query) -> impl Iterator<Item = Result<Day>> + 'a {
        q.reports().filter_map(move |r| self.day(r).transpose())
    }

    /// Every record in the range, in file order, day by day.
    pub fn records(&self, q: &Query) -> Result<Vec<Record>> {
        self.days(q).try_fold(Vec::new(), |mut acc, day| {
            acc.extend(day?.records);
            Ok(acc)
        })
    }
}

/// A price-forming trade after lifecycle messages in the same window were applied.
#[derive(Debug, Clone)]
pub struct Trade {
    /// The latest version of the trade (the correcting record when `corrected`).
    pub record: Record,
    /// Dissemination id of the original `NEWT`.
    pub original_id: String,
    pub corrected: bool,
}

/// Reduce raw dissemination messages to the set of live, price-forming trades:
/// start from `NEWT`/`TRAD`, apply `CORR` (replace terms) and `EROR` (drop), following
/// chains of corrections. Messages that refer to trades outside the window are ignored,
/// so query a few days past the range of interest to catch late corrections.
pub fn net_trades(records: impl IntoIterator<Item = Record>) -> Vec<Trade> {
    let mut trades: Vec<Option<Trade>> = Vec::new();
    // Any dissemination id (original or correcting) -> slot in `trades`.
    let mut slot: HashMap<String, usize> = HashMap::new();
    for rec in records {
        let target = rec
            .original_dissemination_id
            .as_ref()
            .and_then(|o| slot.get(o).copied());
        match (&rec.action, target) {
            _ if rec.is_new_trade() => {
                slot.insert(rec.dissemination_id.clone(), trades.len());
                trades.push(Some(Trade {
                    original_id: rec.dissemination_id.clone(),
                    record: rec,
                    corrected: false,
                }));
            }
            (ActionType::Corr, Some(i)) => {
                if let Some(t) = trades[i].as_mut() {
                    slot.insert(rec.dissemination_id.clone(), i);
                    t.record = rec;
                    t.corrected = true;
                }
            }
            (ActionType::Eror, Some(i)) => trades[i] = None,
            _ => {}
        }
    }
    trades.into_iter().flatten().collect()
}
