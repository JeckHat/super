use rand::prelude::*;
use std::collections::HashMap;

/// Markov-2 model: mempelajari transisi (prev2, prev1) -> next
#[derive(Debug, Clone)]
pub struct Markov2 {
    pub counts: HashMap<(usize, usize), [f64; 25]>,
    pub alpha: f64, // laplace smoothing
}

impl Markov2 {
    pub fn new(alpha: f64) -> Self {
        Self {
            counts: HashMap::new(),
            alpha,
        }
    }

    pub fn train(&mut self, seq: &[usize]) {
        if seq.len() < 3 {
            return;
        }
        for w in seq.windows(3) {
            let a = w[0];
            let b = w[1];
            let c = w[2];
            if a < 25 && b < 25 && c < 25 {
                let entry = self.counts.entry((a, b)).or_insert([0.0; 25]);
                entry[c] += 1.0;
            }
        }
    }

    pub fn update(&mut self, prev2: usize, prev1: usize, next: usize) {
        if prev2 >= 25 || prev1 >= 25 || next >= 25 {
            return;
        }
        let entry = self.counts.entry((prev2, prev1)).or_insert([0.0; 25]);
        entry[next] += 1.0;
    }

    /// Prediksi berbobot probabilistik top-k
    pub fn predict(&self, prev2: usize, prev1: usize, k: usize, temperature: f64) -> Vec<usize> {
        let mut rng = thread_rng();
        let temp = if temperature > 0.0 { temperature } else { 1.0 };
        let mut probs = [self.alpha; 25];

        if let Some(arr) = self.counts.get(&(prev2, prev1)) {
            for i in 0..25 {
                probs[i] += arr[i];
            }
        }

        // normalisasi
        let sum: f64 = probs.iter().sum();
        if sum == 0.0 {
            // fallback acak
            let mut out: Vec<usize> = (0..25).collect();
            out.shuffle(&mut rng);
            return out.into_iter().take(k).collect();
        }

        // apply temperature
        let mut scored: Vec<(usize, f64)> = probs
            .iter()
            .enumerate()
            .map(|(i, &v)| (i, (v / sum).powf(1.0 / temp)))
            .collect();

        // normalisasi lagi
        let total: f64 = scored.iter().map(|(_, p)| *p).sum();
        for (_, p) in &mut scored {
            *p /= total;
        }

        // sort descending by probability
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // ambil top-n namun acak sedikit supaya variasi (tidak selalu 0..17)
        let top_candidates: Vec<usize> = scored.iter().take(25).map(|(i, _)| *i).collect();
        let mut selected = Vec::new();
        for _ in 0..k {
            let pick = top_candidates.choose(&mut rng).cloned().unwrap_or(0);
            if !selected.contains(&pick) {
                selected.push(pick);
            }
            if selected.len() >= k {
                break;
            }
        }

        selected
    }

    /// Marginal (fallback) kalau context tidak ada
    pub fn marginal_topk(&self, k: usize) -> Vec<usize> {
        let mut marginal = [self.alpha; 25];
        for arr in self.counts.values() {
            for i in 0..25 {
                marginal[i] += arr[i];
            }
        }
        let sum: f64 = marginal.iter().sum();
        if sum == 0.0 {
            let mut rng = thread_rng();
            let mut all: Vec<usize> = (0..25).collect();
            all.shuffle(&mut rng);
            return all.into_iter().take(k).collect();
        }
        let mut idx_prob: Vec<(usize, f64)> = marginal
            .iter()
            .enumerate()
            .map(|(i, &p)| (i, p / sum))
            .collect();
        idx_prob.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        idx_prob.iter().take(k).map(|(i, _)| *i).collect()
    }
}
