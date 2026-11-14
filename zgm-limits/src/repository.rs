//! All SQL of the module: plain functions, no trait and no state.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::answer::Answer;
use crate::blocked::{Blocked, BlockedCursor, BlockedPage};
use crate::rule::{Bounds, Count, Period, Rule};

/// One attempt in a window, as one statement. All the counting of a window is here.
///
/// Parameters: `$1` the limit name, `$2` the tenant, `$3` the account, `$4` the hashes
/// (the current version first), `$5` how many attempts a window lets through by default, `$6`
/// the moment of the attempt, `$7` how many seconds a window lasts by default. The tenant's
/// own rule, when it has one, takes the place of the two defaults, held between `$8` and `$9`
/// attempts and between `$10` and `$11` seconds.
///
/// Returns one row of two moments. `refused_until`: NULL when the attempt passed, the end of
/// the window when it was refused. `blocked_until`: the end of the window when the attempt
/// passed and filled it, NULL otherwise.
const HIT_WINDOW: &str = r"
-- Part 0: the rule in force. The tenant's own, when it set one for this limit; the default
-- from the code otherwise. Read here, in the statement that counts, so a change of the rule
-- works from the next attempt on every server and costs no round trip.
WITH tenant_rule AS (
    SELECT rule
    FROM zgm_limits.tenant_rules
    WHERE tenant_id = $2 AND limit_name = $1 AND rule->>'kind' = 'window'
),
in_force AS (
    -- The tenant's numbers are held to the bounds of the limit, $8 to $11: a row written
    -- before the bounds in the code got narrower must not count wider than they are now.
    SELECT LEAST(GREATEST(
               COALESCE((SELECT (rule->>'at_most')::bigint FROM tenant_rule), $5), $8), $9)
               AS at_most,
           $6 + make_interval(secs => LEAST(GREATEST(
               COALESCE((SELECT (rule->>'per_seconds')::bigint FROM tenant_rule), $7), $10), $11))
               AS new_end
),

-- Part 1: read the open window of this key, if there is one. No lock.
-- While the secret key changes, the window may be under the hash of an older version.
cur AS (
    SELECT key_hash, state, expires_at,
           -- How many attempts this window lets through: the number it was opened with. A
           -- window counts the way it started, whatever the rule has become since.
           COALESCE((state->>'at_most')::bigint, (SELECT at_most FROM in_force)) AS room
    FROM zgm_limits.counters
    WHERE limit_name = $1 AND tenant_id = $2 AND subject_id = $3
      AND key_hash = ANY($4)
      AND expires_at > $6
    ORDER BY array_position($4, key_hash)
    LIMIT 1
),

-- Part 2: count the attempt, unless the open window is full.
hit AS (
    INSERT INTO zgm_limits.counters
        (limit_name, tenant_id, subject_id, key_hash, state, expires_at, blocked_at)

    -- A new window with one attempt in it, and with the rule in force now written next to
    -- the count: this window is counted by it to its end. It goes under the hash of the open
    -- window, so that the conflict below finds that row; with no open window, under the
    -- current one.
    SELECT $1, $2, $3,
           COALESCE((SELECT key_hash FROM cur), $4[1]),
           jsonb_build_object('count', 1, 'at_most', (SELECT at_most FROM in_force)),
           (SELECT new_end FROM in_force),
           -- A window that lets one attempt through is full at once.
           CASE WHEN (SELECT at_most FROM in_force) <= 1 THEN $6 END

    -- Open and full: write nothing at all, so a refusal locks nothing.
    WHERE NOT EXISTS (SELECT 1 FROM cur WHERE (state->>'count')::bigint >= room)

    -- The row is already there.
    ON CONFLICT (limit_name, tenant_id, subject_id, key_hash) DO UPDATE SET
        -- Window still open: one attempt more. Window over: start again from one.
        state = CASE WHEN counters.expires_at > $6
                     THEN jsonb_set(counters.state, '{count}',
                                    to_jsonb((counters.state->>'count')::bigint + 1))
                     ELSE excluded.state END,
        -- Window still open: its end stays. Window over: a new end.
        expires_at = CASE WHEN counters.expires_at > $6
                          THEN counters.expires_at
                          ELSE excluded.expires_at END,
        -- The attempt that fills the window blocks the key. A new window starts not blocked.
        blocked_at = CASE WHEN counters.expires_at <= $6
                          THEN excluded.blocked_at
                          WHEN (counters.state->>'count')::bigint + 1
                               >= COALESCE((counters.state->>'at_most')::bigint,
                                           (SELECT at_most FROM in_force))
                          THEN $6
                          END
    -- Checked again at the moment of writing, so that attempts arriving together cannot pass
    -- more than there is room for.
    WHERE counters.expires_at <= $6
       OR (counters.state->>'count')::bigint
          < COALESCE((counters.state->>'at_most')::bigint, (SELECT at_most FROM in_force))
    RETURNING expires_at, blocked_at
)

-- Part 3: the answer. Part 2 wrote something: the attempt passed. After an attempt that
-- passed, `blocked_at` is set only when that attempt filled the window, so it also says
-- whether this attempt blocked the key, and until when.
SELECT NULL::timestamptz AS refused_until,
       CASE WHEN blocked_at IS NOT NULL THEN expires_at END AS blocked_until
FROM hit
UNION ALL
-- Part 2 wrote nothing because part 1 read a full window: refused until its end.
-- When part 1 read a window with room (or none) and part 2 still wrote nothing, somebody
-- changed the row in between; what part 1 read is stale, so there is no answer and the
-- caller asks again.
SELECT expires_at, NULL::timestamptz FROM cur
WHERE NOT EXISTS (SELECT 1 FROM hit) AND (state->>'count')::bigint >= room
";

/// What one attempt in a window came to.
pub struct Hit {
    /// Whether the attempt passed.
    pub answer: Answer,
    /// When the attempt passed and filled its window: the end of the window, until which the
    /// key is blocked from now on.
    pub blocked_until: Option<DateTime<Utc>>,
}

/// Counts one attempt in a window. By default it lets `at_most` attempts through and lasts
/// `per` from its first attempt; the tenant's own rule for the limit, when there is one,
/// takes their place, held to `bounds`. One query; a refused attempt writes nothing.
///
/// `None` when the query gave no answer: what its first part read was no longer true when its
/// second part came to write. Another attempt opened or filled the window in between, or a
/// reset deleted it and attempts filled a new one. Asked again, the query reads the row as it
/// is now; how many times to ask is the caller's decision.
///
/// Whose counter it is: `tenant_id`, `subject_id` and `hashes`.
/// - `subject_id` is the account the counter is about; [`Uuid::nil`] when there is none.
/// - `hashes` are the hashes of the hidden part (an e-mail, an IP address): one for
///   every version of the secret key that can still be read, the current version first. One
///   empty hash when there is no hidden part.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is one parameter of the query"
)]
pub async fn hit_window(
    connection: &mut PgConnection,
    limit_name: &str,
    tenant_id: Uuid,
    subject_id: Uuid,
    hashes: &[Vec<u8>],
    at_most: Count,
    per: Period,
    bounds: Option<Bounds>,
    now: DateTime<Utc>,
) -> Result<Option<Hit>, sqlx::Error> {
    // A limit with no bounds cannot be changed by a tenant: holding the numbers between the
    // default and the default makes the query count by the default whatever a row says.
    let bounds = bounds.unwrap_or(Bounds {
        min_at_most: at_most,
        max_at_most: at_most,
        min_per: per,
        max_per: per,
    });
    // The two moments of the answer: `refused_until` and `blocked_until`.
    type Row = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);
    let row: Option<Row> = sqlx::query_as(HIT_WINDOW)
        .bind(limit_name)
        .bind(tenant_id)
        .bind(subject_id)
        .bind(hashes)
        .bind(i64::from(at_most.get()))
        .bind(now)
        .bind(i64::from(per.as_seconds()))
        .bind(i64::from(bounds.min_at_most.get()))
        .bind(i64::from(bounds.max_at_most.get()))
        .bind(i64::from(bounds.min_per.as_seconds()))
        .bind(i64::from(bounds.max_per.as_seconds()))
        .fetch_optional(connection)
        .await?;

    Ok(row.map(|(refused_until, blocked_until)| Hit {
        answer: match refused_until {
            None => Answer::Allowed,
            Some(until) => Answer::Refused { until },
        },
        blocked_until,
    }))
}

/// One attempt on one element in a window of distinct elements, as one statement, laid out
/// like [`HIT_WINDOW`].
///
/// Parameters `$1` to `$11` as in `HIT_WINDOW`, with "elements" for "attempts"; `$12` the
/// hashes of the element as text, the current version of the secret key first.
///
/// Returns one row of two moments, as `HIT_WINDOW` does.
const HIT_DISTINCT: &str = r"
-- Part 0: the rule in force, as in the window of attempts.
WITH tenant_rule AS (
    SELECT rule
    FROM zgm_limits.tenant_rules
    WHERE tenant_id = $2 AND limit_name = $1 AND rule->>'kind' = 'window_of_distinct'
),
in_force AS (
    SELECT LEAST(GREATEST(
               COALESCE((SELECT (rule->>'at_most')::bigint FROM tenant_rule), $5), $8), $9)
               AS at_most,
           $6 + make_interval(secs => LEAST(GREATEST(
               COALESCE((SELECT (rule->>'per_seconds')::bigint FROM tenant_rule), $7), $10), $11))
               AS new_end
),

-- Part 1: read the open window of this key, if there is one. No lock.
cur AS (
    SELECT key_hash, state, expires_at,
           -- The element is in the window already, under the hash of any version of the key.
           state->'elements' ?| $12 AS has_element,
           jsonb_array_length(state->'elements') AS how_many,
           -- How many elements this window takes: the number it was opened with.
           COALESCE((state->>'at_most')::bigint, (SELECT at_most FROM in_force)) AS room
    FROM zgm_limits.counters
    WHERE limit_name = $1 AND tenant_id = $2 AND subject_id = $3
      AND key_hash = ANY($4)
      AND expires_at > $6
    ORDER BY array_position($4, key_hash)
    LIMIT 1
),

-- Part 2: put the element into the window, unless it is there already or the window is full.
hit AS (
    INSERT INTO zgm_limits.counters
        (limit_name, tenant_id, subject_id, key_hash, state, expires_at, blocked_at)

    -- A new window with this one element in it, and with the rule in force now written
    -- next to it: this window is counted by it to its end.
    SELECT $1, $2, $3,
           COALESCE((SELECT key_hash FROM cur), $4[1]),
           jsonb_build_object('elements', jsonb_build_array($12[1]),
                              'at_most', (SELECT at_most FROM in_force)),
           (SELECT new_end FROM in_force),
           -- A window for one element is full at once.
           CASE WHEN (SELECT at_most FROM in_force) <= 1 THEN $6 END

    -- The element is there already, or the window is full: write nothing, lock nothing.
    WHERE NOT EXISTS (
        SELECT 1 FROM cur
        WHERE has_element OR how_many >= room
    )

    -- The row is already there.
    ON CONFLICT (limit_name, tenant_id, subject_id, key_hash) DO UPDATE SET
        -- Window still open: one element more. Window over: start again with this one.
        state = CASE WHEN counters.expires_at > $6
                     THEN jsonb_set(counters.state, '{elements}',
                                    counters.state->'elements' || to_jsonb($12[1]))
                     ELSE excluded.state END,
        expires_at = CASE WHEN counters.expires_at > $6
                          THEN counters.expires_at
                          ELSE excluded.expires_at END,
        -- The element that fills the window blocks the key for new elements.
        blocked_at = CASE WHEN counters.expires_at <= $6
                          THEN excluded.blocked_at
                          WHEN jsonb_array_length(counters.state->'elements') + 1
                               >= COALESCE((counters.state->>'at_most')::bigint,
                                           (SELECT at_most FROM in_force))
                          THEN $6
                          END
    -- Checked again at the moment of writing: elements arriving together cannot put in more
    -- than there is room for, nor the same element twice.
    WHERE counters.expires_at <= $6
       OR (NOT counters.state->'elements' ?| $12
           AND jsonb_array_length(counters.state->'elements')
               < COALESCE((counters.state->>'at_most')::bigint, (SELECT at_most FROM in_force)))
    RETURNING expires_at, blocked_at
)

-- Part 3: the answer. Part 2 wrote the element: the attempt passed.
SELECT NULL::timestamptz AS refused_until,
       CASE WHEN blocked_at IS NOT NULL THEN expires_at END AS blocked_until
FROM hit
UNION ALL
-- Part 2 wrote nothing because part 1 read the element in the window: passed, nothing new.
SELECT NULL::timestamptz, NULL::timestamptz FROM cur
WHERE NOT EXISTS (SELECT 1 FROM hit) AND has_element
UNION ALL
-- Part 2 wrote nothing because part 1 read a full window without the element: refused.
-- Anything else part 2 did not write was changed in between, and the caller asks again.
SELECT expires_at, NULL::timestamptz FROM cur
WHERE NOT EXISTS (SELECT 1 FROM hit) AND NOT has_element
  AND how_many >= room
";

/// Counts one attempt on one element in a window of distinct elements; see [`hit_window`] for
/// the arguments it shares. `element_hashes` are the hashes of the element as text, the
/// current version of the secret key first: the window is searched by all of them and the
/// first is written.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is one parameter of the query"
)]
pub async fn hit_distinct(
    connection: &mut PgConnection,
    limit_name: &str,
    tenant_id: Uuid,
    subject_id: Uuid,
    hashes: &[Vec<u8>],
    at_most: Count,
    per: Period,
    bounds: Option<Bounds>,
    element_hashes: &[String],
    now: DateTime<Utc>,
) -> Result<Option<Hit>, sqlx::Error> {
    let bounds = bounds.unwrap_or(Bounds {
        min_at_most: at_most,
        max_at_most: at_most,
        min_per: per,
        max_per: per,
    });
    type Row = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);
    let row: Option<Row> = sqlx::query_as(HIT_DISTINCT)
        .bind(limit_name)
        .bind(tenant_id)
        .bind(subject_id)
        .bind(hashes)
        .bind(i64::from(at_most.get()))
        .bind(now)
        .bind(i64::from(per.as_seconds()))
        .bind(i64::from(bounds.min_at_most.get()))
        .bind(i64::from(bounds.max_at_most.get()))
        .bind(i64::from(bounds.min_per.as_seconds()))
        .bind(i64::from(bounds.max_per.as_seconds()))
        .bind(element_hashes)
        .fetch_optional(connection)
        .await?;

    Ok(row.map(|(refused_until, blocked_until)| Hit {
        answer: match refused_until {
            None => Answer::Allowed,
            Some(until) => Answer::Refused { until },
        },
        blocked_until,
    }))
}

/// Deletes the counter of a key, under the hash of every version of the secret key, so that
/// the key starts from nothing. Says whether a block that was still on went with it.
pub async fn delete_counter(
    connection: &mut PgConnection,
    limit_name: &str,
    tenant_id: Uuid,
    subject_id: Uuid,
    hashes: &[Vec<u8>],
    now: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let lifted_blocks: Vec<bool> = sqlx::query_scalar(
        "DELETE FROM zgm_limits.counters \
         WHERE limit_name = $1 AND tenant_id = $2 AND subject_id = $3 AND key_hash = ANY($4) \
         RETURNING blocked_at IS NOT NULL AND expires_at > $5",
    )
    .bind(limit_name)
    .bind(tenant_id)
    .bind(subject_id)
    .bind(hashes)
    .bind(now)
    .fetch_all(connection)
    .await?;

    Ok(lifted_blocks.contains(&true))
}

/// How many blocked keys one page of [`list_blocked`] holds at most.
pub const BLOCKED_PAGE: usize = 100;

/// One page of the blocked keys of one tenant, the newest block first, after the row `after` points at, or
/// from the start. `limit_name` and `whole_tenant` narrow the list when given. With
/// `include_expired`, also the keys whose block is over but whose row is still there.
pub async fn list_blocked(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    limit_name: Option<&str>,
    whole_tenant: Option<bool>,
    include_expired: bool,
    after: Option<BlockedCursor>,
    now: DateTime<Utc>,
) -> Result<BlockedPage, sqlx::Error> {
    type Row = (
        i64,
        String,
        Uuid,
        Uuid,
        Vec<u8>,
        DateTime<Utc>,
        DateTime<Utc>,
    );
    // One row more than a page holds: it only says whether there is a next page.
    // No cursor means "from infinity", and not "cursor IS NULL OR ...": with an OR the
    // database cannot use the cursor to jump into the index, and reads every row before the page.
    let mut rows: Vec<Row> = sqlx::query_as(
        "SELECT id, limit_name, tenant_id, subject_id, key_hash, blocked_at, expires_at \
         FROM zgm_limits.counters \
         WHERE blocked_at IS NOT NULL \
           AND ($1 OR expires_at > $2) \
           AND tenant_id = $3 \
           AND ($4::text IS NULL OR limit_name = $4) \
           AND (blocked_at, id) < (COALESCE($5::timestamptz, 'infinity'), \
                                   COALESCE($6::bigint, 9223372036854775807)) \
           AND ($8::boolean IS NULL \
                OR (subject_id = '00000000-0000-0000-0000-000000000000' \
                    AND key_hash = '') = $8) \
         ORDER BY blocked_at DESC, id DESC \
         LIMIT $7",
    )
    .bind(include_expired)
    .bind(now)
    .bind(tenant_id)
    .bind(limit_name)
    .bind(after.map(|cursor| cursor.blocked_at))
    .bind(after.map(|cursor| cursor.id))
    .bind(BLOCKED_PAGE as i64 + 1)
    .bind(whole_tenant)
    .fetch_all(connection)
    .await?;

    let has_next_page = rows.len() > BLOCKED_PAGE;
    rows.truncate(BLOCKED_PAGE);

    let items: Vec<Blocked> = rows
        .into_iter()
        .map(
            |(id, limit_name, tenant_id, subject_id, key_hash, blocked_at, blocked_until)| {
                Blocked {
                    id,
                    limit_name,
                    tenant_id,
                    whole_tenant: subject_id.is_nil() && key_hash.is_empty(),
                    subject_id,
                    key_hash,
                    blocked_at,
                    blocked_until,
                    expired: blocked_until <= now,
                }
            },
        )
        .collect();
    let next_cursor = match items.last() {
        Some(last) if has_next_page => Some(BlockedCursor {
            blocked_at: last.blocked_at,
            id: last.id,
        }),
        _ => None,
    };

    Ok(BlockedPage { items, next_cursor })
}

/// A blocked key that [`delete_blocked`] deleted.
pub struct DeletedBlocked {
    /// The name of its limit.
    pub limit_name: String,
    /// The account the key was about; nil when it had none.
    pub subject_id: Uuid,
    /// Whether the key had a hidden part.
    pub has_hidden_part: bool,
    /// Whether its block was still on.
    pub was_still_blocked: bool,
}

/// Deletes the counter of the blocked key `id` of the tenant `tenant_id`, so that the key
/// starts from nothing, and says what it deleted. `None` when that tenant had no such blocked
/// key: an id of another tenant deletes nothing.
pub async fn delete_blocked(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    id: i64,
    now: DateTime<Utc>,
) -> Result<Option<DeletedBlocked>, sqlx::Error> {
    let deleted: Option<(String, Uuid, bool, bool)> = sqlx::query_as(
        "DELETE FROM zgm_limits.counters \
         WHERE id = $1 AND tenant_id = $2 AND blocked_at IS NOT NULL \
         RETURNING limit_name, subject_id, key_hash <> '', expires_at > $3",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(now)
    .fetch_optional(connection)
    .await?;
    Ok(deleted.map(
        |(limit_name, subject_id, has_hidden_part, was_still_blocked)| DeletedBlocked {
            limit_name,
            subject_id,
            has_hidden_part,
            was_still_blocked,
        },
    ))
}

/// Deletes at most `limit` counters whose `expires_at` is not later than `dead_since`, the
/// oldest first, and says how many. `ORDER BY expires_at` is there for the plan: with it the
/// database walks the index of `expires_at` whichever kind of plan it picks, and without it a
/// general plan would read the whole table. Safe with
/// several copies of the application cleaning at once: each skips the rows another one is
/// just deleting.
pub async fn delete_expired(
    connection: &mut PgConnection,
    dead_since: DateTime<Utc>,
    limit: i64,
) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        "DELETE FROM zgm_limits.counters \
         WHERE ctid = ANY(ARRAY( \
             SELECT ctid FROM zgm_limits.counters \
             WHERE expires_at <= $1 \
             ORDER BY expires_at \
             LIMIT $2 \
             FOR UPDATE SKIP LOCKED))",
    )
    .bind(dead_since)
    .bind(limit)
    .execute(connection)
    .await?;
    Ok(deleted.rows_affected())
}

/// The rules the tenant set for itself: the name of the limit and the rule, for every limit it
/// changed. A rule of a way of counting this version of the module does not know is left out.
pub async fn list_tenant_rules(
    connection: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<Vec<(String, Rule)>, sqlx::Error> {
    type Row = (String, String, Option<i64>, Option<i64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT limit_name, rule->>'kind', \
                (rule->>'at_most')::bigint, (rule->>'per_seconds')::bigint \
         FROM zgm_limits.tenant_rules \
         WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_all(connection)
    .await?;

    // The module trusts its own table: only `set_tenant_rule` writes it, from a `Rule`.
    Ok(rows
        .into_iter()
        .filter_map(|(limit_name, kind, at_most, per_seconds)| {
            let rule = Rule::of_kind(
                &kind,
                Count::new(at_most? as u32),
                Period::seconds(per_seconds? as u32),
            )?;
            Some((limit_name, rule))
        })
        .collect())
}

/// Writes the tenant's own rule for a limit: the way of counting `kind` with its two numbers.
/// Says whether anything changed; the same rule written again changes nothing.
pub async fn set_tenant_rule(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    limit_name: &str,
    kind: &str,
    at_most: Count,
    per: Period,
) -> Result<bool, sqlx::Error> {
    let changed = sqlx::query(
        "INSERT INTO zgm_limits.tenant_rules (tenant_id, limit_name, rule) \
         VALUES ($1, $2, jsonb_build_object( \
             'kind', $5::text, 'at_most', $3::bigint, 'per_seconds', $4::bigint)) \
         ON CONFLICT (tenant_id, limit_name) DO UPDATE SET rule = excluded.rule \
         WHERE tenant_rules.rule IS DISTINCT FROM excluded.rule",
    )
    .bind(tenant_id)
    .bind(limit_name)
    .bind(i64::from(at_most.get()))
    .bind(i64::from(per.as_seconds()))
    .bind(kind)
    .execute(connection)
    .await?;
    Ok(changed.rows_affected() > 0)
}

/// Deletes the tenant's own rule for a limit, so the default is in force again. Says whether
/// the tenant had one.
pub async fn delete_tenant_rule(
    connection: &mut PgConnection,
    tenant_id: Uuid,
    limit_name: &str,
) -> Result<bool, sqlx::Error> {
    let deleted =
        sqlx::query("DELETE FROM zgm_limits.tenant_rules WHERE tenant_id = $1 AND limit_name = $2")
            .bind(tenant_id)
            .bind(limit_name)
            .execute(connection)
            .await?;
    Ok(deleted.rows_affected() > 0)
}
