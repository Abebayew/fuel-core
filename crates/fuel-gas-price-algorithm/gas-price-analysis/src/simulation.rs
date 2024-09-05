use fuel_gas_price_algorithm::v1::{
    AlgorithmUpdaterV1,
    RecordedBlock,
};
use std::{
    iter,
    iter::{
        Enumerate,
        Zip,
    },
    num::NonZeroU64,
    slice::Iter,
};

use super::*;

pub struct SimulationResults {
    pub gas_prices: Vec<u64>,
    pub exec_gas_prices: Vec<u64>,
    pub da_gas_prices: Vec<u64>,
    pub fullness: Vec<(u64, u64)>,
    pub bytes_and_costs: Vec<(u64, u64)>,
    pub actual_profit: Vec<i128>,
    pub projected_profit: Vec<i128>,
    pub pessimistic_costs: Vec<u128>,
}

pub struct Simulator {
    da_cost_per_byte: Vec<u64>,
}

pub fn get_da_cost_per_byte_from_source(
    source: Source,
    update_period: usize,
) -> Vec<u64> {
    match source {
        Source::Generated { size } => arbitrary_cost_per_byte(size, update_period),
        Source::Predefined {
            file_path,
            sample_size,
        } => {
            let original = get_costs_from_csv_file(&file_path, sample_size);
            original
                .into_iter()
                .map(|x| iter::repeat(x).take(update_period))
                .flatten()
                .collect()
        }
    }
}

// block_number,excess_blob_gas,blob_gas_used,blob_fee_wei,blob_fee_wei_for_1_blob,blob_fee_wei_for_2_blobs,blob_fee_wei_for_3_blobs
#[derive(Debug, serde::Deserialize)]
struct Record {
    block_number: u64,
    excess_blob_gas: u64,
    blob_gas_used: u64,
    blob_fee_wei: u64,
    blob_fee_wei_for_1_blob: u64,
    blob_fee_wei_for_2_blobs: u64,
    blob_fee_wei_for_3_blobs: u64,
}

fn get_costs_from_csv_file(file_path: &str, sample_size: Option<usize>) -> Vec<u64> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(file_path)
        .unwrap();
    let mut costs = vec![];
    let headers = csv::StringRecord::from(vec![
        "block_number",
        "excess_blob_gas",
        "blob_gas_used",
        "blob_fee_wei",
        "blob_fee_wei_for_1_blob",
        "blob_fee_wei_for_2_blobs",
        "blob_fee_wei_for_3_blobs",
    ]);
    let mut max_cost = 0;
    for record in rdr.records().skip(1) {
        if let Some(size) = sample_size {
            if costs.len() >= size {
                break;
            }
        };
        let record: Record = record.unwrap().deserialize(Some(&headers)).unwrap();
        let cost = record.blob_fee_wei;
        if cost > max_cost {
            max_cost = cost;
        }
        costs.push(cost);
    }
    println!("Max cost: {}", pretty(max_cost));
    costs
}

impl Simulator {
    pub fn new(da_cost_per_byte: Vec<u64>) -> Self {
        Simulator { da_cost_per_byte }
    }

    pub fn run_simulation(
        &self,
        da_p_component: i64,
        da_d_component: i64,
        da_recording_rate: usize,
    ) -> SimulationResults {
        let capacity = 30_000_000;
        let gas_per_byte = 63;
        let max_block_bytes = capacity / gas_per_byte;
        let size = self.da_cost_per_byte.len();
        let fullness_and_bytes = fullness_and_bytes_per_block(size, capacity);

        let l2_blocks = fullness_and_bytes
            .iter()
            .map(|(fullness, bytes)| (*fullness, *bytes))
            .collect::<Vec<_>>();
        let da_blocks =
            self.zip_l2_blocks_with_da_blocks(da_recording_rate, &fullness_and_bytes);

        let blocks = l2_blocks.iter().zip(da_blocks.iter()).enumerate();

        let updater = self.build_updater(da_p_component, da_d_component);

        self.execute_simulation(
            capacity,
            max_block_bytes,
            fullness_and_bytes,
            blocks,
            updater,
        )
    }

    fn build_updater(
        &self,
        da_p_component: i64,
        da_d_component: i64,
    ) -> AlgorithmUpdaterV1 {
        let gas_price_factor = 100;
        let updater = AlgorithmUpdaterV1 {
            min_exec_gas_price: 10,
            min_da_gas_price: 10,
            new_scaled_exec_price: 10 * gas_price_factor,
            // last_da_gas_price: *starting_da_gas_price,
            last_da_gas_price: 100,
            gas_price_factor: NonZeroU64::new(gas_price_factor).unwrap(),
            l2_block_height: 0,
            l2_block_fullness_threshold_percent: 50,
            exec_gas_price_change_percent: 2,
            max_da_gas_price_change_percent: 10,
            total_da_rewards: 0,
            da_recorded_block_height: 0,
            // latest_da_cost_per_byte: *starting_da_gas_price as u128,
            latest_da_cost_per_byte: 0,
            projected_total_da_cost: 0,
            latest_known_total_da_cost: 0,
            unrecorded_blocks: vec![],
            da_p_component,
            da_d_component,
            last_profit: 0,
            second_to_last_profit: 0,
        };
        updater
    }

    fn execute_simulation(
        &self,
        capacity: u64,
        max_block_bytes: u64,
        fullness_and_bytes: Vec<(u64, u64)>,
        blocks: Enumerate<Zip<Iter<(u64, u64)>, Iter<Option<Vec<RecordedBlock>>>>>,
        mut updater: AlgorithmUpdaterV1,
    ) -> SimulationResults {
        let mut gas_prices = vec![];
        let mut exec_gas_prices = vec![];
        let mut da_gas_prices = vec![];
        let mut actual_reward_totals = vec![];
        let mut projected_cost_totals = vec![];
        let mut actual_costs = vec![];
        let mut pessimistic_costs = vec![];
        for (index, ((fullness, bytes), da_block)) in blocks {
            let height = index as u32 + 1;
            exec_gas_prices.push(updater.new_scaled_exec_price);
            let gas_price = updater.algorithm().calculate(max_block_bytes);
            gas_prices.push(gas_price);
            // Update DA blocks on the occasion there is one

            if let Some(mut da_blocks) = da_block.clone() {
                let mut total_costs = updater.latest_known_total_da_cost;
                for block in &mut da_blocks {
                    total_costs += block.block_cost as u128;
                    actual_costs.push(total_costs);
                }
                updater.update_da_record_data(da_blocks.to_owned()).unwrap();
                assert_eq!(total_costs, updater.projected_total_da_cost);
                assert_eq!(total_costs, updater.latest_known_total_da_cost);
            }
            updater
                .update_l2_block_data(
                    height,
                    *fullness,
                    capacity.try_into().unwrap(),
                    *bytes,
                    gas_price,
                )
                .unwrap();
            da_gas_prices.push(updater.last_da_gas_price);
            pessimistic_costs
                .push(max_block_bytes as u128 * updater.latest_da_cost_per_byte);
            actual_reward_totals.push(updater.total_da_rewards);
            projected_cost_totals.push(updater.projected_total_da_cost);
        }

        let (fullness_without_capacity, bytes): (Vec<_>, Vec<_>) =
            fullness_and_bytes.iter().cloned().unzip();
        let fullness = fullness_without_capacity
            .iter()
            .map(|&fullness| (fullness, capacity))
            .collect();
        let bytes_and_costs = bytes
            .iter()
            .zip(self.da_cost_per_byte.iter())
            .map(|(bytes, cost_per_byte)| (*bytes, (*bytes * cost_per_byte) as u64))
            .collect();

        let actual_profit: Vec<i128> = actual_costs
            .iter()
            .zip(actual_reward_totals.iter())
            .map(|(cost, reward)| *reward as i128 - *cost as i128)
            .collect();

        let projected_profit: Vec<i128> = projected_cost_totals
            .iter()
            .zip(actual_reward_totals.iter())
            .map(|(cost, reward)| *reward as i128 - *cost as i128)
            .collect();

        SimulationResults {
            gas_prices,
            exec_gas_prices,
            da_gas_prices,
            fullness,
            bytes_and_costs,
            actual_profit,
            projected_profit,
            pessimistic_costs,
        }
    }

    fn zip_l2_blocks_with_da_blocks(
        &self,
        da_recording_rate: usize,
        fullness_and_bytes: &Vec<(u64, u64)>,
    ) -> Vec<Option<Vec<RecordedBlock>>> {
        let (_, da_blocks) = fullness_and_bytes
            .iter()
            .zip(self.da_cost_per_byte.iter())
            .enumerate()
            .fold(
                (vec![], vec![]),
                |(mut delayed, mut recorded),
                 (index, ((_fullness, bytes), cost_per_byte))| {
                    let total_cost = *bytes * cost_per_byte;
                    let height = index as u32 + 1;
                    let converted = RecordedBlock {
                        height,
                        block_bytes: *bytes,
                        block_cost: total_cost as u64,
                    };
                    delayed.push(converted);
                    if delayed.len() == da_recording_rate {
                        recorded.push(Some(delayed));
                        (vec![], recorded)
                    } else {
                        recorded.push(None);
                        (delayed, recorded)
                    }
                },
            );
        da_blocks
    }
}

// Naive Fourier series
fn gen_noisy_signal(input: f64, components: &[f64]) -> f64 {
    components
        .iter()
        .fold(0f64, |acc, &c| acc + f64::sin(input / c))
        / components.len() as f64
}

fn noisy_fullness<T: TryInto<f64>>(input: T) -> f64
where
    <T as TryInto<f64>>::Error: core::fmt::Debug,
{
    const COMPONENTS: &[f64] = &[-30.0, 40.0, 700.0, -340.0, 400.0];
    let input = input.try_into().unwrap();
    gen_noisy_signal(input, COMPONENTS)
}

fn fullness_and_bytes_per_block(size: usize, capacity: u64) -> Vec<(u64, u64)> {
    let mut rng = StdRng::seed_from_u64(888);

    let fullness_noise: Vec<_> = std::iter::repeat(())
        .take(size)
        .map(|_| rng.gen_range(-0.25..0.25))
        .map(|val| val * capacity as f64)
        .collect();

    const ROUGH_GAS_TO_BYTE_RATIO: f64 = 0.01;
    let bytes_scale: Vec<_> = std::iter::repeat(())
        .take(size)
        .map(|_| rng.gen_range(0.5..1.0))
        .map(|x| x * ROUGH_GAS_TO_BYTE_RATIO)
        .collect();

    (0usize..size)
        .map(|val| val as f64)
        .map(noisy_fullness)
        .map(|signal| (0.5 * signal + 0.5) * capacity as f64) // Scale and shift so it's between 0 and capacity
        .zip(fullness_noise)
        .map(|(fullness, noise)| fullness + noise)
        .map(|x| f64::min(x, capacity as f64))
        .map(|x| f64::max(x, 5.0))
        .zip(bytes_scale)
        .map(|(fullness, bytes_scale)| {
            let bytes = fullness * bytes_scale;
            (fullness, bytes)
        })
        .map(|(fullness, bytes)| (fullness as u64, std::cmp::max(bytes as u64, 1)))
        .collect()
}

fn noisy_eth_price<T: TryInto<f64>>(input: T) -> f64
where
    <T as TryInto<f64>>::Error: core::fmt::Debug,
{
    const COMPONENTS: &[f64] = &[3.0, 4.0];
    let input = input.try_into().unwrap();
    gen_noisy_signal(input, COMPONENTS)
}

fn arbitrary_cost_per_byte(size: usize, update_period: usize) -> Vec<u64> {
    let actual_size = size.div_ceil(update_period);

    const ROUGH_COST_AVG: f64 = 5.0;

    (0u32..actual_size as u32)
        .map(noisy_eth_price)
        .map(|x| x * ROUGH_COST_AVG + ROUGH_COST_AVG) // Sine wave is between -1 and 1, scale and shift
        .map(|x| x as u64)
        .map(|x| std::cmp::max(x, 1))
        .map(|x| iter::repeat(x).take(update_period as usize))
        .flatten()
        .take(size as usize)
        .collect()
}
