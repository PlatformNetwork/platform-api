//! Query executor types

use sqlx::PgPool;

/// Query executor for safe SQL execution
pub struct QueryExecutor {
    pub(super) db_pool: PgPool,
    pub(super) query_timeout: u64,
}

impl QueryExecutor {
    pub fn new(db_pool: PgPool, query_timeout: u64) -> Self {
        Self {
            db_pool,
            query_timeout,
        }
    }
}
