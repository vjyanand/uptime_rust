use std::{env, sync::Arc, time::Duration};

use chrono::Utc;
use futures::future::join_all;
use log::{debug, error, info, warn};
use memcache::MemcacheError;
use rand::{RngExt, rng, seq::SliceRandom};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

use crate::{
    backoff::Backoff,
    notify::{self, Notifier},
    url_json::URL_JSON,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    url: String,
    empty: Option<bool>,
    dnd: Option<bool>,
    threshold: u64,
    rtimeout: u64,
    check_interval: Option<u64>,
    disabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entries {
    domain: Vec<Entry>,
}

pub async fn check() {
    let Some(entries) = get_entries() else {
        warn!("Failed to get entries");
        return;
    };
    let server = env::var("MEMCACHE_SERVER").unwrap_or_else(|_| {
        "memcache://localhost:11211?connect_timeout=4&timeout=4&tcp_nodelay=true".to_owned()
    });
    info!("Connecting to memcache {server}");
    let client = match memcache::connect(server) {
        Ok(client) => client,
        Err(e) => {
            let error_msg = format!("memcache is down: {e}");
            warn!("{error_msg}");
            notify::Pushover::default()
                .notify(&error_msg, Some("mcache.iavian.net"), Some("spacealarm"))
                .await;
            return;
        }
    };
    let client = Arc::new(client);
    let mut futures = Vec::new();
    for entry in entries.domain {
        let check_interval = entry.check_interval.unwrap_or(60);
        // Start each entry at a random offset so ticks stay staggered, not just the first one
        let random_delay = rng().random_range(10..60);
        let mut interval = interval_at(
            Instant::now() + Duration::from_secs(random_delay),
            Duration::from_secs(check_interval),
        );
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let client = Arc::clone(&client);
        let future = tokio::spawn(async move {
            loop {
                interval.tick().await;
                process_entry(&entry, &client).await;
            }
        });
        futures.push(future);
    }
    join_all(futures).await;
}

async fn process_entry(entry: &Entry, client: &memcache::Client) {
    // Make an HTTP request with the entry's URL and timeout
    let mut result = make_request(&entry.url, entry.rtimeout).await;
    // Retry once on transient failures before counting it as a failure
    if is_transient(&result) {
        warn!("Transient failure for url {}, retrying", entry.url);
        sleep(Duration::from_secs(2)).await;
        result = make_request(&entry.url, entry.rtimeout).await;
    }
    // Generate a cache key with "uc-" prefix
    let key = format!("uc-{:x}", md5::compute(&entry.url));

    match result {
        Ok(result) => {
            let status = result.status();
            info!("status:{} for url {}", status.as_str(), entry.url);

            if status == StatusCode::TOO_MANY_REQUESTS {
                // Rate limited: the site is up, so neither alert nor reset the failure state
                warn!("Rate limited for url {}, skipping", entry.url);
            } else if status.is_success() {
                // Entries marked as empty only need a success status, so skip the body
                let body_ok = entry.empty.unwrap_or(false)
                    || result.text().await.is_ok_and(|body| body.len() >= 20);

                // If the body couldn't be read or is too short, send 202 notification
                if !body_ok {
                    send_notification(entry, client, &key, 202).await;
                } else {
                    notify_recovery(entry, client, &key).await;
                    // Delete cache entry
                    let Ok(result) = client.delete(&key) else {
                        warn!("Failed to delete memcache for url {}", entry.url);
                        return;
                    };
                    info!("key deleted for {} with result {}", entry.url, result);
                }
            } else {
                // If request wasn't successful, send notification with status code
                send_notification(entry, client, &key, status.as_u16()).await;
            }
        }
        Err(err) => {
            warn!("Error: {err:?}");
            send_notification(entry, client, &key, 500).await;
        }
    }
}

async fn send_notification(
    entry: &Entry,
    client: &memcache::Client,
    cache_key: &str,
    status_code: u16,
) {
    warn!("Trying to send notification for url {}", entry.url);
    let Ok(value): Result<Option<Vec<u8>>, MemcacheError> = client.get(cache_key) else {
        error!("Failed to fetch from memcache {}", entry.url);
        return;
    };
    let Some(value) = value else {
        warn!("No memcache set: {}", entry.url);
        let mut backoff = Backoff::with_time(entry.threshold);
        backoff.increment_by_factor();
        let serialized = serde_json::to_string(&backoff).unwrap();
        if let Err(err) = client.set(cache_key, serialized.as_bytes(), 0) {
            warn!("Error: {err:?}");
        };
        warn!("new memcache set: {}", entry.url);
        return;
    };
    let Ok(mut backoff): Result<Backoff, serde_json::Error> = serde_json::from_slice(&value) else {
        error!("Conversion error");
        return;
    };

    let now = Utc::now();
    if now < backoff.get_date() {
        warn!("within threshold for url {}", entry.url);
        return;
    }
    debug!("new backoff: {now}, backoff:{backoff}");
    let time_diff = now.signed_duration_since(backoff.get_date());
    let new_threshold: i64 = (entry.threshold * 60).try_into().unwrap_or_default();
    if time_diff.num_minutes() > new_threshold {
        backoff = Backoff::with_time(entry.threshold);
        warn!("new backoff: {now}, backoff:{backoff}");
    }
    backoff.increment_by_factor();
    backoff.mark_notified();
    let serialized = serde_json::to_string(&backoff).unwrap();
    if let Err(err) = client.set(cache_key, serialized.as_bytes(), 0) {
        error!("Error: {err:?}");
    };

    let sound = if entry.dnd.unwrap_or(false) {
        Some("none")
    } else {
        Some("gamelan")
    };
    notify::Pushover::default()
        .notify(
            &format!("{} {} is down", status_code, entry.url),
            Some(&entry.url),
            sound,
        )
        .await;
}

// Send a "back up" alert if a "down" alert was sent for the outage that just ended
async fn notify_recovery(entry: &Entry, client: &memcache::Client, cache_key: &str) {
    let Ok(Some(value)): Result<Option<Vec<u8>>, MemcacheError> = client.get(cache_key) else {
        return;
    };
    let Ok(backoff): Result<Backoff, serde_json::Error> = serde_json::from_slice(&value) else {
        return;
    };
    if !backoff.is_notified() {
        return;
    }
    info!("Sending recovery notification for url {}", entry.url);
    let sound = if entry.dnd.unwrap_or(false) {
        Some("none")
    } else {
        Some("gamelan")
    };
    notify::Pushover::default()
        .notify(&format!("{} is back up", entry.url), Some(&entry.url), sound)
        .await;
}

fn is_transient(result: &Result<Response, reqwest::Error>) -> bool {
    match result {
        Ok(response) => {
            let status = response.status();
            status.is_server_error()
                || status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::TOO_MANY_REQUESTS
        }
        Err(_) => true,
    }
}

async fn make_request(url: &str, timeout: u64) -> Result<Response, reqwest::Error> {
    let timeout = Duration::from_millis(timeout).min(Duration::from_secs(30));

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout) // Total timeout for the request
        .gzip(true)
        .brotli(true)
        .tcp_nodelay(true)
        .user_agent("uptime-check")
        .build()?;
    info!("Checking url {url}");
    let response = client.get(url).send().await?;
    Ok(response)
}

pub fn get_entries() -> Option<Entries> {
    let mut result: Entries = match serde_json::from_str(URL_JSON) {
        Ok(result) => result,
        Err(err) => {
            error!("Failed to parse URL_JSON: {err}");
            return None;
        }
    };
    result
        .domain
        .retain(|entry| !entry.disabled.unwrap_or(false));
    let mut rng = rng();
    result.domain.shuffle(&mut rng);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_json_parses() {
        let entries = get_entries().expect("URL_JSON should parse");
        assert!(!entries.domain.is_empty());
    }

    #[test]
    fn unknown_field_is_rejected() {
        let json = r#"{"url":"https://example.com","threshold":1,"rtimeout":1000,"check_intervl":120}"#;
        assert!(serde_json::from_str::<Entry>(json).is_err());
    }
}
