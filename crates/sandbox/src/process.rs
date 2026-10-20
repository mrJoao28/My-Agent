use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Child,
};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    Cancelled,
    Limit,
    Io,
}
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "process supervision failed: {self:?}")
    }
}
impl std::error::Error for ProcessError {}
#[derive(Debug)]
pub struct ProcessOutput {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}
async fn drain(
    reader: Option<impl AsyncRead + Unpin>,
    budget: Arc<AtomicUsize>,
) -> Result<Vec<u8>, ProcessError> {
    let Some(mut reader) = reader else {
        return Ok(vec![]);
    };
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|_| ProcessError::Io)?;
        if read == 0 {
            return Ok(output);
        }
        budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(read)
            })
            .map_err(|_| ProcessError::Limit)?;
        output.extend_from_slice(&buffer[..read]);
    }
}
/// Adota um processo já autorizado/criado pelo sandbox; nunca monta comandos.
pub async fn supervise_child(
    child: Child,
    cancel: CancellationToken,
    output_limit: usize,
) -> Result<ProcessOutput, ProcessError> {
    let mut child = KillOnDrop(child);
    drop(child.0.stdin.take());
    let stdout = child.0.stdout.take();
    let stderr = child.0.stderr.take();
    let budget = Arc::new(AtomicUsize::new(output_limit));
    let result = tokio::select! {biased;
        _=cancel.cancelled()=>Err(ProcessError::Cancelled),
        result=async {
            let (status,stdout,stderr)=tokio::try_join!(
                async {child.0.wait().await.map_err(|_|ProcessError::Io)},
                drain(stdout,budget.clone()),
                drain(stderr,budget)
            )?;
            Ok(ProcessOutput{status,stdout,stderr})
        }=>result,
    };
    if result.is_err() {
        let _ = child.0.start_kill();
        let _ = child.0.wait().await;
    }
    result
}
