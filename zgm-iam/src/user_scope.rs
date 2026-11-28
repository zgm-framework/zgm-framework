//! What the module does with one user of a tenant.

use uuid::Uuid;

use crate::error::IamError;
use crate::iam_setting::IamSetting;
use crate::repository;
use crate::state::State;
use crate::tenant_scope::{TenantScope, refuse_unless_active, within_the_dates_the_module_holds};
use crate::user::User;

/// One user of a tenant: every function of one user hangs from here.
///
/// ```ignore
/// let in_kowalski = iam.tenant_id(kowalski.id);
/// let anna = in_kowalski.create_user().await?;
/// let found = in_kowalski.user_id(anna.id).get().await?; // Some(anna)
/// let anna = in_kowalski.user_id(anna.id).delete().await?; // Some(anna), gone in 30 days
/// let anna = in_kowalski.user_id(anna.id).set_state(State::Active).await?; // back again
/// ```
///
/// [`TenantScope::user_id`] gives it without asking the database; a user the tenant does not
/// have is `None` from the function called.
#[derive(Debug, Clone)]
pub struct UserScope {
    tenant: TenantScope,
    user_id: Uuid,
}

impl UserScope {
    pub(crate) fn new(tenant: TenantScope, user_id: Uuid) -> Self {
        Self { tenant, user_id }
    }

    /// The user, in whatever state it is, or `None` when the tenant has no such user; a user
    /// of another tenant is `None` too.
    ///
    /// Only an active tenant shows its users. Reading is refused like writing: when the
    /// tenant does not exist ([`IamError::NoSuchTenant`]), or when it is suspended or waits
    /// for deletion ([`IamError::TenantNotActive`], with its state), even for an id of no
    /// user.
    pub async fn get(&self) -> Result<Option<User>, IamError> {
        let mut connection = self.tenant.iam().pool().acquire().await?;
        let (state_of_tenant, row) =
            repository::select_user(&mut connection, self.tenant.id(), self.user_id).await?;
        refuse_unless_active(state_of_tenant)?;
        Ok(row.map(|row| self.tenant.user_of(row)))
    }

    /// Puts the user in a state: blocks it ([`State::Suspended`]), unblocks it
    /// ([`State::Active`]), or brings it back from waiting for deletion into either of them,
    /// also after its date of deletion, as long as the user is not gone. It gives the user
    /// back, or `None` when the tenant has no such user; a user of another tenant is `None`
    /// too, and nothing changes.
    ///
    /// ```ignore
    /// let anna = in_kowalski.user_id(anna.id).set_state(State::Suspended).await?; // blocked
    /// let anna = in_kowalski.user_id(anna.id).set_state(State::Active).await?;    // unblocked
    /// ```
    ///
    /// A user who leaves [`State::PendingDeletion`] has no date of deletion any more. A user
    /// in the state given already stays as it was, and not even `updated_at` moves.
    ///
    /// Nothing changes for [`State::PendingDeletion`] ([`IamError::NotSettable`]): a user waits
    /// for deletion only through [`delete`](Self::delete), which gives it its date. Only an
    /// active tenant changes its users, as in [`delete`](Self::delete): nothing changes when
    /// the tenant does not exist ([`IamError::NoSuchTenant`]), or when it is suspended or
    /// waits for deletion ([`IamError::TenantNotActive`], with its state), even for an id of
    /// no user. A user whose state the module does not know is
    /// [`IamError::UnknownState`], and its row keeps the word.
    pub async fn set_state(&self, state: State) -> Result<Option<User>, IamError> {
        match state {
            State::PendingDeletion => return Err(IamError::NotSettable(state)),
            State::Active | State::Suspended => {}
        }
        let mut transaction = self.tenant.iam().pool().begin().await?;
        let state_of_tenant =
            repository::lock_state_of_tenant(&mut transaction, self.tenant.id()).await?;
        refuse_unless_active(state_of_tenant)?;
        let row =
            repository::set_state_of_user(&mut transaction, self.tenant.id(), self.user_id, state)
                .await?;
        transaction.commit().await?;
        Ok(row.map(|row| self.tenant.user_of(row)))
    }

    /// Deletes the user, after a window: the user is [`State::PendingDeletion`] and gone at
    /// `deletes_at`, until when [`set_state`](Self::set_state) can bring it back. It gives the
    /// user back, or `None` when the tenant has no such user; a user of another tenant is
    /// `None` too, and nothing changes.
    ///
    /// The window is the tenant's [`IamSetting::UserDeletionWindow`], 30 days unless the
    /// tenant set otherwise, asked of the application's [`TenantSettings`] first; any length
    /// works, and 0 deletes at once. The date is the moment of the change, by the clock of the
    /// database, and the window, to the microsecond.
    ///
    /// An active or a suspended user waits for deletion. A user who waits already stays as
    /// it was, with the date it was given, and not even `updated_at` moves.
    ///
    /// Only an active tenant deletes its users, as in
    /// [`create_user`](TenantScope::create_user):
    /// nothing changes when the tenant does not exist ([`IamError::NoSuchTenant`]), or when
    /// it is suspended or waits for deletion ([`IamError::TenantNotActive`], with its state),
    /// even for an id of no user. Nothing changes either when the settings fail
    /// ([`IamError::Settings`]), or when the date would be past the end of the year 9999
    /// ([`IamError::DeletionDateOutOfRange`]), the latest date the module holds.
    ///
    /// [`TenantSettings`]: crate::TenantSettings
    pub async fn delete(&self) -> Result<Option<User>, IamError> {
        let window = self
            .tenant
            .iam()
            .settings()
            .duration(self.tenant.id(), IamSetting::UserDeletionWindow.as_ref())
            .await?;
        let window = within_the_dates_the_module_holds(window)?;
        let mut transaction = self.tenant.iam().pool().begin().await?;
        let state_of_tenant =
            repository::lock_state_of_tenant(&mut transaction, self.tenant.id()).await?;
        refuse_unless_active(state_of_tenant)?;
        let row = repository::schedule_deletion_of_user(
            &mut transaction,
            self.tenant.id(),
            self.user_id,
            window,
        )
        .await?;
        transaction.commit().await?;
        Ok(row.map(|row| self.tenant.user_of(row)))
    }
}
