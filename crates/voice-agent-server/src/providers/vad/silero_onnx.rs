use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use ort::{session::Session, value::Tensor};

use crate::providers::{VadError, VadInput, VadProbability, VadProvider, VadSession};

pub(crate) struct UnavailableVad;

impl VadProvider for UnavailableVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Err(VadError::Failed("VAD provider is not initialized".into()))
    }
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

/// Direct ONNX Runtime binding for Silero v5. Endpoint policy remains outside this adapter.
pub(crate) struct LoadedSileroVad {
    model: PathBuf,
    num_threads: i32,
}

impl LoadedSileroVad {
    pub(crate) fn load(
        model: String,
        runtime_library: PathBuf,
        num_threads: i32,
    ) -> Result<Self, VadError> {
        let model = PathBuf::from(model);
        if !model.is_file() {
            return Err(VadError::Failed(format!(
                "Silero model is missing: {}",
                model.display()
            )));
        }
        initialize_ort(&runtime_library)?;
        // Validate the graph during startup, before the server can bind its socket.
        drop(build_session(&model, num_threads)?);
        Ok(Self { model, num_threads })
    }
}

impl VadProvider for LoadedSileroVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SileroVadSession {
            session: build_session(&self.model, self.num_threads)?,
            state: vec![0.0; 2 * 128],
            context: vec![0.0; 64],
        }))
    }
    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }
}

struct SileroVadSession {
    session: Session,
    state: Vec<f32>,
    context: Vec<f32>,
}

impl VadSession for SileroVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        if input.pcm.len() != 512 {
            return Err(VadError::Failed(
                "Silero requires exactly 512 samples".into(),
            ));
        }
        let model_input = with_context(&self.context, &input.pcm);
        let outputs = self.session.run(ort::inputs! {
            "input" => Tensor::<f32>::from_array(([1usize, 576], model_input)).map_err(ort_error)?,
            "state" => Tensor::<f32>::from_array(([2usize, 1, 128], self.state.clone())).map_err(ort_error)?,
            "sr" => Tensor::<i64>::from_array(([1usize], vec![16_000i64])).map_err(ort_error)?,
        }).map_err(ort_error)?;
        let probability = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(ort_error)?
            .1[0];
        self.state = outputs["stateN"]
            .try_extract_tensor::<f32>()
            .map_err(ort_error)?
            .1
            .to_vec();
        self.context.copy_from_slice(&input.pcm[448..]);
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        self.state.fill(0.0);
        self.context.fill(0.0);
        Ok(())
    }
}

fn build_session(model: &Path, num_threads: i32) -> Result<Session, VadError> {
    Session::builder()
        .map_err(ort_error)?
        .with_intra_threads(
            num_threads
                .try_into()
                .map_err(|_| VadError::Failed("Silero num_threads must be positive".into()))?,
        )
        .map_err(ort_error)?
        .commit_from_file(model)
        .map_err(ort_error)
}

pub(crate) fn initialize_ort(configured_library: &Path) -> Result<(), VadError> {
    static ORT_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    ORT_INIT
        .get_or_init(|| {
            // Operators may override the deployment path, but startup never searches the system.
            let path = std::env::var_os("VOICE_ONNX_RUNTIME_LIB")
                .map(PathBuf::from)
                .unwrap_or_else(|| configured_library.to_path_buf());
            if !path.is_file() {
                return Err(format!(
                    "ONNX Runtime dynamic library is missing: {}",
                    path.display()
                ));
            }
            ort::init_from(path)
                .map_err(|error| error.to_string())?
                .commit()
                .then_some(())
                .ok_or_else(|| {
                    "ONNX Runtime was initialized before the configured library could be applied"
                        .to_owned()
                })
        })
        .as_ref()
        .map_err(|error| VadError::Failed(error.clone()))
        .copied()
}

/// Loads the deployment-selected ONNX Runtime before an offline qualification gate proceeds.
pub fn verify_onnx_runtime(configured_library: &Path) -> Result<(), VadError> {
    initialize_ort(configured_library)
}

fn ort_error(error: impl std::fmt::Display) -> VadError {
    VadError::Failed(format!("Silero ONNX Runtime failure: {error}"))
}

fn with_context(context: &[f32], current: &[f32]) -> Vec<f32> {
    debug_assert_eq!(context.len(), 64);
    debug_assert_eq!(current.len(), 512);
    let mut model_input = Vec::with_capacity(context.len() + current.len());
    model_input.extend_from_slice(context);
    model_input.extend_from_slice(current);
    model_input
}

#[cfg(test)]
mod tests {
    use super::with_context;

    #[test]
    fn inference_input_prepends_the_previous_64_samples() {
        let context = vec![1.0; 64];
        let current = vec![2.0; 512];
        let input = with_context(&context, &current);

        assert_eq!(input.len(), 576);
        assert_eq!(&input[..64], context.as_slice());
        assert_eq!(&input[64..], current.as_slice());
    }
}
