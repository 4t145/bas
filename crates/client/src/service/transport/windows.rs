//! Same-user, local-only named pipes. Authentication precedes application I/O.

use std::{
    ffi::c_void,
    io,
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    pin::Pin,
    ptr::null_mut,
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions},
    time::{sleep, timeout},
};
use windows_sys::Win32::{
    Foundation::{ERROR_PIPE_BUSY, FILETIME, LocalFree, WAIT_TIMEOUT},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        },
        GetTokenInformation, OWNER_SECURITY_INFORMATION, RevertToSelf, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        Pipes::{GetNamedPipeServerProcessId, ImpersonateNamedPipeClient},
        SystemInformation::GetSystemTimeAsFileTime,
        Threading::{
            GetCurrentProcess, GetCurrentThread, GetProcessTimes, OpenProcess, OpenProcessToken,
            OpenThreadToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    },
};

use crate::service::Target;

#[cfg(all(test, feature = "server"))]
mod tests;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const BUSY_RETRY: Duration = Duration::from_millis(20);
const HANDSHAKE: u8 = 1;

pub(super) enum Stream {
    Client(NamedPipeClient),
    #[cfg(feature = "server")]
    Server(NamedPipeServer),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Client(pipe) => Pin::new(pipe).poll_read(cx, buf),
            #[cfg(feature = "server")]
            Self::Server(pipe) => Pin::new(pipe).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Client(pipe) => Pin::new(pipe).poll_write(cx, buf),
            #[cfg(feature = "server")]
            Self::Server(pipe) => Pin::new(pipe).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Client(pipe) => Pin::new(pipe).poll_flush(cx),
            #[cfg(feature = "server")]
            Self::Server(pipe) => Pin::new(pipe).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Client(pipe) => Pin::new(pipe).poll_shutdown(cx),
            #[cfg(feature = "server")]
            Self::Server(pipe) => Pin::new(pipe).poll_shutdown(cx),
        }
    }
}

/// # Errors
/// Returns an error if the process token's user SID cannot be read.
pub(super) fn default_target() -> io::Result<Target> {
    Ok(Target::Pipe(format!("bas-{}", current_user()?)))
}

/// # Errors
/// Rejects non-local names, inaccessible or differently owned servers, and
/// busy servers or authentication handshakes exceeding five seconds.
pub(super) async fn connect(target: &Target) -> io::Result<Stream> {
    let path = pipe_path(target)?;
    let user = current_user()?;
    timeout(CONNECT_TIMEOUT, async {
        loop {
            // Capture before opening, not after reading the PID: a process
            // created later cannot have owned this connection when it opened.
            let opened_after = system_time();
            // Tokio defaults to SECURITY_IDENTIFICATION: the server can inspect
            // our token, but cannot act with our privileges.
            match ClientOptions::new().open(&path) {
                Ok(mut pipe) => {
                    verify_server(&pipe, &user, opened_after)?;
                    pipe.write_u8(HANDSHAKE).await?;
                    if pipe.read_u8().await? != HANDSHAKE {
                        return Err(denied("invalid named-pipe authentication acknowledgement"));
                    }
                    return Ok(Stream::Client(pipe));
                }
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    sleep(BUSY_RETRY).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "named-pipe connection timed out"))?
}

#[cfg(feature = "server")]
pub(super) struct Listener {
    target: Target,
    path: String,
    user: String,
    pending: NamedPipeServer,
}

#[cfg(feature = "server")]
impl Listener {
    /// # Errors
    /// Rejects invalid targets, occupied names, and security/pipe creation errors.
    pub(super) async fn bind(target: &Target) -> io::Result<Self> {
        let path = pipe_path(target)?;
        let user = current_user()?;
        let pending = create_pipe(&path, &user, true)?;
        Ok(Self {
            target: target.clone(),
            path,
            user,
            pending,
        })
    }

    /// # Errors
    /// Returns pipe errors or errors authenticating the connected client.
    /// A rejected or timed-out client does not relinquish the listener's name.
    pub(super) async fn accept(&mut self) -> io::Result<Stream> {
        self.pending.connect().await?;
        // Keep the old instance alive until its replacement exists. In
        // particular, cancellation during authentication must not free the name.
        let replacement = match create_pipe(&self.path, &self.user, false) {
            Ok(pipe) => pipe,
            Err(error) => {
                let _ = self.pending.disconnect();
                return Err(error);
            }
        };
        let mut pipe = std::mem::replace(&mut self.pending, replacement);
        timeout(CONNECT_TIMEOUT, async {
            // ImpersonateNamedPipeClient authenticates the last message read.
            // This byte is transport framing, never application-supplied identity.
            if pipe.read_u8().await? != HANDSHAKE {
                return Err(denied("invalid named-pipe authentication preface"));
            }
            verify_client(&pipe, &self.user)?;
            pipe.write_u8(HANDSHAKE).await?;
            Ok(Stream::Server(pipe))
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "named-pipe authentication timed out",
            )
        })?
    }

    pub(super) fn target(&self) -> &Target {
        &self.target
    }
}

/// # Errors
/// Only a single local pipe-name component is accepted.
fn pipe_path(target: &Target) -> io::Result<String> {
    let Target::Pipe(name) = target else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a local pipe target",
        ));
    };
    // Windows limits the whole pipe name to 256 characters, including prefix.
    if name.is_empty()
        || name.encode_utf16().count() > 247
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c == '\0' || c == ':')
        || name == "."
        || name == ".."
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid local pipe name",
        ));
    }
    Ok(format!(r"\\.\pipe\{name}"))
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: this allocation was returned by a LocalAlloc-family API.
        unsafe { LocalFree(self.0) };
    }
}

/// # Errors
/// Returns Windows security descriptor or named-pipe creation errors.
#[cfg(feature = "server")]
fn create_pipe(path: &str, user: &str, first: bool) -> io::Result<NamedPipeServer> {
    // Protected DACL: no inherited access, and no Everyone/Administrators ACE.
    // GA includes FILE_CREATE_PIPE_INSTANCE, needed by our subsequent instances.
    let sddl: Vec<u16> = format!("O:{user}D:P(A;;GA;;;{user})\0")
        .encode_utf16()
        .collect();
    let mut descriptor = null_mut();
    // SAFETY: terminated UTF-16 input and valid output storage.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    })?;
    let descriptor = LocalAllocation(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: attributes and its self-relative descriptor remain valid for
    // CreateNamedPipeW; Windows copies the descriptor during the call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                path,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}

/// # Errors
/// Returns Windows token query errors.
fn current_user() -> io::Result<String> {
    let mut token = null_mut();
    // SAFETY: pseudo-process handle and valid output pointer.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    // SAFETY: successful OpenProcessToken returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    token_user(&token)
}

/// # Errors
/// Returns Windows token/SID conversion errors.
fn token_user(token: &OwnedHandle) -> io::Result<String> {
    let mut length = 0;
    // SAFETY: the initial null-buffer call queries the required size.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut length) };
    if length < size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::last_os_error());
    }
    // Word-aligned storage is required for TOKEN_USER and its embedded pointer.
    let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    // SAFETY: aligned buffer has at least length writable bytes.
    check(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    })?;
    // SAFETY: successful TokenUser query initialized TOKEN_USER and its SID.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut text = null_mut();
    // SAFETY: SID points inside the live token information buffer.
    check(unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) })?;
    let allocation = LocalAllocation(text.cast());
    let mut length = 0;
    // SAFETY: ConvertSidToStringSidW returns a NUL-terminated allocated string.
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16(std::slice::from_raw_parts(text, length))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid Windows SID string"));
        drop(allocation);
        result
    }
}

/// # Errors
/// Fails closed if the server process cannot be pinned and authenticated.
fn verify_server(pipe: &NamedPipeClient, expected: &str, opened_after: u64) -> io::Result<()> {
    verify_pipe_owner(pipe, expected)?;
    let mut pid = 0;
    // SAFETY: connected pipe handle and valid process-id output.
    check(unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) })?;
    // SAFETY: process access is query-only; the returned handle is checked.
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcess returned an owned handle. Keeping it alive
    // pins this process object, preventing further PID reuse during verification.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    // SAFETY: FILETIME is plain integer storage and all outputs are writable.
    let (mut created, mut exited, mut kernel, mut user) =
        unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
    // SAFETY: pinned process handle and valid FILETIME output pointers.
    check(unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    })?;
    // Equal timestamps are ambiguous at the system clock's resolution: fail
    // closed rather than potentially accepting a recycled PID in the same tick.
    if file_time(created) >= opened_after {
        return Err(denied("named-pipe server PID was reused"));
    }
    let mut token = null_mut();
    // SAFETY: pinned process handle and valid token output.
    check(unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut token) })?;
    // SAFETY: successful OpenProcessToken returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    if token_user(&token)? != expected {
        return Err(denied(
            "named-pipe server belongs to a different Windows user",
        ));
    }
    // Reject dead servers (including a pipe handle inherited from a dead owner).
    // SAFETY: query-only zero-duration wait on the pinned process handle.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } != WAIT_TIMEOUT {
        return Err(denied("named-pipe server process is no longer alive"));
    }
    Ok(())
}

/// # Errors
/// Rejects objects owned by another user, even if the original server exited
/// before connection and its PID was recycled while an inherited pipe survived.
fn verify_pipe_owner(pipe: &NamedPipeClient, expected: &str) -> io::Result<()> {
    let mut owner = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: connected pipe and valid output pointers; only owner information
    // is requested. GetSecurityInfo allocates the descriptor with LocalAlloc.
    let error = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let _descriptor = LocalAllocation(descriptor);
    if owner.is_null() {
        return Err(denied("named-pipe server has no owner"));
    }
    let mut text = null_mut();
    // SAFETY: owner SID points into the live descriptor allocation.
    check(unsafe { ConvertSidToStringSidW(owner, &mut text) })?;
    let _text = LocalAllocation(text.cast());
    // Compare the canonical SID's UTF-16 including its terminator. The Windows
    // allocation is a terminated SID string, so stop at its terminator too.
    for (index, expected) in expected.encode_utf16().chain(Some(0)).enumerate() {
        // SAFETY: previous iterations consumed only non-NUL characters.
        let actual = unsafe { *text.add(index) };
        if actual != expected {
            return Err(denied(
                "named-pipe object belongs to a different Windows user",
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "server")]
struct Impersonation;

#[cfg(feature = "server")]
impl Drop for Impersonation {
    fn drop(&mut self) {
        // SAFETY: used only on the thread that successfully impersonated, without
        // an await. Windows explicitly requires terminating if reversion fails:
        // continuing could execute unrelated tasks under the client's identity.
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

/// # Errors
/// Rejects anonymous clients, failed impersonation, and other users' tokens.
#[cfg(feature = "server")]
fn verify_client(pipe: &NamedPipeServer, expected: &str) -> io::Result<()> {
    // SAFETY: live pipe with a just-read authentication message.
    // https://learn.microsoft.com/windows/win32/api/namedpipeapi/nf-namedpipeapi-impersonatenamedpipeclient
    check(unsafe { ImpersonateNamedPipeClient(pipe.as_raw_handle()) })?;
    let impersonation = Impersonation;
    let mut token = null_mut();
    // SAFETY: pseudo-thread handle, valid output; OpenAsSelf permits querying an
    // identification-level token using the server's own access rights.
    check(unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) })?;
    // SAFETY: successful OpenThreadToken returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let actual = token_user(&token)?;
    drop(impersonation);
    if actual != expected {
        return Err(denied(
            "named-pipe client belongs to a different Windows user",
        ));
    }
    Ok(())
}

fn system_time() -> u64 {
    // SAFETY: valid FILETIME output storage.
    let mut time = unsafe { zeroed() };
    unsafe { GetSystemTimeAsFileTime(&mut time) };
    file_time(time)
}

fn file_time(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// # Errors
/// Converts a failed Windows BOOL return to the current OS error.
fn check(success: i32) -> io::Result<()> {
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
