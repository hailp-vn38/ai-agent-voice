use super::TtsError;

pub(super) fn load_ar_graphs<T: Send>(
    load_text_and_prefix: impl FnOnce() -> Result<(T, T), TtsError> + Send,
    load_local: impl FnOnce() -> Result<T, TtsError>,
) -> Result<(T, T, T), TtsError> {
    std::thread::scope(|scope| {
        let text_and_prefix = std::thread::Builder::new()
            .name("zerotts-ar-load".into())
            .spawn_scoped(scope, load_text_and_prefix)
            .map_err(|_| TtsError::Failed)?;
        let local = load_local();
        // Join before returning any error so native loads cannot outlive attempt accounting.
        let (text, prefix) = text_and_prefix.join().map_err(|_| TtsError::Failed)??;
        Ok((text, prefix, local?))
    })
}

#[cfg(test)]
mod tests;
