//! Reference-entity -> issuer map, loaded from data you maintain (never hardcoded).
//!
//! CSV columns: `scheme,id,ticker,lei,name` where `scheme` is `RED`, `ISIN`, `CUSIP`, ...
//! and `id` matches [`crate::cds::EntityKey::id`] (6-char RED entity code for `RED`).

use crate::cds::EntityKey;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;

/// A generated entity-map CSV row. The optional parent fields are informational; older
/// EntityMap readers ignore them and continue to consume the original five columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEntity {
    pub scheme: String,
    pub id: String,
    pub ticker: Option<String>,
    pub exch_code: Option<String>,
    pub lei: Option<String>,
    pub name: Option<String>,
    pub parent_lei: Option<String>,
    pub parent_name: Option<String>,
}

/// Extract LEI and legal name from the GLEIF JSON:API response shape.
pub fn parse_gleif_lei(v: &serde_json::Value) -> Option<(String, String)> {
    let data = v.get("data")?;
    let data = data
        .as_array()
        .and_then(|a| a.first())
        .or_else(|| data.is_object().then_some(data))?;
    let attrs = data.get("attributes")?;
    let lei = attrs.get("lei")?.as_str()?.to_owned();
    let name = attrs
        .pointer("/entity/legalName/name")?
        .as_str()?
        .to_owned();
    Some((lei, name))
}

/// Resolve an ultimate-parent response, falling back to the issuer for missing/empty results.
pub fn parse_gleif_parent(
    v: Option<&serde_json::Value>,
    issuer_lei: &str,
    issuer_name: &str,
) -> (String, String) {
    v.and_then(parse_gleif_lei)
        .unwrap_or_else(|| (issuer_lei.to_owned(), issuer_name.to_owned()))
}

/// Extract ISINs from a GLEIF ISIN relationship response.
pub fn parse_gleif_isins(v: &serde_json::Value) -> Vec<String> {
    v.get("data")
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|d| {
            d.pointer("/attributes/isin")
                .and_then(|x| x.as_str())
                .map(str::to_owned)
        })
        .collect()
}

/// Select the best OpenFIGI equity listing from an ISIN mapping response.
/// OpenFIGI's `US` exchange code is its US composite and is preferred over
/// individual US venues, which in turn are preferred over foreign listings.
pub fn is_us_exchange_code(code: &str) -> bool {
    code == "US" || matches!(code, "UN" | "UW" | "UA" | "UP" | "UQ") || code.starts_with("US")
}

pub fn parse_openfigi_equity_listing(v: &serde_json::Value) -> Option<(String, String)> {
    let rows: Vec<_> = v
        .as_array()?
        .iter()
        .filter_map(|job| job.get("data")?.as_array())
        .flatten()
        .collect();
    let mut choices: Vec<_> = rows
        .iter()
        .filter(|r| {
            r.get("marketSector").and_then(|x| x.as_str()) == Some("Equity")
                && matches!(
                    r.get("securityType").and_then(|x| x.as_str()),
                    Some("Common Stock" | "ADR" | "Depositary Receipt")
                )
                && r.get("ticker").and_then(|x| x.as_str()).is_some()
        })
        .collect();
    choices.sort_by_key(|r| {
        let code = r.get("exchCode").and_then(|x| x.as_str()).unwrap_or("");
        if code == "US" {
            0
        } else if is_us_exchange_code(code) {
            1
        } else {
            2
        }
    });
    let best = choices.first()?;
    Some((
        best.get("ticker")?.as_str()?.to_owned(),
        best.get("exchCode")?.as_str()?.to_owned(),
    ))
}

/// Select only the ticker from the best OpenFIGI equity listing.
pub fn parse_openfigi_equity_ticker(v: &serde_json::Value) -> Option<String> {
    parse_openfigi_equity_listing(v).map(|(ticker, _)| ticker)
}

/// Order issuer ISINs so likely US equity identifiers are tried before the long tail.
pub fn order_issuer_isins(isins: &[String]) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for isin in isins.iter().filter(|i| i.starts_with("US") && i.len() >= 8) {
        *counts.entry(isin[2..8].to_owned()).or_default() += 1;
    }
    let mut ordered = isins.to_vec();
    ordered.sort_by_key(|isin| {
        let us = !isin.starts_with("US");
        let prefix = isin.get(2..8).unwrap_or("");
        (
            us,
            std::cmp::Reverse(counts.get(prefix).copied().unwrap_or(0)),
            isin.clone(),
        )
    });
    ordered
}

/// Build output rows and propagate each ISIN's resolution to its co-occurring RED key.
pub fn build_rows(
    entities: &BTreeMap<String, (String, String, String, String, String, String)>,
    red_by_isin: &BTreeMap<String, String>,
    filter: Option<&std::collections::BTreeSet<String>>,
) -> Vec<ResolvedEntity> {
    let mut rows = Vec::new();
    for (isin, (ticker, exch_code, lei, name, parent, parent_name)) in entities {
        if filter.is_some_and(|f| !f.contains(&ticker.to_ascii_uppercase())) {
            continue;
        }
        for (scheme, id) in std::iter::once(("ISIN", isin.as_str()))
            .chain(red_by_isin.get(isin).map(|r| ("RED", r.as_str())))
        {
            rows.push(ResolvedEntity {
                scheme: scheme.into(),
                id: id.into(),
                ticker: Some(ticker.clone()),
                exch_code: Some(exch_code.clone()),
                lei: Some(lei.clone()),
                name: Some(name.clone()),
                parent_lei: Some(parent.clone()),
                parent_name: Some(parent_name.clone()),
            });
        }
    }
    rows
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    #[test]
    fn parses_gleif_isin_and_parent_shape() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/entities/gleif_isin.json"))
                .unwrap();
        assert_eq!(
            parse_gleif_lei(&v),
            Some(("FAKELEI0000000000001".into(), "Example Corp".into()))
        );
        let isins: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/entities/gleif_isins.json"))
                .unwrap();
        assert_eq!(parse_gleif_isins(&isins), vec!["US0000000001"]);
        assert_eq!(
            parse_gleif_parent(None, "ISSUER", "Issuer Name"),
            ("ISSUER".into(), "Issuer Name".into())
        );
        assert_eq!(
            parse_gleif_parent(
                Some(&serde_json::json!({"data":[]})),
                "ISSUER",
                "Issuer Name"
            ),
            ("ISSUER".into(), "Issuer Name".into())
        );
        assert_eq!(parse_gleif_lei(&serde_json::json!({"data":[]})), None);
    }
    #[test]
    fn prefers_us_equity_and_propagates_red_with_filter() {
        let figi: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/entities/openfigi.json")).unwrap();
        assert_eq!(parse_openfigi_equity_ticker(&figi).as_deref(), Some("USX"));
        assert_eq!(
            parse_openfigi_equity_listing(&figi),
            Some(("USX".into(), "US".into()))
        );
        let mut e = BTreeMap::new();
        e.insert(
            "US0000000001".into(),
            (
                "USX".into(),
                "US".into(),
                "L".into(),
                "Name".into(),
                "P".into(),
                "Parent".into(),
            ),
        );
        let mut red = BTreeMap::new();
        red.insert("US0000000001".into(), "ABC123".into());
        let f = ["USX".to_owned()].into_iter().collect();
        let rows = build_rows(&e, &red, Some(&f));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].scheme, "RED");
        assert_eq!(rows[0].exch_code.as_deref(), Some("US"));
        assert!(build_rows(&e, &red, Some(&["OTHER".to_owned()].into_iter().collect())).is_empty());
    }

    #[test]
    fn parses_parent_object_and_scans_all_figi_jobs_and_orders_isins() {
        let parent = serde_json::json!({"data":{"id":"PARENT","attributes":{"lei":"PARENT","entity":{"legalName":{"name":"Parent Co"}}}}});
        assert_eq!(
            parse_gleif_parent(Some(&parent), "ISSUER", "Issuer"),
            ("PARENT".into(), "Parent Co".into())
        );
        let multi = serde_json::json!([{"warning":"none"},{"data":[{"ticker":"GOOD","marketSector":"Equity","securityType":"Common Stock","exchCode":"US"}]}]);
        assert_eq!(
            parse_openfigi_equity_ticker(&multi).as_deref(),
            Some("GOOD")
        );
        let listings = serde_json::json!([{"data":[
            {"ticker":"CCOGBP","exchCode":"ER","marketSector":"Equity","securityType":"Common Stock"},
            {"ticker":"CCO","exchCode":"US","marketSector":"Equity","securityType":"Common Stock"},
            {"ticker":"CCO.N","exchCode":"UN","marketSector":"Equity","securityType":"Common Stock"}
        ]}]);
        assert_eq!(
            parse_openfigi_equity_listing(&listings),
            Some(("CCO".into(), "US".into()))
        );
        let foreign_only = serde_json::json!([{"data":[{"ticker":"CCOGBP","exchCode":"ER","marketSector":"Equity","securityType":"Common Stock"}]}]);
        assert_eq!(
            parse_openfigi_equity_listing(&foreign_only),
            Some(("CCOGBP".into(), "ER".into()))
        );
        let isins = vec![
            "FR0000000001".into(),
            "US1234560001".into(),
            "US1234560002".into(),
            "US9999990001".into(),
        ];
        assert_eq!(order_issuer_isins(&isins)[0], "US1234560001");
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issuer {
    pub scheme: String,
    pub id: String,
    #[serde(default)]
    pub ticker: Option<String>,
    #[serde(default)]
    pub lei: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EntityMap {
    by_key: HashMap<(String, String), Issuer>,
}

impl EntityMap {
    pub fn from_csv_path(path: &Path) -> Result<Self> {
        let cerr = |source| Error::Csv {
            context: path.display().to_string(),
            source,
        };
        let mut rdr = csv::Reader::from_path(path).map_err(cerr)?;
        let issuers = rdr
            .deserialize::<Issuer>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(cerr)?;
        Ok(Self::from_issuers(issuers))
    }

    pub fn from_issuers(issuers: impl IntoIterator<Item = Issuer>) -> Self {
        let by_key = issuers
            .into_iter()
            .map(|i| {
                (
                    (i.scheme.to_ascii_uppercase(), i.id.to_ascii_uppercase()),
                    i,
                )
            })
            .collect();
        Self { by_key }
    }

    pub fn get(&self, key: &EntityKey) -> Option<&Issuer> {
        self.by_key
            .get(&(key.scheme.to_ascii_uppercase(), key.id.to_ascii_uppercase()))
    }

    /// First issuer matching any of the keys (pass them most-specific first).
    pub fn resolve<'a>(&self, keys: impl IntoIterator<Item = &'a EntityKey>) -> Option<&Issuer> {
        keys.into_iter().find_map(|k| self.get(k))
    }

    /// Every key mapped to a ticker (case-insensitive).
    pub fn keys_for_ticker(&self, ticker: &str) -> Vec<EntityKey> {
        self.by_key
            .values()
            .filter(|i| {
                i.ticker
                    .as_deref()
                    .is_some_and(|t| t.eq_ignore_ascii_case(ticker))
            })
            .map(|i| EntityKey {
                scheme: i.scheme.to_ascii_uppercase(),
                id: i.id.to_ascii_uppercase(),
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}
