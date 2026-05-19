use crate::{Error, Result};

pub(crate) fn mean_pool_and_normalize(hidden_states: Vec<Vec<Vec<f32>>>, masks: &[Vec<u32>]) -> Result<Vec<Vec<f32>>> {
    if hidden_states.len() != masks.len() {
        return Err(Error::InvalidVector {
            reason: "batch and attention-mask length differ".to_owned(),
        });
    }

    let mut vectors = Vec::with_capacity(hidden_states.len());
    for (sequence, mask) in hidden_states.into_iter().zip(masks) {
        if sequence.len() != mask.len() {
            return Err(Error::InvalidVector {
                reason: "sequence and attention-mask length differ".to_owned(),
            });
        }
        let first_token = sequence.first().ok_or_else(|| Error::InvalidVector {
            reason: "sequence has no tokens".to_owned(),
        })?;
        let dimension = first_token.len();
        if dimension == 0 {
            return Err(Error::InvalidVector {
                reason: "hidden dimension is zero".to_owned(),
            });
        }

        let mut pooled = vec![0.0_f32; dimension];
        let mut token_count = 0.0_f32;
        for (token, attention) in sequence.iter().zip(mask) {
            if token.len() != dimension {
                return Err(Error::InvalidVector {
                    reason: "hidden dimensions are inconsistent".to_owned(),
                });
            }
            if *attention == 0 {
                continue;
            }
            token_count += 1.0;
            for (slot, value) in pooled.iter_mut().zip(token) {
                *slot += *value;
            }
        }
        if token_count == 0.0 {
            return Err(Error::InvalidVector {
                reason: "attention mask selects no tokens".to_owned(),
            });
        }
        for value in &mut pooled {
            *value /= token_count;
        }
        vectors.push(normalize(pooled)?);
    }
    Ok(vectors)
}

pub(crate) fn normalize(mut vector: Vec<f32>) -> Result<Vec<f32>> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(Error::InvalidVector {
            reason: "zero or non-finite norm".to_owned(),
        });
    }
    for value in &mut vector {
        *value /= norm;
    }
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pooling_ignores_padding_and_normalizes() -> Result<()> {
        let vectors = mean_pool_and_normalize(
            vec![vec![vec![3.0, 0.0], vec![0.0, 4.0], vec![100.0, 100.0]]],
            &[vec![1, 1, 0]],
        )?;
        let vector = vectors.first().ok_or_else(|| Error::InvalidVector {
            reason: "missing vector".to_owned(),
        })?;
        assert!((vector.iter().map(|value| value * value).sum::<f32>() - 1.0).abs() < 1e-5);
        assert!((vector.first().copied().unwrap_or_default() - 0.6).abs() < 1e-5);
        assert!((vector.get(1).copied().unwrap_or_default() - 0.8).abs() < 1e-5);
        Ok(())
    }

    #[test]
    fn all_padding_is_rejected() {
        assert!(matches!(
            mean_pool_and_normalize(vec![vec![vec![1.0, 2.0]]], &[vec![0]]),
            Err(Error::InvalidVector { .. })
        ));
    }
}
