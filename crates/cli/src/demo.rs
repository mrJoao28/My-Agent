//! Offline provider for exercising the real journal, approval and undo pipeline.
use myagent_core::{
    AgentError, BoxFuture, CancellationToken, ModelRole, Provider, ProviderEvent, ProviderRequest,
    Result, ToolCall,
};
use myagent_protocol::MessagePart;
use serde_json::json;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::mpsc;
pub struct DemoProvider;
static NEXT: AtomicU64 = AtomicU64::new(1);
impl Provider for DemoProvider {
    fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
        events: mpsc::Sender<ProviderEvent>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let can_write = request.tools.iter().any(|tool| tool.name == "write_file");
            let recent = request
                .messages
                .iter()
                .rposition(|message| message.role == ModelRole::User)
                .unwrap_or(0);
            let writes = request.messages[recent..]
                .iter()
                .flat_map(|message| &message.content)
                .filter(|part| matches!(part, MessagePart::ToolResult { .. }))
                .count();
            if !can_write || writes >= 2 {
                let text = if can_write {
                    "Demonstração concluída. Use /undo para restaurar o último arquivo aprovado."
                } else {
                    "PLAN: criar demo.rs e tests/demo_test.rs após mudar para BUILD. Esta demonstração não usa rede nem credenciais."
                };
                for word in text.split_inclusive(' ') {
                    send(&events, &cancel, ProviderEvent::Text(word.into())).await?;
                    pause(&cancel).await?;
                }
            } else {
                let (path, content) = if writes == 0 {
                    (
                        "demo.rs",
                        "pub fn greet(name: &str) -> String {\n    format!(\"Olá, {name}!\")\n}\n",
                    )
                } else {
                    (
                        "tests/demo_test.rs",
                        "#[test]\nfn greeting_is_readable() {\n    assert!(\"Olá, mundo!\".starts_with(\"Olá\"));\n}\n",
                    )
                };
                send(
                    &events,
                    &cancel,
                    ProviderEvent::Text(format!(
                        "Preparando {path}. A escrita aguardará sua aprovação.\n"
                    )),
                )
                .await?;
                let id = format!("demo_{}", NEXT.fetch_add(1, Ordering::Relaxed));
                let args = json!({"path":path,"content":content});
                let raw = args.to_string();
                for (index, _) in raw.char_indices().step_by(8) {
                    send(
                        &events,
                        &cancel,
                        ProviderEvent::ToolPreview {
                            id: id.clone(),
                            name: "write_file".into(),
                            arguments: raw[..index].into(),
                        },
                    )
                    .await?;
                    pause(&cancel).await?;
                }
                send(
                    &events,
                    &cancel,
                    ProviderEvent::ToolCall(ToolCall {
                        id,
                        name: "write_file".into(),
                        args: args.as_object().unwrap().clone(),
                    }),
                )
                .await?;
            }
            send(&events, &cancel, ProviderEvent::Usage(1200)).await?;
            send(&events, &cancel, ProviderEvent::Done).await
        })
    }
}
async fn pause(cancel: &CancellationToken) -> Result<()> {
    tokio::select! { _=cancel.cancelled()=>Err(AgentError::Cancelled), _=tokio::time::sleep(Duration::from_millis(35))=>Ok(()) }
}
async fn send(
    events: &mpsc::Sender<ProviderEvent>,
    cancel: &CancellationToken,
    event: ProviderEvent,
) -> Result<()> {
    tokio::select! { _=cancel.cancelled()=>Err(AgentError::Cancelled), result=events.send(event)=>result.map_err(|_|AgentError::Cancelled) }
}
