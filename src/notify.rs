use std::{env, time::Duration};

use log::warn;
use reqwest::Client;

pub(crate) trait Notifier {
    async fn notify(&self, status: &str, url: Option<&str>, sound: Option<&str>) -> Option<String>;
}

pub(crate) struct Pushover {
    pub(crate) token: String,
    pub(crate) user: String,
}

impl Default for Pushover {
    fn default() -> Self {
        Self {
            token: env::var("PUSHOVER_TOKEN").unwrap_or_default(),
            user: env::var("PUSHOVER_USER").unwrap_or_default(),
        }
    }
}

impl Notifier for Pushover {
    async fn notify(&self, status: &str, url: Option<&str>, sound: Option<&str>) -> Option<String> {
        if self.token.is_empty() || self.user.is_empty() {
            warn!("PUSHOVER_TOKEN or PUSHOVER_USER not set, skipping notification: {status}");
            return None;
        }

        let mut params = vec![
            ("message", status.to_owned()),
            ("token", self.token.clone()),
            ("user", self.user.clone()),
        ];

        // Add optional parameters
        if let Some(url) = url {
            params.push(("url", url.to_owned()));
        }

        params.push(("sound", sound.unwrap_or("none").to_owned()));

        let Ok(client) = Client::builder().timeout(Duration::from_secs(10)).build() else {
            warn!("Failed to initialize for pushover");
            return None;
        };

        // Make POST request
        let Ok(response) = client
            .post("https://api.pushover.net/1/messages.json")
            .form(&params)
            .send()
            .await
        else {
            warn!("Failed sending for pushover");
            return None;
        };

        // Read and return response
        let Ok(response_text) = response.text().await else {
            return None;
        };

        Some(response_text)
    }
}
