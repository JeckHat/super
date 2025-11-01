
use std::{sync::Arc, time::Duration};

use ore_api::{consts::SPLIT_ADDRESS, sdk::{checkpoint, claim_sol, deploy}, state::{Board, Miner, Round, Treasury, miner_pda, round_pda}};
use solana_client::{nonblocking::rpc_client::RpcClient};
use solana_sdk::{commitment_config::CommitmentConfig, message::Message, signature::{Keypair, Signature}, signer::Signer, transaction::Transaction};
use steel::{AccountDeserialize, Instruction, Numeric};
use tokio::time::Instant;
use anyhow::Result;

use crate::{BOARD_ADDRESS, PROGRAM_ID, app_state::AppState, ev::compute_ev_star_for_block, ore_env::{OreEnv, fetch_ore_env}, slot_miner::SlotMiner};
use crate::ai::HybridPredictor;

pub async fn update_data_system(connection: RpcClient, app_state: AppState) {
    println!("Starting...");
    // tracing::info!("Starting update_data_system");
    let mut model = HybridPredictor::new();
    let mut total = 0;
    let mut total_win_pred = 0.0;
    let mut total_win_pred2 = 0.0;
    // let mut pred = model.predict_hybrid();
    let (mut pred, mut pred2) = model.predict_two_alt();
    let mut ev1: f64 = 0.0;
    let mut ev2: f64 = 0.0;
    let logic = [0, 1, 2, 4, 5, 6, 7, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 24];
    let mut win_1 = 0;
    let mut win_2 = 0;
    let mut lose = 0;
    let mut negative_sol = 0;
    let mut positive_sol = 0;
    tokio::spawn(async move {
        let mut last_deployed_round = None;
        loop {

            tokio::time::sleep(Duration::from_secs(1)).await;

            let board = if let Ok(board) = connection.get_account_data(&BOARD_ADDRESS).await {
                if let Ok(board) = Board::try_from_bytes(&board) {
                    board.clone()
                } else {
                    println!("Failed to parse Board account");
                    // tracing::error!("Failed to parse Board account");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            } else {
                println!("Failed to load board account data");
                // tracing::error!("Failed to load board account data");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };

            if last_deployed_round != Some(board.round_id) {
                last_deployed_round = Some(board.round_id);
                println!("\n\nround {}", board.round_id);
                // (pred, pred2) = model.predict_two_alt();
                total += 1;

                let amount = if lose >= 1 {
                    10_000
                } else {
                    10_000
                };

                negative_sol += amount * 4;

                tokio::time::sleep(Duration::from_secs(20)).await;

                match fetch_ore_env(&connection, BOARD_ADDRESS, ore_api::id()).await {
                    Ok(env) => {
                        (pred, pred2) = model.predict_two_alt();

                        ev1 = 3.0;
                        ev2 = 3.0;
                        println!("Predictions 1 : {:?}", pred);
                        println!("Predictions 2 : {:?} \n", pred2);
            
                        println!("🎯 ALT1 EV = {:.6} | ALT2 EV = {:.6}", ev1, ev2);            

                        if ev1 > 0.0 {
                            println!("✅ Miner 1 EV positif ({:.6}) — deploy ALT1", ev1);
                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    println!("Skipped");
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    println!("Skipped");
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                        } else {
                            println!("⚠️ Miner 1 skip ronde (EV negatif)");
                        }
            
                        // 🚀 Miner 2
                        if ev2 > 0.0 {
                            println!("✅ Miner 2 EV positif ({:.6}) — deploy ALT2", ev2);
                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred2, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    println!("Skipped");
                                    // tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    // tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred2, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    println!("Skipped");
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                        } else {
                            println!("⚠️ Miner 2 skip ronde (EV negatif)");
                        }
                    }
                    Err(e) => println!("❌ Gagal ambil data: {:?}", e),
                }

            }

            // update board
            let r = app_state.board.clone();
            let mut l = r.write().await;
            *l = board.into();
            drop(l);

            let last_deployable_slot = board.end_slot;
            let current_slot = if let Ok(current_slot) = connection.get_slot().await {
                current_slot
            } else {
                println!("Failed to get slot from rpc");
                // tracing::error!("Failed to get slot from rpc");
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
                                    println!(
                                        "✅ Round {} RNG available after {}s (rng={})",
                                        board.round_id,
                                        start_wait.elapsed().as_secs(),
                                        rng
                                    );
                                    // tracing::info!(
                                    //     "✅ Round {} RNG available after {}s (rng={})",
                                    //     board.round_id,
                                    //     start_wait.elapsed().as_secs(),
                                    //     rng
                                    // );
                                    break round_owned;
                                } else {
                                    println!(
                                        "⌛ Round {} still missing slot_hash... waiting 5s",
                                        board.round_id
                                    );
                                    // tracing::info!(
                                    //     "⌛ Round {} still missing slot_hash... waiting 5s",
                                    //     board.round_id
                                    // );
                                }
                            }
                            Err(e) => {
                                println!(
                                   "⚠️ Failed to parse Round {}: {:?}, retrying in 5s...",
                                    board.round_id,
                                    e
                                );
                                // tracing::warn!(
                                //     "⚠️ Failed to parse Round {}: {:?}, retrying in 5s...",
                                //     board.round_id,
                                //     e
                                // );
                            }
                        },
                        Ok(_) => {
                            println!(
                                "ℹ️ Round account {} empty, waiting 5s...",
                                board.round_id
                            );
                            // tracing::info!(
                            //     "ℹ️ Round account {} empty, waiting 5s...",
                            //     board.round_id
                            // );
                        }
                        Err(e) => {
                            println!(
                                "⚠️ RPC error fetching round {}: {:?}, retrying in 5s...",
                                board.round_id,
                                e
                            );
                            // tracing::warn!(
                            //     "⚠️ RPC error fetching round {}: {:?}, retrying in 5s...",
                            //     board.round_id,
                            //     e
                            // );
                        } 
                    }
            
                    tokio::time::sleep(Duration::from_secs(5)).await; // <== POLL SETIAP 5 DETIK
                };
                
                // compute rng once and branch on it
                if let Some(rng) = round.rng() {
                    // winning square (0..24)
                    let winning_square = round.winning_square(rng) as usize;
                
                    // debug logging
                    println!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);
                    // tracing::info!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);
                
                    // check whether our prediction hit
                    let hit_pred = pred.contains(&winning_square);
                    let hit_pred2 = pred2.contains(&winning_square);
                    println!("Predictions CNN  : {:?}", pred);
                    println!("Predictions EC  : {:?}", pred2);
                    println!("Win Block : {}", winning_square);
                    println!("AI (CNN) Result  : {}", if hit_pred { "✅ Correct" } else { "❌ Incorrect" });
                    println!("AI (EC)  Result : {}", if hit_pred2 { "✅ Correct" } else { "❌ incorrect" });
                    
                    if hit_pred {
                        total_win_pred += 1.0;
                        win_1 += 1;
                        if lose > 0 {
                            lose -= 1;
                        }
                    } else {
                        lose += 1;
                        win_1 = 0;
                    }

                    if hit_pred2 {
                        total_win_pred2 += 1.0;
                        win_2 += 1;
                    } else {
                        win_2 = 0;
                    }

                    println!("AI (CNN)  : {:.2}%", ((total_win_pred as f64 / total as f64) * 100.0));
                    println!("AI (EC)   : {:.2}%", ((total_win_pred2 as f64 / total as f64) * 100.0));
                    println!("number of round: {:}\n", "82");

                    if win_1 > 1 {
                        win_1 = 0;
                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                println!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                println!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                println!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                println!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }
                        
                    }

                    if win_2 > 0 {
                        win_2 = 0;
                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                println!("Claim submitted: {}", sig);
                                // tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                println!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                // tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                // tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                println!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                println!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }
                    }
                
                    model.update(winning_square, &pred);
                
                    // denom: total deployed on the winning square
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
                    // no RNG available
                    println!("Failed to get round rng for round {}", round.id);
                    // tracing::error!("Failed to get round rng for round {}", round.id);
                    (None, None, None)
                };
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }


        }
    });
}

pub async fn update_data_system_hybrid(connection: RpcClient, app_state: AppState) {
    // tracing::info!("Starting update_data_system");
    println!("Starting...");
    let mut model = HybridPredictor::new();
    let mut total = 0;
    let mut total_win_pred = 0.0;
    let mut total_win_logic = 0.0;
    // let mut pred = model.predict_hybrid();
    let (mut pred, mut pred2, mut pred3) = model.predict_3way();
    let logic = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24];
    let mut win = 0;
    let mut lose = 0;
    tokio::spawn(async move {
        let mut last_deployed_round = None;
        loop {

            tokio::time::sleep(Duration::from_secs(1)).await;

            let board = if let Ok(board) = connection.get_account_data(&BOARD_ADDRESS).await {
                if let Ok(board) = Board::try_from_bytes(&board) {
                    board.clone()
                } else {
                    println!("Failed to parse Board account");
                    // tracing::error!("Failed to parse Board account");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            } else {
                println!("Failed to parse Board account data");
                // tracing::error!("Failed to load board account data");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };

            if last_deployed_round != Some(board.round_id) {
                last_deployed_round = Some(board.round_id);
                println!("round {}", board.round_id);
                (pred, pred2, pred3) = model.predict_3way();
                total += 1;
                println!("Prediksi : {:?}", pred);

                let amount = if lose >= 1 {
                    10_000
                } else {
                    10_000
                };

                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                    Ok(DeployOutcome::Deployed(sig)) => {
                        // sukses -> tandai last_deployed_round
                        last_deployed_round = Some(board.round_id);
                        println!("Deployed for round {} sig {}", board.round_id, sig);
                    }
                    Ok(DeployOutcome::Skipped) => {
                        // kode sebelumnya banyak 'continue' diganti dengan ini
                        println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                        // tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                        continue; // keep old behavior: lanjut loop utama
                    }
                    Err(e) => {
                        println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                        // tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                        continue;
                    }
                }

                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred2, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                    Ok(DeployOutcome::Deployed(sig)) => {
                        // sukses -> tandai last_deployed_round
                        last_deployed_round = Some(board.round_id);
                        println!("Deployed for round {} sig {}", board.round_id, sig);
                    }
                    Ok(DeployOutcome::Skipped) => {
                        // kode sebelumnya banyak 'continue' diganti dengan ini
                        println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                        continue; // keep old behavior: lanjut loop utama
                    }
                    Err(e) => {
                        println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                        continue;
                    }
                }

                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred3, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                    Ok(DeployOutcome::Deployed(sig)) => {
                        // sukses -> tandai last_deployed_round
                        last_deployed_round = Some(board.round_id);
                        println!("Deployed for round {} sig {}", board.round_id, sig);
                    }
                    Ok(DeployOutcome::Skipped) => {
                        // kode sebelumnya banyak 'continue' diganti dengan ini
                        println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                        continue; // keep old behavior: lanjut loop utama
                    }
                    Err(e) => {
                        println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                        continue;
                    }
                }

                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                    Ok(DeployOutcome::Deployed(sig)) => {
                        // sukses -> tandai last_deployed_round
                        last_deployed_round = Some(board.round_id);
                        println!("Deployed for round {} sig {}", board.round_id, sig);
                    }
                    Ok(DeployOutcome::Skipped) => {
                        // kode sebelumnya banyak 'continue' diganti dengan ini
                        println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                        continue; // keep old behavior: lanjut loop utama
                    }
                    Err(e) => {
                        println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                        continue;
                    }
                }

            }

            // update board
            let r = app_state.board.clone();
            let mut l = r.write().await;
            *l = board.into();
            drop(l);

            let last_deployable_slot = board.end_slot;
            let current_slot = if let Ok(current_slot) = connection.get_slot().await {
                current_slot
            } else {
                println!("Failed to get slot from rpc");
                // tracing::error!("Failed to get slot from rpc");
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
                                    println!(
                                        "✅ Round {} RNG available after {}s (rng={})",
                                        board.round_id,
                                        start_wait.elapsed().as_secs(),
                                        rng
                                    );
                                    // tracing::info!(
                                    //     "✅ Round {} RNG available after {}s (rng={})",
                                    //     board.round_id,
                                    //     start_wait.elapsed().as_secs(),
                                    //     rng
                                    // );
                                    break round_owned;
                                } else {
                                    println!(
                                        "⌛ Round {} still missing slot_hash... waiting 5s",
                                        board.round_id
                                    );
                                    // tracing::info!(
                                    //     "⌛ Round {} still missing slot_hash... waiting 5s",
                                    //     board.round_id
                                    // );
                                }
                            }
                            Err(e) => {
                                println!(
                                   "⚠️ Failed to parse Round {}: {:?}, retrying in 5s...",
                                    board.round_id,
                                    e
                                );
                                // tracing::warn!(
                                //     "⚠️ Failed to parse Round {}: {:?}, retrying in 5s...",
                                //     board.round_id,
                                //     e
                                // );
                            }
                        },
                        Ok(_) => {
                            println!(
                                "ℹ️ Round account {} empty, waiting 5s...",
                                board.round_id
                            );
                            // tracing::info!(
                            //     "ℹ️ Round account {} empty, waiting 5s...",
                            //     board.round_id
                            // );
                        }
                        Err(e) => {
                            println!(
                                "⚠️ RPC error fetching round {}: {:?}, retrying in 5s...",
                                board.round_id,
                                e
                            );
                            // tracing::warn!(
                            //     "⚠️ RPC error fetching round {}: {:?}, retrying in 5s...",
                            //     board.round_id,
                            //     e
                            // );
                        } 
                    }
            
                    tokio::time::sleep(Duration::from_secs(5)).await; // <== POLL SETIAP 5 DETIK
                };
                
                // compute rng once and branch on it
                if let Some(rng) = round.rng() {
                    // winning square (0..24)
                    let winning_square = round.winning_square(rng) as usize;
                
                    // debug logging
                    println!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);
                    // tracing::info!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);
                
                    // check whether our prediction hit
                    let hit_pred = pred.contains(&winning_square);
                    let hit_logic = logic.contains(&winning_square);
                    println!("Prediksi  : {:?}", pred);
                    println!("Win Block : {}", winning_square);
                    println!("Hasil AI  : {}", if hit_pred { "✅ BENAR" } else { "❌ SALAH" });
                    println!("Hasil ME  : {}", if hit_logic { "✅ BENAR" } else { "❌ SALAH" });
                    
                    if hit_pred {
                        total_win_pred += 1.0;
                        win += 1;
                        if lose > 0 {
                            lose -= 1;
                        }
                    } else {
                        lose += 1;
                        win = 0;
                    }

                    if hit_logic {
                        total_win_logic += 1.0;
                    }
                    println!("WR AI  : {:.2}%", ((total_win_pred as f64 / total as f64) * 100.0));
                    println!("WR ME  : {:.2}%", ((total_win_logic as f64 / total as f64) * 100.0));

                    if win >= 1 {
                        win = 0;
                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }
                        
                    }
                
                    model.update(winning_square, &pred);
                    println!("accuracy: {:.2}%", model.accuracy());
                    println!("Total count: {}", total);
                
                    // denom: total deployed on the winning square
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
                    // no RNG available
                    tracing::error!("Failed to get round rng for round {}", round.id);
                    (None, None, None)
                };
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }


        }
    });
}


pub async fn update_data_system_all(connection: RpcClient, app_state: AppState) {
    tracing::info!("Starting update_data_system");
    let mut model = HybridPredictor::new();
    let mut total = 0;
    let mut total_win_pred = 0.0;
    let mut total_win_logic = 0.0;
    // let mut pred = model.predict_hybrid();
    let mut pred = model.predict_hybrid();
    let logic = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24];
    let mut win = 0;
    let mut lose = 0;
    tokio::spawn(async move {
        let mut last_deployed_round = None;
        loop {

            tokio::time::sleep(Duration::from_secs(1)).await;

            let board = if let Ok(board) = connection.get_account_data(&BOARD_ADDRESS).await {
                if let Ok(board) = Board::try_from_bytes(&board) {
                    board.clone()
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
                pred = model.predict_hybrid();
                total += 1;
                println!("Prediksi : {:?}", pred);

                let amount = if lose >= 1 {
                    10_000
                } else {
                    10_000
                };

                tokio::time::sleep(Duration::from_secs(20)).await;

                match fetch_ore_env(&connection, BOARD_ADDRESS, ore_api::id()).await {
                    Ok(env) => {
                        let (ev_slots, should_deploy) = evaluate_ev_only(&env, 0.2);

                        if (model.accuracy() >= 80.0) {
                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    // kode sebelumnya banyak 'continue' diganti dengan ini
                                    tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }

                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    // kode sebelumnya banyak 'continue' diganti dengan ini
                                    println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }

                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    // kode sebelumnya banyak 'continue' diganti dengan ini
                                    tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }

                            match try_checkpoint_and_deploy(&connection, board.round_id, amount, &pred, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                                Ok(DeployOutcome::Deployed(sig)) => {
                                    // sukses -> tandai last_deployed_round
                                    last_deployed_round = Some(board.round_id);
                                    println!("Deployed for round {} sig {}", board.round_id, sig);
                                }
                                Ok(DeployOutcome::Skipped) => {
                                    // kode sebelumnya banyak 'continue' diganti dengan ini
                                    tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                    continue; // keep old behavior: lanjut loop utama
                                }
                                Err(e) => {
                                    tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                    continue;
                                }
                            }
                        } else {
                            if (should_deploy) {
                                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &logic, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                                    Ok(DeployOutcome::Deployed(sig)) => {
                                        // sukses -> tandai last_deployed_round
                                        last_deployed_round = Some(board.round_id);
                                        println!("Deployed for round {} sig {}", board.round_id, sig);
                                    }
                                    Ok(DeployOutcome::Skipped) => {
                                        // kode sebelumnya banyak 'continue' diganti dengan ini
                                        tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                        continue; // keep old behavior: lanjut loop utama
                                    }
                                    Err(e) => {
                                        tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                        continue;
                                    }
                                }
            
                                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &logic, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                                    Ok(DeployOutcome::Deployed(sig)) => {
                                        // sukses -> tandai last_deployed_round
                                        last_deployed_round = Some(board.round_id);
                                        println!("Deployed for round {} sig {}", board.round_id, sig);
                                    }
                                    Ok(DeployOutcome::Skipped) => {
                                        // kode sebelumnya banyak 'continue' diganti dengan ini
                                        println!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                        continue; // keep old behavior: lanjut loop utama
                                    }
                                    Err(e) => {
                                        println!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                        continue;
                                    }
                                }
            
                                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &logic, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                                    Ok(DeployOutcome::Deployed(sig)) => {
                                        // sukses -> tandai last_deployed_round
                                        last_deployed_round = Some(board.round_id);
                                        println!("Deployed for round {} sig {}", board.round_id, sig);
                                    }
                                    Ok(DeployOutcome::Skipped) => {
                                        // kode sebelumnya banyak 'continue' diganti dengan ini
                                        tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                        continue; // keep old behavior: lanjut loop utama
                                    }
                                    Err(e) => {
                                        tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                        continue;
                                    }
                                }
            
                                match try_checkpoint_and_deploy(&connection, board.round_id, amount, &logic, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                                    Ok(DeployOutcome::Deployed(sig)) => {
                                        // sukses -> tandai last_deployed_round
                                        last_deployed_round = Some(board.round_id);
                                        println!("Deployed for round {} sig {}", board.round_id, sig);
                                    }
                                    Ok(DeployOutcome::Skipped) => {
                                        // kode sebelumnya banyak 'continue' diganti dengan ini
                                        tracing::info!("Skipped deploy attempt for round {} - will retry next loop", board.round_id);
                                        continue; // keep old behavior: lanjut loop utama
                                    }
                                    Err(e) => {
                                        tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                        continue;
                                    }
                                }
                            }
                        }
                    },
                    Err(e) => println!("❌ Gagal ambil data: {:?}", e),
                }
            }

            // update board
            let r = app_state.board.clone();
            let mut l = r.write().await;
            *l = board.into();
            drop(l);

            let last_deployable_slot = board.end_slot;
            let current_slot = if let Ok(current_slot) = connection.get_slot().await {
                current_slot
            } else {
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
                                    tracing::info!(
                                        "✅ Round {} RNG available after {}s (rng={})",
                                        board.round_id,
                                        start_wait.elapsed().as_secs(),
                                        rng
                                    );
                                    break round_owned;
                                } else {
                                    tracing::info!(
                                        "⌛ Round {} still missing slot_hash... waiting 5s",
                                        board.round_id
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "⚠️ Failed to parse Round {}: {:?}, retrying in 5s...",
                                    board.round_id,
                                    e
                                );
                            }
                        },
                        Ok(_) => {
                            tracing::info!(
                                "ℹ️ Round account {} empty, waiting 5s...",
                                board.round_id
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "⚠️ RPC error fetching round {}: {:?}, retrying in 5s...",
                                board.round_id,
                                e
                            );
                        } 
                    }
            
                    tokio::time::sleep(Duration::from_secs(5)).await; // <== POLL SETIAP 5 DETIK
                };
                
                // compute rng once and branch on it
                if let Some(rng) = round.rng() {
                    // winning square (0..24)
                    let winning_square = round.winning_square(rng) as usize;
                
                    // debug logging
                    tracing::info!("Round {} RNG present. rng={} winning_square={}", round.id, rng, winning_square);
                
                    // check whether our prediction hit
                    let hit_pred = pred.contains(&winning_square);
                    let hit_logic = logic.contains(&winning_square);
                    println!("Prediksi  : {:?}", pred);
                    println!("Win Block : {}", winning_square);
                    println!("Hasil AI  : {}", if hit_pred { "✅ BENAR" } else { "❌ SALAH" });
                    println!("Hasil ME  : {}", if hit_logic { "✅ BENAR" } else { "❌ SALAH" });
                    
                    if hit_pred {
                        total_win_pred += 1.0;
                        win += 1;
                        if lose > 0 {
                            lose -= 1;
                        }
                    } else {
                        lose += 1;
                        win = 0;
                    }

                    if hit_logic {
                        total_win_logic += 1.0;
                    }
                    println!("WR AI  : {:.2}%", ((total_win_pred as f64 / total as f64) * 100.0));
                    println!("WR ME  : {:.2}%", ((total_win_logic as f64 / total as f64) * 100.0));

                    if win > 1 {
                        win = 0;
                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/poolminer3.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/mebest.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }

                        match try_claim_sol(&connection, "/Users/jeckhat/gawean/jeckhat/miners/meminer_1.json").await {
                            Ok(DeployOutcome::Deployed(sig)) => {
                                tracing::info!("Claim submitted: {}", sig);
                            }
                            Ok(DeployOutcome::Skipped) => {
                                // kode sebelumnya banyak 'continue' diganti dengan ini
                                tracing::info!("Skipped claim attempt for round {} - will retry next loop", board.round_id);
                                continue; // keep old behavior: lanjut loop utama
                            }
                            Err(e) => {
                                tracing::error!("Unexpected error in checkpoint/deploy flow: {:?}", e);
                                continue;
                            }
                        }
                        
                    }
                
                    model.update(winning_square, &pred);
                    println!("accuracy: {:.2}%", model.accuracy());
                    println!("Total Winners: {}", total);
                
                    // denom: total deployed on the winning square
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
                    // no RNG available
                    tracing::error!("Failed to get round rng for round {}", round.id);
                    (None, None, None)
                };
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }


        }
    });
}


// pub async fn set_plan_miner(path: String, board: Board, pred: Vec<usize>) {
// let miner_path = Arc::new(SlotMiner::new(Some(String::from(path))));
// let signer_kp: Keypair = miner_path.signer();
// let signer_pubkey = signer_kp.pubkey();

// // compute miner PDA for this authority
// let miner_adr = miner_pda(signer_pubkey).0;

// // fetch miner on-chain (if exists)
// let mut miner_data: Option<Miner> = match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::confirmed()).await {
//     Ok(resp) => {
//         if let Some(acc) = resp.value {
//             match Miner::try_from_bytes(&acc.data) {
//                 Ok(m) => {
//                     Some(m.clone())
//                 }
//                 Err(_) => {
//                     tracing::error!("Failed to parse Miner account data");
//                     None
//                 }
//             }
//         } else {
//             tracing::info!("Miner account {} not found on-chain (will attempt checkpoint-create)", miner_adr);
//             None
//         }
//     }
//     Err(e) => {
//         tracing::error!("RPC error fetching miner account: {:?}", e);
//         None
//     }
// };

// // build squares from prediction
// let mut squares: [bool; 25] = [false; 25];
// for &idx in pred.iter() {
//     if idx < squares.len() {
//         squares[idx] = true;
//     } else {
//         tracing::warn!("Pred index out of range (ignored): {}", idx);
//     }
// }

// let mut ixs: Vec<Instruction> = Vec::new();

// if let Some(ref m) = miner_data {
//     // If the on-chain rule requires m.checkpoint_id == m.round_id,
//     // we must checkpoint with m.round_id and only if checkpoint_id != round_id.
//     let miner_round_id = m.round_id;
//     let need_checkpoint = m.checkpoint_id != miner_round_id;

//     if need_checkpoint {
//         tracing::info!("Miner exists but not checkpointed for its own round (miner.round_id = {}, miner.checkpoint_id = {}), adding checkpoint({})",
//                       miner_round_id, m.checkpoint_id, miner_round_id);
//         ixs.push(checkpoint(signer_pubkey, signer_pubkey, miner_round_id));
//     } else {
//         tracing::info!("Miner already checkpointed for its own round: {}", miner_round_id);
//     }
// } else {
//     // Miner account missing: we don't have m.round_id to use.
//     // Fallback: attempt checkpoint using board.round_id (this will initialize the miner
//     // if the program supports creating miner on first checkpoint). If your program
//     // requires a different behavior (e.g. specific initial round_id), change accordingly.
//     tracing::info!("Miner account missing; attempting checkpoint(create) using board.round_id = {}", board.round_id);
//     ixs.push(checkpoint(signer_pubkey, signer_pubkey, board.round_id));
// }

// // push deploy instruction always (we either checkpoint+deploy in same tx or only deploy)
// ixs.push(deploy(
//     signer_pubkey,
//     signer_pubkey,
//     10_000,
//     board.round_id,
//     squares,
// ));

// // get recent blockhash
// let recent_blockhash = match connection.get_latest_blockhash().await {
//     Ok(bh) => bh,
//     Err(e) => {
//         tracing::error!("Failed to get recent blockhash for deploy: {:?}", e);
//         tokio::time::sleep(Duration::from_secs(5)).await;
//         continue;
//     }
// };

// // build message & tx containing all instructions (atomic)
// let message = Message::new(&ixs, Some(&signer_pubkey_1));
// let mut tx = Transaction::new_unsigned(message);
// if let Err(e) = tx.try_sign(&[&signer_kp_1], recent_blockhash) {
//     tracing::error!("Failed to sign transaction: {:?}", e);
//     continue;
// }

// // send & confirm
// let sig = match connection.send_and_confirm_transaction(&tx).await {
//     Ok(sig) => {
//         println!("Transaction sent. Signature: {}", sig);
//         tracing::info!("Sent tx: {}", sig);
//         sig
//     }
//     Err(e) => {
//         tracing::error!("Failed to send tx: {:?}", e);
//         // allow retry next loop
//         continue;
//     }
// };

// // // fetch tx meta & program logs for debugging
// // match connection.get_transaction(&sig).await {
// //     Ok(Some(txinfo)) => {
// //         if let Some(meta) = txinfo.transaction.meta {
// //             tracing::info!("Tx status: {:?}", meta.status);
// //             if let Some(logs) = meta.log_messages {
// //                 tracing::info!("Program logs:\n{}", logs.join("\n"));
// //             }
// //         } else {
// //             tracing::warn!("No meta returned for tx {}", sig);
// //         }
// //     }
// //     Ok(None) => tracing::warn!("get_transaction returned None for sig {}", sig),
// //     Err(e) => tracing::error!("Failed to fetch tx info for {}: {:?}", sig, e),
// // }

// // After tx finalized, re-fetch miner with finalized commitment to verify checkpoint applied
// // (give a short delay to let validator index it)
// tokio::time::sleep(Duration::from_millis(1200)).await;
// match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::finalized()).await {
//     Ok(resp) => {
//         if let Some(acc) = resp.value {
//             match Miner::try_from_bytes(&acc.data) {
//                 Ok(miner_after) => {
//                     tracing::info!("Miner after tx: checkpoint_id = {}", miner_after.checkpoint_id);
//                     // if miner has checkpointed for this round then mark as deployed
//                     if miner_after.checkpoint_id == board.round_id {
//                         last_deployed_round = Some(board.round_id);
//                     } else {
//                         // if checkpoint not applied but deploy maybe applied — still mark as deployed
//                         // But safer: only mark if miner checkpoint == board.round_id
//                         tracing::warn!("Miner checkpoint_id after tx is {}, expected {}", miner_after.checkpoint_id, board.round_id);
//                         // If you want to be conservative, don't set last_deployed_round here
//                     }
//                 }
//                 Err(_) => tracing::warn!("Failed to parse miner after tx"),
//             }
//         } else {
//             tracing::warn!("Miner account not found after tx (miner_adr={})", miner_adr);
//         }
//     }
//     Err(e) => tracing::error!("Failed to re-fetch miner after tx: {:?}", e),
// }
// }

enum DeployOutcome {
    Deployed(Signature), // sukses, kembalikan signature
    Skipped,             // tidak jadi deploy -> lanjut loop utama (sama efek dengan `continue`)
}

async fn try_checkpoint_and_deploy(
    connection: &RpcClient,
    board_round: u64,
    amount: u64,
    pred: &[usize],
    keyfile_path: &str,
) -> Result<DeployOutcome> {
    // 1) create signer/miner
    let miner = Arc::new(SlotMiner::new(Some(keyfile_path.to_string())));
    let signer_kp: Keypair = miner.signer();
    let signer_pubkey = signer_kp.pubkey();

    // 2) miner PDA & fetch miner on-chain (owned)
    let miner_adr = miner_pda(signer_pubkey).0;
    let miner_data: Option<Miner> = match connection
        .get_account_with_commitment(&miner_adr, CommitmentConfig::confirmed())
        .await
    {
        Ok(resp) => {
            if let Some(acc) = resp.value {
                if acc.data.is_empty() {
                    println!("Miner account {} exists but empty.", miner_adr);
                    None
                } else {
                    match Miner::try_from_bytes(&acc.data) {
                        Ok(m_ref) => Some(m_ref.clone()),
                        Err(e) => {
                            println!("Failed parse miner {}: {:?}", miner_adr, e);
                            None
                        }
                    }
                }
            } else {
                println!("Miner account {} not found; will attempt checkpoint-create", miner_adr);
                None
            }
        }
        Err(e) => {
            println!("RPC error fetching miner {}: {:?}", miner_adr, e);
            // jika RPC error, skip iterasi supaya loop utama bisa retry
            return Ok(DeployOutcome::Skipped);
        }
    };

    // 3) build squares dari pred
    let mut squares: [bool; 25] = [false; 25];
    for &idx in pred.iter() {
        if idx < squares.len() {
            squares[idx] = true;
        } else {
            println!("Pred out of range: {}", idx);
        }
    }

    // 4) decide checkpoint instruction(s)
    let mut ixs: Vec<Instruction> = Vec::new();
    if let Some(ref m) = miner_data {
        // peraturan di program: m.checkpoint_id == m.round_id diperlukan
        let miner_round_id = m.round_id;
        if m.checkpoint_id != miner_round_id {
            println!("Adding checkpoint(miner_round_id={})", miner_round_id);
            ixs.push(checkpoint(signer_pubkey, signer_pubkey, miner_round_id));
        } else {
            println!("Miner already checkpointed for its round {}", miner_round_id);
            // tracing::info!("Miner already checkpointed for its round {}", miner_round_id);
        }
    } else {
        // miner missing -> attempt checkpoint using board round (init)
        println!("Miner missing -> checkpoint(board_round={})", board_round);
        // tracing::info!("Miner missing -> checkpoint(board_round={})", board_round);
        ixs.push(checkpoint(signer_pubkey, signer_pubkey, board_round));
    }

    // always add deploy
    ixs.push(deploy(signer_pubkey, signer_pubkey, amount, board_round, squares));

    // 5) get blockhash (jika gagal -> skip iterasi)
    let recent_blockhash = match connection.get_latest_blockhash().await {
        Ok(bh) => bh,
        Err(e) => {
            println!("Failed to get recent blockhash: {:?}", e);
            return Ok(DeployOutcome::Skipped);
        }
    };

    // 6) build/sign/send tx
    let message = Message::new(&ixs, Some(&signer_pubkey));
    let mut tx = Transaction::new_unsigned(message);

    if let Err(e) = tx.try_sign(&[&signer_kp], recent_blockhash) {
        println!("Failed to sign tx: {:?}", e);
        return Ok(DeployOutcome::Skipped);
    }

    match connection.send_and_confirm_transaction(&tx).await {
        Ok(sig) => {
            println!("Tx sent: {}", sig);
            // re-fetch miner finalized (cek checkpoint id) — jika mau
            // sleep sebentar agar validator index
            tokio::time::sleep(Duration::from_millis(1200)).await;
            match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::finalized()).await {
                Ok(resp) => {
                    if let Some(acc) = resp.value {
                        if !acc.data.is_empty() {
                            if let Ok(miner_after) = Miner::try_from_bytes(&acc.data) {
                                println!("Miner after tx: checkpoint_id={}", miner_after.checkpoint_id);
                                // jika ingin sangat aman: hanya treat sebagai deployed jika checkpoint_id == board_round
                                // tapi kita tetap kembalikan signature karena tx sukses
                            }
                        }
                    }
                }
                Err(e) => println!("Failed to re-fetch miner after tx: {:?}", e),
            }

            return Ok(DeployOutcome::Deployed(sig));
        }
        Err(e) => {
            println!("Failed to send tx ({}): {:?}", signer_pubkey.to_string(), e);
            return Ok(DeployOutcome::Skipped);
        }
    }
}

async fn try_claim_sol(
    connection: &RpcClient,
    keyfile_path: &str
) -> Result<DeployOutcome> {
    let miner = Arc::new(SlotMiner::new(Some(keyfile_path.to_string())));
    let signer_kp = miner.signer();
    let signer_pubkey = signer_kp.pubkey();

    // Compute miner PDA
    let miner_adr = ore_api::state::miner_pda(signer_pubkey).0;

    // Fetch miner account (confirmed)
    let miner_opt: Option<Miner> = match connection
        .get_account_with_commitment(&miner_adr, CommitmentConfig::confirmed())
        .await
    {
        Ok(resp) => {
            if let Some(acc) = resp.value {
                if acc.data.is_empty() {
                    println!("Miner account {} exists but data empty", miner_adr);
                    None
                } else {
                    match Miner::try_from_bytes(&acc.data) {
                        Ok(m_ref) => Some(m_ref.clone()), // clone into owned Miner
                        Err(e) => {
                            println!("Failed to deserialize Miner {}: {:?}", miner_adr, e);
                            None
                        }
                    }
                }
            } else {
                println!("Miner account {} not found on-chain", miner_adr);
                None
            }
        }
        Err(e) => {
            println!("RPC error fetching miner {}: {:?}", miner_adr, e);
            // Treat as transient: return Err so caller can decide (or return Ok(None) to skip)
            return Err(anyhow::anyhow!("RPC error fetching miner: {:?}", e));
        }
    };

    // If miner doesn't exist -> nothing to claim
    let miner = match miner_opt {
        Some(m) => m,
        None => {
            println!("No miner account / no data => nothing to claim");
            // tracing::info!("No miner account / no data => nothing to claim");
            return Ok(DeployOutcome::Skipped);
        }
    };

    // If miner.rewards_sol == 0 -> nothing to claim
    if miner.rewards_sol == 0 {
        println!("Miner {} has 0 rewards_sol -> skipping claim", signer_pubkey);
        // tracing::info!(
        //     "Miner {} has 0 rewards_sol -> skipping claim",
        //     signer_pubkey
        // );
        return Ok(DeployOutcome::Skipped);
    }

    // Build claim instruction (from ore crate)
    let ix: Instruction = claim_sol(signer_pubkey);

    // Get recent blockhash
    let recent_blockhash = match connection.get_latest_blockhash().await {
        Ok(bh) => bh,
        Err(e) => {
            println!("Failed to get recent blockhash for claim: {:?}", e);
            // tracing::error!("Failed to get recent blockhash for claim: {:?}", e);
            return Err(anyhow::anyhow!("Failed to get recent blockhash: {:?}", e));
        }
    };

    // Create message & transaction
    let message = Message::new(&[ix], Some(&signer_pubkey));
    let mut tx = Transaction::new_unsigned(message);

    if let Err(e) = tx.try_sign(&[&signer_kp], recent_blockhash) {
        println!("Failed to sign claim transaction: {:?}", e);
        // tracing::error!("Failed to sign claim transaction: {:?}", e);
        return Err(anyhow::anyhow!("Failed to sign claim tx: {:?}", e));
    }

    // Send & confirm
    match connection.send_and_confirm_transaction(&tx).await {
        Ok(sig) => {
            println!("Claim SOL tx sent for {}: {}", signer_pubkey, sig);
            // tracing::info!("Claim SOL tx sent for {}: {}", signer_pubkey, sig);
            // optional: wait a bit then re-fetch miner to confirm rewards_sol updated/cleared
            tokio::time::sleep(Duration::from_millis(1200)).await;
            match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::finalized()).await {
                Ok(resp) => {
                    if let Some(acc) = resp.value {
                        if !acc.data.is_empty() {
                            if let Ok(miner_after) = Miner::try_from_bytes(&acc.data) {
                                println!("Miner after claim: rewards_sol = {}", miner_after.rewards_sol);
                                // tracing::info!("Miner after claim: rewards_sol = {}", miner_after.rewards_sol);
                                // optionally update DB/state here
                            }
                        }
                    }
                }
                Err(e) => println!("Failed to re-fetch miner after claim: {:?}", e),
            }

            Ok(DeployOutcome::Deployed(sig))
        }
        Err(e) => {
            println!("Failed to send claim tx: {:?}", e);
            Err(anyhow::anyhow!("Failed to send claim tx: {:?}", e))
        }
    }
}


pub fn infer_refined_ore(miner: &Miner, treasury: &Treasury) -> u64 {
    let delta = treasury.miner_rewards_factor - miner.rewards_factor;
    if delta < Numeric::ZERO {
        // Defensive: shouldn't happen, but keep behavior sane.
        return miner.refined_ore;
    }
    let accrued = (delta * Numeric::from_u64(miner.rewards_ore)).to_u64();
    miner.refined_ore.saturating_add(accrued)
}

pub fn refinement_level_percent(refined_ore: f64, unclaimed_ore: f64) -> f64 {
    if unclaimed_ore <= 0.0 {
        if refined_ore <= 0.0 {
            -10.0
        } else {
            f64::INFINITY
        }
    } else {
        -10.0 + 100.0 * (refined_ore / unclaimed_ore)
    }
}

pub fn evaluate_ev_only(env: &OreEnv, ev_threshold: f64) -> (Vec<(usize, f64)>, bool) {
    let mut ev_list = Vec::new();

    // hitung EV setiap slot
    for i in 0..25 {
        let result = compute_ev_star_for_block(env.os[i], env.total_t, env.ore_value_in_sol);
        let ev = result.ev;

        if ev > ev_threshold {
            ev_list.push((i, ev));
        }
    }

    // urutkan slot berdasarkan EV tertinggi
    ev_list.sort_by(|a, b| b.1.total_cmp(&a.1));

    // kalau ada minimal 1 EV positif → deploy
    let should_deploy = !ev_list.is_empty();

    (ev_list, should_deploy)
}
