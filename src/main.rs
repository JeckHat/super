use std::{env, fs::{File, OpenOptions}, io::{BufReader, Read}, str::FromStr, sync::Arc, time::{Duration, Instant}};

use anyhow::{Context, anyhow, bail};
use sqlx::sqlite::SqliteConnectOptions;
use thiserror::Error;
use axum::{body::Body, extract::{Path, Query, State}, http::{Request, Response, StatusCode}, middleware::{self, Next}, routing::get, Json, Router};
use const_crypto::ed25519;
use ore_api::{consts::{BOARD, ROUND, SPLIT_ADDRESS, TREASURY_ADDRESS}, state::{Board, Miner, Round, Treasury, round_pda}};
use serde::{Deserialize, Serialize};
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_filter::RpcFilterType};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use steel::{AccountDeserialize, Pubkey};
use tokio::{signal, sync::{Mutex, RwLock}};
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

use crate::{app_state::{AppBoard, AppMiner, AppRound, AppState, AppTreasury}, database::{CreateDeployment, DbMinerSnapshot, DbTreasury, MinerLeaderboardRow, MinerOreLeaderboardRow, MinerTotalsRow, RoundRow, get_deployments_by_round}, ore_env::fetch_ore_env, rpc::{DeployOutcome, evaluate_ev_only, infer_refined_ore, try_checkpoint_and_deploy, try_claim_sol}};

const PROGRAM_ID: [u8; 32] = unsafe { *(&ore_api::id() as *const Pubkey as *const [u8; 32]) };

pub const BOARD_ADDRESS: Pubkey =
    Pubkey::new_from_array(ed25519::derive_program_address(&[BOARD], &PROGRAM_ID).0);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().expect("Failed to load env");

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();

    tracing::info!("Running migrations...");

    let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set");
    let prefix = "https://".to_string();
    let connection = RpcClient::new_with_commitment(prefix + &rpc_url, CommitmentConfig { commitment: CommitmentLevel::Confirmed });

    let treasury = if let Ok(treasury) = connection.get_account_data(&TREASURY_ADDRESS).await {
        if let Ok(treasury) = Treasury::try_from_bytes(&treasury) {
            treasury.clone()
        } else {
            bail!("Failed to parse Treasury account");
        }
    } else {
        bail!("Failed to load treasury account data");
    };

    // Sleep between RPC Calls
    tokio::time::sleep(Duration::from_secs(1)).await;

    let board = if let Ok(board) = connection.get_account_data(&BOARD_ADDRESS).await {
        if let Ok(board) = Board::try_from_bytes(&board) {
            board.clone()
        } else {
            bail!("Failed to parse Board account");
        }
    } else {
        bail!("Failed to load board account data");
    };
    tokio::time::sleep(Duration::from_secs(1)).await;

    let out_path = "output.txt";
    let need_header = !FsPath::new(out_path).exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out_path)
        .context("error path")?;

    if need_header {
        writeln!(file, "round,angka_valid")?;
    }

    let board_data = connection.get_account_data(&BOARD_ADDRESS).await?;
    let board = Board::try_from_bytes(&board_data)?;
    let start_round = &board.round_id - 300;
    for i in start_round..=board.round_id {
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
                            writeln!(file, "{},{}", i, winning_square)?;
                        }
                    },
                    Err(e) => {

                    }
                }
                // let data = account.data;
                // Anchor accounts usually have 8-byte discriminator at start
                // let offset = 8usize;
                // let expected_size = size_of::<Round>();
                // if data.len() < offset + expected_size {
                //     eprintln!("Account {} data too small for Round (id {})", pda, id);
                //     continue;
                // }

                // let slice = &data[offset..offset + expected_size];
                // Safety: Round is marked Pod and Zeroable; ensure on-chain layout exactly matches.
                // let round: &Round = Round::try_from_bytes(&data);

                // Ambil waktu dari expires_at (diasumsikan unix timestamp detik)
                // let ts = round.expires_at;
                // let dt = NaiveDateTime::from_timestamp_opt(ts as i64, 0)
                //     .map(|n| DateTime::<Utc>::from_utc(n, Utc))
                //     .unwrap_or_else(|| Utc::now()); // fallback kalau nilai tidak valid

                // let timestamp_str = dt.format("%Y-%m-%d %H:%M:%S").to_string();

                
            }
            Err(err) => {
                eprintln!("Failed getting account for id {}: {}", i, err);
                // lanjut ke id berikutnya
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    

    update_data_system_all(connection).await;

    // let state = app_state.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await?;

    tracing::debug!("Listening on {}", listener.local_addr()?);

    Ok(())
}

pub async fn update_data_system_all(connection: RpcClient) {
    tracing::info!("Starting update_data_system (Markov-2)");

    use std::fs::File;
    use std::io::{BufRead, BufReader};

    // bring Markov2 into scope
    use crate::markov_chain::Markov2;

    fn load_history(path: &str) -> Vec<usize> {
        let f = File::open(path);
        if f.is_err() { return vec![]; }
        let mut content = String::new();
        if let Err(_) = std::fs::read_to_string(path).and_then(|s| { content = s; Ok(()) }) { return vec![]; }

        // detect one-based
        let mut one_based = false;
        for (i, line) in content.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() { continue; }
            if i == 0 && l.to_lowercase().contains("round") && l.to_lowercase().contains("angka") { continue; }
            let parts: Vec<&str> = l.split(',').collect();
            let tok = if parts.len() >= 2 { parts[1].trim() } else { parts[0].trim() };
            if let Ok(v) = tok.parse::<i64>() {
                if v > 24 { one_based = true; break; }
            }
        }

        let mut out = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() { continue; }
            if i == 0 && l.to_lowercase().contains("round") && l.to_lowercase().contains("angka") { continue; }
            let parts: Vec<&str> = l.split(',').collect();
            let tok = if parts.len() >= 2 { parts[1].trim() } else { parts[0].trim() };
            if let Ok(v) = tok.parse::<i64>() {
                if one_based {
                    if (1..=25).contains(&v) { out.push((v - 1) as usize); }
                } else {
                    if (0..=24).contains(&v) { out.push(v as usize); }
                }
            }
        }
        out
    }

    // load last N rounds
    let mut history = load_history("output.txt");
    let last_n = 300usize;
    if history.len() > last_n {
        history = history.split_off(history.len() - last_n);
    }

    // build Markov2 and train
    let mut mc = Markov2::new(1.0);
    if !history.is_empty() {
        mc.train(&history);
        tracing::info!("Markov2 trained on {} entries", history.len());
    } else {
        tracing::warn!("No history found; Markov2 untrained (will use marginal fallback)");
    }

    // bookkeeping similar to original
    let mut total = 0usize;
    let mut total_win_pred = 0.0;
    let mut total_win_logic = 0.0;
    let mut pred: Vec<usize> = vec![];
    let logic = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24];
    let mut win = 0i32;
    let mut lose = 0u32;
    let paths = [
        "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json",
        "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json",
        "/Users/jeckhat/gawean/jeckhat/miners/mebest.json",
        "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json",
    ];

    // last two observed values for conditioning
    let mut last1: Option<usize> = history.last().copied();
    let mut last2: Option<usize> = if history.len() >= 2 { Some(history[history.len()-2]) } else { None };

    // tokio::spawn(async move {
        let mut last_deployed_round = None;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            // fetch board
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
                last_deployed_round = Some(board.round_id);
                println!("round {}", board.round_id);
                total += 1;

                // produce prediction using Markov-2 if we have two previous observations
                pred.clear();
                if let (Some(a), Some(b)) = (last2, last1) {
                    pred = mc.predict(a, b, 18, 1.0); // temperature 1.0 default
                    if pred.len() < 18 {
                        // fill from marginal if needed
                        let mut fill = mc.marginal_topk(18);
                        fill.retain(|x| !pred.contains(x));
                        pred.extend(fill.into_iter().take(18 - pred.len()));
                    }
                } else if let Some(b) = last1 {
                    // if only one previous, fallback to marginal weighted but prefer neighbors
                    pred = mc.marginal_topk(18);
                } else {
                    // cold-start uniform top-18 (0..17) — but better use marginal if present
                    pred = (0..25usize).take(18).collect();
                }

                pred.sort_unstable();
                println!("Prediksi (0-based): {:?}", pred);

                let amount = 10_000 * 10u64.pow(lose);

                // deploy/ev logic (preserve previous behavior but use pred)
                match fetch_ore_env(&connection, BOARD_ADDRESS, ore_api::id()).await {
                    Ok(env) => {
                        let (ev_slots, should_deploy) = evaluate_ev_only(&env, 0.0);

                        // simple rule: if we have any training then use pred, else use should_deploy+logic
                        let trained = !mc.counts.is_empty();
                        if trained {
                            for path in &paths {
                                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, path).await {
                                    Ok(DeployOutcome::Deployed(sig)) => {
                                        last_deployed_round = Some(board.round_id);
                                        println!("Deployed for round {} sig {}", board.round_id, sig);
                                    }
                                    Ok(DeployOutcome::Skipped) => {
                                        tracing::info!("Skipped deploy attempt for round {} (path {}) - will retry next loop", board.round_id, path);
                                        continue;
                                    }
                                    Err(e) => {
                                        tracing::error!("Unexpected error in checkpoint/deploy flow (path {}): {:?}", path, e);
                                        continue;
                                    }
                                }
                            }
                        } else {
                            if should_deploy {
                                for path in &paths {
                                    match try_checkpoint_and_deploy(&connection, board.round_id, 10_000, &logic, path).await {
                                        Ok(DeployOutcome::Deployed(sig)) => {
                                            last_deployed_round = Some(board.round_id);
                                            println!("Deployed for round {} sig {}", board.round_id, sig);
                                        }
                                        Ok(DeployOutcome::Skipped) => {
                                            tracing::info!("Skipped deploy attempt for round {} (path {}) - will retry next loop", board.round_id, path);
                                            continue;
                                        }
                                        Err(e) => {
                                            tracing::error!("Unexpected error in checkpoint/deploy flow (path {}): {:?}", path, e);
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => println!("❌ Gagal ambil data: {:?}", e),
                }
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
                    tracing::info!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);

                    // scoring
                    let hit_pred = pred.contains(&winning_square);
                    let hit_logic = logic.contains(&winning_square);
                    println!("Prediksi  : {:?}", pred);
                    println!("Win Block : {}", winning_square);
                    println!("Hasil AI  : {}", if hit_pred { "✅ BENAR" } else { "❌ SALAH" });
                    println!("Hasil ME  : {}", if hit_logic { "✅ BENAR" } else { "❌ SALAH" });

                    if hit_pred {
                        total_win_pred += 1.0;
                        win += 1;
                        if lose > 0 { lose = 0; }
                    } else {
                        lose += 1;
                        win = 0;
                    }
                    if hit_logic {
                        total_win_logic += 1.0;
                    }

                    println!("WR AI  : {:.2}%", ((total_win_pred as f64 / total as f64) * 100.0));
                    println!("WR ME  : {:.2}%", ((total_win_logic as f64 / total as f64) * 100.0));

                    tokio::time::sleep(Duration::from_secs(20)).await;

                    if win > 0 {
                        win = 0;
                        for path in &paths {
                            match try_claim_sol(&connection, path).await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    tracing::info!("Claim submitted: {}", sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                    continue;
                                }
                                Err(e) => {
                                    tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                        }
                    }

                    // Update Markov with the observed transition
                    if let (Some(p2), Some(p1)) = (last2, last1) {
                        mc.update(p2, p1, winning_square);
                    }
                    // shift history
                    last2 = last1;
                    last1 = Some(winning_square);

                    println!("(Markov2) updated transition, now contexts: {}", mc.counts.len());

                    // denom etc same as before
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
    // });
}
