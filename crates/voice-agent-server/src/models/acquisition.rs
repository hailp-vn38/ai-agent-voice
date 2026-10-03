//! Bounded HTTP acquisition. Model identity and checksum verification belong to Model Preparation.

use super::{ModelAcquirer, ModelError};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

const ATTEMPTS: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(900);

pub struct HttpModelAcquirer;

struct AttemptError {
    error: ModelError,
    retryable: bool,
}

impl ModelAcquirer for HttpModelAcquirer {
    fn acquire(&self, remote: &str, destination: &Path) -> Result<(), ModelError> {
        let url = reqwest::Url::parse(remote)
            .map_err(|_| ModelError::Acquire("invalid artifact URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ModelError::Acquire("artifact source must use HTTP(S)".into()));
        }
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .map_err(|error| ModelError::Acquire(error.without_url().to_string()))?;
        for attempt in 1..=ATTEMPTS {
            match acquire_once(&client, &url, destination) {
                Ok(()) => return Ok(()),
                Err(failure) => {
                    let _ = fs::remove_file(destination);
                    if !failure.retryable || attempt == ATTEMPTS {
                        return Err(failure.error);
                    }
                    tracing::warn!(attempt, max_attempts = ATTEMPTS, "model artifact download interrupted; retrying");
                    std::thread::sleep(Duration::from_secs(u64::from(attempt)));
                }
            }
        }
        unreachable!("the final attempt always returns")
    }
}

fn acquire_once(
    client: &reqwest::blocking::Client,
    url: &reqwest::Url,
    destination: &Path,
) -> Result<(), AttemptError> {
    let mut response = client.get(url.clone()).send().map_err(|error| AttemptError {
        retryable: error.is_timeout() || error.is_connect() || error.is_body(),
        error: ModelError::Acquire(error.without_url().to_string()),
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AttemptError {
            retryable: status.is_server_error() || matches!(status.as_u16(), 408 | 429),
            error: ModelError::Acquire(format!("artifact HTTP status {}", status.as_u16())),
        });
    }
    let expected_bytes = response.content_length();
    let mut file = File::create(destination).map_err(file_error)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut downloaded_bytes = 0_u64;
    let mut progress = Instant::now();
    loop {
        let count = response.read(&mut buffer).map_err(|error| AttemptError {
            error: ModelError::Acquire(error.to_string()),
            retryable: true,
        })?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count]).map_err(file_error)?;
        downloaded_bytes += count as u64;
        if progress.elapsed() >= Duration::from_secs(5) {
            tracing::info!(downloaded_bytes, expected_bytes, "model artifact download progress");
            progress = Instant::now();
        }
    }
    if expected_bytes.is_some_and(|expected| expected != downloaded_bytes) {
        return Err(AttemptError {
            error: ModelError::Acquire("incomplete artifact HTTP response".into()),
            retryable: true,
        });
    }
    file.sync_all().map_err(file_error)?;
    tracing::info!(downloaded_bytes, "model artifact download completed");
    Ok(())
}

fn file_error(error: std::io::Error) -> AttemptError {
    AttemptError {
        error: ModelError::Read(error),
        retryable: false,
    }
}
