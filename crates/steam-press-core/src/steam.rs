//! Steam app metadata from `api.steamcmd.net` and the public store API.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration;

const INFO_URL: &str = "https://api.steamcmd.net/v1/info";
const STORE_URL: &str = "https://store.steampowered.com/api/appdetails";
const SEARCH_URL: &str = "https://store.steampowered.com/api/storesearch";
const USER_AGENT: &str = "SteamPress/0.5";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotEntry {
    pub depot_id: String,
    pub name: String,
    pub manifest_id: String,
    pub dlc_appid: Option<String>,
    pub size_bytes: Option<u64>,
    pub shared_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub appid: String,
    pub name: String,
    pub install_dir: Option<String>,
    pub build_id: String,
    pub build_time: Option<DateTime<Utc>>,
    pub depots: Vec<DepotEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSearchItem {
    pub appid: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    pub name: String,
    pub short_description: String,
    pub website: Option<String>,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("HTTP client failed: {error}"))
}

fn get_text(url: &str) -> Result<String, String> {
    let response = client()?
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|error| format!("Steam request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Steam returned HTTP {}", response.status()));
    }
    response
        .text()
        .map_err(|error| format!("unreadable Steam response: {error}"))
}

fn number_or_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

pub fn parse_app_info(appid: &str, branch: &str, json: &str) -> Result<AppInfo, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid appinfo JSON: {error}"))?;

    if let Some(status) = root.get("status").and_then(Value::as_str) {
        if status != "success" {
            return Err(format!("appinfo indisponible pour {appid}"));
        }
    }

    let app = root
        .get("data")
        .and_then(|data| data.get(appid))
        .ok_or_else(|| format!("no appinfo data for {appid}"))?;

    let name = app
        .pointer("/common/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return Err(format!("game name not found for {appid}"));
    }

    let install_dir = app
        .pointer("/config/installdir")
        .and_then(Value::as_str)
        .map(str::to_string);

    let depots_object = app.get("depots");
    let branch_object = depots_object
        .and_then(|depots| depots.get("branches"))
        .and_then(|branches| branches.get(branch));

    let build_id = branch_object
        .and_then(|branch| branch.get("buildid"))
        .and_then(number_or_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Unknown".to_string());

    let build_time = branch_object
        .and_then(|branch| branch.get("timeupdated"))
        .and_then(number_or_string)
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0));

    let shared = app
        .get("shareddepots")
        .or_else(|| depots_object.and_then(|depots| depots.get("shareddepots")))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut depots = Vec::new();
    if let Some(depots_object) = depots_object.and_then(Value::as_object) {
        for (depot_id, depot) in depots_object {
            if depot_id.eq_ignore_ascii_case("branches")
                || depot_id.eq_ignore_ascii_case("shareddepots")
                || depot_id.parse::<u64>().is_err()
            {
                continue;
            }
            let Some(depot_object) = depot.as_object() else {
                continue;
            };
            let manifest = depot_object
                .get("manifests")
                .and_then(|manifests| manifests.get(branch).or_else(|| manifests.get("public")));
            let manifest_id = manifest
                .and_then(|manifest| manifest.get("gid"))
                .and_then(number_or_string)
                .unwrap_or_default();
            if manifest_id.is_empty() {
                continue;
            }
            depots.push(DepotEntry {
                depot_id: depot_id.clone(),
                name: depot_object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                manifest_id,
                dlc_appid: depot_object.get("dlcappid").and_then(number_or_string),
                size_bytes: manifest
                    .and_then(|manifest| manifest.get("size"))
                    .and_then(number_or_string)
                    .and_then(|size| size.parse::<u64>().ok()),
                shared_owner: shared
                    .get(depot_id)
                    .and_then(number_or_string)
                    .filter(|owner| !owner.is_empty()),
            });
        }
    }

    for (depot_id, owner) in &shared {
        if depots.iter().any(|depot| &depot.depot_id == depot_id) {
            continue;
        }
        let Some(owner) = number_or_string(owner).filter(|owner| !owner.is_empty()) else {
            continue;
        };
        let depot_object = depots_object
            .and_then(|depots| depots.get(depot_id))
            .and_then(Value::as_object);
        let manifest = depot_object
            .and_then(|depot| depot.get("manifests"))
            .and_then(|manifests| manifests.get(branch).or_else(|| manifests.get("public")));
        let manifest_id = manifest
            .and_then(|manifest| manifest.get("gid"))
            .and_then(number_or_string)
            .unwrap_or_default();
        if manifest_id.is_empty() {
            continue;
        }
        depots.push(DepotEntry {
            depot_id: depot_id.clone(),
            name: depot_object
                .and_then(|depot| depot.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            manifest_id,
            dlc_appid: depot_object
                .and_then(|depot| depot.get("dlcappid"))
                .and_then(number_or_string),
            size_bytes: manifest
                .and_then(|manifest| manifest.get("size"))
                .and_then(number_or_string)
                .and_then(|size| size.parse::<u64>().ok()),
            shared_owner: Some(owner),
        });
    }
    depots.sort_by_key(|depot| depot.depot_id.parse::<u64>().unwrap_or(u64::MAX));

    Ok(AppInfo {
        appid: appid.to_string(),
        name,
        install_dir,
        build_id,
        build_time,
        depots,
    })
}

pub fn fetch_app_info(appid: &str, branch: &str) -> Result<AppInfo, String> {
    let url = format!("{INFO_URL}/{appid}");
    let body = get_text(&url)?;
    parse_app_info(appid, branch, &body)
}

pub fn parse_store_info(appid: &str, json: &str) -> Result<StoreInfo, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid store JSON: {error}"))?;
    let app = root
        .get(appid)
        .ok_or_else(|| format!("no store data for {appid}"))?;
    if !app.get("success").and_then(Value::as_bool).unwrap_or(false) {
        return Err(format!("store entry unavailable for {appid}"));
    }
    let data = app
        .get("data")
        .ok_or_else(|| format!("empty store entry for {appid}"))?;
    Ok(StoreInfo {
        name: data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        short_description: data
            .get("short_description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        website: data
            .get("website")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(normalize_https),
    })
}

pub fn fetch_store_info(appid: &str) -> Result<StoreInfo, String> {
    let url = format!("{STORE_URL}?appids={appid}");
    let body = get_text(&url)?;
    parse_store_info(appid, &body)
}

/// Searches the public Steam store by game name.
pub fn search_store(term: &str) -> Result<Vec<StoreSearchItem>, String> {
    let term = term.trim();
    if term.len() < 2 {
        return Ok(Vec::new());
    }
    let response = client()?
        .get(SEARCH_URL)
        .query(&[("term", term), ("l", "english"), ("cc", "US")])
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|error| format!("Steam search failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Steam search returned HTTP {}", response.status()));
    }
    let body = response
        .text()
        .map_err(|error| format!("unreadable Steam search response: {error}"))?;
    parse_store_search(&body)
}

pub fn parse_store_search(json: &str) -> Result<Vec<StoreSearchItem>, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid search JSON: {error}"))?;
    let items = root
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "Steam search returned no items".to_string())?;
    let mut results = Vec::new();
    for item in items {
        let Some(appid) = item
            .get("id")
            .and_then(number_or_string)
            .filter(|value| value.parse::<u64>().is_ok())
        else {
            continue;
        };
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty()
            || results
                .iter()
                .any(|entry: &StoreSearchItem| entry.appid == appid)
        {
            continue;
        }
        results.push(StoreSearchItem {
            appid,
            name: name.to_string(),
        });
    }
    Ok(results)
}

/// Upgrades a bare `http://` URL without mangling an existing `https://`.
pub fn normalize_https(url: &str) -> String {
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_balatro_appinfo() {
        let json = include_str!("../tests/fixtures/appinfo.json");
        let info = parse_app_info("2379780", "public", json).unwrap();
        assert_eq!(info.name, "Balatro");
        assert_eq!(info.install_dir.as_deref(), Some("Balatro"));
        assert_eq!(info.build_id, "18674832");
        assert_eq!(info.depots.len(), 2);
        assert_eq!(info.depots[1].depot_id, "2379781");
        assert_eq!(info.depots[1].manifest_id, "4851806656204679952");
        assert!(info.build_time.is_some());
    }

    #[test]
    fn unknown_branch_keeps_unknown_build() {
        let json = include_str!("../tests/fixtures/appinfo.json");
        let info = parse_app_info("2379780", "beta", json).unwrap();
        assert_eq!(info.build_id, "Unknown");
        assert!(info.build_time.is_none());
    }

    #[test]
    fn parses_store_info_and_normalizes_website() {
        let json = include_str!("../tests/fixtures/store.json");
        let store = parse_store_info("2379780", json).unwrap();
        assert_eq!(store.name, "Balatro");
        assert_eq!(
            store.website.as_deref(),
            Some("https://www.playbalatro.com")
        );
        assert!(store.short_description.starts_with("The poker"));
    }

    #[test]
    fn parses_store_search_results() {
        let json = r#"{
            "total": 2,
            "items": [
                {"type": "app", "name": "Balatro", "id": 2379780},
                {"type": "app", "name": "Balatro Soundtrack", "id": 2834460},
                {"type": "app", "name": "Broken", "id": "not-a-number"}
            ]
        }"#;
        let results = parse_store_search(json).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].appid, "2379780");
        assert_eq!(results[0].name, "Balatro");
        assert_eq!(results[1].name, "Balatro Soundtrack");
    }

    #[test]
    fn parses_shared_depots_and_manifest_sizes() {
        let json = r#"{
          "status": "success",
          "data": { "42": {
            "common": {"name": "Shared Game"},
            "config": {"installdir": "SharedGame"},
            "shareddepots": {"100": "228980"},
            "depots": {
              "branches": {"public": {"buildid": "7", "timeupdated": "0"}},
              "10": {"name": "Game", "manifests": {"public": {"gid": "11", "size": "1000"}}},
              "100": {"name": "Redistributable", "manifests": {"public": {"gid": "22", "size": "2000"}}}
            }
          }}
        }"#;
        let info = parse_app_info("42", "public", json).unwrap();
        assert_eq!(info.depots.len(), 2);
        assert_eq!(info.depots[0].size_bytes, Some(1000));
        assert_eq!(info.depots[1].manifest_id, "22");
        assert_eq!(info.depots[1].shared_owner.as_deref(), Some("228980"));
    }

    #[test]
    fn normalize_https_only_rewrites_the_scheme() {
        assert_eq!(normalize_https("http://example.com"), "https://example.com");
        assert_eq!(
            normalize_https("https://example.com"),
            "https://example.com"
        );
        assert_eq!(
            normalize_https("https://example.com/http-guide"),
            "https://example.com/http-guide"
        );
    }
}
