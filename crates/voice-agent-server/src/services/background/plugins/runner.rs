//! Job attempts run as supervised subprocesses in the shared runner container.
use super::{PluginError,builder,protocol::MAX_STDOUT};
use crate::config::BackgroundConfig;
use serde_json::json;
use std::{path::Path,time::Duration};
use tokio::time::{sleep,timeout};

pub(super) struct Prepared {
    installation: String,
    job: String,
    attempt: String,
}

impl Prepared {
    pub(super) async fn create(
        config:&BackgroundConfig,
        installation:&str,
        _workspace:&Path,
        job:&str,
        attempt:&str,
    )->Result<Self,PluginError>{
        builder::runner_preflight(config).await?;
        uuid::Uuid::parse_str(job).map_err(|_|PluginError::InvalidPackage)?;
        uuid::Uuid::parse_str(attempt).map_err(|_|PluginError::InvalidPackage)?;
        Ok(Self{installation:installation.to_owned(),job:job.to_owned(),attempt:attempt.to_owned()})
    }
    async fn call(&self,config:&BackgroundConfig,value:serde_json::Value)->Result<serde_json::Value,PluginError>{
        let socket=config.plugin_runner_socket.as_deref().ok_or(PluginError::SandboxUnavailable)?;
        builder::rpc(socket,value).await
    }
    pub(super) async fn execute<F,Fut>(
        &self,config:&BackgroundConfig,request:&[u8],limit:Duration,after_start:F
    )->Result<Vec<u8>,PluginError>
    where F:FnOnce()->Fut,Fut:std::future::Future<Output=Result<(),PluginError>>
    {
        let text=std::str::from_utf8(request).map_err(|_|PluginError::WorkerProtocolError)?;
        let result=self.call(config,json!({
            "action":"Execute","attempt_id":self.attempt,"job_id":self.job,
            "installation_digest":self.installation,"jsonl_request":text
        })).await?;
        if result["state"]!="running" {return Err(PluginError::PluginRuntimeError);}
        after_start().await?;
        let output=timeout(limit,async {
            loop {
                let response=self.call(config,json!({"action":"GetAttempt","attempt_id":self.attempt})).await?;
                match response["state"].as_str(){
                    Some("running")|Some("cancelling")=>sleep(Duration::from_millis(100)).await,
                    Some("terminal")=>{
                        if response["ok"]!=true {return Err(PluginError::PluginRuntimeError);}
                        let bytes=response["stdout"].as_str().ok_or(PluginError::WorkerProtocolError)?.as_bytes().to_vec();
                        if bytes.len()>MAX_STDOUT {return Err(PluginError::WorkerProtocolError);}
                        return Ok(bytes);
                    }
                    _=>return Err(PluginError::WorkerUnreaped)
                }
            }
        }).await;
        output.map_err(|_|PluginError::WorkerTimeout)?
    }
    pub(super) async fn remove(&self,config:&BackgroundConfig)->Result<(),PluginError>{
        let result=self.call(config,json!({"action":"Cancel","attempt_id":self.attempt})).await?;
        match result["state"].as_str(){
            Some("not_found")|Some("terminal")=>return Ok(()),
            Some("cancelling")=>{},
            _=>return Err(PluginError::WorkerUnreaped)
        }
        timeout(Duration::from_secs(10),async {
            loop{
                let status=self.call(config,json!({"action":"GetAttempt","attempt_id":self.attempt})).await?;
                if status["state"]=="terminal" || status["state"]=="not_found"{return Ok(());}
                sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_|PluginError::WorkerUnreaped)?
    }
}
