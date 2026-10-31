//! `cftc`: command-line access to DTCC PPD public swap data.
//! Data goes to stdout; structured JSON logs go to stderr.

use cftc::cds::{
    daily_series, flag_off_scale, roll_dates, CdsKind, CdsPrint, EntityKey, QuoteConvention,
};
use cftc::entity::EntityMap;
use cftc::{net_trades, AssetClass, Client, Query, Regulator};
use chrono::{NaiveDate, Utc};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(
    version,
    about = "Query free DTCC public swap data (CFTC Part 43 / SEC SBSR) by date range"
)]
struct Cli {
    /// Cache directory (default: platform cache dir / cftc-rs).
    #[arg(long, global = true, env = "CFTC_RS_CACHE")]
    cache_dir: Option<PathBuf>,
    /// Read-only DTCC archive directory (also settable with CFTC_RS_ARCHIVE).
    #[arg(long, global = true, env = "CFTC_RS_ARCHIVE")]
    archive: Option<PathBuf>,
    /// Use only cached files; never hit the network.
    #[arg(long, global = true)]
    offline: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct Range {
    /// First day (YYYY-MM-DD).
    #[arg(long)]
    from: NaiveDate,
    /// Last day, inclusive (default: today UTC).
    #[arg(long)]
    to: Option<NaiveDate>,
}

impl Range {
    fn to(&self) -> NaiveDate {
        self.to.unwrap_or_else(|| Utc::now().date_naive())
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Entity map tools.
    #[command(subcommand)]
    Entities(EntitiesCmd),
    /// Download and cache report files for a range.
    Fetch {
        #[command(flatten)]
        range: Range,
        #[arg(long, value_delimiter = ',', default_value = "sec,cftc")]
        regulator: Vec<Regulator>,
        #[arg(long, value_delimiter = ',', default_value = "credits")]
        asset: Vec<AssetClass>,
        #[arg(long)]
        intraday: bool,
    },
    /// Every raw record in a range as JSON lines.
    Records {
        #[command(flatten)]
        range: Range,
        #[arg(long)]
        regulator: Regulator,
        #[arg(long, default_value = "credits")]
        asset: AssetClass,
        #[arg(long)]
        intraday: bool,
    },
    /// Current live ticker trades as JSON lines.
    Ticker {
        #[arg(long)]
        regulator: Regulator,
        #[arg(long)]
        asset: AssetClass,
    },
    /// Credit default swap views.
    #[command(subcommand)]
    Cds(CdsCmd),
}

#[derive(Subcommand)]
enum EntitiesCmd {
    /// Build a ticker/LEI map from cached tape identifiers and public entity registries.
    Resolve {
        #[arg(long)]
        tickers: Option<PathBuf>,
        #[arg(long)]
        from: NaiveDate,
        #[arg(long)]
        to: NaiveDate,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 2000)]
        max_isins_per_issuer: usize,
    },
}

#[derive(Args, Clone)]
struct CdsFilter {
    #[command(flatten)]
    range: Range,
    /// Feeds to read (sec = single names, cftc = indices).
    #[arg(long, value_delimiter = ',', default_value = "sec,cftc")]
    regulator: Vec<Regulator>,
    /// Restrict to a kind: single_name, index, index_tranche, index_swaption, ...
    #[arg(long)]
    kind: Option<String>,
    /// Index family, exact (e.g. CDX.NA.IG, "ITRAXX EUROPE").
    #[arg(long)]
    family: Option<String>,
    /// Reference entity key SCHEME:ID (e.g. RED:2H6677, ISIN:US...). Repeatable.
    #[arg(long)]
    entity: Vec<String>,
    /// Ticker, resolved through --map.
    #[arg(long, requires = "map")]
    ticker: Option<String>,
    /// Entity map CSV (scheme,id,ticker,lei,name).
    #[arg(long, env = "CFTC_RS_ENTITY_MAP")]
    map: Option<PathBuf>,
    /// Case-insensitive substring of the name hint / underlier name.
    #[arg(long = "match")]
    name_match: Option<String>,
    /// Only trades at the scheduled on-the-run 5Y maturity.
    #[arg(long)]
    otr5y: bool,
    /// Only this tenor bucket in years (e.g. 5).
    #[arg(long)]
    tenor: Option<f64>,
}

#[derive(Subcommand)]
enum CdsCmd {
    /// Individual CDS prints (JSON lines).
    Prints(CdsFilter),
    /// One CSV row per series per report day, with staleness columns.
    Daily(CdsFilter),
    /// Distinct reference entities seen, with trade counts and name hints (CSV).
    Underliers(CdsFilter),
    /// Standard roll dates (20 Mar / 20 Sep) in a range.
    Rolls {
        #[command(flatten)]
        range: Range,
    },
}

fn log(event: &str, fields: serde_json::Value) {
    let mut obj = json!({"ts": Utc::now().to_rfc3339(), "event": event});
    if let (Some(o), serde_json::Value::Object(f)) = (obj.as_object_mut(), fields) {
        o.extend(f);
    }
    eprintln!("{obj}");
}

fn main() {
    if let Err(e) = run() {
        log("error", json!({"error": e.to_string()}));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut client = match &cli.cache_dir {
        Some(d) => Client::with_cache_dir(d),
        None => Client::new()?,
    };
    if let Some(archive) = &cli.archive {
        client = client.with_archive(archive);
    }
    let client = client.offline(cli.offline);
    log(
        "start",
        json!({"cache_dir": client.cache_dir(), "archive": cli.archive, "offline": cli.offline}),
    );
    let started = Instant::now();
    let out = std::io::stdout().lock();
    match cli.cmd {
        Cmd::Entities(EntitiesCmd::Resolve {
            tickers,
            from,
            to,
            out: path,
            max_isins_per_issuer,
        }) => entities_resolve(
            &client,
            from,
            to,
            tickers.as_deref(),
            path.as_deref(),
            cli.offline,
            max_isins_per_issuer,
            out,
        ),
        Cmd::Fetch {
            range,
            regulator,
            asset,
            intraday,
        } => fetch(&client, &range, &regulator, &asset, intraday),
        Cmd::Records {
            range,
            regulator,
            asset,
            intraday,
        } => records(&client, &range, regulator, asset, intraday, out),
        Cmd::Ticker { regulator, asset } => {
            let mut w = out;
            for r in client.ticker(regulator, asset)? {
                serde_json::to_writer(&mut w, &r)?;
                writeln!(w)?;
            }
            Ok(())
        }
        Cmd::Cds(CdsCmd::Rolls { range }) => {
            let mut w = out;
            roll_dates(range.from, range.to())
                .iter()
                .try_for_each(|d| writeln!(w, "{d}"))?;
            Ok(())
        }
        Cmd::Cds(c) => cds(&client, c, out),
    }?;
    log("done", json!({"elapsed_ms": started.elapsed().as_millis()}));
    Ok(())
}

fn cached_entity_get(
    client: &Client,
    url: &str,
    offline: bool,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let path = client
        .cache_dir()
        .join("entities")
        .join(format!("{:016x}.json", h.finish()));
    if path.exists() {
        return Ok(serde_json::from_slice(&std::fs::read(path)?)?);
    }
    if offline {
        return Ok(None);
    }
    let resp = match ureq::get(url).set("User-Agent", "cftc-rs").call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => {
            eprintln!(
                "{{\"event\":\"warning\",\"error\":{}}}",
                serde_json::to_string(&e.to_string())?
            );
            return Ok(None);
        }
    };
    let value: serde_json::Value = serde_json::from_reader(resp.into_reader())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec(&value)?)?;
    Ok(Some(value))
}

fn cached_entity_post(
    client: &Client,
    url: &str,
    body: &str,
    offline: bool,
    api_key: Option<&str>,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    body.hash(&mut h);
    let path = client
        .cache_dir()
        .join("entities")
        .join(format!("{:016x}.json", h.finish()));
    if path.exists() {
        return Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?));
    }
    if offline {
        return Ok(None);
    }
    // OpenFIGI: 25 req/min unauthenticated, 25 req/6s with a key. Cache hits skip this.
    std::thread::sleep(std::time::Duration::from_millis(if api_key.is_some() {
        250
    } else {
        2500
    }));
    let mut req = ureq::post(url).set("Content-Type", "application/json");
    if let Some(k) = api_key {
        req = req.set("X-OPENFIGI-APIKEY", k);
    }
    for attempt in 0..5 {
        match req.clone().send_string(body) {
            Ok(resp) => {
                let v: serde_json::Value = serde_json::from_reader(resp.into_reader())?;
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(path, serde_json::to_vec(&v)?)?;
                return Ok(Some(v));
            }
            Err(ureq::Error::Status(429, response)) if attempt < 4 => {
                let delay = response
                    .header("Retry-After")
                    .and_then(|v| v.parse::<u64>().ok())
                    .or_else(|| {
                        response
                            .header("ratelimit-reset")
                            .and_then(|v| v.parse::<u64>().ok())
                    })
                    .unwrap_or(2u64.pow(attempt));
                std::thread::sleep(std::time::Duration::from_secs(delay));
            }
            Err(e) => {
                eprintln!("warning: OpenFIGI request skipped: {e}");
                return Ok(None);
            }
        }
    }
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
fn entities_resolve(
    client: &Client,
    from: NaiveDate,
    to: NaiveDate,
    tickers: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
    offline: bool,
    max_isins_per_issuer: usize,
    mut stdout: impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    use cftc::cds::entity_keys;
    use cftc::entity::{
        build_rows, is_us_exchange_code, parse_gleif_isins, parse_gleif_lei, parse_gleif_parent,
        parse_openfigi_equity_listing,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let filter = tickers
        .map(
            |p| -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
                let mut r = csv::Reader::from_path(p)?;
                let headers = r.headers()?.clone();
                let ix = headers
                    .iter()
                    .position(|x| x == "ticker")
                    .ok_or("ticker CSV requires ticker column")?;
                Ok(r.records()
                    .filter_map(|x| x.ok())
                    .filter_map(|x| x.get(ix).map(|t| t.to_ascii_uppercase()))
                    .collect())
            },
        )
        .transpose()?;
    let mut pairs = BTreeMap::<String, String>::new();
    for regulator in [Regulator::Sec, Regulator::Cftc] {
        let q = Query::new(regulator, AssetClass::Credits, from, to);
        for day in client.days(&q) {
            match day {
                Ok(d) => {
                    for rec in d.records {
                        let keys = entity_keys(&rec);
                        if let (Some(i), Some(r)) = (
                            keys.iter().find(|k| k.scheme == "ISIN"),
                            keys.iter().find(|k| k.scheme == "RED"),
                        ) {
                            pairs.insert(i.id.clone(), r.id.clone());
                        }
                    }
                }
                Err(e) => eprintln!("warning: tape day skipped: {e}"),
            }
        }
    }
    let mut resolved = BTreeMap::new();
    let mut parent_tickers = BTreeMap::<String, Option<(String, String)>>::new();
    let mut isins = BTreeSet::new();
    for isin in pairs.keys() {
        isins.insert(isin.clone());
    }
    for isin in isins {
        let url = format!("https://api.gleif.org/api/v1/lei-records?filter%5Bisin%5D={isin}");
        let Some(v) = cached_entity_get(client, &url, offline)? else {
            eprintln!("warning: GLEIF ISIN cache miss or lookup failure: {isin}");
            continue;
        };
        let Some((lei, name)) = parse_gleif_lei(&v) else {
            eprintln!("warning: GLEIF has no LEI for {isin}");
            continue;
        };
        let purl = format!("https://api.gleif.org/api/v1/lei-records/{lei}/ultimate-parent");
        let parent_response = cached_entity_get(client, &purl, offline)?;
        let (parent, parent_name) = parse_gleif_parent(parent_response.as_ref(), &lei, &name);
        match parent_tickers.get(&parent) {
            Some(Some((ticker, exch_code))) => {
                resolved.insert(
                    isin.clone(),
                    (
                        ticker.clone(),
                        exch_code.clone(),
                        lei,
                        name,
                        parent,
                        parent_name,
                    ),
                );
                continue;
            }
            Some(None) => continue,
            None => {}
        }
        let mut parent_isins = Vec::new();
        for page in 1..=100 {
            let iurl = format!("https://api.gleif.org/api/v1/lei-records/{parent}/isins?page%5Bsize%5D=200&page%5Bnumber%5D={page}");
            let Some(v) = cached_entity_get(client, &iurl, offline)? else {
                break;
            };
            parent_isins.extend(parse_gleif_isins(&v));
            if parent_isins.len() >= max_isins_per_issuer {
                break;
            }
            let last = v
                .pointer("/meta/pagination/lastPage")
                .and_then(|x| x.as_u64())
                .unwrap_or(1);
            if page >= last {
                break;
            }
        }
        parent_isins.push(isin.clone());
        parent_isins.sort();
        parent_isins.dedup();
        let mut listing = None;
        let max_isins = max_isins_per_issuer;
        parent_isins = cftc::entity::order_issuer_isins(&parent_isins);
        if parent_isins.len() > max_isins {
            eprintln!("{{\"event\":\"warning\",\"issuer\":{parent:?},\"message\":\"ISIN cap reached\",\"cap\":{max_isins}}}");
            parent_isins.truncate(max_isins);
        }
        let chunk_size = if std::env::var_os("OPENFIGI_API_KEY").is_some() {
            100
        } else {
            10
        };
        for chunk in parent_isins.chunks(chunk_size) {
            let jobs: Vec<_> = chunk
                .iter()
                .map(
                    |x| serde_json::json!({"idType":"ID_ISIN","idValue":x,"marketSecDes":"Equity"}),
                )
                .collect();
            let body = serde_json::to_string(&serde_json::Value::Array(jobs))?;
            if let Some(v) = cached_entity_post(
                client,
                "https://api.openfigi.com/v3/mapping",
                &body,
                offline,
                std::env::var("OPENFIGI_API_KEY").ok().as_deref(),
            )? {
                if let Some(candidate) = parse_openfigi_equity_listing(&v) {
                    let candidate_is_us = is_us_exchange_code(&candidate.1);
                    let current_is_us = listing
                        .as_ref()
                        .is_some_and(|x: &(String, String)| is_us_exchange_code(&x.1));
                    if listing.is_none()
                        || (candidate_is_us && !current_is_us)
                        || candidate.1 == "US"
                    {
                        listing = Some(candidate);
                    }
                }
                if listing.as_ref().is_some_and(|x| x.1 == "US") {
                    break;
                }
            }
        }
        parent_tickers.insert(parent.clone(), listing.clone());
        let hits = parent_tickers.values().filter(|t| t.is_some()).count();
        eprintln!(
            "{}",
            json!({"event": "issuer_resolved", "isin": isin, "parent_lei": parent,
                   "parent_name": parent_name, "ticker": listing.as_ref().map(|x| &x.0),
                   "exch_code": listing.as_ref().map(|x| &x.1),
                   "issuers_done": parent_tickers.len(), "tickers": hits})
        );
        if let Some((t, exch_code)) = listing {
            if !is_us_exchange_code(&exch_code) {
                eprintln!(
                    "{}",
                    json!({"event":"warning", "message":"ticker has no US composite or US venue listing", "ticker":t, "exch_code":exch_code, "isin":isin})
                );
            }
            resolved.insert(isin.clone(), (t, exch_code, lei, name, parent, parent_name));
        }
    }
    let rows = build_rows(&resolved, &pairs, filter.as_ref());
    let writer: Box<dyn Write> = match output {
        Some(p) => Box::new(std::fs::File::create(p)?),
        None => Box::new(&mut stdout),
    };
    let mut w = csv::Writer::from_writer(writer);
    for row in rows {
        w.serialize(row)?;
    }
    w.flush()?;
    Ok(())
}

fn fetch(
    client: &Client,
    range: &Range,
    regs: &[Regulator],
    assets: &[AssetClass],
    intraday: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut found, mut missing, mut bytes) = (0, 0, 0usize);
    for &reg in regs {
        for &asset in assets {
            for report in Query::new(reg, asset, range.from, range.to()).reports() {
                match client.zip_bytes(&report)? {
                    Some(b) => {
                        found += 1;
                        bytes += b.len();
                        log(
                            "cached",
                            json!({"report": report.file_stem(), "bytes": b.len()}),
                        );
                    }
                    None => {
                        missing += 1;
                        log("missing", json!({"report": report.file_stem()}));
                    }
                }
            }
            if intraday {
                for date in range.from.iter_days().take_while(|d| *d <= range.to()) {
                    let rows = client.slices(reg, asset, date)?;
                    log(
                        "slices",
                        json!({"regulator": reg, "asset": asset, "date": date, "records": rows.len()}),
                    );
                }
            }
        }
    }
    log(
        "fetch_summary",
        json!({"found": found, "missing": missing, "bytes": bytes}),
    );
    Ok(())
}

fn records(
    client: &Client,
    range: &Range,
    reg: Regulator,
    asset: AssetClass,
    intraday: bool,
    mut out: impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let q = Query::new(reg, asset, range.from, range.to());
    let mut n = 0;
    let mut covered = BTreeSet::new();
    for day in client.days(&q) {
        let day = day?;
        covered.insert(day.report.date);
        log(
            "day",
            json!({"report": day.report.file_stem(), "records": day.records.len()}),
        );
        for r in &day.records {
            serde_json::to_writer(&mut out, r)?;
            writeln!(out)?;
            n += 1;
        }
    }
    // A day's cumulative file already contains every slice; slices only fill days without one (today).
    if intraday {
        for date in range
            .from
            .iter_days()
            .take_while(|d| *d <= range.to())
            .filter(|d| !covered.contains(d))
        {
            for r in client.slices(reg, asset, date)? {
                serde_json::to_writer(&mut out, &r)?;
                writeln!(out)?;
                n += 1;
            }
        }
    }
    log("records_summary", json!({"records": n}));
    Ok(())
}

fn load_prints(
    client: &Client,
    f: &CdsFilter,
) -> Result<(Vec<CdsPrint>, BTreeSet<NaiveDate>), Box<dyn std::error::Error>> {
    let map = f
        .map
        .as_deref()
        .map(EntityMap::from_csv_path)
        .transpose()?
        .unwrap_or_default();
    let mut wanted: Vec<EntityKey> = f
        .entity
        .iter()
        .map(|s| {
            let (scheme, id) = s
                .split_once(':')
                .ok_or_else(|| format!("--entity wants SCHEME:ID, got {s}"))?;
            Ok::<_, String>(EntityKey {
                scheme: scheme.to_ascii_uppercase(),
                id: id.to_ascii_uppercase(),
            })
        })
        .collect::<Result<_, _>>()?;
    if let Some(t) = &f.ticker {
        let keys = map.keys_for_ticker(t);
        log(
            "ticker_resolved",
            json!({"ticker": t, "keys": keys.iter().map(|k| k.to_string()).collect::<Vec<_>>()}),
        );
        if keys.is_empty() {
            return Err(format!("ticker {t} has no entries in the entity map").into());
        }
        wanted.extend(keys);
    }
    let kind: Option<CdsKind> = f
        .kind
        .as_deref()
        .map(|k| serde_json::from_value(json!(k)))
        .transpose()?;
    let needle = f.name_match.as_ref().map(|s| s.to_ascii_lowercase());
    let mut days = BTreeSet::new();
    let mut prints = Vec::new();
    for &reg in &f.regulator {
        let q = Query::new(reg, AssetClass::Credits, f.range.from, f.range.to());
        let mut recs = Vec::new();
        for day in client.days(&q) {
            let day = day?;
            log(
                "day",
                json!({"report": day.report.file_stem(), "records": day.records.len()}),
            );
            days.insert(day.report.date);
            recs.extend(day.records);
        }
        let raw = recs.len();
        let trades = net_trades(recs);
        log(
            "netted",
            json!({"regulator": reg, "records": raw, "live_trades": trades.len()}),
        );
        prints.extend(trades.iter().map(CdsPrint::from_trade));
    }
    flag_off_scale(&mut prints);
    let before = prints.len();
    prints.retain(|p| {
        kind.map_or(true, |k| p.kind == k)
            && f.family.as_ref().map_or(true, |fam| {
                p.index_family
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(fam))
            })
            && (wanted.is_empty() || p.all_entity_keys.iter().any(|k| wanted.contains(k)))
            && needle.as_ref().map_or(true, |n| {
                p.name_hint
                    .as_deref()
                    .is_some_and(|h| h.to_ascii_lowercase().contains(n))
            })
            && (!f.otr5y || p.on_the_run_5y)
            && f.tenor.map_or(true, |t| p.tenor_bucket == Some(t))
    });
    let mut unknown_families: BTreeMap<String, usize> = BTreeMap::new();
    for p in prints
        .iter()
        .filter(|p| p.quote_convention == QuoteConvention::Unknown)
    {
        if let Some(family) = &p.index_family {
            *unknown_families.entry(family.clone()).or_default() += 1;
        }
    }
    for (family, count) in unknown_families {
        log(
            "warning",
            json!({"message": "unknown index quote convention", "family": family, "count": count}),
        );
    }
    log(
        "filtered",
        json!({"before": before, "after": prints.len(), "report_days": days.len()}),
    );
    Ok((prints, days))
}

fn cds(client: &Client, cmd: CdsCmd, out: impl Write) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        CdsCmd::Prints(f) => {
            let (prints, _) = load_prints(client, &f)?;
            let mut out = out;
            for p in &prints {
                serde_json::to_writer(&mut out, p)?;
                writeln!(out)?;
            }
        }
        CdsCmd::Daily(f) => {
            let (prints, days) = load_prints(client, &f)?;
            let rows = daily_series(&prints, &days, f.range.from, f.range.to());
            let mut w = csv::Writer::from_writer(out);
            rows.iter().try_for_each(|r| w.serialize(r))?;
            w.flush()?;
            log(
                "daily_summary",
                json!({"rows": rows.len(), "series": rows.iter().map(|r| &r.series).collect::<BTreeSet<_>>().len()}),
            );
        }
        CdsCmd::Underliers(f) => {
            let (prints, _) = load_prints(client, &f)?;
            #[derive(serde::Serialize, Default)]
            struct U {
                entity: String,
                trades: usize,
                first_trade: Option<NaiveDate>,
                last_trade: Option<NaiveDate>,
                other_keys: String,
                name_hints: String,
                usage: &'static str,
            }
            let mut by: BTreeMap<String, (U, BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
            for p in prints.iter().filter(|p| p.entity.is_some()) {
                let e = p.entity.as_ref().expect("filtered").to_string();
                let (u, keys, names) = by.entry(e.clone()).or_default();
                u.entity = e;
                u.trades += 1;
                u.first_trade = u.first_trade.min(Some(p.trade_date)).or(Some(p.trade_date));
                u.last_trade = u.last_trade.max(Some(p.trade_date));
                keys.extend(p.all_entity_keys.iter().skip(1).map(|k| k.to_string()));
                names.extend(p.name_hint.clone());
            }
            let mut rows: Vec<U> = by
                .into_values()
                .map(|(mut u, k, n)| {
                    u.other_keys = k.into_iter().collect::<Vec<_>>().join(";");
                    u.name_hints = n.into_iter().take(5).collect::<Vec<_>>().join(" | ");
                    u.usage = "internal_only";
                    u
                })
                .collect();
            rows.sort_by_key(|a| std::cmp::Reverse(a.trades));
            let mut w = csv::Writer::from_writer(out);
            rows.iter().try_for_each(|r| w.serialize(r))?;
            w.flush()?;
            log("underliers_summary", json!({"entities": rows.len()}));
        }
        CdsCmd::Rolls { .. } => unreachable!("handled in run"),
    }
    Ok(())
}
