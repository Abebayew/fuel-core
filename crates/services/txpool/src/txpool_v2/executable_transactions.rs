use crate::{
    containers::tip_per_gas_sort::RatioGasTipSort,
    txpool_v2::{
        collision_detector::CollisionDetector,
        CoinMessage,
    },
    types::TxId,
    TxInfo,
};
use fuel_core_types::{
    entities::coins::coin::CompressedCoin,
    fuel_tx::{
        Input,
        Output,
        UtxoId,
    },
    services::txpool::{
        ArcPoolTx,
        Error,
    },
};
use num_rational::Ratio;
use std::collections::HashMap;

pub type RemovedCoins = Vec<UtxoId>;

pub enum RemoveReason {
    UpcomingTxIsMoreProfitable(TxId),
    UpcomingTxCrowdOutLowestTx(TxId),
}

pub struct RemovedTransaction {
    pub tx_id: TxId,
    pub removed_coins: RemovedCoins,
    pub reason: RemoveReason,
}

pub type RemovedTransactions = Vec<RemovedTransaction>;
pub type UpcomingCoins = Vec<(UtxoId, CompressedCoin)>;

pub enum InsertionResult {
    Successfully(UpcomingCoins),
    SuccessfullyWithRemovedTransactions {
        upcoming_coins: UpcomingCoins,
        removed_transaction: RemovedTransactions,
    },
}

pub struct ExecutableTransactions {
    max_pool_gas: u64,
    current_pool_gas: u64,
    tx_info: HashMap<TxId, TxInfo>,
    collision_detector: CollisionDetector,
    // TODO: Add sorting by the maximum gas price as well. Maybe use `Treap` data structure.
    sort: RatioGasTipSort,
    upcoming_inputs: HashMap<UtxoId, CompressedCoin>,
}

impl ExecutableTransactions {
    pub fn new(max_pool_gas: u64) -> Self {
        Self {
            max_pool_gas,
            current_pool_gas: 0,
            tx_info: HashMap::default(),
            collision_detector: CollisionDetector::default(),
            sort: RatioGasTipSort::default(),
            upcoming_inputs: HashMap::default(),
        }
    }

    pub fn contains(&self, tx_id: &TxId) -> bool {
        self.tx_info.contains_key(tx_id)
    }

    pub fn insert(&mut self, tx: ArcPoolTx) -> Result<InsertionResult, Error> {
        let new_tx_ratio = Ratio::new(tx.tip(), tx.max_gas());
        let target_gas = self.max_pool_gas.saturating_sub(tx.max_gas());
        let info = TxInfo::new(tx);

        if self.current_pool_gas > target_gas {
            let Some((_, tx)) = self.sort.lowest() else {
                unreachable!(
                    "The `current_pool_gas` is non zero, \
                    so there should be at least one transaction in the pool"
                );
            };
            let lowest_tx_ratio = Ratio::new(tx.tip(), tx.max_gas());

            if lowest_tx_ratio >= new_tx_ratio {
                return Err(Error::NotInsertedLimitHit);
            }
        }

        let removed_txs_because_of_collision =
            self.remove_collided_transactions(&info.tx)?;

        // Below this point, the transaction must be inserted
        // because we removed already collided transactions.

        let removed_txs_because_of_limit = self.crowd_out_transactions(&info.tx);

        let mut total_removed_txs = removed_txs_because_of_collision;
        total_removed_txs.extend(removed_txs_because_of_limit);

        let upcoming_coins = self.insert_tx(info);

        if total_removed_txs.is_empty() {
            Ok(InsertionResult::Successfully(upcoming_coins))
        } else {
            Ok(InsertionResult::SuccessfullyWithRemovedTransactions {
                upcoming_coins,
                removed_transaction: total_removed_txs,
            })
        }
    }

    /// The method just inserts the transaction without any validation and can't return an error.
    fn insert_tx(&mut self, info: TxInfo) -> UpcomingCoins {
        self.sort.insert(&info);

        let upcoming_coins = upcoming_coins(info.tx()).collect::<Vec<_>>();
        for (utxo_id, coin) in upcoming_coins.iter() {
            self.upcoming_inputs.insert(*utxo_id, *coin);
        }

        self.current_pool_gas = self.current_pool_gas.saturating_add(info.tx.max_gas());
        self.tx_info.insert(info.tx.id(), info);
        upcoming_coins
    }

    pub fn remove(
        &mut self,
        tx_id: &TxId,
        reason: RemoveReason,
    ) -> Option<RemovedTransaction> {
        let info = self.tx_info.remove(tx_id);

        let Some(info) = info else { return None };
        self.sort.remove(&info);
        self.current_pool_gas = self.current_pool_gas.saturating_sub(info.tx.max_gas());

        let removed_inputs = upcoming_coins(info.tx())
            .map(|(utxo_id, _)| utxo_id)
            .collect::<Vec<_>>();
        for utxo_id in removed_inputs.iter() {
            self.upcoming_inputs.remove(utxo_id);
        }
        self.collision_detector.remove(tx_id);
        Some(RemovedTransaction {
            tx_id: *tx_id,
            removed_coins: removed_inputs,
            reason,
        })
    }

    /// Gets the upcoming coin.
    pub fn get_coin(&self, utxo_id: &UtxoId) -> Option<&CompressedCoin> {
        self.upcoming_inputs.get(utxo_id)
    }

    fn remove_collided_transactions(
        &mut self,
        tx: &ArcPoolTx,
    ) -> Result<RemovedTransactions, Error> {
        let new_tx_ratio = Ratio::new(tx.tip(), tx.max_gas());

        let collision = self.collision_detector.insert(tx.clone());

        let result = if let Some(collision) = collision {
            let (total_tip, total_gas) = collision.iter_collided_transactions().fold(
                (0u64, 0u64),
                |(total_tip, total_gas), collided_tx| {
                    let total_tip = total_tip.saturating_add(collided_tx.tip());
                    let total_gas = total_gas.saturating_add(collided_tx.max_gas());
                    (total_tip, total_gas)
                },
            );

            let collided_ratio = Ratio::new(total_tip, total_gas);

            if new_tx_ratio <= collided_ratio {
                return Err(Error::NotInsertedBecauseCollidedTransactionsMoreProfitable);
            }

            let removed_txs = collision.apply_and_remove_collided_transactions();
            removed_txs
                .into_iter()
                .filter_map(|(tx_id, _)| {
                    self.remove(&tx_id, RemoveReason::UpcomingTxIsMoreProfitable(tx_id))
                })
                .collect()
        } else {
            vec![]
        };

        Ok(result)
    }

    fn crowd_out_transactions(&mut self, new_tx: &ArcPoolTx) -> RemovedTransactions {
        let target_gas = self.max_pool_gas.saturating_sub(new_tx.max_gas());
        let mut removed_txs = vec![];
        while self.current_pool_gas > target_gas {
            let Some((_, tx)) = self.sort.lowest() else {
                unreachable!(
                    "The `current_pool_gas` is non zero, \
                    so there should be at least one transaction in the pool"
                );
            };

            if let Some(removed) = self.remove(
                &tx.id(),
                RemoveReason::UpcomingTxCrowdOutLowestTx(new_tx.id()),
            ) {
                removed_txs.push(removed);
            }
        }
        removed_txs
    }
}

fn upcoming_coins<'a>(
    tx: &'a ArcPoolTx,
) -> impl Iterator<Item = (UtxoId, CompressedCoin)> + 'a {
    tx.outputs()
        .iter()
        .enumerate()
        .filter_map(|(output_index, output)| {
            match output {
                Output::Coin {
                    amount,
                    to,
                    asset_id,
                } => {
                    let utxo_id = UtxoId::new(
                        tx.id(),
                        u16::try_from(output_index)
                            .expect("The maximum number of outputs is `u16::MAX`"),
                    );
                    let mut coin = CompressedCoin::default();
                    coin.set_amount(*amount);
                    coin.set_owner(*to);
                    coin.set_asset_id(*asset_id);
                    Some((utxo_id, coin))
                }
                Output::Contract(_) => {
                    // Do nothing
                    None
                }
                Output::Change { .. }
                | Output::Variable { .. }
                | Output::ContractCreated { .. } => {
                    // We only allow the creation of dependent transactions from `Coin` outputs.
                    // The `Change` and `Variable` are dynamic and only known after
                    // the transaction is executed. Transactions that use these outputs
                    // are not allowed to be dependent but allowed to live in the `ResolutionQueue`.
                    // So if a parent transaction with `Change` or `Variable` output is executed
                    // successfully, dependent transactions have a chance to move from
                    // `ResolutionQueue` to `ExecutableTransactions` or `DependentTransactions`.
                    // The same rules applies to the `ContractCreated` output. The contract
                    // doesn't exist until the transaction is executed and
                    // we don't allow dependent transactions on this contract.
                    None
                }
            }
        })
}
