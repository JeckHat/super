use rand::prelude::*;
use std::collections::HashMap;
use rand::distributions::WeightedIndex;

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

    pub fn apply_decay(&mut self, factor: f64) {
        // decay each count
        let mut to_remove = Vec::new();
        for (k, arr) in self.counts.iter_mut() {
            let mut nonzero = false;
            for x in arr.iter_mut() {
                *x *= factor;
                if *x >= 1e-6 { nonzero = true; } // threshold
            }
            if !nonzero {
                to_remove.push(*k);
            }
        }
        // prune empty contexts
        for k in to_remove {
            self.counts.remove(&k);
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
        // temperature param sanity
        let temp = if temperature > 0.0 { temperature } else { 1.0 };
    
        // build base probs (alpha + counts)
        let mut base = [self.alpha; 25];
        if let Some(arr) = self.counts.get(&(prev2, prev1)) {
            for i in 0..25 {
                base[i] += arr[i];
            }
        } else {
            // if no context, fallback to marginal sampling (use marginal_topk below)
            return self.marginal_topk(k);
        }
    
        // convert to probabilities and apply temperature: p_i = (base_i / sum)^(1/temp)
        let sum: f64 = base.iter().sum();
        if sum <= 0.0 {
            return self.marginal_topk(k);
        }
        let mut scored: Vec<(usize, f64)> = base
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let p = (v / sum).max(0.0);
                (i, p.powf(1.0 / temp))
            })
            .collect();
    
        // normalize again
        let s: f64 = scored.iter().map(|(_, p)| *p).sum();
        if s <= 0.0 {
            return self.marginal_topk(k);
        }
        for (_i, p) in scored.iter_mut() {
            *p /= s;
        }
    
        // sample k unique indices weighted by scored probabilities
        sample_k_weighted_no_replace(&scored, k)
    }

    /// Marginal (fallback) kalau context tidak ada
    pub fn marginal_topk(&self, k: usize) -> Vec<usize> {
        // build marginal
        let mut marginal = [self.alpha; 25];
        for arr in self.counts.values() {
            for i in 0..25 {
                marginal[i] += arr[i];
            }
        }
        let sum: f64 = marginal.iter().sum();
        if sum <= 0.0 {
            let mut rng = thread_rng();
            let mut all: Vec<usize> = (0..25).collect();
            all.shuffle(&mut rng);
            return all.into_iter().take(k).collect();
        }
        // apply temperature 1.0 (can be parameterized)
        let mut scored: Vec<(usize, f64)> = marginal
            .iter()
            .enumerate()
            .map(|(i, &v)| (i, v / sum))
            .collect();
    
        sample_k_weighted_no_replace(&scored, k)
    }
}

pub fn sample_k_weighted_no_replace(probs: &[(usize, f64)], k: usize) -> Vec<usize> {
    let mut rng = thread_rng();
    // copy into vec for mutation
    let mut items = probs.to_vec(); // Vec<(idx, prob)>
    let mut out = Vec::with_capacity(k);

    // if all probs zero -> uniform sample
    if items.iter().all(|(_, p)| *p <= 0.0) {
        let mut all: Vec<usize> = items.iter().map(|(i, _)| *i).collect();
        all.shuffle(&mut rng);
        all.truncate(k);
        return all;
    }

    // iterative WeightedIndex; remove chosen each round
    for _ in 0..k {
        if items.is_empty() { break; }
        let weights: Vec<f64> = items.iter().map(|(_, p)| *p).collect();
        // safe WeightedIndex creation (non-zero check done)
        let dist = WeightedIndex::new(weights).unwrap();
        let pick_idx = dist.sample(&mut rng);
        let (idx, _) = items.remove(pick_idx);
        out.push(idx);
    }
    out
}