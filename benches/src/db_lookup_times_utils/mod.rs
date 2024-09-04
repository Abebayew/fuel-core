pub mod full_block_table;
pub mod matrix;
pub mod seed;
pub mod utils;

#[cfg(test)]
mod tests {
    use crate::db_lookup_times_utils::{
        full_block_table::BenchDatabase,
        utils::LookupMethod,
    };
    use fuel_core::state::rocks_db::RocksDb;

    use crate::db_lookup_times_utils::seed::{
        insert_compressed_block,
        insert_full_block,
    };
    use tempdir::TempDir;

    const TEST_HEIGHT: u32 = 1;
    const TEST_TX_COUNT: u32 = 10;

    fn setup_test_db() -> RocksDb<BenchDatabase> {
        let temp_dir = TempDir::new("test_database_bench").unwrap();
        RocksDb::default_open(temp_dir.path(), None).unwrap()
    }

    #[test]
    fn test_insert_and_fetch_compressed_block() {
        // given
        let mut db = setup_test_db();

        // when
        let inserted_block =
            insert_compressed_block(&mut db, TEST_HEIGHT.into(), TEST_TX_COUNT).unwrap();

        // then
        let fetched_block =
            LookupMethod::get_block(&LookupMethod::HeaderAndTx, &db, TEST_HEIGHT.into())
                .unwrap();
        assert_eq!(inserted_block, fetched_block);
    }

    #[test]
    fn test_insert_and_fetch_full_block() {
        // given
        let mut db = setup_test_db();

        // when
        let block =
            insert_full_block(&mut db, TEST_HEIGHT.into(), TEST_TX_COUNT).unwrap();

        // then
        let fetched_block =
            LookupMethod::get_block(&LookupMethod::FullBlock, &db, TEST_HEIGHT.into())
                .unwrap();
        assert_eq!(block, fetched_block);
    }

    #[test]
    fn test_insert_and_multi_get_block() {
        // given
        let mut db = setup_test_db();

        // when
        let block =
            insert_compressed_block(&mut db, TEST_HEIGHT.into(), TEST_TX_COUNT).unwrap();

        // then
        let fetched_block =
            LookupMethod::get_block(&LookupMethod::MultiGet, &db, TEST_HEIGHT.into())
                .unwrap();
        assert_eq!(block, fetched_block);
    }
}
