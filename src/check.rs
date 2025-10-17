use std::{env, sync::Arc, time::Duration};

use chrono::Utc;
use futures::future::join_all;
use log::{debug, error, info, warn};
use memcache::MemcacheError;
use rand::{Rng, rng, seq::SliceRandom};
use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::Error;
use tokio::time::{MissedTickBehavior, interval, sleep};

use crate::{
    backoff::Backoff,
    notify::{self, Notifier},
    url_json::URL_JSON,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    url: String,
    empty: Option<bool>,
    dnd: Option<bool>,
    threshold: u64,
    rtimeout: u64,
    check_interval: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entries {
    domain: Vec<Entry>,
}

pub async fn check() {
    let Some(entries) = get_entries() else {
        warn!("Failed to get entries");
        return;
    };
    let server = env::var("MEMCACHE_SERVER").unwrap_or_else(|_| "memcache://localhost:11211?connect_timeout=4&timeout=4&tcp_nodelay=true".to_owned()); 
    info!("Connecting to memcache {server}");
    let client = match memcache::connect(
        server,
    ) {
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
        let mut interval = interval(Duration::from_secs(check_interval));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let client = Arc::clone(&client);
        let random_delay = rng().random_range(10..60);
        let future = tokio::spawn(async move {
            sleep(Duration::from_secs(random_delay)).await;
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
    let result = make_request(&entry.url, entry.rtimeout).await;
    // Generate a cache key with "uc-" prefix
    let key = format!("uc-{:x}", md5::compute(&entry.url));

    match result {
        Ok(result) => {
            let status = result.status();
            info!("status:{} for url {}", status.as_str(), entry.url);

            if status.is_success() {
                let is_empty = entry.empty.unwrap_or(false);

                // Try to get response body as text
                let Ok(body) = result.text().await else {
                    // If we can't get body text and entry isn't marked as empty,
                    // send 202 notification
                    if !is_empty {
                        send_notification(entry, client, &key, 202).await;
                    }
                    return;
                };

                // If entry isn't marked as empty and body is too short
                if !is_empty && (body.is_empty() || body.len() < 20) {
                    send_notification(entry, client, &key, 202).await;
                } else {
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
        error!("Failed to fetch from memcache{}", entry.url);
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
        warn!("within thersold for url {}", entry.url);
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
    let Ok(mut result): Result<Entries, Error> = serde_json::from_str(URL_JSON) else {
        return None;
    };
    let mut rng = rng();
    result.domain.shuffle(&mut rng);
    Some(result)
}
