use crate::{
    Access, CallCtx, Policy, ToolError, ToolSpec, dispatch::call_sync, result::bound_result_chars,
    schema::sanitize_schema,
};
use llm_wiki::{is_read_only_wiki_tool, tools::definitions::build_llm_tool_definitions};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiki_core::Vault;

pub struct ToolHost {
    vault: Arc<Vault>,
    policy: std::sync::RwLock<Policy>,
    pub(super) active_requests: Arc<AtomicUsize>,
    read_tools: Vec<ToolSpec>,
    write_tools: Vec<ToolSpec>,
}

impl ToolHost {
    pub fn new(vault: Arc<Vault>, policy: Policy) -> Self {
        let mut read_tools = Vec::new();
        let mut write_tools = Vec::new();
        for spec in build_llm_tool_definitions() {
            let mut parameters = spec.parameters;
            sanitize_schema(&mut parameters);
            let tool = ToolSpec {
                name: spec.name.clone(),
                description: spec.description,
                parameters,
            };
            if is_read_only_wiki_tool(&spec.name) {
                read_tools.push(tool.clone());
            }
            write_tools.push(tool);
        }
        Self {
            vault,
            policy: std::sync::RwLock::new(policy),
            active_requests: Arc::new(AtomicUsize::new(0)),
            read_tools,
            write_tools,
        }
    }

    pub fn update_policy(&self, policy: Policy) {
        *self.policy.write().expect("ToolHost policy lock") = policy;
    }

    pub fn active_requests(&self) -> usize {
        self.active_requests.load(Ordering::Acquire)
    }

    pub fn list_tools(&self, access: Access) -> Vec<ToolSpec> {
        let policy = *self.policy.read().expect("ToolHost policy lock");
        if access == Access::ReadWrite && policy.allow_write {
            self.write_tools.clone()
        } else {
            self.read_tools.clone()
        }
    }

    pub async fn call(&self, ctx: CallCtx, name: &str, args: Value) -> Result<Value, ToolError> {
        let policy = *self.policy.read().expect("ToolHost policy lock");
        let write_allowed = ctx.access == Access::ReadWrite && policy.allow_write;
        if !self
            .list_tools(ctx.access)
            .iter()
            .any(|tool| tool.name == name)
        {
            return Err(ToolError::Denied);
        }
        if !is_read_only_wiki_tool(name) && !write_allowed {
            return Err(ToolError::Denied);
        }
        if !args.is_object() {
            return Err(ToolError::InvalidArguments(
                "tool arguments must be a JSON object".into(),
            ));
        }
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        if tokio::time::Instant::now() >= ctx.deadline {
            return Err(ToolError::Timeout);
        }
        let max_concurrency = policy.max_concurrency.max(1);
        let mut active = self.active_requests.load(Ordering::Acquire);
        loop {
            if active >= max_concurrency {
                return Err(ToolError::Unavailable);
            }
            match self.active_requests.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => active = current,
            }
        }
        let _active = ActiveRequest(Arc::clone(&self.active_requests));
        let vault = Arc::clone(&self.vault);
        let name = name.to_owned();
        let max_chars = policy.max_result_chars;
        let deadline = ctx
            .deadline
            .min(tokio::time::Instant::now() + policy.call_timeout);
        if tokio::time::Instant::now() >= deadline {
            return Err(ToolError::Timeout);
        }
        let mut task =
            tokio::task::spawn_blocking(move || call_sync(&vault, &name, args, max_chars));
        let value = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                task.abort();
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => {
                task.abort();
                return Err(ToolError::Timeout);
            }
            result = &mut task => result.map_err(|_| ToolError::Unavailable)??,
        };
        Ok(bound_result_chars(value, max_chars))
    }
}

struct ActiveRequest(Arc<AtomicUsize>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
