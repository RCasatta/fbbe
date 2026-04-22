use std::{sync::Arc, time::Duration};

use tokio::time::sleep;

use crate::{threads::index_addresses::Database, ROCKSDB_ESTIMATED_KEYS_GAUGE};

pub(crate) async fn update_db_stats_infallible(db: Arc<Database>) {
    if let Err(e) = update_db_stats(db).await {
        log::error!("{:?}", e);
    }
}

async fn update_db_stats(db: Arc<Database>) -> Result<(), rocksdb::Error> {
    log::info!("Starting update_db_stats");

    loop {
        for (cf, value) in db.estimated_num_keys()? {
            ROCKSDB_ESTIMATED_KEYS_GAUGE
                .with_label_values(&[cf])
                .set(value as f64);
        }

        sleep(Duration::from_secs(5 * 60)).await;
    }
}
