use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use backon::{ExponentialBuilder, Retryable};
use bitcoin::absolute::Height;
use bitcoin::consensus::encode::deserialize;
use bitcoin::hex::FromHex;
use bitcoin::BlockHash;
use futures::StreamExt;
use reqwest::Client;
use tokio::sync::RwLock;

const MEMPOOL: &str = "mempool.sirion.io";

#[derive(Debug)]
struct HeaderChain {
    pub tip_height: Height,
    /// `sparse_headers[i]` is the header of the block at height `i * 2016`.
    pub sparse_headers: Vec<bitcoin::block::Header>,
}

impl Default for HeaderChain {
    fn default() -> Self {
        Self {
            tip_height: Height::MIN,
            sparse_headers: Vec::new(),
        }
    }
}

type SharedState = Arc<RwLock<HeaderChain>>;

async fn get_difficulties(State(state): State<SharedState>) -> Json<Vec<f64>> {
    let difficulties = state
        .read()
        .await
        .sparse_headers
        .iter()
        .map(bitcoin::block::Header::difficulty_float)
        .collect();
    Json(difficulties)
}

#[allow(dead_code)]
static DIFFICULTY_PERIOD: Mutex<u32> = Mutex::new(10);

async fn get_tip_height(client: &Client) -> anyhow::Result<Height> {
    let url = format!("https://{MEMPOOL}/api/blocks/tip/height");
    let response = client.get(url).send().await?;
    let text = response.text().await?;
    let height = text.parse()?;
    println!("Got tip: height {height}");
    Ok(height)

    // let mut lock = DIFFICULTY_PERIOD.lock().expect("not poisoned");
    // *lock += 2;
    // Ok(Height::from_consensus(*lock * 2016).expect("height should be valid"))
}

async fn get_block_hash(client: &Client, height: Height) -> anyhow::Result<BlockHash> {
    let url = format!("https://{MEMPOOL}/api/block-height/{height}");
    let response = client.get(&url).send().await?;
    let text = response.text().await?;
    let block_hash = text.parse()?;
    Ok(block_hash)
}

async fn get_header(client: &Client, height: Height) -> anyhow::Result<bitcoin::block::Header> {
    let block_hash = get_block_hash(client, height).await?;
    let url = format!("https://{MEMPOOL}/api/block/{block_hash}/header");
    let response = client.get(&url).send().await?;
    let hex = response.text().await?;
    let bytes = Vec::<u8>::from_hex(hex.as_str())?;
    let header: bitcoin::block::Header = deserialize(&bytes)?;
    println!("Got header: height {height}");
    Ok(header)
}

pub const fn round_down_to_difficulty_adjustment(height: u32) -> u32 {
    (height / 2016) * 2016
}

async fn push_new_headers(client: &Client, state: &SharedState) {
    let lock = state.read().await;
    let first_new_height = (lock.sparse_headers.len() as u32 + 1) * 2016;
    let last_new_height = round_down_to_difficulty_adjustment(lock.tip_height.to_consensus_u32());
    drop(lock);

    let mut it = futures::stream::iter((first_new_height..=last_new_height).step_by(2016))
        .map(|height: u32| async move {
            let height = Height::from_consensus(height).unwrap();
            let closure = || async { get_header(client, height).await };
            closure
                .retry(ExponentialBuilder::default())
                .await
                .unwrap_or_else(|_| panic!(
                    "Failed to fetch header after multiple retries: height {height}"
                ))
        })
        .buffered(10);
    while let Some(header) = it.next().await {
        state.write().await.sparse_headers.push(header);
    }

    let lock = state.read().await;
    debug_assert_eq!(lock.sparse_headers.len() * 2016, last_new_height as usize);
}

async fn update_state(client: &Client, state: &SharedState) {
    let closure = || async { get_tip_height(client).await };
    let new_tip_height = closure
        .retry(ExponentialBuilder::default())
        .await
        .expect("Tip: failed to fetch after multiple retries");

    let mut lock = state.write().await;
    let old_tip_height = lock.tip_height;
    lock.tip_height = new_tip_height;
    drop(lock);

    println!("Syncing headers: height {old_tip_height} -> height {new_tip_height}");
    push_new_headers(client, state).await;
    println!("Completed sync: height {new_tip_height}");
}

#[tokio::main]
async fn main() {
    let state = Arc::new(RwLock::new(HeaderChain::default()));

    let local_state = state.clone();
    tokio::spawn(async move {
        let client = Client::new();
        loop {
            update_state(&client, &local_state).await;
            println!("Waiting: 10 minutes");
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });

    let app = Router::new()
        .route("/difficulties", get(get_difficulties))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
