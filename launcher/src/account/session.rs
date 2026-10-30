//! Launcher-owned authentication operations. UI, CLI and a future game bridge receive only
//! public status; credentials stay behind SecretStore and the existing session-file handoff.
use super::*;
use std::fs::{File, OpenOptions};
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

/// Public progress only; no account credentials or provider response bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    Keychain,
    Microsoft,
    Xbox,
    Minecraft,
    Ownership,
    Profile,
    Saving,
    SigningOut,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Waiting for account access…",
            Self::Keychain => "Reading keychain…",
            Self::Microsoft => "Renewing Microsoft sign-in…",
            Self::Xbox => "Signing in to Xbox Live…",
            Self::Minecraft => "Getting Minecraft session…",
            Self::Ownership => "Checking Minecraft ownership…",
            Self::Profile => "Loading Minecraft profile…",
            Self::Saving => "Saving sign-in…",
            Self::SigningOut => "Signing out…",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ready,
    NeedsRefresh,
    SignInRequired,
    Retry,
    OwnershipRequired,
}

/// Safe to show or serialize: contains no credential or secret-store contents.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub state: State,
    pub expires_at: Option<u64>,
    pub message: String,
}

const DAMAGED_SIGN_IN: &str = "Could not read the saved sign-in. Sign in again.";

impl Status {
    pub fn label(&self) -> &'static str {
        match self.state {
            State::Ready => "Ready for online play",
            State::NeedsRefresh => "Needs refresh",
            State::SignInRequired => "Sign in again",
            State::Retry => "Could not check sign-in",
            State::OwnershipRequired => "Minecraft account needs attention",
        }
    }

    fn failed(message: &str) -> Self {
        Self {
            state: if message == DAMAGED_SIGN_IN {
                State::SignInRequired
            } else {
                State::Retry
            },
            expires_at: None,
            message: message.into(),
        }
    }

    fn from_secrets(secrets: Option<&Secrets>) -> Self {
        let state = match secrets {
            None => State::SignInRequired,
            Some(s) if s.sign_in_required => State::SignInRequired,
            Some(s) if !s.owns => State::OwnershipRequired,
            Some(s) if s.minecraft_expires_at <= now() + RENEW_BEFORE => State::NeedsRefresh,
            Some(_) => State::Ready,
        };
        let mut status = Self {
            state,
            expires_at: secrets.map(|s| s.minecraft_expires_at),
            message: String::new(),
        };
        status.message = match state {
            State::SignInRequired => "Sign in again in your browser.".into(),
            State::OwnershipRequired => {
                "Check Minecraft: Java Edition ownership and your profile at minecraft.net.".into()
            }
            _ => status.label().into(),
        };
        status
    }
}

static OPERATIONS: Mutex<()> = Mutex::new(());

pub(super) struct Operation {
    _process: MutexGuard<'static, ()>,
    _file: Option<File>,
}

/// How long a status read waits behind another account operation (a refresh or a launch check
/// can hold the lock for a network timeout) before it answers "busy" instead.
const STATUS_WAIT: Duration = Duration::from_secs(3);
const BUSY: &str = "Another account operation is running. Try again in a moment.";

/// Serialize refresh with sign-in, sign-out and launch, before reading any credential.
/// The file is a lock only: it never contains tokens. Fail closed if locking fails.
pub(super) fn lock(store: &dyn SecretStore) -> Result<Operation, String> {
    lock_within(store, None)
}

/// [`lock`], giving up with [`BUSY`] after `within` when one is given.
pub(super) fn lock_within(
    store: &dyn SecretStore,
    within: Option<Duration>,
) -> Result<Operation, String> {
    let deadline = within.map(|d| Instant::now() + d);
    // Waits for `take` to stop answering None (busy), or until the deadline.
    let wait = |take: &mut dyn FnMut() -> Result<Option<()>, String>| loop {
        match take()? {
            Some(()) => return Ok(()),
            None if deadline.is_some_and(|d| Instant::now() >= d) => return Err(BUSY.to_string()),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let failed = "Account operation failed. Restart the launcher.";
    let mut process = None;
    match deadline {
        None => process = Some(OPERATIONS.lock().map_err(|_| failed)?),
        Some(_) => wait(&mut || match OPERATIONS.try_lock() {
            Ok(guard) => {
                process = Some(guard);
                Ok(Some(()))
            }
            Err(std::sync::TryLockError::WouldBlock) => Ok(None),
            Err(std::sync::TryLockError::Poisoned(_)) => Err(failed.into()),
        })?,
    }
    let file = store
        .lock_file()
        .map(|path| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|_| "Could not create the account lock.")?;
            }
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)
                .map_err(|_| "Could not open the account lock.")?;
            let failed = "Could not lock account operations.";
            match deadline {
                None => file.lock().map_err(|_| failed)?,
                Some(_) => wait(&mut || match file.try_lock() {
                    Ok(()) => Ok(Some(())),
                    Err(std::fs::TryLockError::WouldBlock) => Ok(None),
                    Err(std::fs::TryLockError::Error(_)) => Err(failed.into()),
                })?,
            }
            Ok::<_, String>(file)
        })
        .transpose()?;
    Ok(Operation {
        _process: process.expect("taken above"),
        _file: file,
    })
}

fn read(store: &dyn SecretStore, uuid: &str) -> Result<Option<Secrets>, String> {
    store
        .get(uuid)
        .map_err(|_| "Could not read the keychain. Unlock it and retry.".to_string())?
        .map(|text| serde_json::from_str(&text).map_err(|_| DAMAGED_SIGN_IN.into()))
        .transpose()
}

/// Inspect local expiry and remembered failures without contacting authentication services.
/// Call off the UI thread: unlocking the keychain can require user interaction.
pub fn status(store: &dyn SecretStore, uuid: &str) -> Status {
    status_with_progress(store, uuid, &|_| {})
}

pub fn status_with_progress(
    store: &dyn SecretStore,
    uuid: &str,
    progress: &dyn Fn(Phase),
) -> Status {
    let result = (|| {
        progress(Phase::Queued);
        let _operation = lock_within(store, Some(STATUS_WAIT))?;
        progress(Phase::Keychain);
        Ok::<_, String>(Status::from_secrets(read(store, uuid)?.as_ref()))
    })();
    result.unwrap_or_else(|e| Status::failed(&e))
}

/// Force a renewal even when the access token is fresh. Does not select an account, change
/// the public account list, or return a token. A future IPC handler can call this same API,
/// then use write_session_file for an explicit game-session handoff.
pub fn refresh(who: &str) -> Result<Status, String> {
    refresh_with_progress(who, &|_| {})
}

pub fn refresh_with_progress(who: &str, progress: &dyn Fn(Phase)) -> Result<Status, String> {
    let account = Accounts::load()
        .find(who)
        .cloned()
        .ok_or("That account isn't signed in.")?;
    Ok(refresh_account(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &account,
        progress,
    ))
}

#[cfg(test)]
pub(super) fn refresh_with(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    account: &Account,
) -> Status {
    refresh_account(ep, client_id, store, account, &|_| {})
}

pub(super) fn refresh_account(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    account: &Account,
    progress: &dyn Fn(Phase),
) -> Status {
    let result = (|| {
        progress(Phase::Queued);
        let _operation = lock(store)?;
        progress(Phase::Keychain);
        let Some(secrets) = read(store, &account.uuid)? else {
            return Ok(Status::from_secrets(None));
        };
        let before =
            serde_json::to_string(&secrets).map_err(|_| "Could not read the saved sign-in.")?;
        let (mut status, secrets) =
            match check_with(ep, client_id, account, secrets, true, progress) {
                Check::Owns(_, s) => (Status::from_secrets(Some(&s)), s),
                Check::DoesNot(s) => (Status::from_secrets(Some(&s)), s),
                Check::NoAnswer(e, s) => {
                    let mut status = Status::from_secrets(Some(&s));
                    if !s.sign_in_required {
                        status.state = State::Retry;
                    }
                    status.message = e;
                    (status, s)
                }
            };
        let after =
            serde_json::to_string(&secrets).map_err(|_| "Could not save the renewed sign-in.")?;
        if before != after {
            progress(Phase::Saving);
        }
        if before != after && store.set(&account.uuid, &after).is_err() {
            status.state = State::Retry;
            status.message =
                "The renewed sign-in could not be saved in the keychain. Unlock it and retry."
                    .into();
        }
        Ok::<_, String>(status)
    })();
    result.unwrap_or_else(|e| Status::failed(&e))
}

/// Renew `who` for a game already running (a server refused its session, or a splitscreen
/// player joins one) and hand it over as at launch: a fresh read-once file in `dir`. No file
/// unless the renewal left the account ready; the status then says why (sign in again, retry).
pub fn renew_for_game(dir: &Path, who: &str) -> Result<PathBuf, Status> {
    let account = Accounts::load()
        .find(who)
        .cloned()
        .ok_or_else(|| Status::from_secrets(None))?;
    renew_for_game_with(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &account,
        dir,
    )
}

pub(super) fn renew_for_game_with(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    account: &Account,
    dir: &Path,
) -> Result<PathBuf, Status> {
    let status = refresh_account(ep, client_id, store, account, &|_| {});
    if status.state != State::Ready {
        return Err(status);
    }
    let _operation = lock(store).map_err(|e| Status::failed(&e))?;
    let secrets = read(store, &account.uuid)
        .map_err(|e| Status::failed(&e))?
        .ok_or_else(|| Status::from_secrets(None))?;
    write_session_file(dir, account, &secrets).map_err(|e| Status::failed(&e))
}
