// SPDX-License-Identifier: Apache-2.0
// Included in the shared service client. The wire protocol, cancellation and
// receipt handling remain identical on Windows and Unix.
fn startup_process_lock(socket: &Path, timeout: Duration) -> Result<std::fs::File, String> {
    let parent = socket.parent().ok_or("service endpoint has no directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    nola_windows_permissions::protect_private_path(parent)?;
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true)
        .truncate(false).open(socket.with_extension("startup.lock")).map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(format!("acquire service startup lock: {error}")),
        }
    }
}

fn database_owner_is_busy(database: &Path) -> Result<bool, String> {
    let mut name = database_identity_path(database).into_os_string();
    name.push(".service.lock");
    let file = match std::fs::OpenOptions::new().read(true).write(true).open(name) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    match file.try_lock_shared() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(error) => Err(error.to_string()),
    }
}

fn connect_with_deadline(path: &Path, deadline: Option<Instant>, cancellation: Option<&RequestCancellation>)
    -> std::io::Result<UnixStream> {
    if cancellation.is_some_and(RequestCancellation::is_cancelled) {
        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, APPLICATION_SERVICE_REQUEST_CANCELLED));
    }
    if deadline.is_some_and(|t| Instant::now() >= t) {
        return Err(std::io::ErrorKind::TimedOut.into());
    }
    UnixStream::connect(path)
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_read_timeout(self.remaining()?)?;
        self.stream.read(bytes)
    }
}
