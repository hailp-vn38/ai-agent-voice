//! Shared plugin installer/runner. Deliberately no Docker control interface.
//! The server remains authoritative for admission, schemas, and job state.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, os::unix::{fs::PermissionsExt, process::CommandExt}, path::{Path, PathBuf}, process::Stdio, sync::Arc, time::{Duration, Instant}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{UnixListener, UnixStream}, process::Command, sync::{Mutex, Semaphore}};

const RPC_LIMIT: u64 = 80 * 1024;
const STDOUT_LIMIT: u64 = 64 * 1024;
const STDERR_LIMIT: u64 = 4 * 1024;
const RUNTIME_VERSION: &str = "shared-v1";

#[derive(Clone)]
struct Attempt {
    pgid: Option<i32>,
    started: Instant,
    result: Option<Value>,
}

#[derive(Clone)]
struct State {
    mode: String,
    attempts: Arc<Mutex<HashMap<String, Attempt>>>,
    slots: Arc<Semaphore>,
}

fn token(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_hexdigit())
}
fn required<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value.get(name).and_then(Value::as_str).context(format!("missing {name}"))
}
fn installation_id(digest: &str) -> String {
    let bytes = Sha256::digest(format!("{digest}:{RUNTIME_VERSION}").as_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn read_manifest(root: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&std::fs::read_to_string(root.join("plugin.toml"))?)?)
}
fn plugin_kind(manifest: &toml::Value) -> Result<&str> {
    manifest.get("runtime").and_then(|v| v.get("kind")).and_then(toml::Value::as_str).context("missing runtime.kind")
}
fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() { bail!("symlink not allowed"); }
        let target = dst.join(entry.file_name());
        if ft.is_dir() { copy_tree(&entry.path(), &target)?; }
        else if ft.is_file() { std::fs::copy(entry.path(), target)?; }
        else { bail!("special file not allowed"); }
    }
    Ok(())
}
async fn prepare(request: &Value) -> Result<Value> {
    let digest = required(request, "package_digest")?;
    if digest.len()!=64 || !token(digest) { bail!("invalid package digest"); }
    let stage_id = required(request, "stage_id")?;
    uuid::Uuid::parse_str(stage_id)?;
    let src = Path::new("/staging").join(stage_id);
    let target_id = installation_id(digest);
    let target = Path::new("/installations").join(&target_id);
    if target.exists() {
        let receipt = std::fs::read(target.join(".installation.json"));
        if let Ok(bytes) = receipt {
            let saved: Value=serde_json::from_slice(&bytes)?;
            if saved["installation_digest"] != target_id || saved["package_digest"] != digest {
                bail!("existing installation mismatch");
            }
            return Ok(json!({"installation_digest":target_id}));
        }
        // An interrupted Prepare never publishes an installation receipt.
        std::fs::remove_dir_all(&target)?;
    }
    let tmp = Path::new("/installations").join(format!(".prepare-{}",uuid::Uuid::new_v4()));
    copy_tree(&src, &tmp)?;
    let result = async {
        let manifest = read_manifest(&tmp)?;
        let kind = plugin_kind(&manifest)?;
        if kind == "rust" {
            let binary = tmp.join("bin/plugin");
            let bytes = std::fs::read(&binary)?;
            if bytes.len()<64 || !bytes.starts_with(b"\x7fELF") { bail!("invalid Rust executable"); }
            std::fs::set_permissions(binary, std::fs::Permissions::from_mode(0o500))?;
        } else if kind != "python" {
            bail!("unsupported runtime");
        }
        // The venv must be installed at its final absolute path: moving a
        // prepared venv breaks entry-point shebangs and editable paths.
        std::fs::rename(&tmp,&target)?;
        if kind == "python" {
            let output = tokio::time::timeout(Duration::from_secs(180), Command::new("uv")
                .args(["sync", "--frozen", "--no-dev", "--no-install-project"])
                .env("UV_PROJECT_ENVIRONMENT", target.join(".venv"))
                .env("UV_CACHE_DIR", "/tmp/uv-cache")
                .env("UV_PYTHON_DOWNLOADS", "never")
                .current_dir(&target).output()).await??;
            if !output.status.success() { bail!("locked Python dependency preparation failed"); }
        }
        let receipt = json!({"installation_digest":target_id,"package_digest":digest,"runtime":RUNTIME_VERSION});
        let pending=target.join(".installation.pending");
        std::fs::write(&pending,serde_json::to_vec(&receipt)?)?;
        std::fs::rename(pending,target.join(".installation.json"))?;
        Ok::<(),anyhow::Error>(())
    }.await;
    if result.is_err() {
        let _=std::fs::remove_dir_all(&tmp);
        let _=std::fs::remove_dir_all(&target);
    }
    result?;
    Ok(json!({"installation_digest":target_id}))
}

fn signal_group(pgid: i32) {
    if pgid>1 { unsafe { libc::kill(-pgid,libc::SIGKILL); } }
}
async fn execute(request: &Value, state: &State) -> Result<Value> {
    let attempt = required(request,"attempt_id")?.to_owned();
    let job = required(request,"job_id")?;
    uuid::Uuid::parse_str(&attempt)?;
    uuid::Uuid::parse_str(job)?;
    let digest = required(request,"installation_digest")?;
    if digest.len()!=64 || !token(digest) { bail!("invalid installation"); }
    let payload = required(request,"jsonl_request")?;
    if payload.len()>65536 || !payload.ends_with('\n') || payload.as_bytes().iter().filter(|&&b|b==b'\n').count()!=1 {
        bail!("invalid JSONL request");
    }
    let mut attempts=state.attempts.lock().await;
    if attempts.contains_key(&attempt) { bail!("attempt already exists"); }
    if attempts.len()>2048 {
        attempts.retain(|_,a| a.result.is_none() || a.started.elapsed()<Duration::from_secs(3600));
        if attempts.len()>2048 { bail!("attempt capacity reached"); }
    }
    let slot=state.slots.clone().try_acquire_owned().context("runner at capacity")?;
    let root=Path::new("/installations").join(digest);
    let receipt: Value=serde_json::from_slice(&std::fs::read(root.join(".installation.json"))?)?;
    if receipt["installation_digest"]!=digest { bail!("installation mismatch"); }
    let manifest=read_manifest(&root)?;
    let kind=plugin_kind(&manifest)?;
    let workspace=Path::new("/attempts").join(job).join(&attempt).join("out");
    std::fs::create_dir_all(&workspace)?;
    let (program,args): (PathBuf,Vec<String>)=match kind {
        "rust" => (root.join("bin/plugin"),vec![]),
        "python" => (root.join(".venv/bin/python"),vec![root.join("src/main.py").to_string_lossy().into_owned()]),
        _ => bail!("unsupported runtime")
    };
    let mut envelope:Value=serde_json::from_str(payload.trim_end())?;
    envelope["artifact_dir"]=json!(workspace.to_string_lossy().as_ref());
    let input=format!("{}\n",serde_json::to_string(&envelope)?);
    let mut command=Command::new(program);
    command.args(args).current_dir(&root)
        .env_clear().env("PATH","/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .kill_on_drop(true);
    // A unique session/process group lets Cancel terminate descendants as well.
    unsafe { command.as_std_mut().pre_exec(|| {
        if libc::setsid()<0 {return Err(std::io::Error::last_os_error());}
        let limit=libc::rlimit{rlim_cur:20*1024*1024,rlim_max:20*1024*1024};
        if libc::setrlimit(libc::RLIMIT_FSIZE,&limit)<0 {return Err(std::io::Error::last_os_error());}
        Ok(())
    }); }
    let mut child=command.spawn()?;
    let pgid=child.id().context("missing process ID")? as i32;
    let mut stdin=child.stdin.take().context("missing stdin")?;
    stdin.write_all(input.as_bytes()).await?;
    drop(stdin);
    let stdout=child.stdout.take().context("missing stdout")?;
    let stderr=child.stderr.take().context("missing stderr")?;
    attempts.insert(attempt.clone(),Attempt{pgid:Some(pgid),started:Instant::now(),result:None});
    drop(attempts);
    let state=state.clone();
    tokio::spawn(async move {
        let _slot=slot;
        let output = async {
            let mut out=Vec::new();let mut err=Vec::new();
            let mut stdout=stdout.take(STDOUT_LIMIT+1);
            let mut stderr=stderr.take(STDERR_LIMIT+1);
            let (a,b,c)=tokio::join!(stdout.read_to_end(&mut out),stderr.read_to_end(&mut err),child.wait());
            a?;b?;
            let exit=c?;
            if out.len() as u64>STDOUT_LIMIT || err.len() as u64>STDERR_LIMIT { bail!("output limit exceeded"); }
            Ok::<Value,anyhow::Error>(json!({"state":"terminal","ok":exit.success(),"stdout":String::from_utf8(out)?,"stderr":String::from_utf8_lossy(&err),"exit_code":exit.code()}))
        }.await;
        // Even a successful plugin can leave descendants behind; an Attempt
        // owns its whole process group, not just its original parent.
        signal_group(pgid);
        let result=output.unwrap_or_else(|e| json!({"state":"terminal","ok":false,"error":e.to_string()}));
        if let Some(entry)=state.attempts.lock().await.get_mut(&attempt){
            entry.pgid=None;entry.result=Some(result);
        }
    });
    Ok(json!({"state":"running"}))
}
async fn dispatch(value:Value,state:State)->Result<Value>{
    match (state.mode.as_str(),required(&value,"action")?) {
        ("installer","Prepare")=>{
            let _permit=state.slots.acquire().await.context("installer unavailable")?;
            prepare(&value).await
        },
        ("runner","Execute")=>execute(&value,&state).await,
        ("runner","Cancel")=>{
            let id=required(&value,"attempt_id")?;
            let attempts=state.attempts.lock().await;
            match attempts.get(id) {
                Some(attempt)=>{if let Some(pgid)=attempt.pgid {signal_group(pgid);}
                    Ok(json!({"state":if attempt.result.is_some(){"terminal"}else{"cancelling"}}))}
                None=>Ok(json!({"state":"not_found"}))
            }
        },
        ("runner","GetAttempt")=>{
            let id=required(&value,"attempt_id")?;
            let attempts=state.attempts.lock().await;
            Ok(match attempts.get(id) {
                None=>json!({"state":"not_found"}),
                Some(a)=>a.result.clone().unwrap_or_else(||json!({"state":"running"}))
            })
        },
        _=>bail!("unsupported action")
    }
}
async fn connection(mut stream:UnixStream,state:State)->Result<()> {
    let mut bytes=Vec::new();
    (&mut stream).take(RPC_LIMIT+1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64>RPC_LIMIT { bail!("oversized RPC"); }
    let value:Value=serde_json::from_slice(&bytes)?;
    let answer=match dispatch(value,state).await {
        Ok(v)=>json!({"ok":true,"value":v}),
        Err(err)=>json!({"ok":false,"error":err.to_string()}),
    };
    let mut payload=serde_json::to_vec(&answer)?;
    payload.push(b'\n');
    stream.write_all(&payload).await?;
    Ok(())
}
#[tokio::main]
async fn main()->Result<()> {
    let mode=std::env::args().nth(1).context("usage: plugin-runtime installer|runner")?;
    if mode!="installer" && mode!="runner" { bail!("invalid mode"); }
    if mode=="installer" {
        // A crash during preparation may leave incomplete hidden directories.
        for entry in std::fs::read_dir("/installations")? {
            let entry=entry?;
            if entry.file_name().to_string_lossy().starts_with(".prepare-")
                && entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
    }
    let path=format!("/control/{mode}.sock");
    std::fs::create_dir_all("/control")?;
    if Path::new(&path).exists() {std::fs::remove_file(&path)?;}
    let listener=UnixListener::bind(&path)?;
    std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o660))?;
    let state=State{mode,attempts:Arc::new(Mutex::new(HashMap::new())),slots:Arc::new(Semaphore::new(if mode=="installer"{1}else{4}))};
    loop {
        let (stream,_)=listener.accept().await?;
        let shared=state.clone();
        tokio::spawn(async move {let _=connection(stream,shared).await;});
    }
}
