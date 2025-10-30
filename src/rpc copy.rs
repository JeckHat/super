
use std::{str::FromStr, sync::Arc, time::Duration};

use ore_api::{consts::{SPLIT_ADDRESS, TREASURY_ADDRESS}, sdk::{checkpoint, claim_sol, deploy}, state::{Board, Miner, Round, Treasury, miner_pda, round_pda}};
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_filter::RpcFilterType};
use solana_sdk::{commitment_config::{CommitmentConfig, CommitmentLevel}, message::Message, signature::Keypair, signer::Signer, transaction::Transaction};
use steel::{AccountDeserialize, Instruction, Numeric, Pubkey};
use tokio::time::Instant;

use crate::{BOARD_ADDRESS, app_state::{AppMiner, AppState}, database::{self, CreateDeployment, CreateMinerSnapshot, insert_miner_snapshots}, get_board, slot_miner::SlotMiner};
use crate::ai::HybridPredictor;

pub struct MinerSnapshot {
    round_id: u64,
    miners: Vec<AppMiner>,
    completed: bool,
}

pub async fn update_data_system(connection: RpcClient, app_state: AppState) {
    tracing::info!("Starting update_data_system");
    let db_pool = app_state.db_pool.clone();
    let mut model = HybridPredictor::new();
    let mut total = 0;
    let mut pred = model.predict();
    tokio::spawn(async move {
        let mut board_snapshot = false;
        let mut miners_snapshot = MinerSnapshot {
            round_id: 0,
            miners: vec![],
            completed: false,
        };
        let mut last_deployed_round = None;
        loop {
            
            let treasury = if let Ok(treasury) = connection.get_account_data(&TREASURY_ADDRESS).await {
                if let Ok(treasury) = Treasury::try_from_bytes(&treasury) {
                    treasury.clone()
                } else {
                    tracing::error!("Failed to parse Treasury account");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue
                }
            } else {
                tracing::error!("Failed to load treasury account data");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue
            };

            // update treasury
            let r = app_state.treasury.clone();
            let mut l = r.write().await;
            *l = treasury.into();
            drop(l);

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

            // if let Some(prev) = last_deployed_round {
            //     if prev != board.round_id {
            //         tracing::info!("Detected new round (was {}, now {}), resetting set_slot/last_deployed_round", prev, board.round_id);
            //         last_deployed_round = None;
            //     }
            // }

            // If not deployed yet for this round, perform checkpoint+deploy flow
            if last_deployed_round != Some(board.round_id) {
                last_deployed_round = Some(board.round_id);
                println!("round {}", board.round_id);
                pred = model.predict();
                println!("Prediksi : {:?}", pred);

                // create signer/miner once
                let miner_1 = Arc::new(SlotMiner::new(Some(String::from("/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json"))));
                let signer_kp_1: Keypair = miner_1.signer();
                let signer_pubkey_1 = signer_kp_1.pubkey();

                // compute miner PDA for this authority
                let miner_adr = miner_pda(signer_pubkey_1).0;

                // fetch miner on-chain (if exists)
                let mut miner_data: Option<Miner> = match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::confirmed()).await {
                    Ok(resp) => {
                        if let Some(acc) = resp.value {
                            match Miner::try_from_bytes(&acc.data) {
                                Ok(m) => {
                                    Some(m.clone())
                                }
                                Err(_) => {
                                    tracing::error!("Failed to parse Miner account data");
                                    None
                                }
                            }
                        } else {
                            tracing::info!("Miner account {} not found on-chain (will attempt checkpoint-create)", miner_adr);
                            None
                        }
                    }
                    Err(e) => {
                        tracing::error!("RPC error fetching miner account: {:?}", e);
                        None
                    }
                };

                // build squares from prediction
                let mut squares: [bool; 25] = [false; 25];
                for &idx in pred.iter() {
                    if idx < squares.len() {
                        squares[idx] = true;
                    } else {
                        tracing::warn!("Pred index out of range (ignored): {}", idx);
                    }
                }

                let mut ixs: Vec<Instruction> = Vec::new();

                if let Some(ref m) = miner_data {
                    // If the on-chain rule requires m.checkpoint_id == m.round_id,
                    // we must checkpoint with m.round_id and only if checkpoint_id != round_id.
                    let miner_round_id = m.round_id;
                    let need_checkpoint = m.checkpoint_id != miner_round_id;
                
                    if need_checkpoint {
                        tracing::info!("Miner exists but not checkpointed for its own round (miner.round_id = {}, miner.checkpoint_id = {}), adding checkpoint({})",
                                      miner_round_id, m.checkpoint_id, miner_round_id);
                        ixs.push(checkpoint(signer_pubkey_1, signer_pubkey_1, miner_round_id));
                    } else {
                        tracing::info!("Miner already checkpointed for its own round: {}", miner_round_id);
                    }
                } else {
                    // Miner account missing: we don't have m.round_id to use.
                    // Fallback: attempt checkpoint using board.round_id (this will initialize the miner
                    // if the program supports creating miner on first checkpoint). If your program
                    // requires a different behavior (e.g. specific initial round_id), change accordingly.
                    tracing::info!("Miner account missing; attempting checkpoint(create) using board.round_id = {}", board.round_id);
                    ixs.push(checkpoint(signer_pubkey_1, signer_pubkey_1, board.round_id));
                }

                // push deploy instruction always (we either checkpoint+deploy in same tx or only deploy)
                ixs.push(deploy(
                    signer_pubkey_1,
                    signer_pubkey_1,
                    10_000,
                    board.round_id,
                    squares,
                ));

                // get recent blockhash
                let recent_blockhash = match connection.get_latest_blockhash().await {
                    Ok(bh) => bh,
                    Err(e) => {
                        tracing::error!("Failed to get recent blockhash for deploy: {:?}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };

                // build message & tx containing all instructions (atomic)
                let message = Message::new(&ixs, Some(&signer_pubkey_1));
                let mut tx = Transaction::new_unsigned(message);
                if let Err(e) = tx.try_sign(&[&signer_kp_1], recent_blockhash) {
                    tracing::error!("Failed to sign transaction: {:?}", e);
                    continue;
                }

                // send & confirm
                let sig = match connection.send_and_confirm_transaction(&tx).await {
                    Ok(sig) => {
                        println!("Transaction sent. Signature: {}", sig);
                        tracing::info!("Sent tx: {}", sig);
                        sig
                    }
                    Err(e) => {
                        tracing::error!("Failed to send tx: {:?}", e);
                        // allow retry next loop
                        continue;
                    }
                };

                // // fetch tx meta & program logs for debugging
                // match connection.get_transaction(&sig).await {
                //     Ok(Some(txinfo)) => {
                //         if let Some(meta) = txinfo.transaction.meta {
                //             tracing::info!("Tx status: {:?}", meta.status);
                //             if let Some(logs) = meta.log_messages {
                //                 tracing::info!("Program logs:\n{}", logs.join("\n"));
                //             }
                //         } else {
                //             tracing::warn!("No meta returned for tx {}", sig);
                //         }
                //     }
                //     Ok(None) => tracing::warn!("get_transaction returned None for sig {}", sig),
                //     Err(e) => tracing::error!("Failed to fetch tx info for {}: {:?}", sig, e),
                // }

                // After tx finalized, re-fetch miner with finalized commitment to verify checkpoint applied
                // (give a short delay to let validator index it)
                tokio::time::sleep(Duration::from_millis(1200)).await;
                match connection.get_account_with_commitment(&miner_adr, CommitmentConfig::finalized()).await {
                    Ok(resp) => {
                        if let Some(acc) = resp.value {
                            match Miner::try_from_bytes(&acc.data) {
                                Ok(miner_after) => {
                                    tracing::info!("Miner after tx: checkpoint_id = {}", miner_after.checkpoint_id);
                                    // if miner has checkpointed for this round then mark as deployed
                                    if miner_after.checkpoint_id == board.round_id {
                                        last_deployed_round = Some(board.round_id);
                                    } else {
                                        // if checkpoint not applied but deploy maybe applied — still mark as deployed
                                        // But safer: only mark if miner checkpoint == board.round_id
                                        tracing::warn!("Miner checkpoint_id after tx is {}, expected {}", miner_after.checkpoint_id, board.round_id);
                                        // If you want to be conservative, don't set last_deployed_round here
                                    }
                                }
                                Err(_) => tracing::warn!("Failed to parse miner after tx"),
                            }
                        } else {
                            tracing::warn!("Miner account not found after tx (miner_adr={})", miner_adr);
                        }
                    }
                    Err(e) => tracing::error!("Failed to re-fetch miner after tx: {:?}", e),
                }
            }
            
            // if last_deployed_round != Some(board.round_id) {
            //     println!("round {}", board.round_id);
            //     pred = model.predict();
            //     println!("Prediksi : {:?}", pred);

            //     let miner_1 = Arc::new(SlotMiner::new(Some(String::from("/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json"))));
            //     let signer_kp_1: Keypair = miner_1.signer();
            //     let signer_pubkey_1: Pubkey = signer_kp_1.pubkey();

            //     let miner_adr = miner_pda(signer_pubkey_1).0;
            //     // let account = match connection.get_account(&miner_adr).await {
            //     //     Ok(ac) => ac,
            //     //     Err(e) => {
            //     //         tracing::warn!("Pred index out of range (ignored): {}", idx);
            //     //     }
            //     // };
            //     // println!("Miner: {:#?}", account_miner);
            //     let mut miner_data: Option<Miner> = None;
            //     if let Ok(account) = connection.get_account(&miner_adr).await {
            //         if let Ok(miner) = Miner::try_from_bytes(&account.data) {
            //             println!("Miner: {:#?}", miner);
            //             miner_data = Some(miner.clone());
            //         } else {
            //             tracing::error!("Failed to parse Miner account data");
            //         }
            //     } else {
            //         tracing::error!("Failed to get account for miner {}", miner_adr);
            //     }
            //     // buat signer/miner & squares (sama seperti kode Anda)
            //     let miner_1 = Arc::new(SlotMiner::new(Some(String::from("/Users/jeckhat/gawean/jeckhat/miners/poolminer1.json"))));
            //     let signer_kp_1: Keypair = miner_1.signer();
            //     let signer_pubkey_1: Pubkey = signer_kp_1.pubkey();
            //     let mut squares: [bool; 25] = [false; 25];
    
            //     for &idx in pred.iter() {
            //         if idx < squares.len() {
            //             squares[idx] = true;
            //         } else {
            //             tracing::warn!("Pred index out of range (ignored): {}", idx);
            //         }
            //     }
    
            //     println!("address authority: {}", signer_pubkey_1);

            //     // let mut ixs: Vec<Instruction> = Vec::new();

            //     // if let Some(miner) = miner_data {
            //     //     ixs.push(checkpoint(signer_pubkey_1, signer_pubkey_1, miner.round_id));
            //     // }

                

            //     // let message = Message::new(&ixs, Some(&signer_pubkey_1));
            //     // let recent_blockhash = match connection.get_latest_blockhash().await {
            //     //     Ok(bh) => bh,
            //     //     Err(e) => {
            //     //         tracing::error!("Failed to get recent blockhash: {:?}", e);
            //     //         // jangan ubah flags: kita bisa coba lagi nanti untuk round yang sama
            //     //         tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            //     //         continue;
            //     //     }
            //     // };
            //     let mut tx = Transaction::new_unsigned(message);
            //     if let Err(e) = tx.try_sign(&[&signer_kp_1], recent_blockhash) {
            //         tracing::error!("Failed to sign transaction: {:?}", e);
            //         // jangan set last_deployed_round karena gagal sign
            //         continue;
            //     }

            //     println!("SUKSES CHECKPOINT");

            //     ixs = Vec::new();

            //     ixs.push(deploy(
            //         signer_pubkey_1,
            //         signer_pubkey_1,
            //         10_000,
            //         board.round_id,
            //         squares,
            //     ));
    
            //     let recent_blockhash = match connection.get_latest_blockhash().await {
            //         Ok(bh) => bh,
            //         Err(e) => {
            //             tracing::error!("Failed to get recent blockhash: {:?}", e);
            //             // jangan ubah flags: kita bisa coba lagi nanti untuk round yang sama
            //             tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            //             continue;
            //         }
            //     };
    
            //     let message = Message::new(&ixs, Some(&signer_pubkey_1));
            //     let mut tx = Transaction::new_unsigned(message);
            //     if let Err(e) = tx.try_sign(&[&signer_kp_1], recent_blockhash) {
            //         tracing::error!("Failed to sign transaction: {:?}", e);
            //         // jangan set last_deployed_round karena gagal sign
            //         continue;
            //     }
            //     match connection.send_and_confirm_transaction(&tx).await {
            //         Ok(sig) => {
            //             println!("Transaction sent. Signature: {}", sig);
            //             // sukses: tandai bahwa kita sudah deploy untuk round ini
            //             last_deployed_round = Some(board.round_id);
            //         },
            //         Err(e) => {
            //             println!("Failed to send tx: {:?}", e);
            //             // jangan set last_deployed_round agar percobaan ulang dimungkinkan
            //         },
            //     }
            // }

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

            println!("Slots left for round: {}", slots_left_in_round);
            tokio::time::sleep(Duration::from_secs(1)).await;

            if slots_left_in_round < 0 {
                if !board_snapshot {
                    tracing::info!("Updating data");
                    let round = if let Ok(round) = connection.get_account_data(&round_pda(board.round_id).0).await {
                        if let Ok(round) = Round::try_from_bytes(&round) {
                            round.clone()
                        } else {
                            tracing::error!("Failed to parse Round account");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    } else {
                        tracing::error!("Failed to load round account data");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue
                    };

                    tokio::time::sleep(Duration::from_secs(1)).await;

                    let mut miners: Vec<AppMiner> = vec![];
                    if let Ok(miners_data_raw) = connection.get_program_accounts_with_config(
                        &ore_api::id(),
                        solana_client::rpc_config::RpcProgramAccountsConfig { 
                            filters: Some(vec![RpcFilterType::DataSize(size_of::<Miner>() as u64 + 8)]),
                            account_config: solana_client::rpc_config::RpcAccountInfoConfig {
                                encoding: Some(UiAccountEncoding::Base64),
                                data_slice: None,
                                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Confirmed }),
                                min_context_slot: None,
                            },
                            with_context: None,
                            sort_results: None
                        } 
                    ).await {
                        for miner_data in miners_data_raw {
                            if let Ok(miner) = Miner::try_from_bytes(&miner_data.1.data) {
                                let mut miner = *miner;
                                miner.refined_ore = infer_refined_ore(&miner, &treasury);
                                miners.push(miner.clone().into());
                            }
                        }
                    }

                    if miners.len() > 0 {
                        miners_snapshot.round_id = round.id;
                        miners_snapshot.miners = miners.clone();
                        miners_snapshot.completed = false;
                        miners.sort_by(|a, b| b.rewards_ore.partial_cmp(&a.rewards_ore).unwrap());

                        tracing::info!("Setting miners snapshot completed to false");
                        
                    } else {
                        miners_snapshot.round_id = round.id;
                        miners_snapshot.miners = vec![];
                        miners_snapshot.completed = true;
                        tracing::info!("Setting miners snapshot completed to true");
                    }
                    board_snapshot = true;
                }
            } else if slots_left_in_round > 0 {
                board_snapshot = false;
                tracing::info!("Checking miner snapshot status: {}", miners_snapshot.completed);
                if !miners_snapshot.completed {
                    let r_now = Instant::now();
                    tracing::info!("Performing snapshot and updating round");
                    // load previous round
                    let round_id = board.round_id - 1;
                    let mut round = if let Ok(round) = connection.get_account_data(&round_pda(round_id).0).await {
                        if let Ok(round) = Round::try_from_bytes(&round) {
                            round.clone()
                        } else {
                            tracing::error!("Failed to parse Round account");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    } else {
                        tracing::error!("Failed to load round account data");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue
                    };


                    if round.slot_hash == [0; 32] {
                        tracing::error!("Round slot hash should not be 0's");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    } else if round.slot_hash == [u8::MAX; 32] {
                        tracing::error!("Round reset failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tracing::error!("");
                        // Update miners
                        let r = app_state.miners.clone();
                        let mut l = r.write().await;
                        *l = miners_snapshot.miners.clone();
                        drop(l);
                        miners_snapshot.completed = true;

                        let mut db_snapshot: Vec<CreateMinerSnapshot> = vec![];

                        for m in miners_snapshot.miners.iter() {
                            let m = m.clone();
                            db_snapshot.push(m.into());
                        }

                        // update round
                        let r = app_state.rounds.clone();
                        let mut l = r.write().await;
                        l.push(round.into());
                        drop(l);

                        continue;
                    } else {
                        // process round data
                        if let Some(_r) = round.rng() {
                            let (winning_square_opt, top_sample_opt, denom_opt) = if let Some(r) = round.rng() {
                                let winning_square = round.winning_square(r) as usize;
                                let hit = pred.contains(&winning_square);
                                println!("Prediksi : {:?}", pred);
                                println!("Hasil     : {}\n", if hit { "✅ BENAR" } else { "❌ SALAH" });
                                println!("Winning {}", winning_square);
                                if hit {
                                    total += 1
                                }

                                model.update(winning_square, &pred);
                                println!("Akurasi Hybrid: {:.2}%", model.accuracy());
                                println!("total menang: {:}", total);

                                // Total deployed on winning square (denominator for pro-rata shares)
                                let denom = round.deployed[winning_square];
                                if denom == 0 {
                                    // Degenerate case: nothing deployed on the winning square → no rewards
                                    (Some(winning_square), None, Some(denom))
                                } else {
                                    // If split, every miner on the winning square shares top_miner_reward pro-rata.
                                    // If not split, one miner (whose cumulative range contains top_sample) takes it all.
                                    let top_sample = if round.top_miner == SPLIT_ADDRESS {
                                        None
                                    } else {
                                        Some(round.top_miner_sample(r, winning_square))
                                    };
                                    (Some(winning_square), top_sample, Some(denom))
                                }
                            } else {
                                (None, None, None)
                            };

                            let mut deployments: Vec<CreateDeployment> = Vec::new();

                            // Convenience captures
                            let winning_square = winning_square_opt;
                            let denom = denom_opt.unwrap_or(0);
                            let is_split = round.top_miner == SPLIT_ADDRESS;
                            let motherlode_amt = round.motherlode; // you already set this earlier if did_hit_motherlode
                            let total_winnings = round.total_winnings;
                            let top_sample = top_sample_opt; // same for all miners if not split

                            for miner in miners_snapshot.miners.iter() {
                                if miner.round_id == round.id {
                                     for (square_index, amount) in miner.deployed.iter().enumerate() {
                                         if *amount == 0 {
                                             continue;
                                         }

                                         // Defaults for non-winning squares (or missing RNG)
                                         let mut sol_earned_u64: u64 = 0;
                                         let mut ore_earned_u64: u64 = 0;

                                         // Only compute rewards on the winning square and when we had RNG
                                         if let Some(ws) = winning_square {
                                             if square_index == ws && denom > 0 {
                                                 // ---- SOL rewards ----
                                                 // Base = original_deployment - admin_fee (admin_fee = max(1, original/100))
                                                 let original = *amount as u64;
                                                 let admin_fee = (original / 100).max(1);
                                                 let mut rewards_sol = original.saturating_sub(admin_fee);

                                                 // Pro-rata share of round.total_winnings
                                                 let share = ((total_winnings as u128 * original as u128) / denom as u128) as u64;
                                                 rewards_sol = rewards_sol.saturating_add(share);

                                                 sol_earned_u64 = rewards_sol;

                                                 // ---- ORE rewards ----
                                                 // Top miner reward: split evenly pro-rata if split, else winner-takes-all by sample
                                                 if is_split {
                                                     let split_share = ((round.top_miner_reward as u128 * original as u128)
                                                         / denom as u128) as u64;
                                                     ore_earned_u64 = ore_earned_u64.saturating_add(split_share);
                                                 } else if let Some(sample) = top_sample {
                                                     // Check if this miner's cumulative interval covers the sample
                                                     let start = miner.cumulative[ws];
                                                     let end = start.saturating_add(original);
                                                     if sample >= start && sample < end {
                                                         ore_earned_u64 = ore_earned_u64.saturating_add(round.top_miner_reward);
                                                         round.top_miner = Pubkey::from_str(&miner.authority).unwrap();
                                                     }
                                                 }

                                                 // Motherlode reward (if any)
                                                 if motherlode_amt > 0 {
                                                     let ml_share = ((motherlode_amt as u128 * original as u128)
                                                         / denom as u128) as u64;
                                                     ore_earned_u64 = ore_earned_u64.saturating_add(ml_share);
                                                 }
                                             }
                                         }

                                         let deployment = CreateDeployment {
                                             round_id: miner.round_id as i64,
                                             pubkey: miner.authority.to_string(),
                                             square_id: square_index as i64,
                                             amount: *amount as i64,
                                             sol_earned: sol_earned_u64 as i64,
                                             ore_earned: ore_earned_u64 as i64,
                                             unclaimed_ore: miner.rewards_ore as i64,
                                             created_at: chrono::Utc::now().to_rfc3339(),
                                         };

                                         deployments.push(deployment);
                                     }
                                }

                            }

                        } else {
                            tracing::error!("Failed to get round rng.");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue
                        }
                        
                        // Update miners
                        let r = app_state.miners.clone();
                        let mut l = r.write().await;
                        *l = miners_snapshot.miners.clone();
                        drop(l);

                        let mut db_snapshot: Vec<CreateMinerSnapshot> = vec![];

                        for m in miners_snapshot.miners.iter() {
                            let m = m.clone();
                            db_snapshot.push(m.into());
                        }

                        // update round
                        let r = app_state.rounds.clone();
                        let mut l = r.write().await;
                        l.push(round.into());
                        drop(l);

                        tracing::info!("Successfully snapshot round and updated database in {}ms", r_now.elapsed().as_millis());
                        miners_snapshot.completed = true;
                    }
                }



                let sleep_time = slots_left_in_round as u64 * 400;
                println!("Sleeping until round is over in {} ms", sleep_time + 5000);
                tokio::time::sleep(Duration::from_millis(sleep_time)).await;
            } else {
                board_snapshot = false;
                println!("Sleeping for 5 seconds");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }


        }
    });
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


