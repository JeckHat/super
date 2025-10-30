use solana_sdk::{
    signature::{read_keypair_file, Keypair},
};

pub struct SlotMiner {
    pub keypair_filepath: Option<String>,
}

impl SlotMiner {
    pub fn new(
        keypair_filepath: Option<String>
    ) -> Self {
        Self {
            keypair_filepath
        }
    }

    pub fn signer(&self) -> Keypair {
        match self.keypair_filepath.clone() {
            Some(filepath) => read_keypair_file(filepath.clone())
                .expect(format!("No keypair found at {}", filepath).as_str()),
            None => panic!("No keypair provided"),
        }
    }
}