use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
};
use fuel_core_benches::db_lookup_times_utils::{
    full_block_table::BenchDatabase,
    matrix::{
        matrix,
        should_clean,
    },
    seed::{
        seed_compressed_blocks_and_transactions_matrix,
        seed_full_block_matrix,
    },
    utils::{
        get_random_block_height,
        open_rocks_db,
        LookupMethod,
        Result as DbLookupBenchResult,
    },
};

use rand::thread_rng;

pub fn header_and_tx_lookup(c: &mut Criterion) -> DbLookupBenchResult<impl FnOnce()> {
    let method = LookupMethod::HeaderAndTx;
    let mut rng = thread_rng();

    let cleaner = seed_compressed_blocks_and_transactions_matrix(method)?;
    let mut group = c.benchmark_group(method.as_ref());

    for (block_count, tx_count) in matrix() {
        let database = open_rocks_db::<BenchDatabase>(block_count, tx_count, method)?;
        group.bench_function(format!("{block_count}/{tx_count}"), |b| {
            b.iter(|| {
                let height = get_random_block_height(&mut rng, block_count);
                let block = method.get_block(&database, height);
                assert!(block.is_ok());
            });
        });
    }

    group.finish();
    Ok(cleaner)
}

pub fn multi_get_lookup(c: &mut Criterion) -> DbLookupBenchResult<impl FnOnce()> {
    let method = LookupMethod::MultiGet;
    let mut rng = thread_rng();

    let cleaner = seed_compressed_blocks_and_transactions_matrix(method)?;
    let mut group = c.benchmark_group(method.as_ref());

    for (block_count, tx_count) in matrix() {
        let database = open_rocks_db(block_count, tx_count, method)?;
        group.bench_function(format!("{block_count}/{tx_count}"), |b| {
            b.iter(|| {
                let height = get_random_block_height(&mut rng, block_count);
                let block = method.get_block(&database, height);
                assert!(block.is_ok());
            });
        });
    }

    group.finish();
    Ok(cleaner)
}

pub fn full_block_lookup(c: &mut Criterion) -> DbLookupBenchResult<impl FnOnce()> {
    let method = LookupMethod::FullBlock;
    let mut rng = thread_rng();

    let cleaner = seed_full_block_matrix()?;
    let mut group = c.benchmark_group(method.as_ref());

    for (block_count, tx_count) in matrix() {
        let database = open_rocks_db(block_count, tx_count, method)?;
        group.bench_function(format!("{block_count}/{tx_count}"), |b| {
            b.iter(|| {
                let height = get_random_block_height(&mut rng, block_count);
                let full_block = method.get_block(&database, height);
                assert!(full_block.is_ok());
            });
        });
    }

    group.finish();
    Ok(cleaner)
}

fn construct_and_run_benchmarks(c: &mut Criterion) {
    let header_and_tx_cleaner = header_and_tx_lookup(c).unwrap();
    let multi_get_cleaner = multi_get_lookup(c).unwrap();
    let full_block_cleaner = full_block_lookup(c).unwrap();

    if should_clean() {
        header_and_tx_cleaner();
        multi_get_cleaner();
        full_block_cleaner();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).measurement_time(std::time::Duration::from_secs(10));
    targets = construct_and_run_benchmarks
}
criterion_main!(benches);
