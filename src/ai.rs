
use rand::prelude::*;
use rand::distributions::WeightedIndex;
use std::{cmp::Ordering, collections::HashSet};

use crate::{ev::compute_ev_star_for_block, ore_env::OreEnv};

pub struct HybridPredictor {
    counts: Vec<f64>,      // frekuensi dasar
    total_rounds: usize,   // total ronde
    correct_hybrid: usize, // akurasi hybrid
}

const GA_POP: usize = 120;
const GA_GENS: usize = 60;
const GA_MUT_PROB: f64 = 0.12;
const SUBSET_K: usize = 18;

fn cluster_score(subset: &Vec<usize>) -> f64 {
    let set: std::collections::HashSet<usize> = subset.iter().copied().collect();
    let mut score = 0usize;
    for &x in subset.iter() {
        if x > 0 && set.contains(&(x - 1)) { score += 1; }
        if x + 1 < 25 && set.contains(&(x + 1)) { score += 1; }
    }
    // normalisasi ke [0,1]
    (score as f64) / ((subset.len() * 2) as f64)
}

fn avg_freq_score(subset: &Vec<usize>, freq: &Vec<f64>) -> f64 {
    subset.iter().map(|&i| freq[i]).sum::<f64>() / (subset.len() as f64)
}

fn fitness_of(subset: &Vec<usize>, freq: &Vec<f64>, w_freq: f64, w_cluster: f64) -> f64 {
    let f = avg_freq_score(subset, freq);
    let c = cluster_score(subset);
    w_freq * f + w_cluster * c
}

fn crossover(a: &Vec<usize>, b: &Vec<usize>, rng: &mut impl Rng) -> Vec<usize> {
    // union then randomly pick SUBSET_K unique items
    let mut union: Vec<usize> = a.iter().chain(b.iter()).copied().collect();
    union.sort_unstable();
    union.dedup();
    // if union < K, fill from remaining
    if union.len() < SUBSET_K {
        let mut remaining: Vec<usize> = (0..25).filter(|x| !union.contains(x)).collect();
        remaining.shuffle(rng);
        union.extend(remaining.into_iter().take(SUBSET_K - union.len()));
    }
    union.shuffle(rng);
    union.truncate(SUBSET_K);
    union.sort_unstable();
    union
}

fn tournament_select(scored: &[(f64, Vec<usize>)], rng: &mut impl Rng) -> Vec<usize> {
    let n = scored.len();
    if n == 0 {
        return Vec::new();
    }
    let a = rng.gen_range(0..n);
    let b = rng.gen_range(0..n);
    let c = rng.gen_range(0..n);

    // pilih index terbaik dari tiga
    let best_idx = [a, b, c]
        .iter()
        .copied()
        .max_by(|&i, &j| {
            // compare fitness safely
            scored[i]
                .0
                .partial_cmp(&scored[j].0)
                .unwrap_or(Ordering::Equal)
        })
        .unwrap();

    scored[best_idx].1.clone()
}

fn mutate(sub: &mut Vec<usize>, rng: &mut impl Rng) {
    // replace 1..2 items at random with numbers not in subset
    let replace_n = if rng.gen_bool(0.5) { 1 } else { 2 };
    let mut pool: Vec<usize> = (0..25).filter(|x| !sub.contains(x)).collect();
    pool.shuffle(rng);
    for i in 0..replace_n {
        if pool.is_empty() { break; }
        let replace_idx = rng.gen_range(0..sub.len());
        let new_val = pool.pop().unwrap();
        sub[replace_idx] = new_val;
    }
    sub.sort_unstable();
}

fn random_subset(rng: &mut impl Rng) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..25).collect();
    pool.shuffle(rng);
    pool.truncate(SUBSET_K);
    pool.sort_unstable();
    pool
}

impl HybridPredictor {
    /// Membuat predictor baru
    pub fn new() -> Self {
        Self {
            counts: vec![1.0; 25], // Laplace smoothing awal
            total_rounds: 0,
            correct_hybrid: 0,
        }
    }

    /// Update setelah menerima angka nyata
    pub fn update(&mut self, actual: usize, predicted: &[usize]) {
        self.total_rounds += 1;
        if predicted.contains(&actual) {
            self.correct_hybrid += 1;
        }
        // Update frekuensi dasar
        self.counts[actual] += 1.0;
    }

    /// Hitung probabilitas dengan jitter acak terkontrol ±20%
    pub fn probabilities(&self) -> Vec<f64> {
        let mut rng = thread_rng();
        let mut probs: Vec<f64> = self.counts.iter().copied().collect();
        let total: f64 = probs.iter().sum();
        for p in probs.iter_mut() {
            let jitter: f64 = rng.gen_range(-0.20..0.20);
            *p = *p / total * (1.0 + jitter);
            if *p < 0.0 {
                *p = 0.0;
            }
        }
        let norm: f64 = probs.iter().sum();
        probs.iter_mut().for_each(|p| *p /= norm);
        probs
    }

    /// Hasilkan prediksi hybrid adaptif (18 angka utama + 2 booster)
    pub fn predict(&self) -> Vec<usize> {
        let probs = self.probabilities(); // Vec<f64>
    
        // create Vec<(index, value)> with owned f64
        let mut indexed: Vec<(usize, f64)> = probs.iter().cloned().enumerate().collect();
    
        // sort descending safely for f64
        indexed.sort_by(|a, b| b.1.total_cmp(&a.1));
    
        // take top 18 indexes
        let mut top: Vec<usize> = indexed.iter().take(18).map(|(i, _)| *i).collect();
    
        // booster: ambil 2 angka acak dari tail (angka "dingin")
        let mut rng = thread_rng();
        let low_pool: Vec<usize> = indexed.iter().rev().take(7).map(|(i, _)| *i).collect();
        let mut booster = Vec::new();
        while booster.len() < 2 && !low_pool.is_empty() {
            let j = rng.gen_range(0..low_pool.len());
            let candidate = low_pool[j];
            if !top.contains(&candidate) {
                booster.push(candidate);
            }
        }
    
        top.extend(booster);
        top.sort_unstable();
        top
    }

    /// Dapatkan akurasi hybrid
    pub fn accuracy(&self) -> f64 {
        if self.total_rounds == 0 {
            return 0.0;
        }
        (self.correct_hybrid as f64 / self.total_rounds as f64) * 100.0
    }

    pub fn predict_hybrid(&self) -> Vec<usize> {
        // param: batas ambil dari masing2 sumber
        const N_FREQ: usize = 12;
        const N_EC: usize = 6;
        const K_MAIN: usize = 18; // jumlah utama
        const N_BOOSTER: usize = 2; // optional booster tambahan
    
        // 1) Kalkulasi ranking frekuensi (owned f64)
        let probs = self.probabilities();
        let mut freq_indexed: Vec<(usize, f64)> = probs.iter().cloned().enumerate().collect();
        freq_indexed.sort_by(|a, b| b.1.total_cmp(&a.1));
    
        // 2) Ambil top N_FREQ from frequency
        let freq_top: Vec<usize> = freq_indexed.iter().take(N_FREQ).map(|(i, _)| *i).collect();
    
        // 3) Ambil dari EC (GA)
        let ec_set = self.predict_ec(); // pastikan predict_ec mengembalikan subset unik
        let ec_top: Vec<usize> = ec_set.iter().take(N_EC).copied().collect();
    
        // 4) Gabungkan ke HashSet untuk menjamin unik
        let mut set: HashSet<usize> = HashSet::new();
        for v in freq_top.iter().chain(ec_top.iter()) {
            set.insert(*v);
        }
    
        // 5) Jika kurang dari K_MAIN, isi dari urutan frekuensi (prioritas)
        for (i, _) in freq_indexed.iter() {
            if set.len() >= K_MAIN { break; }
            set.insert(*i);
        }
    
        // buat vector dari set, tapi kita ingin mempertahankan urutan frekuensi sebaik mungkin:
        let mut out: Vec<usize> = freq_indexed
            .iter()
            .map(|(i, _)| *i)
            .filter(|i| set.contains(i))
            .take(K_MAIN)
            .collect();
    
        // safety: jika karena sesuatu out < K_MAIN (seharusnya tidak), pad dengan sisa angka
        if out.len() < K_MAIN {
            for i in 0..25 {
                if out.len() >= K_MAIN { break; }
                if !out.contains(&i) { out.push(i); }
            }
        }
    
        // 6) Tambah booster unik — pilih dari yang jarang (tail of freq_indexed)
        let mut rng = thread_rng();
        let mut boosters: Vec<usize> = Vec::new();
        let low_pool: Vec<usize> = freq_indexed.iter().rev().map(|(i, _)| *i).collect();
        let mut idxs: Vec<usize> = (0..low_pool.len()).collect();
        idxs.shuffle(&mut rng);
    
        for j in idxs {
            if boosters.len() >= N_BOOSTER { break; }
            let candidate = low_pool[j];
            if !out.contains(&candidate) {
                boosters.push(candidate);
            }
        }
    
        // gabungkan main + booster (booster unik guaranteed)
        out.extend(boosters.iter());
        // optional: sort_unstable atau simpan urutan as-is (saat ini main berdasarkan freq)
        out.sort_unstable();
        out
    }

    pub fn predict_ec(&self) -> Vec<usize> {
        let mut rng = thread_rng();
    
        // get (jittered) frequency probs to use as base fitness input
        let freq = self.probabilities(); // Vec<f64> owned
    
        // GA population: Vec<Vec<usize>>
        let mut pop: Vec<Vec<usize>> = (0..GA_POP).map(|_| random_subset(&mut rng)).collect();
        // elitism count
        let elite_n = (GA_POP as f64 * 0.05).max(1.0) as usize;
    
        for _gen in 0..GA_GENS {
            // score population (consume pop -> produce scored)
            let mut scored: Vec<(f64, Vec<usize>)> = pop
                .into_iter()
                .map(|ind| {
                    let fit = fitness_of(&ind, &freq, 0.7, 0.3); // w_freq=0.7, w_cluster=0.3
                    (fit, ind)
                })
                .collect();
    
            // sort desc by fitness safely using total_cmp when comparing f64 pairs
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    
            // keep elites (clone individu terbaik)
            let mut new_pop: Vec<Vec<usize>> = scored.iter().take(elite_n).map(|(_, ind)| ind.clone()).collect();
    
            // fill rest of population
            while new_pop.len() < GA_POP {
                // tournament selection (explicit rng borrow)
                let parent1 = tournament_select(&scored, &mut rng);
                let parent2 = tournament_select(&scored, &mut rng);
    
                // crossover & mutation (pass rng mutably)
                let mut child = crossover(&parent1, &parent2, &mut rng);
                if rng.gen_bool(GA_MUT_PROB) {
                    mutate(&mut child, &mut rng);
                }
                new_pop.push(child);
            }
    
            pop = new_pop;
        }
    
        // final scoring & select best
        let freq_final = self.probabilities();
        let mut scored_final: Vec<(f64, Vec<usize>)> = pop
            .into_iter()
            .map(|ind| {
                let fit = fitness_of(&ind, &freq_final, 0.7, 0.3);
                (fit, ind)
            })
            .collect();
        scored_final.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let best = scored_final
            .first()
            .map(|(_, ind)| ind.clone())
            .unwrap_or_else(|| {
                let mut v: Vec<usize> = (0..25).collect();
                v.truncate(SUBSET_K);
                v
            });
    
        best
    }

    pub fn predict_3way(&self) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        const K: usize = 18;

        // ranking frekuensi (desc)
        let probs = self.probabilities();
        let mut freq_rank: Vec<(usize, f64)> = probs.iter().cloned().enumerate().collect();
        freq_rank.sort_by(|a, b| b.1.total_cmp(&a.1));
        let freq_list: Vec<usize> = freq_rank.iter().map(|(i, _)| *i).collect();

        // --- Alt1: top K by frequency ---
        let mut alt1: Vec<usize> = Vec::with_capacity(K);
        for &i in freq_list.iter().take(K) {
            alt1.push(i);
        }

        // --- Alt2: next by frequency excluding Alt1 ---
        let mut used1: HashSet<usize> = alt1.iter().copied().collect();
        let mut alt2: Vec<usize> = Vec::with_capacity(K);
        for &i in freq_list.iter() {
            if alt2.len() >= K { break; }
            if used1.contains(&i) { continue; }
            alt2.push(i);
        }
        // fallback (shouldn't be needed) - fill from all numbers
        if alt2.len() < K {
            for i in 0..25 {
                if alt2.len() >= K { break; }
                if !alt2.contains(&i) && !alt1.contains(&i) {
                    alt2.push(i);
                }
            }
        }

        // --- Alt3: independen dari EC ---
        // predict_ec() seharusnya mengembalikan subset (unik) — ambil top K dari EC
        let mut ec_list = self.predict_ec();
        // ensure uniqueness inside alt3
        ec_list.sort_unstable();
        ec_list.dedup();
        let mut alt3: Vec<usize> = ec_list.into_iter().take(K).collect();

        // jika EC kurang dari K, isi sisanya dari frekuensi (tanpa memperhatikan Alt1/Alt2)
        if alt3.len() < K {
            for &i in freq_list.iter() {
                if alt3.len() >= K { break; }
                if !alt3.contains(&i) {
                    alt3.push(i);
                }
            }
        }
        // final safety fill (shouldn't happen)
        if alt3.len() < K {
            for i in 0..25 {
                if alt3.len() >= K { break; }
                if !alt3.contains(&i) {
                    alt3.push(i);
                }
            }
        }

        // sort untuk keterbacaan (opsional)
        alt1.sort_unstable();
        alt2.sort_unstable();
        alt3.sort_unstable();

        (alt1, alt2, alt3)
    }

    pub fn predict_two_alt(&self) -> (Vec<usize>, Vec<usize>) {
        const K: usize = 18;
        let mut rng = thread_rng();

        // --- 1️⃣ ALT1: dari EC/Hybrid ---
        let mut alt1 = self.predict_hybrid(); // kamu bisa ganti ke predict_ec() jika mau
        alt1.sort_unstable();
        alt1.dedup();
        // pastikan panjang = 18
        if alt1.len() > K {
            alt1.truncate(K);
        } else if alt1.len() < K {
            let probs = self.probabilities();
            let mut freq_idx: Vec<(usize, f64)> = probs.iter().cloned().enumerate().collect();
            freq_idx.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (i, _) in freq_idx.iter() {
                if alt1.len() >= K { break; }
                if !alt1.contains(i) {
                    alt1.push(*i);
                }
            }
        }

        // --- 2️⃣ ALT2: semua angka yang tidak ada di ALT1 ---
        let used1: HashSet<usize> = alt1.iter().copied().collect();
        let mut alt2: Vec<usize> = (0..25).filter(|i| !used1.contains(i)).collect();

        // --- 3️⃣ Isi sisa sampai 18 dengan probabilitas frekuensi ---
        let probs = self.probabilities();
        while alt2.len() < K {
            // kandidat = semua angka 0..25 yang belum ada di alt2
            let candidates: Vec<usize> = (0..25).filter(|i| !alt2.contains(i)).collect();
            let weights: Vec<f64> = candidates.iter().map(|&i| probs[i]).collect();

            // jika semua bobot nol → uniform
            let dist = if weights.iter().all(|w| *w <= 0.0) {
                WeightedIndex::new(vec![1.0f64; candidates.len()]).unwrap()
            } else {
                WeightedIndex::new(weights).unwrap()
            };

            let idx = dist.sample(&mut rng);
            let chosen = candidates[idx];
            if !alt2.contains(&chosen) {
                alt2.push(chosen);
            }
        }

        // --- 4️⃣ Sort agar rapi (opsional) ---
        alt1.sort_unstable();
        alt2.sort_unstable();

        (alt1, alt2)
    }

    pub fn predict_with_env(&self, env: &OreEnv, alpha: f64) -> Vec<usize> {
        let probs = self.probabilities();
        let mut ev_values = vec![0.0; 25];
    
        for i in 0..25 {
            ev_values[i] = compute_ev_star_for_block(env.os[i], env.total_t, env.ore_value_in_sol).ev;
        }
    
        let min_ev = ev_values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_ev = ev_values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let norm_ev: Vec<f64> = ev_values
            .iter()
            .map(|&e| if max_ev > min_ev { (e - min_ev) / (max_ev - min_ev) } else { 0.0 })
            .collect();
    
        let mut scores: Vec<(usize, f64)> = (0..25)
            .map(|i| {
                let s = alpha * probs[i] + (1.0 - alpha) * norm_ev[i];
                (i, s)
            })
            .collect();
    
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        scores.iter().take(18).map(|(i, _)| *i).collect()
    }

    pub fn predict_two_alt_with_ev(
        &self,
        env: &OreEnv,
        alpha: f64,
        ev_threshold: f64, // threshold untuk eliminasi slot (misal 0.0)
    ) -> ((Vec<usize>, f64), (Vec<usize>, f64)) {
        let (alt1, alt2) = self.predict_two_alt();
        let probs = self.probabilities();
    
        // --- 1️⃣ Hitung EV tiap slot dari kondisi environment ---
        let mut ev_values = vec![0.0; 25];
        for i in 0..25 {
            ev_values[i] = compute_ev_star_for_block(env.os[i], env.total_t, env.ore_value_in_sol).ev;
        }
    
        // --- 2️⃣ Normalisasi EV ke skala [0, 1] agar sebanding dengan probabilitas ---
        let min_ev = ev_values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        let max_ev = ev_values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
        let norm_ev: Vec<f64> = ev_values
            .iter()
            .map(|&e| {
                if max_ev > min_ev {
                    (e - min_ev) / (max_ev - min_ev)
                } else {
                    0.0
                }
            })
            .collect();
    
        // --- 3️⃣ Fungsi bantu: hitung EV hybrid + filter ---
        let calc_ev_and_filter = |subset: &Vec<usize>| -> (Vec<usize>, f64) {
            // 3a. Hitung hybrid score per-slot
            let mut slot_scores: Vec<(usize, f64)> = subset
                .iter()
                .map(|&i| {
                    let hybrid = alpha * probs[i] + (1.0 - alpha) * norm_ev[i];
                    (i, hybrid)
                })
                .collect();
    
            // 3b. Filter slot yang EV-nya di bawah threshold
            slot_scores.retain(|(i, _)| ev_values[*i] > ev_threshold);
    
            // 3c. Urutkan descending (opsional)
            slot_scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    
            // 3d. Ambil indeks final
            let filtered: Vec<usize> = slot_scores.iter().map(|(i, _)| *i).collect();
    
            // 3e. Hitung rata-rata EV hybrid subset yang tersisa
            let avg_ev = if filtered.is_empty() {
                0.0
            } else {
                filtered.iter().map(|&i| slot_scores.iter().find(|(idx, _)| *idx == i).unwrap().1).sum::<f64>() / (filtered.len() as f64)
            };
    
            (filtered, avg_ev)
        };
    
        // --- 4️⃣ Hitung hasil final untuk alt1 & alt2 ---
        let (filtered1, ev1) = calc_ev_and_filter(&alt1);
        let (filtered2, ev2) = calc_ev_and_filter(&alt2);
    
        ((filtered1, ev1), (filtered2, ev2))
    }

}