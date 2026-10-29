//! Download, cache and unzip PPD reports.
//!
//! A cumulative report for a past day never changes, so it is cached forever. The report for
//! the current UTC day is still growing, so it is re-downloaded unless the client is offline.
//! Days without a report (weekends, holidays, dates outside DTCC's retention) come back as
//! `Ok(None)` rather than an error.

use crate::error::{Error, Result};
use crate::source::{ReportId, DEFAULT_BASE_URL};
use chrono::Utc;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    cache_dir: PathBuf,
    offline: bool,
    agent: ureq::Agent,
}

impl Client {
    /// Client with the platform cache dir (`~/Library/Caches/cftc-rs` on macOS,
    /// `$XDG_CACHE_HOME/cftc-rs` on Linux).
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir().ok_or(Error::NoCacheDir)?.join("cftc-rs");
        Ok(Self::with_cache_dir(cache_dir))
    }

    pub fn with_cache_dir(cache_dir: impl Into<PathBuf>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(120))
            .user_agent(concat!("cftc-rs/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            cache_dir: cache_dir.into(),
            offline: false,
            agent,
        }
    }

    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Never touch the network; serve only what is already cached.
    pub fn offline(mut self, offline: bool) -> Self {
        self.offline = offline;
        self
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn cache_path(&self, report: &ReportId) -> PathBuf {
        self.cache_dir
            .join(report.regulator.path())
            .join(report.asset_class.token().to_ascii_lowercase())
            .join(format!("{}.zip", report.file_stem()))
    }

    fn missing_marker(&self, report: &ReportId) -> PathBuf {
        self.cache_path(report).with_extension("missing")
    }

    /// Raw zip bytes for a report, from cache or network. `None` = DTCC has no such file.
    pub fn zip_bytes(&self, report: &ReportId) -> Result<Option<Vec<u8>>> {
        let path = self.cache_path(report);
        let is_today = report.date >= Utc::now().date_naive();
        if path.exists()
            && (!is_today
                || self.offline
                || matches!(report.kind, crate::source::ReportKind::Slice(_)))
        {
            return Ok(Some(std::fs::read(&path)?));
        }
        if self.missing_marker(report).exists() && !is_today {
            return Ok(None);
        }
        if self.offline {
            return Err(Error::NotCached { report: *report });
        }
        match self.download(report)? {
            Some(bytes) => {
                write_atomic(&path, &bytes)?;
                Ok(Some(bytes))
            }
            None => {
                if !is_today {
                    write_atomic(&self.missing_marker(report), b"")?;
                }
                Ok(None)
            }
        }
    }

    fn download(&self, report: &ReportId) -> Result<Option<Vec<u8>>> {
        let url = report.url(&self.base_url);
        match self.agent.get(&url).call() {
            Ok(resp) => {
                let mut bytes = Vec::new();
                resp.into_reader().read_to_end(&mut bytes)?;
                // DTCC answers some missing days with 200 and a non-zip body.
                Ok(bytes.starts_with(b"PK").then_some(bytes))
            }
            Err(ureq::Error::Status(404 | 400 | 403, _)) => Ok(None),
            Err(e) => Err(Error::Http {
                report: *report,
                source: Box::new(e),
            }),
        }
    }

    /// The CSV inside a report's zip.
    pub fn csv_bytes(&self, report: &ReportId) -> Result<Option<Vec<u8>>> {
        self.zip_bytes(report)?
            .map(|zip| unzip_csv(report, &zip))
            .transpose()
    }

    /// Fetch all published intraday slices. The first missing sequence number ends the day.
    pub fn slices(
        &self,
        regulator: crate::source::Regulator,
        asset: crate::source::AssetClass,
        date: chrono::NaiveDate,
    ) -> Result<Vec<crate::record::Record>> {
        let mut records = Vec::new();
        for n in 1.. {
            let report = ReportId::slice(regulator, asset, date, n);
            match self.csv_bytes(&report)? {
                Some(bytes) => records.extend(crate::record::parse_csv(report, &bytes)?),
                None => break,
            }
        }
        Ok(records)
    }

    /// Fetch the live ticker JSON and parse its trade objects into records. Ticker responses are never cached.
    pub fn ticker(
        &self,
        regulator: crate::source::Regulator,
        asset: crate::source::AssetClass,
    ) -> Result<Vec<crate::record::Record>> {
        let url = format!(
            "{}/ticker/{}/{}",
            self.base_url.trim_end_matches("/report"),
            regulator.path(),
            asset.token()
        );
        let response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Error::Ticker(e.to_string()))?;
        let mut body = String::new();
        response
            .into_reader()
            .read_to_string(&mut body)
            .map_err(|e| Error::Ticker(e.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| Error::Ticker(e.to_string()))?;
        let rows = value
            .get("tradeList")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| Error::Ticker("missing or invalid tradeList".into()))?;
        if let Some(errors) = value.get("errorList").and_then(serde_json::Value::as_array) {
            if !errors.is_empty() {
                eprintln!("ticker response contained {} errors", errors.len());
            }
        }
        let report = ReportId::cumulative(regulator, asset, Utc::now().date_naive());
        rows.iter()
            .map(|row| crate::record::from_ticker_json(report, row))
            .collect()
    }
}

pub(crate) fn unzip_csv(report: &ReportId, zip: &[u8]) -> Result<Vec<u8>> {
    let zerr = |source| Error::Zip {
        report: *report,
        source,
    };
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(zerr)?;
    let idx = (0..archive.len())
        .find(|&i| {
            archive
                .name_for_index(i)
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".csv"))
        })
        .ok_or(Error::EmptyArchive { report: *report })?;
    let mut out = Vec::new();
    archive.by_index(idx).map_err(zerr)?.read_to_end(&mut out)?;
    Ok(out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("part");
    std::fs::File::create(&tmp)?.write_all(bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
