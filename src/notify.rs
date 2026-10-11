use std::time::Duration;

use log::warn;
use reqwest::Client;

pub(crate) trait Notifier {
    async fn notify(&self, status: &str, url: Option<&str>, sound: Option<&str>) -> Option<String>;
}

// Pushover credentials are baked in at build time; the build fails if they are unset or empty
const PUSHOVER_TOKEN: &str = env!("PUSHOVER_TOKEN");
const PUSHOVER_USER: &str = env!("PUSHOVER_USER");
const _: () = assert!(!PUSHOVER_TOKEN.is_empty(), "PUSHOVER_TOKEN must not be empty");
const _: () = assert!(!PUSHOVER_USER.is_empty(), "PUSHOVER_USER must not be empty");

pub(crate) struct Pushover {
    pub(crate) token: String,
    pub(crate) user: String,
}

impl Default for Pushover {
    fn default() -> Self {
        Self {
            token: String::from(PUSHOVER_TOKEN),
            user: String::from(PUSHOVER_USER),
        }
    }
}

impl Notifier for Pushover {
    async fn notify(&self, status: &str, url: Option<&str>, sound: Option<&str>) -> Option<String> {
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
