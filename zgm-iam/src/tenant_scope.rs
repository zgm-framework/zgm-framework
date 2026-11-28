//! What the module does within one tenant.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::error::IamError;
use crate::iam::Iam;
use crate::iam_setting::IamSetting;
use crate::order::Order;
use crate::repository::{self, UserRow};
use crate::state::State;
use crate::tenant::Tenant;
use crate::user::{User, UsersPage};
use crate::user_scope::UserScope;

/// The most users a page of the list holds.
const PAGE: usize = 100;

/// The module within one tenant: every function of a tenant hangs from here, so none can be
/// called without the tenant, and none reaches past it.
///
/// ```ignore
/// let in_kowalski = iam.tenant_id(kowalski.id);
/// let anna = in_kowalski.create_user().await?;
/// let page = in_kowalski.list_users().call().await?; // the newest 100 users
/// let found = in_kowalski.user_id(anna.id).get().await?; // Some(anna)
/// let kowalski = in_kowalski.delete().await?; // Some(kowalski), gone in 90 days
/// ```
///
/// [`Iam::tenant_id`] gives it without asking the database; a tenant that does not exist
/// shows up as an error of the first function called. It holds its own copy of [`Iam`], so
/// an application with one tenant can make it once, when it starts, and clone it where it
/// needs it.
#[derive(Debug, Clone)]
pub struct TenantScope {
    iam: Iam,
    tenant_id: Uuid,
}

#[bon::bon]
impl TenantScope {
    pub(crate) fn new(iam: Iam, tenant_id: Uuid) -> Self {
        Self { iam, tenant_id }
    }

    /// Creates a user of the tenant and gives it back, with the id, the state and the dates
    /// the database gave it. A new user is [`State::Active`] and waits for no deletion.
    ///
    /// Only an active tenant gets a user. Nothing is written when the tenant does not exist
    /// ([`IamError::NoSuchTenant`]), or when it is suspended or waits for deletion
    /// ([`IamError::TenantNotActive`], with its state). A tenant being suspended at the same
    /// moment is waited for: the user is written either before the suspension or not at all.
    pub async fn create_user(&self) -> Result<User, IamError> {
        let mut transaction = self.iam.pool().begin().await?;
        let state_of_tenant =
            repository::lock_state_of_tenant(&mut transaction, self.tenant_id).await?;
        refuse_unless_active(state_of_tenant)?;
        let row = repository::insert_user(&mut transaction, self.tenant_id).await?;
        transaction.commit().await?;
        Ok(self.user_of(row))
    }

    /// One page of the users of the tenant, in whatever state each user is: at most 100, the
    /// newest first unless `order` says otherwise.
    ///
    /// ```ignore
    /// let page = in_kowalski.list_users().call().await?;
    /// let next = in_kowalski
    ///     .list_users()
    ///     .maybe_after(page.next_cursor)
    ///     .call()
    ///     .await?;
    /// let suspended = in_kowalski
    ///     .list_users()
    ///     .order(Order::OldestFirst)
    ///     .state(State::Suspended)
    ///     .call()
    ///     .await?;
    /// ```
    ///
    /// A tenant without users, or without users in `state`, gives an empty page. Only an
    /// active tenant shows its users, as in [`UserScope::get`]: when the tenant
    /// does not exist the list is [`IamError::NoSuchTenant`], and when it is suspended or
    /// waits for deletion [`IamError::TenantNotActive`], with its state.
    #[builder]
    pub async fn list_users(
        &self,
        /// The newest first, or the oldest first; without it, the newest first.
        #[builder(default)]
        order: Order,
        /// Only the users in this state; without it, all of them.
        state: Option<State>,
        /// `next_cursor` of the page before, with the same `order` and `state`; without it,
        /// the first page.
        after: Option<Uuid>,
    ) -> Result<UsersPage, IamError> {
        let mut connection = self.iam.pool().acquire().await?;
        // One row more than a page holds: it only says whether there is a next page.
        let (state_of_tenant, mut rows) = repository::select_users(
            &mut connection,
            self.tenant_id,
            order,
            state,
            after,
            PAGE as i64 + 1,
        )
        .await?;
        refuse_unless_active(state_of_tenant)?;
        let has_next_page = rows.len() > PAGE;
        rows.truncate(PAGE);
        let items: Vec<User> = rows.into_iter().map(|row| self.user_of(row)).collect();
        let next_cursor = if has_next_page {
            items.last().map(|user| user.id)
        } else {
            None
        };
        Ok(UsersPage { items, next_cursor })
    }

    /// Puts the tenant in a state: blocks it ([`State::Suspended`]), unblocks it
    /// ([`State::Active`]), or brings it back from waiting for deletion into either of them,
    /// as long as it is not gone. It works in any state of the tenant, as it is the way out of
    /// them. It gives the tenant back, with its name decrypted, or `None` when there is no
    /// such tenant.
    ///
    /// ```ignore
    /// let kowalski = in_kowalski.set_state(State::Suspended).await?; // blocked
    /// let kowalski = in_kowalski.set_state(State::Active).await?;    // unblocked
    /// ```
    ///
    /// A tenant that leaves [`State::PendingDeletion`] has no date of deletion any more. A
    /// tenant in the state given already stays as it was, and not even `updated_at` moves. The
    /// users of the tenant stay as they are.
    ///
    /// Nothing changes for [`State::PendingDeletion`] ([`IamError::NotSettable`]): a tenant
    /// waits for deletion only through [`delete`](Self::delete). Nothing changes either when
    /// the cipher cannot decrypt the name ([`IamError::Cipher`], [`IamError::UnreadableName`]):
    /// the name is decrypted before the change is committed. A tenant whose state the module
    /// does not know is [`IamError::UnknownState`], and its row keeps the word.
    pub async fn set_state(&self, state: State) -> Result<Option<Tenant>, IamError> {
        match state {
            State::PendingDeletion => return Err(IamError::NotSettable(state)),
            State::Active | State::Suspended => {}
        }
        let mut transaction = self.iam.pool().begin().await?;
        let row = repository::set_state_of_tenant(&mut transaction, self.tenant_id, state).await?;
        let tenant = match row {
            None => None,
            Some(row) => Some(self.iam.tenant_of(row).await?),
        };
        transaction.commit().await?;
        Ok(tenant)
    }

    /// Deletes the tenant, with all its users, after a window: the tenant is
    /// [`State::PendingDeletion`] and gone at `deletes_at`, until when
    /// [`set_state`](Self::set_state) can bring it back. Meanwhile nothing works in it, and
    /// its users stay as they are. It gives the tenant back, with its name decrypted, or `None`
    /// when there is no such tenant.
    ///
    /// The window is the tenant's [`IamSetting::TenantDeletionWindow`], 90 days unless the
    /// tenant set otherwise, asked of the application's [`TenantSettings`] first. The date is
    /// the moment of the change, by the clock of the database, and the window, to the
    /// microsecond.
    ///
    /// An active or a suspended tenant waits for deletion. A tenant who waits already stays as
    /// it was, with the date it was given, and not even `updated_at` moves.
    ///
    /// Nothing changes when the settings fail ([`IamError::Settings`]), when the date would be
    /// past the end of the year 9999 ([`IamError::DeletionDateOutOfRange`]), or when the
    /// cipher cannot decrypt the name ([`IamError::Cipher`], [`IamError::UnreadableName`]). A
    /// tenant whose state the module does not know is [`IamError::UnknownState`].
    ///
    /// [`TenantSettings`]: crate::TenantSettings
    pub async fn delete(&self) -> Result<Option<Tenant>, IamError> {
        let window = self
            .iam
            .settings()
            .duration(self.tenant_id, IamSetting::TenantDeletionWindow.as_ref())
            .await?;
        let window = within_the_dates_the_module_holds(window)?;
        let mut transaction = self.iam.pool().begin().await?;
        let row = repository::schedule_deletion_of_tenant(&mut transaction, self.tenant_id, window)
            .await?;
        let tenant = match row {
            None => None,
            Some(row) => Some(self.iam.tenant_of(row).await?),
        };
        transaction.commit().await?;
        Ok(tenant)
    }

    /// The functions of one user of the tenant. It does not ask the database whether the
    /// user exists: the function called says so.
    pub fn user_id(&self, user_id: Uuid) -> UserScope {
        UserScope::new(self.clone(), user_id)
    }

    /// The module the tenant is in.
    pub(crate) fn iam(&self) -> &Iam {
        &self.iam
    }

    /// The id of the tenant.
    pub(crate) fn id(&self) -> Uuid {
        self.tenant_id
    }

    /// The user of this tenant that a row is.
    pub(crate) fn user_of(&self, row: UserRow) -> User {
        User {
            id: row.id,
            tenant_id: self.tenant_id,
            state: row.state,
            deletes_at: row.deletes_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Nothing works in a tenant that does not exist (`None`) or that is not active.
pub(crate) fn refuse_unless_active(state_of_tenant: Option<State>) -> Result<(), IamError> {
    match state_of_tenant {
        None => Err(IamError::NoSuchTenant),
        Some(State::Active) => Ok(()),
        Some(state) => Err(IamError::TenantNotActive(state)),
    }
}

/// The window in whole microseconds, the precision of the database, when a date that far from
/// now is still one the module can hold, by the clock of the application. The database adds
/// it to its own clock, which may differ by a few seconds; that matters only at the very end
/// of the year 9999.
pub(crate) fn within_the_dates_the_module_holds(window: Duration) -> Result<i64, IamError> {
    SignedDuration::try_from(window)
        .ok()
        .and_then(|window| Timestamp::now().checked_add(window).ok())
        .and_then(|_| i64::try_from(window.as_micros()).ok())
        .ok_or(IamError::DeletionDateOutOfRange)
}
