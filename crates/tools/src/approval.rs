use crate::{Error, Result, mutation_io::identifier};
use myagent_core::ToolContext;
use myagent_protocol::StreamEvent;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::oneshot, time::Instant};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Once,
    Session,
    Deny,
    ConfirmMain,
}
impl ApprovalDecision {
    pub fn from_key(key: char) -> Option<Self> {
        match key {
            'Y' | 'y' => Some(Self::Once),
            'A' | 'a' => Some(Self::Session),
            'N' | 'n' => Some(Self::Deny),
            'M' | 'm' => Some(Self::ConfirmMain),
            _ => None,
        }
    }
}
const INDIVIDUAL: &[&str] = &[
    "delete_file",
    "git_commit",
    "git_init",
    "git_remote_add",
    "git_push",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalGrant {
    pub choice: String,
    pub scope: String,
    pub confirmation: Option<String>,
    #[serde(skip)]
    pub(crate) epoch: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct Scope {
    pub key: String,
    pub label: String,
    pub allow_session: bool,
    pub main: Option<String>,
}
struct Pending {
    session: String,
    allow_session: bool,
    main: bool,
    reply: oneshot::Sender<ApprovalDecision>,
}
#[derive(Default)]
struct State {
    grants: HashMap<String, HashSet<String>>,
    pending: HashMap<String, Pending>,
    epochs: HashMap<String, u64>,
}
#[derive(Clone, Default)]
pub struct ApprovalManager {
    state: Arc<Mutex<State>>,
}
impl ApprovalManager {
    pub(crate) fn epoch(&self, session: &str) -> Result<u64> {
        Ok(*self
            .state
            .lock()
            .map_err(|_| Error::Internal)?
            .epochs
            .get(session)
            .unwrap_or(&0))
    }
    pub fn decide(&self, session: &str, id: &str, decision: ApprovalDecision) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(p) = state.pending.get(id) else {
            return false;
        };
        if p.session != session
            || (p.main
                && !matches!(
                    decision,
                    ApprovalDecision::ConfirmMain | ApprovalDecision::Deny
                ))
            || (!p.main && decision == ApprovalDecision::ConfirmMain)
            || (decision == ApprovalDecision::Session && !p.allow_session)
        {
            return false;
        }
        state
            .pending
            .remove(id)
            .is_some_and(|p| p.reply.send(decision).is_ok())
    }
    pub fn forget(&self, session: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.grants.remove(session);
            *state.epochs.entry(session.into()).or_default() += 1;
            state.pending.retain(|_, p| p.session != session);
        }
    }
    pub fn pending_count(&self) -> usize {
        self.state.lock().map(|s| s.pending.len()).unwrap_or(0)
    }
    pub(crate) async fn request(
        &self,
        name: &str,
        args: Map<String, Value>,
        scope: &Scope,
        ctx: &ToolContext,
    ) -> Result<ApprovalGrant> {
        self.request_with_timeout(name, args, scope, ctx, Duration::from_secs(120))
            .await
    }
    async fn request_with_timeout(
        &self,
        name: &str,
        args: Map<String, Value>,
        scope: &Scope,
        ctx: &ToolContext,
        timeout: Duration,
    ) -> Result<ApprovalGrant> {
        let scope = Scope {
            allow_session: scope.allow_session && !INDIVIDUAL.contains(&name),
            ..scope.clone()
        };
        let scope = &scope;
        if ctx.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let epoch = {
            let state = self.state.lock().map_err(|_| Error::Internal)?;
            if scope.allow_session
                && state
                    .grants
                    .get(&ctx.session_id)
                    .is_some_and(|s| s.contains(&scope.key))
            {
                return Ok(ApprovalGrant {
                    choice: "cached".into(),
                    scope: scope.label.clone(),
                    confirmation: None,
                    epoch: *state.epochs.get(&ctx.session_id).unwrap_or(&0),
                });
            }
            *state.epochs.get(&ctx.session_id).unwrap_or(&0)
        };
        let deadline = Instant::now() + timeout;
        let choice = self
            .wait(name, args.clone(), scope, ctx, None, deadline)
            .await?;
        let confirmation = if let Some(main) = &scope.main {
            if self
                .wait(name, args, scope, ctx, Some(main.clone()), deadline)
                .await?
                != ApprovalDecision::ConfirmMain
            {
                return Err(Error::Denied);
            }
            Some(main.clone())
        } else {
            None
        };
        if ctx.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut state = self.state.lock().map_err(|_| Error::Internal)?;
        if *state.epochs.get(&ctx.session_id).unwrap_or(&0) != epoch {
            return Err(Error::Denied);
        }
        if choice == ApprovalDecision::Session {
            let grants = state.grants.entry(ctx.session_id.clone()).or_default();
            if grants.len() >= 256 && !grants.contains(&scope.key) {
                return Err(Error::ScopeLimit);
            }
            grants.insert(scope.key.clone());
        }
        Ok(ApprovalGrant {
            choice: if choice == ApprovalDecision::Session {
                "session"
            } else {
                "once"
            }
            .into(),
            scope: scope.label.clone(),
            confirmation,
            epoch,
        })
    }
    async fn wait(
        &self,
        name: &str,
        args: Map<String, Value>,
        scope: &Scope,
        ctx: &ToolContext,
        confirmation: Option<String>,
        deadline: Instant,
    ) -> Result<ApprovalDecision> {
        let sink = ctx.events.as_ref().ok_or(Error::Denied)?;
        let id = identifier()?;
        let (reply, rx) = oneshot::channel();
        self.state
            .lock()
            .map_err(|_| Error::Internal)?
            .pending
            .insert(
                id.clone(),
                Pending {
                    session: ctx.session_id.clone(),
                    allow_session: scope.allow_session && confirmation.is_none(),
                    main: confirmation.is_some(),
                    reply,
                },
            );
        let _cleanup = PendingGuard {
            state: self.state.clone(),
            id: id.clone(),
        };
        let event = StreamEvent::ApprovalRequired {
            approval_id: id,
            tool_name: name.into(),
            args,
            confirmation: confirmation.clone(),
            scope: Some(scope.label.clone()),
            allow_session: Some(scope.allow_session && confirmation.is_none()),
            tool_call_id: Some(ctx.tool_call_id.clone()),
        };
        use myagent_protocol::Validate;
        event.validate().map_err(|_| Error::InputLimit)?;
        tokio::select! {biased;
            _=ctx.cancellation.cancelled()=>Err(Error::Cancelled),
            _=tokio::time::sleep_until(deadline)=>Err(Error::ApprovalExpired),
            value=async {
                sink.send(event).await.map_err(|_|Error::Denied)?;
                let decision=rx.await.map_err(|_|Error::Denied)?;
                if decision==ApprovalDecision::Deny{return Err(Error::Denied);}Ok(decision)
            }=>value
        }
    }
}
struct PendingGuard {
    state: Arc<Mutex<State>>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.pending.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use myagent_core::CancellationToken;
    use tokio::sync::mpsc;
    fn scope() -> Scope {
        Scope {
            key: "tool/root/exact-file".into(),
            label: "exact-file".into(),
            allow_session: true,
            main: None,
        }
    }
    #[tokio::test]
    async fn expired_and_cancelled_requests_are_consumed_and_forgotten() {
        let manager = ApprovalManager::default();
        let (tx, mut rx) = mpsc::channel(4);
        let ctx = ToolContext {
            session_id: "s".into(),
            cancellation: CancellationToken::new(),
            tool_call_id: "c".into(),
            events: Some(tx),
        };
        assert!(matches!(
            manager
                .request_with_timeout("edit_file", Map::new(), &scope(), &ctx, Duration::ZERO)
                .await,
            Err(Error::ApprovalExpired)
        ));
        assert_eq!(manager.pending_count(), 0);
        let test_scope = scope();
        let pending = manager.request_with_timeout(
            "edit_file",
            Map::new(),
            &test_scope,
            &ctx,
            Duration::from_secs(1),
        );
        tokio::pin!(pending);
        tokio::select! {_=&mut pending=>panic!("approval must wait"),_=tokio::time::sleep(Duration::from_millis(1))=>{}}
        ctx.cancellation.cancel();
        assert!(matches!(pending.await, Err(Error::Cancelled)));
        assert_eq!(manager.pending_count(), 0);
        while let Ok(event) = rx.try_recv() {
            if let StreamEvent::ApprovalRequired { approval_id, .. } = event {
                assert!(!manager.decide("s", &approval_id, ApprovalDecision::Once));
            }
        }
    }
    #[tokio::test]
    async fn session_grants_are_capped_and_individual_tools_cannot_remember() {
        let manager = ApprovalManager::default();
        let (tx, mut rx) = mpsc::channel(4);
        let ctx = ToolContext {
            session_id: "scopes".into(),
            cancellation: CancellationToken::new(),
            tool_call_id: "call".into(),
            events: Some(tx),
        };
        for n in 0..257 {
            let scope = Scope {
                key: format!("root/tool/file{n}"),
                label: format!("file{n}"),
                allow_session: true,
                main: None,
            };
            let (result, ()) = tokio::join!(
                manager.request("edit_file", Map::new(), &scope, &ctx),
                async {
                    let StreamEvent::ApprovalRequired { approval_id, .. } =
                        rx.recv().await.unwrap()
                    else {
                        panic!("approval expected")
                    };
                    assert!(manager.decide("scopes", &approval_id, ApprovalDecision::Session));
                }
            );
            if n < 256 {
                assert_eq!(result.unwrap().choice, "session");
            } else {
                assert!(matches!(result, Err(Error::ScopeLimit)));
            }
        }
        for name in INDIVIDUAL {
            let scope = scope();
            let (result, ()) =
                tokio::join!(manager.request(name, Map::new(), &scope, &ctx), async {
                    let StreamEvent::ApprovalRequired {
                        approval_id,
                        allow_session,
                        ..
                    } = rx.recv().await.unwrap()
                    else {
                        panic!("approval expected")
                    };
                    assert_eq!(allow_session, Some(false));
                    assert!(!manager.decide("scopes", &approval_id, ApprovalDecision::Session));
                    assert!(manager.decide("scopes", &approval_id, ApprovalDecision::Once));
                });
            result.unwrap();
        }
        for key in ['\n', '\r', '\t', ' '] {
            assert_eq!(ApprovalDecision::from_key(key), None);
        }
        assert_eq!(
            ApprovalDecision::from_key('Y'),
            Some(ApprovalDecision::Once)
        );
    }
    #[tokio::test]
    async fn forgetting_or_dropping_a_waiter_invalidates_pending_approvals() {
        let manager = ApprovalManager::default();
        let (tx, mut rx) = mpsc::channel(4);
        let ctx = ToolContext {
            session_id: "forget".into(),
            cancellation: CancellationToken::new(),
            tool_call_id: "call".into(),
            events: Some(tx),
        };
        let scope = scope();
        let mut pending = Box::pin(manager.request("edit_file", Map::new(), &scope, &ctx));
        let id = tokio::select! {_=pending.as_mut()=>panic!("must wait"),event=rx.recv()=>if let StreamEvent::ApprovalRequired{approval_id,..}=event.unwrap(){approval_id}else{panic!("approval expected")}};
        drop(pending);
        assert_eq!(manager.pending_count(), 0);
        assert!(!manager.decide("forget", &id, ApprovalDecision::Once));
        let (result, ()) = tokio::join!(
            manager.request("edit_file", Map::new(), &scope, &ctx),
            async {
                rx.recv().await.unwrap();
                manager.forget("forget");
            }
        );
        assert!(matches!(result, Err(Error::Denied)));
        assert_eq!(manager.pending_count(), 0);
    }
}
