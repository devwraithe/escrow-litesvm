#[cfg(test)]
mod tests {

    use {
        anchor_lang::{
            prelude::{msg, Clock},
            solana_program::program_pack::Pack,
            AccountDeserialize, InstructionData, ToAccountMetas,
        },
        anchor_spl::{
            associated_token::{self, spl_associated_token_account},
            token::spl_token,
            token::spl_token::state::Account,
        },
        litesvm::{types::TransactionMetadata, LiteSVM},
        litesvm_token::{
            spl_token::ID as TOKEN_PROGRAM_ID, CreateAssociatedTokenAccount, CreateMint, MintTo,
        },
        solana_instruction::Instruction,
        solana_keypair::Keypair,
        solana_message::Message,
        solana_native_token::LAMPORTS_PER_SOL,
        solana_pubkey::Pubkey,
        solana_sdk_ids::system_program::ID as SYSTEM_PROGRAM_ID,
        solana_signer::Signer,
        solana_transaction::Transaction,
        std::{future, path::PathBuf, str::FromStr},
    };

    static PROGRAM_ID: Pubkey = crate::ID;

    pub struct TestData {
        payer: Keypair,
        maker: Pubkey,
        vault: Pubkey,
        escrow: Pubkey,
        mint_a: Pubkey,
        mint_b: Pubkey,
        maker_ata_a: Pubkey,
        maker_ata_b: Pubkey,
    }

    // Setup function to initialize LiteSVM and create a payer keypair
    // Also loads an account from devnet into the LiteSVM environment (for testing purposes)
    fn setup() -> (LiteSVM, Keypair) {
        // Initialize LiteSVM and payer
        let mut program = LiteSVM::new();
        let payer = Keypair::new();

        // Airdrop some SOL to the payer keypair
        program
            .airdrop(
                &payer.pubkey(),
                10u64.checked_mul(LAMPORTS_PER_SOL).unwrap(),
            )
            .expect("Failed to airdrop SOL to payer");

        // Load program SO file
        let so_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy/anchor_escrow.so");

        let program_data = std::fs::read(so_path).expect("Failed to read program SO file");

        program.add_program(PROGRAM_ID, &program_data);

        // => Remove this section because it's replacing the payer account with devnet account data
        // => This causes an issue because the lamports loaded from devnet is significantly less than the lamports airdropped to the payer
        // => This results in insufficient funds for transactions
        // Example on how to Load an account from devnet
        // LiteSVM does not have access to real Solana network data since it does not have network access,
        // so we use an RPC client to fetch account data from devnet
        // let rpc_client = RpcClient::new("https://api.devnet.solana.com");
        // let account_address =
        //     Address::from_str("DRYvf71cbF2s5wgaJQvAGkghMkRcp5arvsK2w97vXhi2").unwrap();
        // let fetched_account = rpc_client
        //     .get_account(&account_address)
        //     .expect("Failed to fetch account from devnet");

        // // Set the fetched account in the LiteSVM environment
        // // This allows us to simulate interactions with this account during testing
        // program
        //     .set_account(
        //         payer.pubkey(),
        //         Account {
        //             lamports: fetched_account.lamports,
        //             data: fetched_account.data,
        //             owner: Pubkey::from(fetched_account.owner.to_bytes()),
        //             executable: fetched_account.executable,
        //             rent_epoch: fetched_account.rent_epoch,
        //         },
        //     )
        //     .unwrap();

        // msg!("Lamports of fetched account: {}", fetched_account.lamports);

        // Return the LiteSVM instance and payer keypair
        (program, payer)
    }

    fn setup_with_make() -> (LiteSVM, TestData, TransactionMetadata) {
        // Setup the test environment using the `setup` function
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B) with 6 decimal places
        // Make the `maker` as the authority
        // This done using litesvm-token's CreateMint utility which creates the mint in the LiteSVM environment
        let mint_a = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        // msg!("Mint A: {}", mint_a);

        let mint_b = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        // msg!("Mint B: {}", mint_b);

        // Create the maker's associated token account for Mint A and Mint B
        // This is done using litesvm-token's CreateAssociatedTokenAccount utility
        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a)
            .owner(&maker)
            .send()
            .unwrap();
        // msg!("Maker ATA A: {}\n", maker_ata_a);

        let maker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_b)
            .owner(&maker)
            .send()
            .unwrap();
        // msg!("Maker ATA B: {}\n", maker_ata_b);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &123u64.to_le_bytes()],
            &PROGRAM_ID,
        )
        .0;
        // msg!("Escrow PDA: {}\n", escrow);

        // Derive the PDA for the vault associated token account using the escrow PDA and Mint A
        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);
        // msg!("Vault PDA: {}\n", vault);

        // Define program IDs for associated token program, token program, and system program
        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Mint 1,000 tokens (with 6 decimal places) of Mint A to the maker's associated token account
        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000)
            .send()
            .unwrap();

        // Create the "Make" instruction to deposit tokens into the escrow
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker: maker,
                mint_a: mint_a,
                mint_b: mint_b,
                maker_ata_a: maker_ata_a,
                escrow: escrow,
                vault: vault,
                associated_token_program: associated_token_program,
                token_program: token_program,
                system_program: system_program,
            }
            .to_account_metas(None),
            data: crate::instruction::Make {
                deposit: 10,
                seed: 123u64,
                receive: 10,
            }
            .data(),
        };

        // Create and send the transaction containing the "Make" instruction
        let message = Message::new(&[make_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();

        let transaction = Transaction::new(&[&payer], message, recent_blockhash);

        // Send the transaction and capture the result
        let tx = program.send_transaction(transaction).unwrap();

        let test_data = TestData {
            payer,
            maker,
            vault,
            escrow,
            mint_a,
            mint_b,
            maker_ata_a,
            maker_ata_b,
        };

        (program, test_data, tx)
    }

    #[test]
    fn test_make() {
        // Setup the maker and create an escrow using the "Make" instruction
        let (program, test_data, tx) = setup_with_make();

        // Extract relevant data from the test setup
        let vault = test_data.vault;
        let escrow = test_data.escrow;
        let mint_a = test_data.mint_a;
        let maker = test_data.maker;
        let mint_b = test_data.mint_b;

        // Log transaction details
        msg!("\nMake transaction sucessful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the vault account and escrow account data after the "Make" instruction
        let vault_account = program.get_account(&vault).unwrap();
        let vault_data = spl_token::state::Account::unpack(&vault_account.data).unwrap();
        assert_eq!(vault_data.amount, 10);
        assert_eq!(vault_data.owner, escrow);
        assert_eq!(vault_data.mint, mint_a);

        let escrow_account = program.get_account(&escrow).unwrap();
        let escrow_data =
            crate::state::Escrow::try_deserialize(&mut escrow_account.data.as_ref()).unwrap();
        assert_eq!(escrow_data.seed, 123u64);
        assert_eq!(escrow_data.maker, maker);
        assert_eq!(escrow_data.mint_a, mint_a);
        assert_eq!(escrow_data.mint_b, mint_b);
        assert_eq!(escrow_data.receive, 10);
    }

    #[test]
    fn test_take() {
        // Setup the maker and create an escrow using the "Make" instruction
        let (mut program, test_data, _tx) = setup_with_make();

        // Extract relevant data from the test setup
        let payer = test_data.payer;
        let vault = test_data.vault;
        let escrow = test_data.escrow;
        let mint_a = test_data.mint_a;
        let maker = test_data.maker;
        let mint_b = test_data.mint_b;
        let maker_ata_b = test_data.maker_ata_b;

        // Create a taker keypair and airdrop some SOL to the taker for transaction fees
        let taker = Keypair::new();

        program
            .airdrop(
                &taker.pubkey(),
                10u64.checked_mul(LAMPORTS_PER_SOL).unwrap(),
            )
            .expect("Failed to airdrop SOL to taker");

        // Create the taker's associated token accounts for Mint A and Mint B
        let taker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_a)
            .owner(&taker.pubkey())
            .send()
            .unwrap();
        // msg!(">>> Taker ATA A: {}\n\n", taker_ata_a);

        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b)
            .owner(&taker.pubkey())
            .send()
            .unwrap();
        // msg!(">>> Taker ATA B: {}", taker_ata_b);

        // Define program IDs for associated token program, token program, and system program
        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Mint 1,000 tokens (with 6 decimal places) of Mint B to the taker's associated token account
        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 1000000000)
            .send()
            .unwrap();

        // Create the "Take" instruction to accept and send tokens
        let take_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(),
                maker: maker,
                mint_a: mint_a,
                mint_b: mint_b,
                taker_ata_a: taker_ata_a,
                taker_ata_b: taker_ata_b,
                maker_ata_b: maker_ata_b,
                escrow: escrow,
                vault: vault,
                associated_token_program: associated_token_program,
                token_program: token_program,
                system_program: system_program,
            }
            .to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        // Create and send the transaction containing the "Take" instruction
        let message = Message::new(&[take_ix], Some(&taker.pubkey()));
        let recent_blockhash = program.latest_blockhash();

        let transaction = Transaction::new(&[&taker], message, recent_blockhash);

        // Send the transaction and capture the result
        let tx = program.send_transaction(transaction).unwrap();

        // Log transaction details
        msg!("\n\nTake transaction successful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the escrow account has been closed
        let escrow_account = program.get_account(&escrow);
        if let Some(account) = escrow_account {
            assert_eq!(
                account.lamports, 0,
                "Escrow account should have 0 lamports after closure"
            );
            assert!(
                account.data.is_empty() || account.data.iter().all(|&b| b == 0),
                "Escrow account data should be empty or zeroed"
            );
        }

        // Verify the vault account has been closed
        let vault_account = program.get_account(&vault);
        if let Some(account) = vault_account {
            assert_eq!(
                account.lamports, 0,
                "Vault account should have 0 lamports after closure"
            );
        }

        // Verify taker received the escrowed tokens (mint_a)
        let taker_ata_a_account = program.get_account(&taker_ata_a).unwrap();
        let taker_ata_a_data = Account::unpack(&taker_ata_a_account.data).unwrap();
        assert_eq!(
            taker_ata_a_data.amount, 10,
            "Taker should have received 10 tokens of mint_a"
        );
        assert_eq!(taker_ata_a_data.owner, taker.pubkey());
        assert_eq!(taker_ata_a_data.mint, mint_a);
    }

    #[test]
    fn test_refund() {
        let (mut program, test_data, _tx) = setup_with_make();

        let payer = test_data.payer;
        let maker = test_data.maker;
        let vault = test_data.vault;
        let escrow = test_data.escrow;
        let mint_a = test_data.mint_a;
        let maker_ata_a = test_data.maker_ata_a;

        // Define program IDs for associated token program, token program, and system program
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Create the "Refund" instruction to accept and send tokens
        let refund_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Refund {
                maker: maker,
                mint_a: mint_a,
                maker_ata_a: maker_ata_a,
                escrow: escrow,
                vault: vault,
                token_program: token_program,
                system_program: system_program,
            }
            .to_account_metas(None),
            data: crate::instruction::Refund {}.data(),
        };

        // Create and send the transaction containing the "Refund" instruction
        let message = Message::new(&[refund_ix], Some(&maker));
        let recent_blockhash = program.latest_blockhash();

        let transaction = Transaction::new(&[&payer], message, recent_blockhash);

        // Send the transaction and capture the result
        let tx = program.send_transaction(transaction).unwrap();

        // Log transaction details
        msg!("\n\nRefund transaction successful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // 1. Verify maker received their tokens back
        let maker_ata_a_account = program.get_account(&maker_ata_a).unwrap();
        let maker_ata_a_data =
            spl_token::state::Account::unpack(&maker_ata_a_account.data).unwrap();
        assert_eq!(
            maker_ata_a_data.amount, 1000000000,
            "Maker should have all their tokens back (original 1B)"
        );

        // 2. Verify escrow account has been closed
        let escrow_account = program.get_account(&escrow);
        if let Some(account) = escrow_account {
            assert_eq!(
                account.lamports, 0,
                "Escrow account should have 0 lamports after refund"
            );
            assert!(
                account.data.is_empty() || account.data.iter().all(|&b| b == 0),
                "Escrow account data should be empty or zeroed"
            );
        }

        // 3. Verify vault account has been closed
        let vault_account = program.get_account(&vault);
        if let Some(account) = vault_account {
            assert_eq!(
                account.lamports, 0,
                "Vault account should have 0 lamports after refund"
            );
        }
    }

    #[test]
    fn test_refund_after_delay() {
        let (mut program, test_data, _tx) = setup_with_make();

        let payer = test_data.payer;
        let maker = test_data.maker;
        let vault = test_data.vault;
        let escrow = test_data.escrow;
        let mint_a = test_data.mint_a;
        let maker_ata_a = test_data.maker_ata_a;

        // Define program IDs for associated token program, token program, and system program
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Time travel BEFORE attempting refund
        let mut clock = program.get_sysvar::<Clock>();
        let current_timestamp = clock.unix_timestamp;
        let five_days = 5i64
            .checked_mul(24)
            .unwrap()
            .checked_mul(60)
            .unwrap()
            .checked_mul(60)
            .unwrap();
        let time_jump = current_timestamp.checked_add(five_days).unwrap();

        // Update timestamp to 5 days in the future
        clock.unix_timestamp = time_jump;
        program.set_sysvar::<Clock>(&clock);

        msg!("\n\nTime travelled {} seconds into the future", five_days);

        // Create the "Refund" instruction
        let refund_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Refund {
                maker: maker,
                mint_a: mint_a,
                maker_ata_a: maker_ata_a,
                escrow: escrow,
                vault: vault,
                token_program: token_program,
                system_program: system_program,
            }
            .to_account_metas(None),
            data: crate::instruction::Refund {}.data(),
        };

        // Create and send the transaction containing the "Refund" instruction
        let message = Message::new(&[refund_ix], Some(&maker));
        let recent_blockhash = program.latest_blockhash();

        let transaction = Transaction::new(&[&payer], message, recent_blockhash);

        // Send the transaction and capture the result
        let tx = program.send_transaction(transaction).unwrap();

        // Log transaction details
        msg!("\n\nRefund after delay transaction successful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // 1. Verify maker received their tokens back
        let maker_ata_a_account = program.get_account(&maker_ata_a).unwrap();
        let maker_ata_a_data =
            spl_token::state::Account::unpack(&maker_ata_a_account.data).unwrap();
        assert_eq!(
            maker_ata_a_data.amount, 1000000000,
            "Maker should have all their tokens back after time-locked refund"
        );

        // 2. Verify escrow account has been closed
        let escrow_account = program.get_account(&escrow);
        if let Some(account) = escrow_account {
            assert_eq!(
                account.lamports, 0,
                "Escrow account should have 0 lamports after time-locked refund"
            );
            assert!(
                account.data.is_empty() || account.data.iter().all(|&b| b == 0),
                "Escrow account data should be empty or zeroed after time-locked refund"
            );
        }

        // 3. Verify vault account has been closed
        let vault_account = program.get_account(&vault);
        if let Some(account) = vault_account {
            assert_eq!(
                account.lamports, 0,
                "Vault account should have 0 lamports after time-locked refund"
            );
        }
    }
}
