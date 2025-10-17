use std::time::Duration;

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
            token: String::from("a7mC1bFNLhGd5GGcdaNQxrefuaBxwo"),
            user: String::from("7hwJ5U6uG0dUAS1V9rk9DPDZMjqow1"),
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
            warn!("Failed to intilize for pushover");
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
