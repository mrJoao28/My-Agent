//! Optional interactive gate: command previews and consumable exact-call grants.
use crate::{
    ApprovalManager, Error, Result, SafeWorkspace,
    approval::Scope,
    mutation_io::{hash, workspace_key},
};
use myagent_core::ToolContext;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub(crate) struct CommandGate {
    manager: ApprovalManager,
    permits: Arc<Mutex<HashMap<(String, String), Permit>>>,
}
struct Permit {
    arguments: String,
    epoch: u64,
    expires: Instant,
    cancel: myagent_core::CancellationToken,
}
impl CommandGate {
    pub fn new(manager: ApprovalManager) -> Self {
        Self {
            manager,
            permits: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn forget(&self, session: &str) {
        self.manager.forget(session);
        if let Ok(mut permits) = self.permits.lock() {
            permits.retain(|(id, _), _| id != session);
        }
    }
    pub async fn authorize(
        &self,
        workspace: &SafeWorkspace,
        args: &Map<String, Value>,
        ctx: &ToolContext,
    ) -> Result<()> {
        myagent_protocol::validate_id(&ctx.session_id).map_err(|_| Error::InvalidInput)?;
        myagent_protocol::validate_id(&ctx.tool_call_id).map_err(|_| Error::InvalidInput)?;
        let arguments = myagent_core::guard::key("bash", args);
        let scope = Scope {
            key: hash(format!("{}:{arguments}", workspace_key(workspace.root())).as_bytes()),
            label: format!("Docker · {}", workspace.root().display()),
            allow_session: true,
            main: None,
        };
        let grant = self
            .manager
            .request("bash", args.clone(), &scope, ctx)
            .await?;
        let mut permits = self.permits.lock().map_err(|_| Error::Internal)?;
        permits
            .retain(|_, permit| permit.expires > Instant::now() && !permit.cancel.is_cancelled());
        let key = (ctx.session_id.clone(), ctx.tool_call_id.clone());
        if permits.contains_key(&key) {
            return Err(Error::StaleApproval);
        }
        permits.insert(
            key,
            Permit {
                arguments,
                epoch: grant.epoch,
                expires: Instant::now() + Duration::from_secs(120),
                cancel: ctx.cancellation.clone(),
            },
        );
        Ok(())
    }
    pub fn consume(&self, args: &Map<String, Value>, ctx: &ToolContext) -> Result<()> {
        let permit = self
            .permits
            .lock()
            .map_err(|_| Error::Internal)?
            .remove(&(ctx.session_id.clone(), ctx.tool_call_id.clone()))
            .ok_or(Error::Denied)?;
        if permit.expires <= Instant::now()
            || permit.cancel.is_cancelled()
            || ctx.cancellation.is_cancelled()
            || permit.epoch != self.manager.epoch(&ctx.session_id)?
            || permit.arguments != myagent_core::guard::key("bash", args)
        {
            return Err(Error::StaleApproval);
        }
        Ok(())
    }
}
