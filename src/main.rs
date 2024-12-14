use std::{
    collections::{BTreeMap, BTreeSet},
    default,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
/// I understand that there's an amount of style perfectionism in writing
/// open source code because this is where people go to check in on if you
/// are an appropriate addition to the company. This is a quick, dirty,
/// and small thing to just make rss a little easier to integrate in a social
/// way. I don't know if people have done this much before. I don't remember
/// this kind of thing from the old rss days.
use axum::*;
use chrono::{DateTime, FixedOffset, Utc};
use either::Either;
use extract::State;
use http::{request, StatusCode};
use moka::future::Cache;
use rand::seq::SliceRandom;
use reqwest::{Client, Url};
use routing::post;
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

type Feed = Either<rss::Channel, atom_syndication::Feed>;

type Items = Either<rss::Item, atom_syndication::Entry>;

#[derive(Clone)]
pub struct RssCache {
    pub feed_cache: Cache<Url, Feed>,
    pub request_cache: Cache<RssRequest, RssResponse>,
    pub max_feed_request: usize,
    pub max_items_per_feed: usize,
}

impl RssCache {
    pub fn new(capacity: u64, invalidate_after: std::time::Duration) -> Self {
        Self {
            feed_cache: Cache::builder()
                .time_to_live(invalidate_after)
                .max_capacity(capacity)
                .build(),
            request_cache: Cache::builder()
                .time_to_live(Duration::from_secs(60 * 2))
                .max_capacity(1000)
                .build(),
            max_feed_request: 20,
            max_items_per_feed: 10,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct RssRequest {
    pub feeds: BTreeSet<String>,
    #[serde(default)]
    pub max_feed_items: MaxFeedItems,
    #[serde(default)]
    pub show_mode: ShowMode,
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct MaxFeedItems(usize);

impl Default for MaxFeedItems {
    fn default() -> Self {
        Self(5)
    }
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub enum ShowMode {
    EqualChronoShuffle,
    ReverseChronological,
    EqualChronoAlphabetical,
}

impl Default for ShowMode {
    fn default() -> Self {
        Self::EqualChronoShuffle
    }
}

impl ShowMode {
    fn order_feeds(&self, mut feeds: BTreeMap<Url, Feed>, max_from_feed: usize) -> Vec<Items> {
        fn item_datetime(item: &Items) -> DateTime<FixedOffset> {
            match item {
                Either::Left(rss_item) => rss_item
                    .pub_date
                    .as_ref()
                    .and_then(|date_str| DateTime::parse_from_rfc2822(date_str).ok())
                    .unwrap_or(DateTime::UNIX_EPOCH.into()),
                Either::Right(atom_item) => {
                    atom_item.published.unwrap_or(DateTime::UNIX_EPOCH.into())
                }
            }
        }
        let mut items = vec![];
        match self {
            ShowMode::EqualChronoShuffle => {
                let mut rng = rand::thread_rng();
                let mut keys_to_delete = Vec::new();
                let mut ix = 0usize;
                while !feeds.is_empty() {
                    let mut keys: BTreeSet<_> = feeds.keys().cloned().collect();
                    let mut can_still_choose = Vec::from_iter(keys.iter());
                    can_still_choose.shuffle(&mut rng);
                    while !can_still_choose.is_empty() {
                        let Some(this_key) = can_still_choose.pop() else {
                            continue;
                        };
                        let Some(this_entry) = feeds.get(this_key) else {
                            continue;
                        };
                        match this_entry {
                            Either::Left(rss) => {
                                if rss.items.len().min(max_from_feed) <= ix {
                                    keys_to_delete.push(this_key.clone());
                                } else {
                                    items.push(Either::Left(rss.items[ix].clone()));
                                }
                            }
                            Either::Right(atom) => {
                                if atom.entries.len().min(max_from_feed) <= ix {
                                    keys_to_delete.push(this_key.clone());
                                } else {
                                    items.push(Either::Right(atom.entries[ix].clone()));
                                }
                            }
                        }
                    }
                    for key in keys_to_delete.iter() {
                        feeds.remove(key);
                        keys.remove(key);
                    }
                    keys_to_delete.clear();
                    ix += 1;
                }
            }
            ShowMode::ReverseChronological => {
                for feed in feeds.into_values() {
                    match feed {
                        Either::Left(rss) => items
                            .extend(rss.items.into_iter().take(max_from_feed).map(Either::Left)),
                        Either::Right(atom) => items.extend(
                            atom.entries
                                .into_iter()
                                .take(max_from_feed)
                                .map(Either::Right),
                        ),
                    }
                }
                items.sort_by_cached_key(item_datetime);
            }
            ShowMode::EqualChronoAlphabetical => {
                let mut keys_to_delete = Vec::new();
                let mut ix = 0usize;
                while !feeds.is_empty() {
                    let mut keys: BTreeSet<_> = feeds.keys().cloned().collect();
                    let mut can_still_choose = Vec::from_iter(keys.iter());
                    while !can_still_choose.is_empty() {
                        let Some(this_key) = can_still_choose.pop() else {
                            continue;
                        };
                        let Some(this_entry) = feeds.get(this_key) else {
                            continue;
                        };
                        match this_entry {
                            Either::Left(rss) => {
                                if rss.items.len().min(max_from_feed) <= ix {
                                    keys_to_delete.push(this_key.clone());
                                } else {
                                    items.push(Either::Left(rss.items[ix].clone()));
                                }
                            }
                            Either::Right(atom) => {
                                if atom.entries.len().min(max_from_feed) <= ix {
                                    keys_to_delete.push(this_key.clone());
                                } else {
                                    items.push(Either::Right(atom.entries[ix].clone()));
                                }
                            }
                        }
                    }
                    for key in keys_to_delete.iter() {
                        feeds.remove(key);
                        keys.remove(key);
                    }
                    keys_to_delete.clear();
                    ix += 1;
                }
            }
        }
        items
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssResponse {
    pub items: Vec<Items>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let state = RssCache::new(5000, Duration::from_secs(60 * 15));
    let address = "0.0.0.0:3001";
    println!("Opening at http://{address}");
    let router = Router::new()
        .route("/poll_feeds", post(poll_feeds))
        .route("/poll_feeds_rendered", post(poll_feeds))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    axum::serve(listener, router).await.unwrap();
    Ok(())
}

pub async fn poll_feeds(
    State(state): State<RssCache>,
    Json(input): Json<RssRequest>,
) -> Result<Json<RssResponse>, (StatusCode, String)> {
    if let Some(cached_response) = state.request_cache.get(&input).await {
        return Ok(Json(cached_response));
    }
    let input_cloned = input.clone();
    let mut set: JoinSet<Result<_, anyhow::Error>> = JoinSet::new();
    let urls = input
        .feeds
        .into_iter()
        .filter_map(|url| Url::parse(&url).ok())
        .take(state.max_feed_request);
    for url in urls {
        let cache = state.feed_cache.clone();
        set.spawn(async move {
            let entry = cache.get(&url).await.clone();
            if let Some(entry) = entry {
                Ok((url, entry))
            } else {
                let response = reqwest::get(url.clone())
                    .await
                    .map_err(|err| anyhow!("{err}"))?;
                let text = response.text().await.map_err(|err| anyhow!("{err}"))?;
                let rss = rss::Channel::from_str(&text);
                if let Ok(rss) = rss {
                    cache.insert(url.clone(), Either::Left(rss.clone())).await;
                    Ok((url, Either::Left(rss)))
                } else {
                    let atom = atom_syndication::Feed::from_str(&text).map_err(|err| {
                        anyhow!("Could not parse as RSS or Atom feed: {rss:?}, {err:?}")
                    })?;
                    cache.insert(url.clone(), Either::Right(atom.clone())).await;
                    Ok((url, Either::Right(atom)))
                }
            }
        });
    }

    let mut accumulated_errors = vec![];
    let mut feeds = BTreeMap::new();
    while let Some(joining) = set.join_next().await {
        let Ok(result) = joining else {
            continue;
        };
        let Ok((url, feed)) = result else {
            accumulated_errors.push(result.unwrap_err());
            continue;
        };
        feeds.insert(url, feed);
    }
    let ordered = input.show_mode.order_feeds(feeds, input.max_feed_items.0.min(state.max_items_per_feed));
    let response = RssResponse { items: ordered };
    state.request_cache.insert(input_cloned, response.clone()).await;
    Ok(Json(response))
}
