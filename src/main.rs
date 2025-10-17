mod backoff;
mod check;
mod notify;
mod url_json;

use std::env;

use actix_web::{
    App, HttpResponse, HttpServer, Responder,
    middleware::{self, Logger},
    web,
};
use notify::{Notifier, Pushover};

#[actix_web::get("/ok")]
async fn ok() -> impl Responder {
    HttpResponse::Ok().body("Ok")
}

#[actix_web::get("/notifo")]
async fn notifo_request(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let Some(status) = query.get("status") else {
        return HttpResponse::BadRequest().body("Failed");
    };
    let status = status.as_str();
    let url = query.get("url").map(|x| x.as_str());
    let sound = query.get("sound").map(|x| x.as_str());
    Pushover::default().notify(status, url, sound).await;
    HttpResponse::Ok().body("Ok")
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    Pushover::default()
        .notify("starting uptime check", None, Some("spacealarm"))
        .await;
    tokio::spawn(check::check());

    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| String::from("8080"))
        .parse()
        .expect("PORT must be a number");

    let binding_interface = format!("0.0.0.0:{port}");

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .wrap(middleware::DefaultHeaders::new().add(("X-Version", env!("CARGO_PKG_VERSION"))))
            .service(ok)
            .service(notifo_request)
    })
    .bind(binding_interface)?
    .run()
    .await
}
