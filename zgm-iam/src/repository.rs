//! All SQL of the module: plain functions, no trait and no state.

use jiff::Timestamp;
use sqlx::postgres::PgRow;
use sqlx::postgres::types::PgInterval;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::IamError;
use crate::order::Order;
use crate::state::State;

/// What the database set itself in a new row of a tenant.
pub(crate) struct NewTenant {
    pub(crate) id: Uuid,
    pub(crate) state: State,
    pub(crate) deletes_at: Option<Timestamp>,
    pub(crate) created_at: Timestamp,
    pub(crate) updated_at: Timestamp,
}

/// Writes a tenant with the ciphertext of its name. The database gives the id, the state and
/// the dates.
pub(crate) async fn insert_tenant(
    connection: &mut PgConnection,
    name: &[u8],
) -> Result<NewTenant, IamError> {
    let (id, state, deletes_at, created_at, updated_at): (
        Uuid,
        String,
        Option<jiff_sqlx::Timestamp>,
        jiff_sqlx::Timestamp,
        jiff_sqlx::Timestamp,
    ) = sqlx::query_as(
        "INSERT INTO zgm_iam.tenants (name) VALUES ($1) \
         RETURNING id, state, deletes_at, created_at, updated_at",
    )
    .bind(name)
    .fetch_one(connection)
    .await?;
    Ok(NewTenant {
        id,
        state: state_of(state)?,
        deletes_at: deletes_at.map(|date| date.to_jiff()),
        created_at: created_at.to_jiff(),
        updated_at: updated_at.to_jiff(),
    })
}

/// The state of a tenant, or `None` for a tenant that does not exist. `FOR SHARE` keeps the
/// row as it is until the end of the transaction: a change of the state at the same moment
/// waits for it, and one already under way is waited for and then read.
pub(crate) async fn lock_state_of_tenant(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<Option<State>, IamError> {
    let word: Option<String> =
        sqlx::query_scalar("SELECT state FROM zgm_iam.tenants WHERE id = $1 FOR SHARE")
            .bind(tenant_id)
            .fetch_optional(connection)
            .await?;
    word.map(state_of).transpose()
}

/// A row of a tenant, with its name as the bytes the cipher made of it.
pub(crate) struct TenantRow {
    pub(crate) id: Uuid,
    pub(crate) name: Vec<u8>,
    pub(crate) state: State,
    pub(crate) deletes_at: Option<Timestamp>,
    pub(crate) created_at: Timestamp,
    pub(crate) updated_at: Timestamp,
}

/// Puts an active or a suspended tenant in `pending_deletion`, to be deleted `window`
/// microseconds after the moment of the statement, and gives the row; a tenant that waits for
/// deletion already, or whose state the module does not know, is not changed and is read as it
/// is. `None` when there is no such tenant. The row stays locked until the end of the
/// transaction.
pub(crate) async fn schedule_deletion_of_tenant(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    window: i64,
) -> Result<Option<TenantRow>, IamError> {
    let window = PgInterval {
        months: 0,
        days: 0,
        microseconds: window,
    };
    let changed: Option<PgRow> = sqlx::query(
        "UPDATE zgm_iam.tenants \
         SET state = 'pending_deletion', deletes_at = statement_timestamp() + $2 \
         WHERE id = $1 AND state IN ('active', 'suspended') \
         RETURNING id, name, state, deletes_at, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(window)
    .fetch_optional(&mut *connection)
    .await?;
    match changed {
        Some(row) => tenant_of(&row).map(Some),
        None => select_tenant(connection, tenant_id).await,
    }
}

/// Puts a tenant in `state`, active or suspended, with no date of deletion, and gives the row;
/// a tenant in that state already, or whose state the module does not know, is not changed and
/// is read as it is. `None` when there is no such tenant. The row stays locked until the end of
/// the transaction.
pub(crate) async fn set_state_of_tenant(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    state: State,
) -> Result<Option<TenantRow>, IamError> {
    let changed: Option<PgRow> = sqlx::query(
        "UPDATE zgm_iam.tenants SET state = $2, deletes_at = NULL \
         WHERE id = $1 AND state IN ('active', 'suspended', 'pending_deletion') AND state <> $2 \
         RETURNING id, name, state, deletes_at, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(<&'static str>::from(state))
    .fetch_optional(&mut *connection)
    .await?;
    match changed {
        Some(row) => tenant_of(&row).map(Some),
        None => select_tenant(connection, tenant_id).await,
    }
}

/// The tenant with the given id as its row is, after an `UPDATE` that changed nothing; `None`
/// when there is no such tenant.
async fn select_tenant(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<Option<TenantRow>, IamError> {
    let found: Option<PgRow> = sqlx::query(
        "SELECT id, name, state, deletes_at, created_at, updated_at FROM zgm_iam.tenants \
         WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(connection)
    .await?;
    found.as_ref().map(tenant_of).transpose()
}

/// The tenant in a row of [`schedule_deletion_of_tenant`], [`set_state_of_tenant`] or
/// [`select_tenant`].
fn tenant_of(row: &PgRow) -> Result<TenantRow, IamError> {
    Ok(TenantRow {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        state: state_of(row.try_get("state")?)?,
        deletes_at: row
            .try_get::<Option<jiff_sqlx::Timestamp>, _>("deletes_at")?
            .map(|date| date.to_jiff()),
        created_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
            .to_jiff(),
        updated_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("updated_at")?
            .to_jiff(),
    })
}

/// A row of a user, without its tenant, which the caller knows.
pub(crate) struct UserRow {
    pub(crate) id: Uuid,
    pub(crate) state: State,
    pub(crate) deletes_at: Option<Timestamp>,
    pub(crate) created_at: Timestamp,
    pub(crate) updated_at: Timestamp,
}

/// Writes a user of the tenant. The database gives the id, the state and the dates; the
/// caller has checked the tenant with [`lock_state_of_tenant`] in the same transaction.
pub(crate) async fn insert_user(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<UserRow, IamError> {
    let (id, state, deletes_at, created_at, updated_at): (
        Uuid,
        String,
        Option<jiff_sqlx::Timestamp>,
        jiff_sqlx::Timestamp,
        jiff_sqlx::Timestamp,
    ) = sqlx::query_as(
        "INSERT INTO zgm_iam.users (tenant_id) VALUES ($1) \
         RETURNING id, state, deletes_at, created_at, updated_at",
    )
    .bind(tenant_id)
    .fetch_one(connection)
    .await?;
    Ok(UserRow {
        id,
        state: state_of(state)?,
        deletes_at: deletes_at.map(|date| date.to_jiff()),
        created_at: created_at.to_jiff(),
        updated_at: updated_at.to_jiff(),
    })
}

/// Puts a user of the tenant who is active or suspended in `pending_deletion`, to be deleted
/// `window` microseconds after the moment of the statement, and gives the row; a user who
/// waits for deletion already, or whose state the module does not know, is not changed and is
/// read as it is. `None` when the tenant has no such user. The caller has checked the tenant
/// with [`lock_state_of_tenant`] in the same transaction.
pub(crate) async fn schedule_deletion_of_user(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    window: i64,
) -> Result<Option<UserRow>, IamError> {
    // An interval of microseconds is exact; a number times `interval '1 second'` would go
    // through a double and could lose microseconds of a long window.
    let window = PgInterval {
        months: 0,
        days: 0,
        microseconds: window,
    };
    let changed: Option<PgRow> = sqlx::query(
        "UPDATE zgm_iam.users \
         SET state = 'pending_deletion', deletes_at = statement_timestamp() + $3 \
         WHERE tenant_id = $1 AND id = $2 AND state IN ('active', 'suspended') \
         RETURNING id, state, deletes_at, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(window)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = changed {
        return user_of(&row);
    }
    select_user_of_tenant(connection, tenant_id, user_id).await
}

/// Puts a user of the tenant in `state`, active or suspended, with no date of deletion, and
/// gives the row; a user who is in that state already, or whose state the module does not
/// know, is not changed and is read as it is. `None` when the tenant has no such user. The
/// caller has checked the tenant with [`lock_state_of_tenant`] in the same transaction.
pub(crate) async fn set_state_of_user(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
    state: State,
) -> Result<Option<UserRow>, IamError> {
    let changed: Option<PgRow> = sqlx::query(
        "UPDATE zgm_iam.users SET state = $3, deletes_at = NULL \
         WHERE tenant_id = $1 AND id = $2 \
           AND state IN ('active', 'suspended', 'pending_deletion') AND state <> $3 \
         RETURNING id, state, deletes_at, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(<&'static str>::from(state))
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(row) = changed {
        return user_of(&row);
    }
    select_user_of_tenant(connection, tenant_id, user_id).await
}

/// Deletes at most `limit` users of any tenant who wait for deletion and whose date has come,
/// by the clock of the database, and says how many. The oldest dates first: the `ORDER BY`
/// keeps the database on the index of the dates under every plan. Rows another copy of the
/// application is deleting are skipped, not waited for, and a user brought back by
/// `set_state` before the statement locked it is no longer in `pending_deletion`.
pub(crate) async fn delete_users_past_their_date(
    connection: &mut PgConnection,
    limit: i64,
) -> Result<u64, IamError> {
    let deleted = sqlx::query(
        "DELETE FROM zgm_iam.users \
         WHERE ctid = ANY(ARRAY( \
             SELECT ctid FROM zgm_iam.users \
             WHERE state = 'pending_deletion' AND deletes_at <= statement_timestamp() \
             ORDER BY deletes_at \
             LIMIT $1 \
             FOR UPDATE SKIP LOCKED))",
    )
    .bind(limit)
    .execute(connection)
    .await?;
    Ok(deleted.rows_affected())
}

/// Deletes at most `limit` tenants that wait for deletion and whose date has come, by the
/// clock of the database, and says how many; the database deletes their users with them
/// (`ON DELETE CASCADE`). The oldest dates first, along the index of the dates, and tenants
/// another copy of the application is deleting are skipped.
pub(crate) async fn delete_tenants_past_their_date(
    connection: &mut PgConnection,
    limit: i64,
) -> Result<u64, IamError> {
    let deleted = sqlx::query(
        "DELETE FROM zgm_iam.tenants \
         WHERE ctid = ANY(ARRAY( \
             SELECT ctid FROM zgm_iam.tenants \
             WHERE state = 'pending_deletion' AND deletes_at <= statement_timestamp() \
             ORDER BY deletes_at \
             LIMIT $1 \
             FOR UPDATE SKIP LOCKED))",
    )
    .bind(limit)
    .execute(connection)
    .await?;
    Ok(deleted.rows_affected())
}

/// The user of the tenant with the given id as its row is, after an `UPDATE` that changed
/// nothing; `None` when the tenant has no such user.
async fn select_user_of_tenant(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Option<UserRow>, IamError> {
    let found: Option<PgRow> = sqlx::query(
        "SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(connection)
    .await?;
    match found {
        None => Ok(None),
        Some(row) => user_of(&row),
    }
}

/// In one query, the state of a tenant and its user with the given id: the state is `None`
/// for a tenant that does not exist, and the user `None` when the tenant has no such user,
/// including a user of another tenant. Nothing is locked: the query writes nothing, and it
/// sees the tenant and the user as they were at one moment.
pub(crate) async fn select_user(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<(Option<State>, Option<UserRow>), IamError> {
    let found: Option<PgRow> = sqlx::query(
        "SELECT t.state AS tenant_state, \
                u.id, u.state, u.deletes_at, u.created_at, u.updated_at \
         FROM zgm_iam.tenants t \
         LEFT JOIN zgm_iam.users u ON u.tenant_id = t.id AND u.id = $2 \
         WHERE t.id = $1",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(connection)
    .await?;
    let Some(row) = found else {
        return Ok((None, None));
    };
    let state_of_tenant = state_of(row.try_get("tenant_state")?)?;
    Ok((Some(state_of_tenant), user_of(&row)?))
}

// The four queries of a page of users: both orders, without a state and with one. `$1` is the
// tenant, `$2` the id the page starts after, then the state, if any, and the most rows to give.
//
// The users are joined to the row of the tenant, so one query gives the state of the tenant
// too: no row is no tenant, and a row with no `u.id` a tenant without such users. The id to
// start after is always given, the largest or the smallest UUID on the first page, and is
// compared together with the tenant, and the state, as one row: in this shape the database
// starts at the place in the index of the list and reads only the page. Measured on 500 000
// users, written as `id < $2` or with no id on the first page it picked the primary key
// instead and read the whole table.
const NEWEST_FIRST: &str = "SELECT t.state AS tenant_state, \
            u.id, u.state, u.deletes_at, u.created_at, u.updated_at \
     FROM zgm_iam.tenants t \
     LEFT JOIN LATERAL ( \
         SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users \
         WHERE tenant_id = $1 AND (tenant_id, id) < ($1, $2) \
         ORDER BY id DESC LIMIT $3) u ON true \
     WHERE t.id = $1 \
     ORDER BY u.id DESC";
const OLDEST_FIRST: &str = "SELECT t.state AS tenant_state, \
            u.id, u.state, u.deletes_at, u.created_at, u.updated_at \
     FROM zgm_iam.tenants t \
     LEFT JOIN LATERAL ( \
         SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users \
         WHERE tenant_id = $1 AND (tenant_id, id) > ($1, $2) \
         ORDER BY id LIMIT $3) u ON true \
     WHERE t.id = $1 \
     ORDER BY u.id";
const NEWEST_FIRST_IN_STATE: &str = "SELECT t.state AS tenant_state, \
            u.id, u.state, u.deletes_at, u.created_at, u.updated_at \
     FROM zgm_iam.tenants t \
     LEFT JOIN LATERAL ( \
         SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users \
         WHERE tenant_id = $1 AND state = $3 AND (tenant_id, state, id) < ($1, $3, $2) \
         ORDER BY id DESC LIMIT $4) u ON true \
     WHERE t.id = $1 \
     ORDER BY u.id DESC";
const OLDEST_FIRST_IN_STATE: &str = "SELECT t.state AS tenant_state, \
            u.id, u.state, u.deletes_at, u.created_at, u.updated_at \
     FROM zgm_iam.tenants t \
     LEFT JOIN LATERAL ( \
         SELECT id, state, deletes_at, created_at, updated_at FROM zgm_iam.users \
         WHERE tenant_id = $1 AND state = $3 AND (tenant_id, state, id) > ($1, $3, $2) \
         ORDER BY id LIMIT $4) u ON true \
     WHERE t.id = $1 \
     ORDER BY u.id";

/// In one query, the state of a tenant and at most `limit` of its users in the given order,
/// only those in `state` when it is given, after the user `after` when it is given: the state
/// is `None` for a tenant that does not exist. Nothing is locked, as in [`select_user`].
pub(crate) async fn select_users(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    order: Order,
    state: Option<State>,
    after: Option<Uuid>,
    limit: i64,
) -> Result<(Option<State>, Vec<UserRow>), IamError> {
    // A UUID v7 is never the largest or the smallest UUID, so no user is left out.
    let after = after.unwrap_or(match order {
        Order::NewestFirst => Uuid::max(),
        Order::OldestFirst => Uuid::nil(),
    });
    let query = match (order, state) {
        (Order::NewestFirst, None) => sqlx::query(NEWEST_FIRST),
        (Order::OldestFirst, None) => sqlx::query(OLDEST_FIRST),
        (Order::NewestFirst, Some(_)) => sqlx::query(NEWEST_FIRST_IN_STATE),
        (Order::OldestFirst, Some(_)) => sqlx::query(OLDEST_FIRST_IN_STATE),
    }
    .bind(tenant_id)
    .bind(after);
    let query = match state {
        Some(state) => query.bind(<&'static str>::from(state)),
        None => query,
    };
    let rows: Vec<PgRow> = query.bind(limit).fetch_all(connection).await?;
    let Some(first) = rows.first() else {
        return Ok((None, Vec::new()));
    };
    let state_of_tenant = state_of(first.try_get("tenant_state")?)?;
    let mut users = Vec::with_capacity(rows.len());
    for row in &rows {
        users.extend(user_of(row)?);
    }
    Ok((Some(state_of_tenant), users))
}

/// The user in a row of [`select_user`], [`select_users`] or [`schedule_deletion_of_user`], or
/// `None` when no user was joined and the columns of the user are all NULL.
fn user_of(row: &PgRow) -> Result<Option<UserRow>, IamError> {
    let Some(id) = row.try_get::<Option<Uuid>, _>("id")? else {
        return Ok(None);
    };
    Ok(Some(UserRow {
        id,
        state: state_of(row.try_get("state")?)?,
        deletes_at: row
            .try_get::<Option<jiff_sqlx::Timestamp>, _>("deletes_at")?
            .map(|date| date.to_jiff()),
        created_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
            .to_jiff(),
        updated_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("updated_at")?
            .to_jiff(),
    }))
}

/// The state whose word the database gave, or an error for a word that is no state.
fn state_of(word: String) -> Result<State, IamError> {
    word.parse().map_err(|_| IamError::UnknownState(word))
}
