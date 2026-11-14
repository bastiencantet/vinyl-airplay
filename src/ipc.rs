//! Local IPC: permission-restricted Unix sockets or local-only Windows named pipes.
use std::path::{Path, PathBuf};
use tokio::io::{AsyncRead, AsyncWrite};

pub trait Connection: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Connection for T {}
pub type Stream = Box<dyn Connection>;

pub fn endpoint(root: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        root.join("control/audio.sock")
    }
    #[cfg(windows)]
    {
        let _ = root;
        PathBuf::from(format!(r"\\.\pipe\vinyl-airplay-{}", uuid::Uuid::new_v4()))
    }
}

pub async fn connect(path: &Path) -> std::io::Result<Stream> {
    #[cfg(unix)]
    {
        Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(
            tokio::net::windows::named_pipe::ClientOptions::new().open(path)?,
        ))
    }
}

pub struct Listener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    inner: tokio::net::windows::named_pipe::NamedPipeServer,
    path: PathBuf,
}
impl Listener {
    pub fn bind(path: &Path) -> anyhow::Result<Self> {
        #[cfg(unix)]
        let inner = {
            use std::os::unix::fs::{FileTypeExt, PermissionsExt};
            if path.exists() {
                anyhow::ensure!(
                    std::fs::symlink_metadata(path)?.file_type().is_socket(),
                    "control path is not a Unix socket"
                );
                anyhow::ensure!(
                    std::os::unix::net::UnixStream::connect(path).is_err(),
                    "another sender owns the control socket"
                );
                std::fs::remove_file(path)?;
            }
            let listener = tokio::net::UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            listener
        };
        #[cfg(windows)]
        let inner = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(path)?;
        Ok(Self {
            inner,
            path: path.to_owned(),
        })
    }
    pub async fn accept(&mut self) -> std::io::Result<Stream> {
        #[cfg(unix)]
        {
            Ok(Box::new(self.inner.accept().await?.0))
        }
        #[cfg(windows)]
        {
            self.inner.connect().await?;
            let next = tokio::net::windows::named_pipe::ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.path)?;
            Ok(Box::new(std::mem::replace(&mut self.inner, next)))
        }
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn local_connection_roundtrip_and_single_owner() {
        let root = std::env::temp_dir().join(format!(
            "v{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        std::fs::create_dir_all(root.join("control")).unwrap();
        let path = endpoint(&root);
        let mut listener = Listener::bind(&path).unwrap();
        assert!(Listener::bind(&path).is_err());
        #[cfg(unix)]
        {
            // The existing-owner probe connects and immediately closes.
            let mut probe = listener.accept().await.unwrap();
            assert_eq!(probe.read(&mut [0]).await.unwrap(), 0);
        }
        let server = tokio::spawn(async move {
            let mut connection = listener.accept().await.unwrap();
            let mut bytes = [0; 4];
            connection.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"ping");
            connection.write_all(b"pong").await.unwrap();
            connection.flush().await.unwrap();
            // Windows clients must consume the reply before the pipe handle is closed.
            let _ = connection.read(&mut [0]).await;
        });
        let mut client = connect(&path).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut bytes = [0; 4];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"pong");
        drop(client);
        server.await.unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
