use std::path::Path;

use ort::{
    session::{Session, SessionInputValue, SessionOutputs},
    value::{DynValue, Tensor, TensorElementType, ValueType},
};

use super::{TtsError, failure};

pub(super) fn load(
    path: &Path,
    threads: usize,
    layers: usize,
    acoustic: bool,
    prefill: bool,
) -> Result<Session, TtsError> {
    let session = Session::builder()
        .map_err(failure)?
        .with_intra_threads(threads)
        .map_err(failure)?
        .with_inter_threads(1)
        .map_err(failure)?
        .commit_from_file(path)
        .map_err(failure)?;
    let mut inputs = vec![if acoustic {
        "token_emb".into()
    } else {
        "inputs_embeds".into()
    }];
    if !prefill {
        inputs.push("position_ids".into());
    }
    let mut outputs = vec!["hidden".into()];
    for kind in ["k", "v"] {
        for layer in 0..layers {
            if !prefill {
                inputs.push(format!("past_{kind}_{layer}"));
            }
            outputs.push(format!("present_{kind}_{layer}"));
        }
    }
    if session
        .inputs()
        .iter()
        .map(|value| value.name())
        .ne(inputs.iter().map(String::as_str))
        || session
            .outputs()
            .iter()
            .map(|value| value.name())
            .ne(outputs.iter().map(String::as_str))
    {
        return Err(failure("ONNX graph I/O differs from the pinned model"));
    }
    for input in session.inputs() {
        let ValueType::Tensor { ty, shape, .. } = input.dtype() else {
            return Err(failure("ONNX graph input must be a tensor"));
        };
        let expected_type = if input.name() == "position_ids" {
            TensorElementType::Int64
        } else {
            TensorElementType::Float32
        };
        let expected_rank = if input.name() == "position_ids" {
            2
        } else if input.name().starts_with("past_") {
            4
        } else {
            3
        };
        if *ty != expected_type || shape.len() != expected_rank {
            return Err(failure(
                "ONNX input dtype or rank differs from the pinned model",
            ));
        }
    }
    Ok(session)
}

pub(super) fn tensor<
    T: ort::value::PrimitiveTensorElementType + Clone + std::fmt::Debug + 'static,
>(
    shape: Vec<usize>,
    data: Vec<T>,
) -> Result<Tensor<T>, TtsError> {
    Tensor::from_array((shape, data)).map_err(failure)
}

pub(super) fn hidden(output: &SessionOutputs<'_>, width: usize) -> Result<Vec<f32>, TtsError> {
    let value = output.get("hidden").ok_or(TtsError::Failed)?;
    let (shape, data) = value.try_extract_tensor::<f32>().map_err(failure)?;
    if shape.len() != 3
        || shape[0] != 1
        || shape[2] != width as i64
        || data.is_empty()
        || data.iter().any(|value| !value.is_finite())
    {
        return Err(failure("invalid ONNX hidden state"));
    }
    Ok(data.to_vec())
}

pub(super) fn take_cache(
    output: &mut SessionOutputs<'_>,
    layers: usize,
) -> Result<Vec<(String, DynValue)>, TtsError> {
    let mut cache = Vec::with_capacity(2 * layers);
    for kind in ["k", "v"] {
        for layer in 0..layers {
            let value = output
                .remove(format!("present_{kind}_{layer}"))
                .ok_or(TtsError::Failed)?;
            let (shape, data) = value.try_extract_tensor::<f32>().map_err(failure)?;
            if shape.len() != 4 || shape[0] != 1 || data.iter().any(|value| !value.is_finite()) {
                return Err(failure("invalid ONNX attention cache"));
            }
            cache.push((format!("past_{kind}_{layer}"), value));
        }
    }
    Ok(cache)
}

pub(super) fn with_cache<'a>(
    mut inputs: Vec<(String, SessionInputValue<'a>)>,
    cache: &'a [(String, DynValue)],
) -> Vec<(String, SessionInputValue<'a>)> {
    inputs.extend(
        cache
            .iter()
            .map(|(name, value)| (name.clone(), value.into())),
    );
    inputs
}
