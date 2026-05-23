# Ownership Coin Staking Program

Standalone staking rewards program for ownership coins launched through
MetaDAO.

This crate intentionally has no dependency on any launch program. Governance is
modeled as a generic signer stored in `CoinConfig`; for a coin launched through
MetaDAO, set that signer to the coin's launch/governance execution authority and
route admin lifecycle calls through that governance flow.

## Reward Model

- Users stake an ownership coin, `stake_mint`, into a program-owned stake vault.
- Rewards are minted from `reward_mint` by the program PDA:
  `["reward_mint_authority", reward_mint]`.
- Rewards accrue per slot using `reward_per_epoch / epoch_slots`.
- User-facing `stake`, `unstake`, and `claim_stake_rewards` are permissionless.

## Instruction Tags

- `0` - `init_coin_config`
- `1` - `init_stake_pool`
- `2` - `stake`
- `3` - `unstake`
- `4` - `claim_stake_rewards`
- `5` - `set_stake_pool_rewards`
- `6` - `mint_reward`
- `7` - `transfer_mint_authority`
- `8` - `transfer_config_authority`

## PDA Seeds

- `coin_config`: `["coin_cfg", reward_mint]`
- `stake_pool`: `["stake_pool", stake_mint, reward_mint]`
- `stake_vault`: `["stake_vault", stake_pool]`
- `stake_position`: `["stake_position", stake_pool, user]`
- `reward_mint_authority`: `["reward_mint_authority", reward_mint]`

## Lifecycle

1. Create the reward mint with mint authority set to
   `reward_mint_authority`.
2. Call `init_coin_config` with the desired MetaDAO authority signer.
3. Call `init_stake_pool` for each `(stake_mint, reward_mint)` pair.
4. Users call `stake`, `claim_stake_rewards`, and `unstake`.
5. The configured coin authority can call `set_stake_pool_rewards`,
   `mint_reward`, `transfer_mint_authority`, and
   `transfer_config_authority`.
