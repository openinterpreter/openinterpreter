//! Bounded retries for SQLite operations that can safely restart on external contention.
//! Callers must complete one atomic transaction (or only open a connection) per attempt.
//! Never retry cancellation: a commit whose result was not received may have succeeded.

use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use sqlx::Execute;
use sqlx::Executor;
use sqlx::FromRow;
use sqlx::Sqlite;
use sqlx::SqlitePool;
use sqlx::Transaction;
use sqlx::error::DatabaseError;
use sqlx::query::Query;
use sqlx::query::QueryAs;
use sqlx::sqlite::SqliteArguments;
use sqlx::sqlite::SqliteError;
use sqlx::sqlite::SqliteQueryResult;
use sqlx::sqlite::SqliteRow;

/// SQLite writer admission for pools shared by the state and thread stores.
/// Implementations must acquire the writer slot before any read snapshot and
/// retry only admission, never an already-started transaction or external effects.
pub trait SqlitePoolWriteExt {
    fn begin_immediate(
        &self,
    ) -> impl Future<Output = Result<Transaction<'static, Sqlite>, sqlx::Error>> + Send;
}

impl SqlitePoolWriteExt for SqlitePool {
    async fn begin_immediate(&self) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
        retry_when(
            || self.begin_with("BEGIN IMMEDIATE"),
            |error| is_busy(error),
        )
        .await
    }
}

/// Safe retry boundaries for single prepared SQLite writes, including RETURNING.
/// Implementations decode their row before committing, just as SQLx does in an
/// explicit transaction. Never use this for multiple statements or existing
/// transactions; those must keep using their transaction's connection.
pub(crate) trait SqliteWriteQueryExt<'q>: Execute<'q, Sqlite> + 'q {
    type Output: Send;

    fn decode_returning_row(row: SqliteRow) -> Result<Self::Output, sqlx::Error>;

    /// Retry a standalone autocommit write with exactly the same bound inputs.
    fn execute_with_busy_retry<'e>(
        mut self,
        pool: &'e SqlitePool,
    ) -> impl Future<Output = Result<SqliteQueryResult, sqlx::Error>> + Send + 'e
    where
        'q: 'e,
    {
        async move {
            let persistent = Execute::persistent(&self);
            let arguments = self
                .take_arguments()
                .map_err(sqlx::Error::Encode)?
                .unwrap_or_default();
            let sql = self.sql();
            retry_when(
                || {
                    sqlx::query_with(sql.clone(), arguments.clone())
                        .persistent(persistent)
                        .execute(pool)
                },
                |error| is_busy(error),
            )
            .await
        }
    }

    /// Reserve the writer slot for a RETURNING write; expose its row only after commit.
    fn fetch_one_write<'e>(
        self,
        pool: &'e SqlitePool,
    ) -> impl Future<Output = Result<Self::Output, sqlx::Error>> + Send + 'e
    where
        'q: 'e,
    {
        async move {
            let mut tx = pool.begin_immediate().await?;
            let row = (&mut *tx).fetch_one(self).await?;
            let output = Self::decode_returning_row(row)?;
            tx.commit().await?;
            Ok(output)
        }
    }

    /// As with fetch_one_write, but commit a write that returns no row too.
    fn fetch_optional_write<'e>(
        self,
        pool: &'e SqlitePool,
    ) -> impl Future<Output = Result<Option<Self::Output>, sqlx::Error>> + Send + 'e
    where
        'q: 'e,
    {
        async move {
            let mut tx = pool.begin_immediate().await?;
            let row = (&mut *tx).fetch_optional(self).await?;
            let output = row.map(Self::decode_returning_row).transpose()?;
            tx.commit().await?;
            Ok(output)
        }
    }
}

impl<'q> SqliteWriteQueryExt<'q> for Query<'q, Sqlite, SqliteArguments> {
    type Output = SqliteRow;

    fn decode_returning_row(row: SqliteRow) -> Result<Self::Output, sqlx::Error> {
        Ok(row)
    }
}

impl<'q, O> SqliteWriteQueryExt<'q> for QueryAs<'q, Sqlite, O, SqliteArguments>
where
    O: Send + Unpin + for<'r> FromRow<'r, SqliteRow> + 'q,
{
    type Output = O;

    fn decode_returning_row(row: SqliteRow) -> Result<Self::Output, sqlx::Error> {
        O::from_row(&row)
    }
}

/// Retries only received SQLite BUSY errors. Each operation must drop or roll back
/// its transaction before returning an error so even BUSY_SNAPSHOT starts fresh.
/// SQLite already waits up to five seconds per lock; allow two more attempts.
pub(crate) async fn retry_on_busy<T, F, Fut>(operation: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    retry_when(operation, |error| error.chain().any(is_busy)).await
}

async fn retry_when<T, E, F, Fut>(
    mut operation: F,
    is_retryable: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delays = [Duration::from_millis(100), Duration::from_millis(200)].into_iter();
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                if is_retryable(&error)
                    && let Some(delay) = delays.next()
                {
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
}

fn is_busy(source: &(dyn StdError + 'static)) -> bool {
    let Some(sqlite_error) = source
        .downcast_ref::<sqlx::Error>()
        .and_then(sqlx::Error::as_database_error)
        .and_then(|database_error| database_error.try_downcast_ref::<SqliteError>())
    else {
        return false;
    };
    matches!(
        sqlite_error
            .code()
            .and_then(|code| code.parse::<i32>().ok()),
        Some(
            libsqlite3_sys::SQLITE_BUSY
                | libsqlite3_sys::SQLITE_BUSY_RECOVERY
                | libsqlite3_sys::SQLITE_BUSY_SNAPSHOT
                | libsqlite3_sys::SQLITE_BUSY_TIMEOUT
        )
    )
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
