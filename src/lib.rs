use log::{error, info};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use aligned_sdk::core::types::{Chain, ProvingSystemId, VerificationData};
use aligned_sdk::sdk::{get_next_nonce, submit_and_wait_verification};
use dialoguer::Confirm;
use ethers::prelude::*;
use ethers::providers::{Http, Provider};
use ethers::signers::LocalWallet;
use ethers::types::{Address, U256};

pub mod risc0;
pub mod sp1;
pub mod utils;

const BATCHER_URL: &str = "wss://batcher.alignedlayer.com";
const BATCHER_PAYMENTS_ADDRESS: &str = "0x815aeCA64a974297942D2Bbf034ABEe22a38A003";

pub async fn pay_batcher(
    from: Address,
    signer: Arc<SignerMiddleware<Provider<Http>, LocalWallet>>,
) -> anyhow::Result<()> {
    if !Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("We are going to pay 0.004eth for the proof submission to aligned. Do you want to continue?")
        .interact()
        .expect("Failed to read user input")
    {
        anyhow::bail!("Payment cancelled")
    }

    let addr = Address::from_str(BATCHER_PAYMENTS_ADDRESS).map_err(|e| anyhow::anyhow!(e))?;

    let tx = TransactionRequest::new()
        .from(from)
        .to(addr)
        .value(4000000000000000u128);

    info!("Submitting Payment to  Batcher");
    match signer
        .send_transaction(tx, None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send tx {}", e))?
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit tx {}", e))?
    {
        Some(receipt) => {
            info!(
                "Payment sent. Transaction hash: {:x}",
                receipt.transaction_hash
            );
            Ok(())
        }
        None => {
            anyhow::bail!("Payment failed");
        }
    }
}

//NOTE: we default to submitting to the testnet. When mainnet is live will make mainnet submission the default, testnet an option
pub async fn submit_proof_to_aligned(
    keystore_path: &PathBuf,
    proof_path: &str,
    elf_path: &str,
    pub_input_path: Option<&str>,
    rpc_url: &str,
    chain_id: &u64,
    max_fee: &u128,
    proof_system_id: ProvingSystemId,
) -> anyhow::Result<()> {
    let Ok(keystore_password) = rpassword::prompt_password("Enter keystore password: ") else {
        error!("Failed to read keystore password");
        return Ok(());
    };

    let Ok(local_wallet) = LocalWallet::decrypt_keystore(keystore_path, keystore_password) else {
        error!("Failed to decrypt keystore");
        return Ok(());
    };
    let wallet = local_wallet.with_chain_id(17000u64);

    let Ok(proof) = std::fs::read(proof_path) else {
        error!("Failed to Read Proof");
        return Ok(());
    };
    let Ok(elf_data) = std::fs::read(elf_path) else {
        error!("Failed to Read ELF");
        return Ok(());
    };
    let pub_input = match pub_input_path {
        Some(path) => Some(std::fs::read(path).expect("Failed to Read Public Inputs")),
        None => None,
    };

    let Ok(provider) = Provider::<Http>::try_from(rpc_url) else {
        error!("Failed to connect to provider");
        return Ok(());
    };

    let signer = Arc::new(SignerMiddleware::new(provider.clone(), wallet.clone()));

    pay_batcher(wallet.address(), signer.clone()).await?;

    let max_fee = U256::from(*max_fee);

    let verification_data = VerificationData {
        proving_system: proof_system_id,
        proof,
        proof_generator_addr: wallet.address(),
        vm_program_code: Some(elf_data),
        verification_key: None,
        pub_input,
    };

    let Ok(nonce) = get_next_nonce(rpc_url, wallet.address(), BATCHER_PAYMENTS_ADDRESS).await
    else {
        error!("could not get nonce");
        return Ok(());
    };

    let chain = match chain_id {
        17000 => Chain::Holesky,
        31337 => Chain::Devnet,
        //We default to holesky
        _ => Chain::Holesky,
    };

    info!("Submitting proof to Aligned for Verification");

    let Ok(aligned_verification_data) = submit_and_wait_verification(
        BATCHER_URL,
        rpc_url,
        chain,
        &verification_data,
        max_fee,
        wallet,
        nonce,
        BATCHER_PAYMENTS_ADDRESS,
    )
    .await
    else {
        error!("Proof generation failed");
        return Ok(());
    };
    info!("Proof Submitted to Aligned!");
    info!(
        "https://explorer.alignedlayer.com/batches/0x{}",
        hex::encode(aligned_verification_data.batch_merkle_root)
    );
    Ok(())
}
