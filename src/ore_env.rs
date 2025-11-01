use anyhow::{Result, bail};
use ore_api::state::{Board, Round};
use serde_json::Value;
use solana_client::{client_error::reqwest, nonblocking::rpc_client::RpcClient};
use solana_sdk::pubkey::Pubkey;
use steel::AccountDeserialize;


#[derive(Debug)]
pub struct OreEnv {
    pub os: Vec<f64>,          // stake per slot (O_i)
    pub total_t: f64,          // total stake semua slot (T)
    pub ore_value_in_sol: f64, // harga ORE dalam SOL
    pub motherlode: f64,       // motherlode ORE saat ini
}

pub async fn fetch_ore_env(connection: &RpcClient, board_address: Pubkey, program_id: Pubkey) -> Result<OreEnv> {
    // 1️⃣ Ambil board
    let board_data = connection.get_account_data(&board_address).await?;
    let board = Board::try_from_bytes(&board_data)
        .map_err(|_| anyhow::anyhow!("Gagal parse Board"))?;

    // 2️⃣ Ambil round aktif
    let (round_pda, _bump) =
        Pubkey::find_program_address(&[b"round", &board.round_id.to_le_bytes()], &program_id);

    let round_data = connection.get_account_data(&round_pda).await?;
    let round = Round::try_from_bytes(&round_data)
        .map_err(|_| anyhow::anyhow!("Gagal parse Round"))?;

    // 3️⃣ Ambil stake tiap slot (O_i)
    let os: Vec<f64> = round.deployed.iter().map(|&v| v as f64 / 1e9).collect();

    // 4️⃣ Total stake (T)
    let total_t: f64 = round.total_deployed as f64 / 1e9;

    // 5️⃣ Motherlode (ORE)
    let motherlode = round.motherlode as f64 / 1e9;

    // 6️⃣ Ambil harga ORE/SOL dari DEX
    let ore_addr = "oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp";
    let sol_addr = "So11111111111111111111111111111111111111112";

    let ore_resp: Value = reqwest::get(format!(
        "https://api.dexscreener.com/latest/dex/tokens/{}",
        ore_addr
    ))
    .await?
    .json()
    .await?;

    let sol_resp: Value = reqwest::get(format!(
        "https://api.dexscreener.com/latest/dex/tokens/{}",
        sol_addr
    ))
    .await?
    .json()
    .await?;

    let ore_price_usd = ore_resp["pairs"][0]["priceUsd"]
        .as_str()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);
    let sol_price_usd = sol_resp["pairs"][0]["priceUsd"]
        .as_str()
        .unwrap_or("0")
        .parse::<f64>()
        .unwrap_or(0.0);
    let ore_value_in_sol = ore_price_usd / sol_price_usd;

    if ore_value_in_sol <= 0.0 {
        bail!("Harga ORE belum tersedia dari DEX.");
    }

    Ok(OreEnv {
        os,
        total_t,
        ore_value_in_sol,
        motherlode,
    })
}
