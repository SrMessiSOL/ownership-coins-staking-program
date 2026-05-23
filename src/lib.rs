//! Standalone ownership-coin staking rewards program.
//!
//! Users stake one MetaDAO ownership coin mint and earn a reward mint emitted
//! by this program. It is designed exclusively for ownership coins launched on
//! MetaDAO, not for MetaDAO itself. The configured authority should be the
//! coin's MetaDAO governance execution authority, so staking lifecycle changes
//! can be routed through the coin's proposal flow.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

#[allow(unused_imports)]
use alloc::format;
use alloc::vec::Vec;

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::{clock::Clock, Sysvar},
};

pub fn id() -> Pubkey {
    Pubkey::new_from_array([8u8; 32])
}

pub const FP: u128 = 1u128 << 64;

const IX_INIT_COIN_CONFIG: u8 = 0;
const IX_INIT_STAKE_POOL: u8 = 1;
const IX_STAKE: u8 = 2;
const IX_UNSTAKE: u8 = 3;
const IX_CLAIM_STAKE_REWARDS: u8 = 4;
const IX_SET_STAKE_POOL_REWARDS: u8 = 5;
const IX_MINT_REWARD: u8 = 6;
const IX_TRANSFER_MINT_AUTHORITY: u8 = 7;
const IX_TRANSFER_CONFIG_AUTHORITY: u8 = 8;

const COIN_CFG_DISC: [u8; 8] = *b"CCFG0001";
const POOL_DISC: [u8; 8] = *b"STPOOL01";
const POSITION_DISC: [u8; 8] = *b"STPOS001";

const COIN_CFG_SIZE: usize = 8 + 32;
const POOL_SIZE: usize = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 16 + 8 + 8;
const POSITION_SIZE: usize = 8 + 8 + 8 + 16 + 8;

fn pool_seeds<'a>(stake_mint: &'a Pubkey, reward_mint: &'a Pubkey) -> [&'a [u8]; 3] {
    [b"stake_pool", stake_mint.as_ref(), reward_mint.as_ref()]
}

fn position_seeds<'a>(pool: &'a Pubkey, user: &'a Pubkey) -> [&'a [u8]; 3] {
    [b"stake_position", pool.as_ref(), user.as_ref()]
}

fn stake_vault_seeds(pool: &Pubkey) -> [&[u8]; 2] {
    [b"stake_vault", pool.as_ref()]
}

fn mint_authority_seeds(reward_mint: &Pubkey) -> [&[u8]; 2] {
    [b"reward_mint_authority", reward_mint.as_ref()]
}

fn coin_cfg_seeds(reward_mint: &Pubkey) -> [&[u8]; 2] {
    [b"coin_cfg", reward_mint.as_ref()]
}

struct CoinConfig {
    authority: Pubkey,
}

impl CoinConfig {
    fn deserialize(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < COIN_CFG_SIZE || data[..8] != COIN_CFG_DISC {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            authority: Pubkey::new_from_array(data[8..40].try_into().unwrap()),
        })
    }

    fn serialize(&self, data: &mut [u8]) {
        data[..8].copy_from_slice(&COIN_CFG_DISC);
        data[8..40].copy_from_slice(self.authority.as_ref());
    }
}

struct StakePoolConfig {
    authority: Pubkey,
    stake_mint: Pubkey,
    reward_mint: Pubkey,
    reward_per_epoch: u64,
    epoch_slots: u64,
    start_slot: u64,
    reward_per_token_stored: u128,
    last_update_slot: u64,
    total_staked: u64,
}

impl StakePoolConfig {
    fn deserialize(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < POOL_SIZE || data[..8] != POOL_DISC {
            return Err(ProgramError::InvalidAccountData);
        }
        let mut off = 8;
        let authority = Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        off += 32;
        let stake_mint = Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        off += 32;
        let reward_mint = Pubkey::new_from_array(data[off..off + 32].try_into().unwrap());
        off += 32;
        let reward_per_epoch = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        off += 8;
        let epoch_slots = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        off += 8;
        let start_slot = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        off += 8;
        let reward_per_token_stored =
            u128::from_le_bytes(data[off..off + 16].try_into().unwrap());
        off += 16;
        let last_update_slot = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        off += 8;
        let total_staked = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        Ok(Self {
            authority,
            stake_mint,
            reward_mint,
            reward_per_epoch,
            epoch_slots,
            start_slot,
            reward_per_token_stored,
            last_update_slot,
            total_staked,
        })
    }

    fn serialize(&self, data: &mut [u8]) {
        data[..8].copy_from_slice(&POOL_DISC);
        let mut off = 8;
        data[off..off + 32].copy_from_slice(self.authority.as_ref());
        off += 32;
        data[off..off + 32].copy_from_slice(self.stake_mint.as_ref());
        off += 32;
        data[off..off + 32].copy_from_slice(self.reward_mint.as_ref());
        off += 32;
        data[off..off + 8].copy_from_slice(&self.reward_per_epoch.to_le_bytes());
        off += 8;
        data[off..off + 8].copy_from_slice(&self.epoch_slots.to_le_bytes());
        off += 8;
        data[off..off + 8].copy_from_slice(&self.start_slot.to_le_bytes());
        off += 8;
        data[off..off + 16].copy_from_slice(&self.reward_per_token_stored.to_le_bytes());
        off += 16;
        data[off..off + 8].copy_from_slice(&self.last_update_slot.to_le_bytes());
        off += 8;
        data[off..off + 8].copy_from_slice(&self.total_staked.to_le_bytes());
    }
}

struct StakePosition {
    amount: u64,
    deposit_slot: u64,
    reward_per_token_paid: u128,
    pending_rewards: u64,
}

impl StakePosition {
    fn deserialize(data: &[u8]) -> Result<Self, ProgramError> {
        if data.len() < POSITION_SIZE || data[..8] != POSITION_DISC {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            amount: u64::from_le_bytes(data[8..16].try_into().unwrap()),
            deposit_slot: u64::from_le_bytes(data[16..24].try_into().unwrap()),
            reward_per_token_paid: u128::from_le_bytes(data[24..40].try_into().unwrap()),
            pending_rewards: u64::from_le_bytes(data[40..48].try_into().unwrap()),
        })
    }

    fn serialize(&self, data: &mut [u8]) {
        data[..8].copy_from_slice(&POSITION_DISC);
        data[8..16].copy_from_slice(&self.amount.to_le_bytes());
        data[16..24].copy_from_slice(&self.deposit_slot.to_le_bytes());
        data[24..40].copy_from_slice(&self.reward_per_token_paid.to_le_bytes());
        data[40..48].copy_from_slice(&self.pending_rewards.to_le_bytes());
    }
}

#[cfg(not(feature = "no-entrypoint"))]
solana_program::entrypoint!(process_instruction);

pub fn process_instruction<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut data = instruction_data;
    match read_u8(&mut data)? {
        IX_INIT_COIN_CONFIG => process_init_coin_config(program_id, accounts),
        IX_INIT_STAKE_POOL => process_init_stake_pool(program_id, accounts, &mut data),
        IX_STAKE => process_stake(program_id, accounts, &mut data),
        IX_UNSTAKE => process_unstake(program_id, accounts, &mut data),
        IX_CLAIM_STAKE_REWARDS => process_claim_stake_rewards(program_id, accounts),
        IX_SET_STAKE_POOL_REWARDS => process_set_stake_pool_rewards(program_id, accounts, &mut data),
        IX_MINT_REWARD => process_mint_reward(program_id, accounts, &mut data),
        IX_TRANSFER_MINT_AUTHORITY => process_transfer_mint_authority(program_id, accounts),
        IX_TRANSFER_CONFIG_AUTHORITY => process_transfer_config_authority(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn process_init_coin_config<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let payer = next_account_info(iter)?;
    let authority = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let system_program = next_account_info(iter)?;

    if !payer.is_signer || !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *system_program.key != solana_program::system_program::ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    verify_reward_mint_authority(program_id, reward_mint)?;

    create_pda_account(
        payer,
        coin_cfg_account,
        system_program,
        program_id,
        &coin_cfg_seeds(reward_mint.key),
        COIN_CFG_SIZE,
    )?;

    let mut cfg_data = coin_cfg_account.try_borrow_mut_data()?;
    CoinConfig {
        authority: *authority.key,
    }
    .serialize(&mut cfg_data);
    Ok(())
}

fn process_init_stake_pool<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &mut &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let payer = next_account_info(iter)?;
    let authority = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let stake_mint = next_account_info(iter)?;
    let stake_vault = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;
    let rent_sysvar = next_account_info(iter)?;
    let system_program = next_account_info(iter)?;
    let clock_info = next_account_info(iter)?;

    let reward_per_epoch = read_u64(data)?;
    let epoch_slots = read_u64(data)?;
    if epoch_slots == 0 {
        msg!("epoch_slots must be > 0");
        return Err(ProgramError::InvalidInstructionData);
    }
    if !payer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    verify_token_program(token_program)?;
    require_coin_authority(coin_cfg_account, reward_mint.key, authority, program_id)?;
    verify_mint_account(stake_mint)?;
    verify_reward_mint_authority(program_id, reward_mint)?;

    let pool_seed_arr = pool_seeds(stake_mint.key, reward_mint.key);
    create_pda_account(payer, pool_account, system_program, program_id, &pool_seed_arr, POOL_SIZE)?;

    let clock = Clock::from_account_info(clock_info)?;
    let cfg = StakePoolConfig {
        authority: *authority.key,
        stake_mint: *stake_mint.key,
        reward_mint: *reward_mint.key,
        reward_per_epoch,
        epoch_slots,
        start_slot: clock.slot,
        reward_per_token_stored: 0,
        last_update_slot: clock.slot,
        total_staked: 0,
    };
    let mut pool_data = pool_account.try_borrow_mut_data()?;
    cfg.serialize(&mut pool_data);
    drop(pool_data);

    let vault_seeds_arr = stake_vault_seeds(pool_account.key);
    let (expected_vault, vault_bump) = Pubkey::find_program_address(&vault_seeds_arr, program_id);
    if *stake_vault.key != expected_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    let vault_signer_seeds: [&[u8]; 3] =
        [b"stake_vault", pool_account.key.as_ref(), &[vault_bump]];
    let rent = Rent::from_account_info(rent_sysvar)?;
    invoke_signed(
        &system_instruction::create_account(
            payer.key,
            stake_vault.key,
            rent.minimum_balance(spl_token::state::Account::LEN),
            spl_token::state::Account::LEN as u64,
            &spl_token::ID,
        ),
        &[payer.clone(), stake_vault.clone(), system_program.clone()],
        &[&vault_signer_seeds],
    )?;
    let init_ix = spl_token::instruction::initialize_account2(
        token_program.key,
        stake_vault.key,
        stake_mint.key,
        pool_account.key,
    )?;
    invoke(
        &init_ix,
        &[
            stake_vault.clone(),
            stake_mint.clone(),
            rent_sysvar.clone(),
            token_program.clone(),
        ],
    )
}

fn process_stake<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &mut &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let user = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let user_stake_ata = next_account_info(iter)?;
    let stake_vault = next_account_info(iter)?;
    let position_account = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;
    let system_program = next_account_info(iter)?;
    let clock_info = next_account_info(iter)?;

    let amount = read_u64(data)?;
    if amount == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    if !user.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut pool_data = pool_account.try_borrow_mut_data()?;
    let mut cfg = StakePoolConfig::deserialize(&pool_data)?;
    verify_pool_account(pool_account, &cfg, program_id)?;
    verify_token_program(token_program)?;
    validate_token_account(user_stake_ata, &cfg.stake_mint, user.key)?;
    validate_stake_vault(stake_vault, pool_account.key, &cfg.stake_mint, program_id)?;

    let clock = Clock::from_account_info(clock_info)?;
    update_accumulator(&mut cfg, clock.slot);

    let position_seed_arr = position_seeds(pool_account.key, user.key);
    let (expected_position, _) = Pubkey::find_program_address(&position_seed_arr, program_id);
    if *position_account.key != expected_position {
        return Err(ProgramError::InvalidSeeds);
    }

    let mut position = if position_account.lamports() == 0 || position_account.data_len() == 0 {
        drop(pool_data);
        create_pda_account(
            user,
            position_account,
            system_program,
            program_id,
            &position_seed_arr,
            POSITION_SIZE,
        )?;
        pool_data = pool_account.try_borrow_mut_data()?;
        StakePosition {
            amount: 0,
            deposit_slot: 0,
            reward_per_token_paid: 0,
            pending_rewards: 0,
        }
    } else {
        if position_account.owner != program_id {
            return Err(ProgramError::IllegalOwner);
        }
        let position_data = position_account.try_borrow_data()?;
        let position = StakePosition::deserialize(&position_data)?;
        drop(position_data);
        position
    };

    settle_pending(&mut position, cfg.reward_per_token_stored);
    cfg.total_staked = cfg
        .total_staked
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    cfg.serialize(&mut pool_data);
    drop(pool_data);

    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        user_stake_ata.key,
        stake_vault.key,
        user.key,
        &[],
        amount,
    )?;
    invoke(
        &transfer_ix,
        &[
            user_stake_ata.clone(),
            stake_vault.clone(),
            user.clone(),
            token_program.clone(),
        ],
    )?;

    position.amount = position
        .amount
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    position.deposit_slot = clock.slot;
    position.reward_per_token_paid = cfg.reward_per_token_stored;
    let mut position_data = position_account.try_borrow_mut_data()?;
    position.serialize(&mut position_data);
    Ok(())
}

fn process_unstake<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &mut &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let user = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let user_stake_ata = next_account_info(iter)?;
    let stake_vault = next_account_info(iter)?;
    let position_account = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let user_reward_ata = next_account_info(iter)?;
    let mint_authority = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;
    let clock_info = next_account_info(iter)?;

    let amount = read_u64(data)?;
    if amount == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    if !user.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut pool_data = pool_account.try_borrow_mut_data()?;
    let mut cfg = StakePoolConfig::deserialize(&pool_data)?;
    verify_pool_account(pool_account, &cfg, program_id)?;
    verify_token_program(token_program)?;
    validate_token_account(user_stake_ata, &cfg.stake_mint, user.key)?;
    validate_stake_vault(stake_vault, pool_account.key, &cfg.stake_mint, program_id)?;
    validate_token_account(user_reward_ata, &cfg.reward_mint, user.key)?;
    if *reward_mint.key != cfg.reward_mint {
        return Err(ProgramError::InvalidAccountData);
    }

    let clock = Clock::from_account_info(clock_info)?;
    update_accumulator(&mut cfg, clock.slot);
    let mut position = load_position(position_account, pool_account.key, user.key, program_id)?;
    if amount > position.amount {
        return Err(ProgramError::InsufficientFunds);
    }
    settle_pending(&mut position, cfg.reward_per_token_stored);

    cfg.total_staked = cfg.total_staked.saturating_sub(amount);
    cfg.serialize(&mut pool_data);
    drop(pool_data);

    let pool_seed_arr = pool_seeds(&cfg.stake_mint, &cfg.reward_mint);
    let (_, pool_bump) = Pubkey::find_program_address(&pool_seed_arr, program_id);
    let pool_signer_seeds: [&[u8]; 4] = [
        b"stake_pool",
        cfg.stake_mint.as_ref(),
        cfg.reward_mint.as_ref(),
        &[pool_bump],
    ];
    let transfer_ix = spl_token::instruction::transfer(
        token_program.key,
        stake_vault.key,
        user_stake_ata.key,
        pool_account.key,
        &[],
        amount,
    )?;
    invoke_signed(
        &transfer_ix,
        &[
            stake_vault.clone(),
            user_stake_ata.clone(),
            pool_account.clone(),
            token_program.clone(),
        ],
        &[&pool_signer_seeds],
    )?;

    mint_pending_rewards(
        program_id,
        token_program,
        reward_mint,
        user_reward_ata,
        mint_authority,
        cfg.reward_mint,
        position.pending_rewards,
    )?;
    position.amount -= amount;
    position.pending_rewards = 0;

    if position.amount == 0 {
        close_position(position_account, user)?;
    } else {
        let mut position_data = position_account.try_borrow_mut_data()?;
        position.serialize(&mut position_data);
    }
    Ok(())
}

fn process_claim_stake_rewards<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let user = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let position_account = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let user_reward_ata = next_account_info(iter)?;
    let mint_authority = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;
    let clock_info = next_account_info(iter)?;

    if !user.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let mut pool_data = pool_account.try_borrow_mut_data()?;
    let mut cfg = StakePoolConfig::deserialize(&pool_data)?;
    verify_pool_account(pool_account, &cfg, program_id)?;
    verify_token_program(token_program)?;
    validate_token_account(user_reward_ata, &cfg.reward_mint, user.key)?;
    if *reward_mint.key != cfg.reward_mint {
        return Err(ProgramError::InvalidAccountData);
    }

    let clock = Clock::from_account_info(clock_info)?;
    update_accumulator(&mut cfg, clock.slot);
    cfg.serialize(&mut pool_data);
    drop(pool_data);

    let mut position = load_position(position_account, pool_account.key, user.key, program_id)?;
    settle_pending(&mut position, cfg.reward_per_token_stored);
    mint_pending_rewards(
        program_id,
        token_program,
        reward_mint,
        user_reward_ata,
        mint_authority,
        cfg.reward_mint,
        position.pending_rewards,
    )?;
    position.pending_rewards = 0;
    let mut position_data = position_account.try_borrow_mut_data()?;
    position.serialize(&mut position_data);
    Ok(())
}

fn process_set_stake_pool_rewards<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &mut &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let pool_account = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let clock_info = next_account_info(iter)?;

    let new_reward_per_epoch = read_u64(data)?;
    let new_epoch_slots = read_u64(data)?;
    if new_epoch_slots == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    require_coin_authority(coin_cfg_account, reward_mint.key, authority, program_id)?;

    let mut pool_data = pool_account.try_borrow_mut_data()?;
    let mut cfg = StakePoolConfig::deserialize(&pool_data)?;
    verify_pool_account(pool_account, &cfg, program_id)?;
    if cfg.reward_mint != *reward_mint.key {
        return Err(ProgramError::InvalidAccountData);
    }
    let clock = Clock::from_account_info(clock_info)?;
    update_accumulator(&mut cfg, clock.slot);
    cfg.reward_per_epoch = new_reward_per_epoch;
    cfg.epoch_slots = new_epoch_slots;
    cfg.authority = *authority.key;
    cfg.serialize(&mut pool_data);
    Ok(())
}

fn process_mint_reward<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &mut &[u8],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let destination = next_account_info(iter)?;
    let mint_authority = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;

    let amount = read_u64(data)?;
    if amount == 0 {
        return Err(ProgramError::InvalidInstructionData);
    }
    verify_token_program(token_program)?;
    require_coin_authority(coin_cfg_account, reward_mint.key, authority, program_id)?;
    let dest = load_token_account(destination)?;
    if dest.mint != *reward_mint.key {
        return Err(ProgramError::InvalidAccountData);
    }
    mint_pending_rewards(
        program_id,
        token_program,
        reward_mint,
        destination,
        mint_authority,
        *reward_mint.key,
        amount,
    )
}

fn process_transfer_mint_authority<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let mint_authority = next_account_info(iter)?;
    let new_authority = next_account_info(iter)?;
    let token_program = next_account_info(iter)?;

    verify_token_program(token_program)?;
    require_coin_authority(coin_cfg_account, reward_mint.key, authority, program_id)?;
    let mint_seeds_arr = mint_authority_seeds(reward_mint.key);
    let (expected_mint_authority, mint_bump) =
        Pubkey::find_program_address(&mint_seeds_arr, program_id);
    if *mint_authority.key != expected_mint_authority {
        return Err(ProgramError::InvalidSeeds);
    }
    let bump_bytes = [mint_bump];
    let signer_seeds: [&[u8]; 3] = [
        b"reward_mint_authority",
        reward_mint.key.as_ref(),
        &bump_bytes,
    ];
    let ix = spl_token::instruction::set_authority(
        token_program.key,
        reward_mint.key,
        Some(new_authority.key),
        spl_token::instruction::AuthorityType::MintTokens,
        mint_authority.key,
        &[],
    )?;
    invoke_signed(
        &ix,
        &[
            reward_mint.clone(),
            mint_authority.clone(),
            token_program.clone(),
        ],
        &[&signer_seeds],
    )
}

fn process_transfer_config_authority<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
) -> ProgramResult {
    let iter = &mut accounts.iter();
    let authority = next_account_info(iter)?;
    let reward_mint = next_account_info(iter)?;
    let coin_cfg_account = next_account_info(iter)?;
    let new_authority = next_account_info(iter)?;

    require_coin_authority(coin_cfg_account, reward_mint.key, authority, program_id)?;
    let mut data = coin_cfg_account.try_borrow_mut_data()?;
    CoinConfig {
        authority: *new_authority.key,
    }
    .serialize(&mut data);
    Ok(())
}

fn read_u8(data: &mut &[u8]) -> Result<u8, ProgramError> {
    if data.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let value = data[0];
    *data = &data[1..];
    Ok(value)
}

fn read_u64(data: &mut &[u8]) -> Result<u64, ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let value = u64::from_le_bytes(data[..8].try_into().unwrap());
    *data = &data[8..];
    Ok(value)
}

fn create_pda_account<'a>(
    payer: &AccountInfo<'a>,
    target: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    program_id: &Pubkey,
    seeds: &[&[u8]],
    size: usize,
) -> ProgramResult {
    let (expected, bump) = Pubkey::find_program_address(seeds, program_id);
    if *target.key != expected {
        return Err(ProgramError::InvalidSeeds);
    }
    if target.lamports() > 0 {
        return Err(ProgramError::AccountAlreadyInitialized);
    }
    let rent = Rent::get()?;
    let lamports = rent.minimum_balance(size);
    let mut seeds_with_bump: Vec<&[u8]> = Vec::from(seeds);
    let bump_bytes = [bump];
    seeds_with_bump.push(&bump_bytes);
    invoke_signed(
        &system_instruction::create_account(
            payer.key,
            target.key,
            lamports,
            size as u64,
            program_id,
        ),
        &[payer.clone(), target.clone(), system_program.clone()],
        &[&seeds_with_bump],
    )
}

fn verify_token_program(token_program: &AccountInfo) -> ProgramResult {
    if *token_program.key != spl_token::ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

fn verify_mint_account(mint_account: &AccountInfo) -> ProgramResult {
    if mint_account.owner != &spl_token::ID {
        return Err(ProgramError::IllegalOwner);
    }
    let data = mint_account.try_borrow_data()?;
    spl_token::state::Mint::unpack(&data)?;
    Ok(())
}

fn verify_reward_mint_authority(program_id: &Pubkey, reward_mint: &AccountInfo) -> ProgramResult {
    verify_mint_account(reward_mint)?;
    let mint_data = reward_mint.try_borrow_data()?;
    let mint = spl_token::state::Mint::unpack(&mint_data)?;
    if mint.freeze_authority.is_some() {
        msg!("Reward mint must have freeze_authority = None");
        return Err(ProgramError::InvalidAccountData);
    }
    let (expected_authority, _) =
        Pubkey::find_program_address(&mint_authority_seeds(reward_mint.key), program_id);
    match mint.mint_authority {
        COption::Some(authority) if authority == expected_authority => Ok(()),
        _ => {
            msg!("Reward mint authority must be this program's PDA");
            Err(ProgramError::InvalidAccountData)
        }
    }
}

fn load_token_account(account: &AccountInfo) -> Result<spl_token::state::Account, ProgramError> {
    if account.owner != &spl_token::ID {
        return Err(ProgramError::IllegalOwner);
    }
    let data = account.try_borrow_data()?;
    spl_token::state::Account::unpack(&data).map_err(|_| ProgramError::InvalidAccountData)
}

fn validate_token_account(
    account: &AccountInfo,
    expected_mint: &Pubkey,
    expected_owner: &Pubkey,
) -> ProgramResult {
    let token = load_token_account(account)?;
    if token.mint != *expected_mint || token.owner != *expected_owner {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

fn require_coin_authority(
    coin_cfg_account: &AccountInfo,
    reward_mint: &Pubkey,
    authority: &AccountInfo,
    program_id: &Pubkey,
) -> ProgramResult {
    if !authority.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let (expected_cfg, _) = Pubkey::find_program_address(&coin_cfg_seeds(reward_mint), program_id);
    if *coin_cfg_account.key != expected_cfg {
        return Err(ProgramError::InvalidSeeds);
    }
    if coin_cfg_account.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }
    let cfg_data = coin_cfg_account.try_borrow_data()?;
    let cfg = CoinConfig::deserialize(&cfg_data)?;
    if cfg.authority != *authority.key {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

fn verify_pool_account(
    pool_account: &AccountInfo,
    cfg: &StakePoolConfig,
    program_id: &Pubkey,
) -> ProgramResult {
    if pool_account.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }
    let (expected_pool, _) =
        Pubkey::find_program_address(&pool_seeds(&cfg.stake_mint, &cfg.reward_mint), program_id);
    if *pool_account.key != expected_pool {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(())
}

fn validate_stake_vault(
    stake_vault: &AccountInfo,
    pool: &Pubkey,
    stake_mint: &Pubkey,
    program_id: &Pubkey,
) -> ProgramResult {
    let (expected_vault, _) = Pubkey::find_program_address(&stake_vault_seeds(pool), program_id);
    if *stake_vault.key != expected_vault {
        return Err(ProgramError::InvalidSeeds);
    }
    validate_token_account(stake_vault, stake_mint, pool)
}

fn load_position(
    position_account: &AccountInfo,
    pool: &Pubkey,
    user: &Pubkey,
    program_id: &Pubkey,
) -> Result<StakePosition, ProgramError> {
    if position_account.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }
    let (expected_position, _) =
        Pubkey::find_program_address(&position_seeds(pool, user), program_id);
    if *position_account.key != expected_position {
        return Err(ProgramError::InvalidSeeds);
    }
    let data = position_account.try_borrow_data()?;
    StakePosition::deserialize(&data)
}

fn close_position(position_account: &AccountInfo, user: &AccountInfo) -> ProgramResult {
    let dest_lamports = user.lamports();
    **user.try_borrow_mut_lamports()? = dest_lamports
        .checked_add(position_account.lamports())
        .ok_or(ProgramError::ArithmeticOverflow)?;
    **position_account.try_borrow_mut_lamports()? = 0;
    let mut data = position_account.try_borrow_mut_data()?;
    data.fill(0);
    Ok(())
}

fn mint_pending_rewards<'a>(
    program_id: &Pubkey,
    token_program: &AccountInfo<'a>,
    reward_mint: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    mint_authority: &AccountInfo<'a>,
    expected_reward_mint: Pubkey,
    amount: u64,
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    if *reward_mint.key != expected_reward_mint {
        return Err(ProgramError::InvalidAccountData);
    }
    let mint_seeds_arr = mint_authority_seeds(reward_mint.key);
    let (expected_mint_authority, mint_bump) =
        Pubkey::find_program_address(&mint_seeds_arr, program_id);
    if *mint_authority.key != expected_mint_authority {
        return Err(ProgramError::InvalidSeeds);
    }
    let bump_bytes = [mint_bump];
    let signer_seeds: [&[u8]; 3] = [
        b"reward_mint_authority",
        reward_mint.key.as_ref(),
        &bump_bytes,
    ];
    let ix = spl_token::instruction::mint_to(
        token_program.key,
        reward_mint.key,
        destination.key,
        mint_authority.key,
        &[],
        amount,
    )?;
    invoke_signed(
        &ix,
        &[
            reward_mint.clone(),
            destination.clone(),
            mint_authority.clone(),
            token_program.clone(),
        ],
        &[&signer_seeds],
    )
}

fn update_accumulator(cfg: &mut StakePoolConfig, current_slot: u64) {
    if cfg.total_staked == 0 || current_slot <= cfg.last_update_slot || cfg.epoch_slots == 0 {
        cfg.last_update_slot = current_slot;
        return;
    }
    let elapsed = current_slot - cfg.last_update_slot;
    let reward_elapsed = (cfg.reward_per_epoch as u128).saturating_mul(elapsed as u128);
    let (num_lo, num_hi) = mul_u128_wide(reward_elapsed, FP);
    let denom = (cfg.epoch_slots as u128).saturating_mul(cfg.total_staked as u128);
    if denom > 0 {
        let delta = div_u256_by_u128(num_lo, num_hi, denom);
        cfg.reward_per_token_stored = cfg.reward_per_token_stored.saturating_add(delta);
    }
    cfg.last_update_slot = current_slot;
}

fn settle_pending(position: &mut StakePosition, reward_per_token: u128) {
    if position.amount == 0 {
        position.reward_per_token_paid = reward_per_token;
        return;
    }
    let delta = reward_per_token.saturating_sub(position.reward_per_token_paid);
    let (lo, hi) = mul_u128_wide(position.amount as u128, delta);
    let earned_u128 = (lo >> 64) | (hi << 64);
    let earned = core::cmp::min(earned_u128, u64::MAX as u128) as u64;
    position.pending_rewards = position.pending_rewards.saturating_add(earned);
    position.reward_per_token_paid = reward_per_token;
}

fn mul_u128_wide(a: u128, b: u128) -> (u128, u128) {
    let a_lo = a as u64 as u128;
    let a_hi = a >> 64;
    let b_lo = b as u64 as u128;
    let b_hi = b >> 64;

    let ll = a_lo * b_lo;
    let lh = a_lo * b_hi;
    let hl = a_hi * b_lo;
    let hh = a_hi * b_hi;

    let mid = (ll >> 64) + (lh & 0xFFFF_FFFF_FFFF_FFFF) + (hl & 0xFFFF_FFFF_FFFF_FFFF);
    let lo = (ll & 0xFFFF_FFFF_FFFF_FFFF) | (mid << 64);
    let hi = hh + (lh >> 64) + (hl >> 64) + (mid >> 64);

    (lo, hi)
}

fn div_u256_by_u128(n_lo: u128, n_hi: u128, d: u128) -> u128 {
    if d == 0 {
        return u128::MAX;
    }
    if n_hi == 0 {
        return n_lo / d;
    }
    if n_hi >= d {
        return u128::MAX;
    }

    let mut rem: u128 = n_hi;
    let mut quot: u128 = 0;
    for i in (0..128u32).rev() {
        let bit = (n_lo >> i) & 1;
        let overflow = rem >> 127 != 0;
        rem = rem.wrapping_shl(1) | bit;
        if overflow || rem >= d {
            rem = rem.wrapping_sub(d);
            quot |= 1u128 << i;
        }
    }
    quot
}
