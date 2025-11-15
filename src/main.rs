use std::{collections::{HashMap, VecDeque}, env, fs::{File, OpenOptions}, io::{BufReader, Read}, net::SocketAddr, str::FromStr, sync::Arc, time::{Duration, Instant}};

use anyhow::{Context, anyhow, bail};
use sqlx::sqlite::SqliteConnectOptions;
use thiserror::Error;
use axum::{Json, Router, body::Body, extract::{Path, Query, State, ws::{Message, WebSocket, WebSocketUpgrade}}, http::{Request, Response, StatusCode}, middleware::{self, Next}, response::IntoResponse, routing::get};
use futures::{SinkExt, StreamExt};
use const_crypto::ed25519;
use ore_api::{consts::{BOARD, ROUND, SPLIT_ADDRESS, TREASURY_ADDRESS}, state::{Board, Miner, Round, Treasury, round_pda}};
use serde::{Deserialize, Serialize};
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_filter::RpcFilterType};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use steel::{AccountDeserialize, Pubkey};
use tokio::{signal, sync::{Mutex, RwLock, broadcast}};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use std::io::Write;
use std::path::Path as FsPath;

pub mod app_state;
pub mod rpc;
pub mod database;
pub mod ai;
pub mod slot_miner;
pub mod ore_env;
pub mod ev;
pub mod markov_chain;
pub mod hmm2;
pub mod ws_server;

use crate::{app_state::{AppBoard, AppMiner, AppRound, AppState, AppTreasury}, database::{CreateDeployment, DbMinerSnapshot, DbTreasury, MinerLeaderboardRow, MinerOreLeaderboardRow, MinerTotalsRow, RoundRow, get_deployments_by_round}, hmm2::{predict_next_from_hmm, top_k_from_probs, train_hmm}, ore_env::fetch_ore_env, rpc::{DeployOutcome, evaluate_ev_only, infer_refined_ore, try_checkpoint_and_deploy, try_claim_sol}, ws_server::build_router};

#[derive(serde::Serialize)]
struct PredictionsMsg {
    r#type: &'static str,
    round: u64,
    preds: Vec<usize>,
    status:  &'static str
}

#[derive(serde::Serialize)]
struct AccuracyMsg {
    r#type: &'static str,
    preds: Vec<usize>,
    status:  &'static str,
    accuracy: f64,
    total_round: usize,
    total_win: usize
}


const PROGRAM_ID: [u8; 32] = unsafe { *(&ore_api::id() as *const Pubkey as *const [u8; 32]) };

pub const BOARD_ADDRESS: Pubkey =
    Pubkey::new_from_array(ed25519::derive_program_address(&[BOARD], &PROGRAM_ID).0);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().expect("Failed to load env");

    let (tx, _rx) = broadcast::channel::<String>(200);
    let ws_handle = ws_server::WsServerHandle { tx: tx.clone() };

    let app = build_router(ws_handle.clone());

    // spawn axum server on 0.0.0.0:3000 (atau port lain)
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let make_svc = app.into_make_service_with_connect_info::<SocketAddr>();
    // let server = Server::bind(&addr).serve(make_svc);
    // if let Err(err) = server.await {
    //     tracing::error!("server error: {}", err);
    // }
    // tokio::spawn(async move {
    //     tracing::info!("Starting WS server on {}", addr);
    //     if let Err(e) = Server::bind(&addr).serve(app.into_make_service()).await {
    //         tracing::error!("axum server error: {:?}", e);
    //     }
    // });
    
    // // let server = Server::bind(&addr).serve(make_svc);

    // tracing::info!("Starting WS server on ws://{}", addr);

    // // Jika mau graceful shutdown, bisa wrap .with_graceful_shutdown(...)
    // if let Err(err) = server.await {
    //     tracing::error!("server error: {}", err);
    // }

    // tokio::spawn(async move {
    //     tracing::info!("Starting WS server on {}", addr);
    //     let server = Server::bind(&addr).serve(app.into_make_service());
    //     // if let Err(e) = Server::bind(&addr).serve(app.into_make_service()).await {
    //     //     tracing::error!("axum server error: {:?}", e);
    //     // }
    // });

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();

    tracing::info!("Running migrations...");

    let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set");
    let prefix = "https://".to_string();
    let connection = RpcClient::new_with_commitment(prefix + &rpc_url, CommitmentConfig { commitment: CommitmentLevel::Confirmed });


    let out_path = "output.txt";
    let need_header = !FsPath::new(out_path).exists();

    let mut history: Vec<usize> = Vec::new();

    let board_data = connection.get_account_data(&BOARD_ADDRESS).await?;
    let board = Board::try_from_bytes(&board_data)?;
    let start_round = &board.round_id - 50;
    for i in start_round..=board.round_id + 2{
        let round_key = &round_pda(i).0;
        match connection.get_account_data(&round_key).await {
            Ok(data) => {
                match Round::try_from_bytes(&data) {
                    Ok(round) => {
                        if let Some(rng) = round.rng() {
                            // winning square (0..24)
                            let winning_square = round.winning_square(rng) as usize;
        
                            // writeln!(file, "{},{}", timestamp_str, id)?;
                            println!("Round: {} \nTotal_deployed: {}\nTime: {}\nSquare: {}\n\n",
                                i, round.total_deployed, round.expires_at, winning_square
                            );
                            history.push(winning_square);
                        }
                    },
                    Err(e) => {

                    }
                }
            }
            Err(err) => {
                eprintln!("Failed getting account for id {}: {}", i, err);
                // lanjut ke id berikutnya
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    tokio::spawn(async move {
        update_data_system_all(connection, history, tx.clone()).await;
    });

    // let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
    //     .await?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await?;

    axum::serve(listener, make_svc)
        .await
        .map_err(|e| anyhow!("server error: {:?}", e))?;
    
    // let state = app_state.clone();

    // tracing::debug!("Listening on {}", listener.clone().local_addr()?);

    Ok(())
}

pub async fn update_data_system_all(connection: RpcClient, mut history: Vec<usize>, tx: broadcast::Sender<String>) {
    tracing::info!("Starting update_data_system (Markov-2)");

    let window = 50usize;
    let topk = 20usize;

    // load last N rounds
    let mut buffer: VecDeque<usize> = history.iter().cloned().collect();
    let n_states = 8usize;
    let n_iter = 50usize;
    let retrain_every = 3usize;
    let mut rounds_since_retrain = 0usize;

    let mut total = 0usize;
    let mut total_hit = 0usize;
    let mut preds: Vec<usize> = Vec::new();

    let mut hmm_model = train_hmm(&history, n_states, n_iter);
    tracing::info!("Initial HMM trained on {} observations.", history.len());

    // bookkeeping similar to original
    let mut total = 0usize;
    let mut lose = 0u32;
    // let paths = [
    //     "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json",
    //     "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json",
    //     "/Users/jeckhat/gawean/jeckhat/miners/mebest.json",
    //     "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json",
    // ];

    let mut last_deployed_round = None;

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let board = if let Ok(data) = connection.get_account_data(&BOARD_ADDRESS).await {
            if let Ok(b) = Board::try_from_bytes(&data) {
                b.clone()
            } else {
                tracing::error!("Failed to parse Board account");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        } else {
            tracing::error!("Failed to load board account data");
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };

        if last_deployed_round != Some(board.round_id) {
            let msg = PredictionsMsg {
                r#type: "waiting",
                round: board.round_id,  // gunakan round saat ini
                preds: Vec::new(),
                status: "waiting"
            };

            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = tx.send(json);  // broadcast ke semua client
                tracing::info!("Sent predictions via WS: {:?}", msg.preds);
            }

            last_deployed_round = Some(board.round_id);
            // for path in &paths {
            //     match try_claim_sol(&connection, path).await {
            //         Ok(DeployOutcome::Deployed(sig)) => {
            //             tracing::info!("Claim submitted: {}", sig);
            //         }
            //         Ok(DeployOutcome::Skipped) => {
            //             tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
            //             continue;
            //         }
            //         Err(e) => {
            //             tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
            //             continue;
            //         }
            //     }
            // }
            println!("round {}", board.round_id);
            println!("History {}: {:?}", board.round_id, &history);
            let last_window: Vec<usize> = buffer.iter().cloned().collect();
            let probs = predict_next_from_hmm(&hmm_model, &last_window);
            let top = top_k_from_probs(&probs, topk);
            preds = top.iter().map(|(idx, _p)| *idx).collect();

            let msg = PredictionsMsg {
                r#type: "predictions",
                round: board.round_id,
                preds: preds.clone(),
                status: "waiting"
            };

            if let Ok(json) = serde_json::to_string(&msg) {
                let _ = tx.send(json);  // broadcast ke semua client
                tracing::info!("Sent predictions via WS: {:?}", msg.preds);
            }

            fn calc_amount_by_pair(start: u64, lose: u32) -> u64 {
                // setiap 2 kali kalah = 1 step
                let steps = (lose + 1) / 2;
            
                // kalikan start dengan 10 untuk setiap step, gunakan saturating_mul agar tidak panic on overflow
                let mut amount = start;
                for _ in 0..steps {
                    amount = amount.saturating_mul(10);
                }
            
                amount
            }

            let amount = if lose > 0 { 100_000 } else { 10_000 };

            // let mut amount = 10_000 * 10u64.pow(lose);
            // let step = lose / 2;
            // for i in 0..step {
            //     if i % 2 == 0 {
            //         amount *= 5;
            //     } else {
            //         amount *= 2;
            //     }
            // }
            println!("Setelah kalah {lose} kali, amount = {amount}");
            
            // for path in &paths {
            //     match try_checkpoint_and_deploy(&connection, board.round_id, amount, &preds, path).await {
            //         Ok(DeployOutcome::Deployed(sig)) => {
            //             last_deployed_round = Some(board.round_id);
            //             println!("Deployed for round {} sig {}", board.round_id, sig);
            //         }
            //         Ok(DeployOutcome::Skipped) => {
            //             tracing::info!("Skipped deploy attempt for round {} (path {}) - will retry next loop", board.round_id, path);
            //             continue;
            //         }
            //         Err(e) => {
            //             tracing::error!("Unexpected error in checkpoint/deploy flow (path {}): {:?}", path, e);
            //             continue;
            //         }
            //     }
            // }
        }

        // wait for RNG result (same logic as before)
        let last_deployable_slot = board.end_slot;
        let current_slot = if let Ok(s) = connection.get_slot().await { s } else {
            tracing::error!("Failed to get slot from rpc");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let slots_left_in_round = last_deployable_slot as i64 - current_slot as i64;

        tokio::time::sleep(Duration::from_secs(1)).await;

        let round_key = &round_pda(board.round_id).0;
        let start_wait = Instant::now();

        if slots_left_in_round < 0 {
            let round: Round = loop {
                match connection.get_account_data(&round_key).await {
                    Ok(data) if !data.is_empty() => match Round::try_from_bytes(&data) {
                        Ok(r_ref) => {
                            let round_owned = r_ref.clone();
                            if let Some(rng) = round_owned.rng() {
                                tracing::info!("✅ Round {} RNG available after {}s (rng={})", board.round_id, start_wait.elapsed().as_secs(), rng);
                                break round_owned;
                            } else {
                                tracing::info!("⌛ Round {} still missing slot_hash... waiting 5s", board.round_id);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("⚠️ Failed to parse Round {}: {:?}, retrying in 5s...", board.round_id, e);
                        }
                    },
                    Ok(_) => {
                        tracing::info!("ℹ️ Round account {} empty, waiting 5s...", board.round_id);
                    }
                    Err(e) => {
                        tracing::warn!("⚠️ RPC error fetching round {}: {:?}, retrying in 5s...", board.round_id, e);
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            };

            if let Some(rng) = round.rng() {
                let winning_square = round.winning_square(rng) as usize;
                total += 1;
                
                tracing::info!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);

                let mut squares: Vec<usize> = Vec::new();
                squares.push(winning_square);

                // scoring
                let hit_pred = preds.contains(&winning_square);
                if hit_pred { total_hit += 1; }
                println!("Win Block : {}", winning_square);
                let wr = (total_hit as f64 / total as f64) * 100.0;
                let top4 = top_n_frequent(&history, 4);
                println!("Top4 freq: {:?}", top4);
                println!("Result: {} | WR: {:.2}% ({}/{})\n", if hit_pred { "✅" } else { "❌" }, wr, total_hit, total);

                let msg = AccuracyMsg {
                    r#type: "winning",
                    preds: squares,
                    status: "done",
                    accuracy: wr,
                    total_round: total,
                    total_win: total_hit
                };
    
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = tx.send(json);  // broadcast ke semua client
                    tracing::info!("Sent predictions via WS: {:?}", msg.preds);
                }

                if buffer.len() >= window { buffer.pop_front(); }
                buffer.push_back(winning_square);
                history.push(winning_square);
                
                if history.len() > 50 {
                    history.drain(0..(history.len() - 50));
                }

                if hit_pred {
                    if lose > 0 { lose -= 1; }
                } else {
                    lose += 1;
                }
                rounds_since_retrain += 1;
                if rounds_since_retrain >= retrain_every {
                    rounds_since_retrain = 0;
                    let train_seq = if history.len() > 50 { &history[history.len()-50..] } else { &history[..] };
                    println!("Retraining HMM on {} obs...", train_seq.len());
                    hmm_model = train_hmm(train_seq, n_states, n_iter);
                    println!("Retrain done.");
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
                let denom = round.deployed[winning_square];
                if denom == 0 {
                    (Some(winning_square), None, Some(denom))
                } else {
                    let top_sample = if round.top_miner == SPLIT_ADDRESS {
                        None
                    } else {
                        Some(round.top_miner_sample(rng, winning_square))
                    };
                    (Some(winning_square), top_sample, Some(denom))
                }
            } else {
                tracing::error!("Failed to get round rng for round {}", round.id);
                (None, None, None)
            };
        } else {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

fn top_n_frequent(data: &[usize], n: usize) -> Vec<(usize, usize)> {
    let mut freq: HashMap<usize, usize> = HashMap::new();
    for &val in data {
        *freq.entry(val).or_insert(0) += 1;
    }

    let mut pairs: Vec<(usize, usize)> = freq.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1)); // sort descending by frequency
    pairs.into_iter().take(n).collect()
}
