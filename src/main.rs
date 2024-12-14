use std::{sync::Arc, time::Duration};

/// I understand that there's an amount of style perfectionism in writing
/// open source code because this is where people go to check in on if you
/// are an appropriate addition to the company. This is a quick, dirty,
/// and small thing to just make rss a little easier to integrate in a social
/// way. I don't know if people have done this much before. I don't remember
/// this kind of thing from the old rss days.
use axum::*;
use either::Either;
use extract::State;
use http::StatusCode;
use moka::future::Cache;
use routing::post;
use serde::{Deserialize, Serialize};

type Feed = Either<rss::Channel, atom_syndication::Feed>;

#[derive(Clone)]
pub struct RssCache {
    pub cache: Cache<String, Feed>,
}

impl RssCache {
    pub fn new(capacity: u64, invalidate_after: std::time::Duration) -> Self {
        Self {
            cache: Cache::builder()
                .time_to_live(invalidate_after)
                .max_capacity(capacity)
                .build(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RssRequest {
    pub feeds: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let state = RssCache::new(5000, Duration::from_secs(60 * 15));
    let router = Router::new()
        .route("/poll_feeds", post(poll_feeds))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, router).await.unwrap();
    Ok(())
}

pub async fn poll_feeds(
    State(state): State<RssCache>,
    Json(input): Json<RssRequest>,
) -> Result<String, StatusCode> {
    Ok(String::new())
}
