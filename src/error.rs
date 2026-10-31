use crate::source::ReportId;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error fetching {report}: {source}")]
    Http {
        report: ReportId,
        #[source]
        source: Box<ureq::Error>,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error in {report}: {source}")]
    Zip {
        report: ReportId,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("{report}: zip archive contains no .csv entry")]
    EmptyArchive { report: ReportId },
    #[error("csv error in {context}: {source}")]
    Csv {
        context: String,
        #[source]
        source: csv::Error,
    },
    #[error("{report} not in archive or cache and client is offline")]
    NotCached { report: ReportId },
    #[error("archive error for {report}: {message}")]
    Archive { report: ReportId, message: String },
    #[error("ticker response error: {0}")]
    Ticker(String),
    #[error("no cache directory could be determined; set one with Client::with_cache_dir")]
    NoCacheDir,
}

pub type Result<T> = std::result::Result<T, Error>;
